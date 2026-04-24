use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Detected OData protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ODataVersion {
    V2,
    V4,
}

/// Parsed OData service metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ServiceMetadata {
    pub version: ODataVersion,
    pub schema_namespace: String,
    pub entity_types: Vec<EntityType>,
    pub associations: Vec<Association>,
    pub entity_sets: Vec<EntitySet>,
    pub function_imports: Vec<FunctionImport>,
    /// Raw annotations captured from `$metadata`. Flat list — each entry
    /// points at the thing it was attached to via `target`. V2 SAP inline
    /// attributes (`sap:*`) and V4 `<Annotations>` blocks both land here.
    /// Complex record/collection values are not expanded yet; `value` is
    /// populated only for the common simple-valued forms
    /// (`String=`, `Bool=`, `Int=`, `EnumMember=`).
    pub annotations: Vec<RawAnnotation>,
}

/// A single annotation captured during `$metadata` parsing. The first slice
/// of the annotation feature only needs counts + grouping; richer typed
/// accessors will layer on top of this without re-parsing.
#[derive(Debug, Clone, Serialize)]
pub struct RawAnnotation {
    /// Fully-qualified term name: `Common.Label`, `UI.LineItem`, or `sap:label`
    /// for V2 SAP inline attributes.
    pub term: String,
    /// Display-friendly vocabulary name derived from the term: `Common`, `UI`,
    /// `Capabilities`, `Measures`, `SAP`, ...
    pub namespace: String,
    /// What the annotation is attached to. For V4: the `Target` on the
    /// enclosing `<Annotations>` block with the schema alias stripped
    /// (e.g. `EntityType/PropertyName`). For V2: a synthetic path built
    /// from the containing element (`EntityType/PropertyName`,
    /// `EntityContainer/EntitySetName`, etc.).
    pub target: String,
    /// Literal value when the annotation uses a simple inline form. `None`
    /// for complex `Record`/`Collection` payloads, which are not expanded yet.
    pub value: Option<String>,
    /// Optional `Qualifier` — the same term can be annotated multiple times
    /// on the same target with different qualifiers (e.g. `UI.LineItem
    /// Qualifier="Simplified"`).
    pub qualifier: Option<String>,
}

/// Summary of annotation counts grouped by vocabulary namespace. Used by the
/// desktop app's footer badge and its hover summary.
#[derive(Debug, Clone, Serialize, Default)]
pub struct AnnotationSummary {
    pub total: usize,
    /// Namespace → count. BTreeMap keeps display order deterministic.
    pub by_namespace: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityType {
    pub name: String,
    pub keys: Vec<String>,
    pub properties: Vec<Property>,
    pub nav_properties: Vec<NavigationProperty>,
    /// Parsed `UI.HeaderInfo` annotation if the service exposes one (V4).
    /// Gives the entity's human-readable name and title field.
    pub header_info: Option<HeaderInfo>,
    /// Property names flagged as default filter fields by
    /// `UI.SelectionFields` — the columns a Fiori list report would
    /// expose as selection filters out of the box. Empty if the service
    /// doesn't specify any. Paths are stored verbatim (no alias resolution).
    pub selection_fields: Vec<String>,
    /// Columns a Fiori list report would show by default, from
    /// `UI.LineItem`. Only `UI.DataField` records with a `Value Path="..."`
    /// are surfaced here; `DataFieldFor*` variants (Action, Annotation,
    /// IntentBasedNavigation, ...) are ignored because they don't map to
    /// `$select`-able columns. Order matches the source collection.
    pub line_item: Vec<LineItemField>,
}

/// One `UI.DataField` inside a `UI.LineItem` collection — enough to
/// populate a Fiori-style default `$select` and label the column.
#[derive(Debug, Clone, Serialize)]
pub struct LineItemField {
    /// The `Value Path="..."` — the property path the column displays.
    pub value_path: String,
    /// Optional static `Label String="..."` override. When absent the
    /// frontend should fall back to the referenced property's own label.
    pub label: Option<String>,
    /// Optional `Importance` — one of "High", "Medium", "Low" (enum
    /// member on `UI.ImportanceType`). Stored as-is for the UI to style.
    pub importance: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Property {
    pub name: String,
    pub edm_type: String,
    pub nullable: bool,
    pub max_length: Option<u32>,
    pub label: Option<String>,
    /// Path of a sibling property that holds this property's
    /// human-readable description. Source: `Common.Text` (V4) or
    /// `sap:text` (V2). Example: `MaterialID.text_path = Some("MaterialDescription")`.
    pub text_path: Option<String>,
    /// Path of a sibling property that holds the unit of measure for this
    /// amount/quantity. Source: `Measures.Unit` (V4) or `sap:unit` (V2).
    /// Example: `NetWeight.unit_path = Some("WeightUnit")`.
    pub unit_path: Option<String>,
    /// Path of a sibling property that holds the ISO currency code for
    /// this monetary amount. Source: `Measures.ISOCurrency` (V4).
    /// Example: `NetAmount.iso_currency_path = Some("TransactionCurrency")`.
    pub iso_currency_path: Option<String>,
    /// Property-level flags. Each is `Some(bool)` only when the source
    /// metadata explicitly sets the flag; `None` means "use the OData
    /// default" (generally true for filterable/sortable/creatable/updatable).
    /// Sources: V2 `sap:filterable` / `sap:sortable` / `sap:creatable` /
    /// `sap:updatable` / `sap:required-in-filter`, plus V4
    /// `Capabilities.FilterRestrictions` / `SortRestrictions` /
    /// `InsertRestrictions` / `UpdateRestrictions` applied at the entity-set
    /// level. V4 restriction lists override the default-true interpretation:
    /// listed properties flip to `Some(false)` (or `Some(true)` for
    /// `RequiredProperties`).
    pub filterable: Option<bool>,
    pub sortable: Option<bool>,
    pub creatable: Option<bool>,
    pub updatable: Option<bool>,
    pub required_in_filter: Option<bool>,
    /// Parsed `UI.Criticality` annotation — either a fixed level
    /// (0 = Neutral, 1 = Negative, 2 = Critical, 3 = Positive,
    /// 5 = Information per OData spec) or a path to a sibling property
    /// whose runtime value supplies the level.
    pub criticality: Option<Criticality>,
    /// Parsed `Common.ValueList` annotation — describes a value help
    /// (F4) for this property: which entity set to pull values from,
    /// whether it supports `$search`, and how its properties map back
    /// to this (and sibling) properties on pick. `None` when the
    /// service doesn't declare a value list. Qualifier variants are
    /// not differentiated yet — the first parse wins.
    pub value_list: Option<ValueList>,
    /// `Common.ValueListReferences` URLs captured verbatim. Each entry
    /// points at a separate Fiori value-help service whose `$metadata`
    /// contains the real `Common.ValueList` mapping. Empty when the
    /// property uses inline `ValueList` (see `value_list`) or no value
    /// help at all. Relative URLs are left unresolved — consumers
    /// resolve them against the current service's base URL.
    pub value_list_references: Vec<String>,
    /// `true` when `Common.ValueListWithFixedValues` is present on this
    /// property. Signals "the set of valid values is small and stable"
    /// (Fiori typically renders this as a dropdown). The annotation
    /// carries no mapping of its own, so it's a hint for the UI only.
    pub value_list_fixed: bool,
}

/// `UI.Criticality` value. Absent means "no criticality annotation",
/// not "neutral".
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum Criticality {
    Fixed(u8),
    Path(String),
}

/// Parsed `Common.ValueList` record — the F4/value-help declaration
/// for a property. Enough to drive a picker: where to fetch values
/// from, whether `$search` is available, and how picked rows map
/// back to the local entity's properties.
#[derive(Debug, Clone, Serialize)]
pub struct ValueList {
    /// Name of the entity set that serves as the value-help source.
    /// Path is relative to the service root — callers append it to
    /// the service URL directly.
    pub collection_path: String,
    /// Optional human-readable label for the value help (e.g.
    /// "Warehouse Value Help"). Falls back to `None` if unset.
    pub label: Option<String>,
    /// Whether the value-help collection accepts `$search`.
    /// `None` means the service didn't say; callers can choose to
    /// offer a search box speculatively.
    pub search_supported: Option<bool>,
    /// Parameter mapping between the annotated property's entity and
    /// the value-help entity. In source order.
    pub parameters: Vec<ValueListParameter>,
}

/// One entry inside `Common.ValueList.Parameters`. The kind determines
/// how the picker uses the mapping on open and on pick; `local_property`
/// is populated for In/Out/InOut (it's the column on the local entity),
/// and `constant` is populated only for `Constant`.
#[derive(Debug, Clone, Serialize)]
pub struct ValueListParameter {
    pub kind: ValueListParameterKind,
    /// Property name on the annotated entity that this parameter
    /// reads from or writes to. `None` for `DisplayOnly` (no local
    /// binding) and `Constant` (constant feeds the VL directly).
    pub local_property: Option<String>,
    /// Property name on the value-list entity.
    pub value_list_property: String,
    /// For `Constant` kind only: the literal value sent as a filter
    /// against `value_list_property`.
    pub constant: Option<String>,
}

/// The five standard `Common.ValueListParameter*` variants. Unknown
/// subtypes fall through parsing — we don't want to invent a kind we
/// can't faithfully drive.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueListParameterKind {
    In,
    Out,
    InOut,
    DisplayOnly,
    Constant,
}

/// Parsed `UI.HeaderInfo` record. All fields are optional — services
/// often populate only a subset.
#[derive(Debug, Clone, Serialize, Default)]
pub struct HeaderInfo {
    /// Singular human-readable entity name (e.g. "Warehouse Order").
    pub type_name: Option<String>,
    /// Plural form (e.g. "Warehouse Orders").
    pub type_name_plural: Option<String>,
    /// Path to the property used as the object's title at runtime
    /// (e.g. the `OrderNumber` on a `WarehouseOrder`).
    pub title_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NavigationProperty {
    pub name: String,
    /// V4: the target entity type (e.g., "Namespace.TypeName" or "Collection(...)").
    /// V2: empty — use `relationship` instead.
    pub target_type: String,
    /// V4: the partner nav property on the target side. V2: empty.
    pub partner: String,
    /// V2: the association name. V4: empty.
    pub relationship: String,
    /// V2: FromRole. V4: empty.
    pub from_role: String,
    /// V2: ToRole. V4: empty.
    pub to_role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Association {
    pub name: String,
    pub ends: Vec<AssociationEnd>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssociationEnd {
    pub entity_type: String,
    pub multiplicity: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EntitySet {
    pub name: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionImport {
    pub name: String,
    pub http_method: String,
    pub return_type: Option<String>,
    pub parameters: Vec<FunctionParameter>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionParameter {
    pub name: String,
    pub edm_type: String,
    pub mode: String,
}

impl ServiceMetadata {
    /// Find an entity type by name.
    pub fn find_entity_type(&self, name: &str) -> Option<&EntityType> {
        self.entity_types.iter().find(|et| et.name == name)
    }

    /// Find an entity set by name.
    pub fn find_entity_set(&self, name: &str) -> Option<&EntitySet> {
        self.entity_sets.iter().find(|es| es.name == name)
    }

    /// Get the entity type for a given entity set name.
    pub fn entity_type_for_set(&self, set_name: &str) -> Option<&EntityType> {
        let es = self.find_entity_set(set_name)?;
        let type_name = extract_type_name(&es.entity_type);
        self.find_entity_type(type_name)
    }

    /// Get navigation targets: (nav property name, target entity type name, multiplicity).
    pub fn nav_targets(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        match self.version {
            ODataVersion::V4 => self.nav_targets_v4(entity_type),
            ODataVersion::V2 => self.nav_targets_v2(entity_type),
        }
    }

    fn nav_targets_v2(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        entity_type
            .nav_properties
            .iter()
            .filter_map(|nav| {
                let assoc_name = extract_type_name(&nav.relationship);
                let assoc = self.associations.iter().find(|a| a.name == assoc_name)?;
                let target_end = assoc.ends.iter().find(|e| e.role == nav.to_role)?;
                let target_type = extract_type_name(&target_end.entity_type);
                Some((
                    nav.name.clone(),
                    target_type.to_string(),
                    target_end.multiplicity.clone(),
                ))
            })
            .collect()
    }

    fn nav_targets_v4(&self, entity_type: &EntityType) -> Vec<(String, String, String)> {
        entity_type
            .nav_properties
            .iter()
            .map(|nav| {
                let (type_str, multiplicity) = parse_v4_nav_type(&nav.target_type);
                let target_type = extract_type_name(type_str);
                (nav.name.clone(), target_type.to_string(), multiplicity)
            })
            .collect()
    }

    /// Count annotations grouped by vocabulary namespace. Cheap to call —
    /// it's an O(N) pass over the already-parsed `annotations` field.
    pub fn annotation_summary(&self) -> AnnotationSummary {
        let mut by_namespace: BTreeMap<String, usize> = BTreeMap::new();
        for ann in &self.annotations {
            *by_namespace.entry(ann.namespace.clone()).or_insert(0) += 1;
        }
        AnnotationSummary {
            total: self.annotations.len(),
            by_namespace,
        }
    }
}

/// Extract the simple type name from a qualified name (e.g., "Namespace.TypeName" → "TypeName").
fn extract_type_name(qualified: &str) -> &str {
    qualified.rsplit('.').next().unwrap_or(qualified)
}

/// Parse V4 nav property Type: "Collection(Ns.Type)" → ("Ns.Type", "*") or "Ns.Type" → ("Ns.Type", "1").
fn parse_v4_nav_type(type_str: &str) -> (&str, String) {
    if let Some(inner) = type_str
        .strip_prefix("Collection(")
        .and_then(|s| s.strip_suffix(')'))
    {
        (inner, "*".to_string())
    } else {
        (type_str, "1".to_string())
    }
}

// ── Parsing ──

/// Parse OData $metadata XML into a ServiceMetadata struct. Supports both V2 and V4.
/// Handles multi-schema documents by merging all Schema elements.
pub fn parse_metadata(xml: &str) -> Result<ServiceMetadata, crate::error::ODataError> {
    let doc = roxmltree::Document::parse(xml)
        .map_err(|e| crate::error::ODataError::MetadataParse(e.to_string()))?;

    let version = detect_version(&doc);

    // Collect all Schema elements
    let schema_nodes: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Schema"))
        .collect();

    if schema_nodes.is_empty() {
        return Err(crate::error::ODataError::MetadataParse(
            "no Schema element found".into(),
        ));
    }

    // Use the first schema's namespace as the primary
    let schema_namespace = schema_nodes[0]
        .attribute("Namespace")
        .unwrap_or("")
        .to_string();

    let mut entity_types = Vec::new();
    let mut associations = Vec::new();
    let mut entity_sets = Vec::new();
    let mut function_imports = Vec::new();
    let mut annotation_labels = HashMap::new();
    let mut annotations = Vec::new();

    // Merge data from all schemas
    for schema_node in &schema_nodes {
        entity_types.extend(parse_entity_types(schema_node, version));
        associations.extend(parse_associations(schema_node));

        let (sets, funcs) = parse_entity_container(schema_node, version);
        entity_sets.extend(sets);
        function_imports.extend(funcs);

        match version {
            ODataVersion::V4 => {
                annotation_labels.extend(parse_v4_annotation_labels(schema_node));
                let alias = schema_node.attribute("Alias").unwrap_or("");
                annotations.extend(parse_v4_annotations(schema_node, alias));
                apply_v4_typed_annotations(&mut entity_types, &entity_sets, schema_node, alias);
            }
            ODataVersion::V2 => {
                annotations.extend(parse_v2_sap_annotations(schema_node));
            }
        }
    }

    // Apply V4 annotation labels to properties
    if version == ODataVersion::V4 {
        apply_annotation_labels(&mut entity_types, &annotation_labels, &schema_namespace);
    }

    Ok(ServiceMetadata {
        version,
        schema_namespace,
        entity_types,
        associations,
        entity_sets,
        function_imports,
        annotations,
    })
}

/// Detect OData version from the EDMX root element.
fn detect_version(doc: &roxmltree::Document) -> ODataVersion {
    for node in doc.descendants() {
        if node.has_tag_name("Edmx") {
            if let Some(ver) = node.attribute("Version") {
                if ver.starts_with("4") {
                    return ODataVersion::V4;
                }
            }
            // Also check namespace on the tag
            if let Some(ns) = node.tag_name().namespace() {
                if ns.contains("oasis-open.org") {
                    return ODataVersion::V4;
                }
            }
        }
    }
    ODataVersion::V2
}

fn children_by_tag<'a>(
    parent: &roxmltree::Node<'a, 'a>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'a>> {
    parent
        .children()
        .filter(|n| n.is_element() && n.has_tag_name(local_name))
        .collect()
}

fn parse_entity_types(schema: &roxmltree::Node, version: ODataVersion) -> Vec<EntityType> {
    children_by_tag(schema, "EntityType")
        .into_iter()
        .map(|node| {
            let name = node.attribute("Name").unwrap_or("").to_string();

            let keys: Vec<String> = children_by_tag(&node, "Key")
                .into_iter()
                .flat_map(|key_node| children_by_tag(&key_node, "PropertyRef"))
                .filter_map(|pr| pr.attribute("Name").map(String::from))
                .collect();

            let properties = children_by_tag(&node, "Property")
                .into_iter()
                .map(|p| Property {
                    name: p.attribute("Name").unwrap_or("").to_string(),
                    edm_type: p.attribute("Type").unwrap_or("").to_string(),
                    nullable: p.attribute("Nullable").unwrap_or("true") != "false",
                    max_length: p.attribute("MaxLength").and_then(|v| v.parse().ok()),
                    label: p
                        .attribute(("http://www.sap.com/Protocols/SAPData", "label"))
                        .map(String::from),
                    // V2 `sap:text` — sibling property holding this property's
                    // description. V4 services get this set later when we
                    // collect the Common.Text annotations.
                    text_path: p
                        .attribute(("http://www.sap.com/Protocols/SAPData", "text"))
                        .map(String::from),
                    // V2 `sap:unit` — can carry either a unit-of-measure or a
                    // currency reference; V2 doesn't distinguish. Land it in
                    // unit_path; V4 services differentiate via separate
                    // Measures.Unit vs Measures.ISOCurrency annotations.
                    unit_path: p
                        .attribute(("http://www.sap.com/Protocols/SAPData", "unit"))
                        .map(String::from),
                    iso_currency_path: None,
                    filterable: parse_sap_bool(&p, "filterable"),
                    sortable: parse_sap_bool(&p, "sortable"),
                    creatable: parse_sap_bool(&p, "creatable"),
                    updatable: parse_sap_bool(&p, "updatable"),
                    required_in_filter: parse_sap_bool(&p, "required-in-filter"),
                    // Populated later by the V4 annotation pass where applicable.
                    criticality: None,
                    value_list: None,
                    value_list_references: Vec::new(),
                    value_list_fixed: false,
                })
                .collect();

            let nav_properties = children_by_tag(&node, "NavigationProperty")
                .into_iter()
                .map(|np| match version {
                    ODataVersion::V4 => NavigationProperty {
                        name: np.attribute("Name").unwrap_or("").to_string(),
                        target_type: np.attribute("Type").unwrap_or("").to_string(),
                        partner: np.attribute("Partner").unwrap_or("").to_string(),
                        relationship: String::new(),
                        from_role: String::new(),
                        to_role: String::new(),
                    },
                    ODataVersion::V2 => NavigationProperty {
                        name: np.attribute("Name").unwrap_or("").to_string(),
                        target_type: String::new(),
                        partner: String::new(),
                        relationship: np.attribute("Relationship").unwrap_or("").to_string(),
                        from_role: np.attribute("FromRole").unwrap_or("").to_string(),
                        to_role: np.attribute("ToRole").unwrap_or("").to_string(),
                    },
                })
                .collect();

            EntityType {
                name,
                keys,
                properties,
                nav_properties,
                header_info: None,
                selection_fields: Vec::new(),
                line_item: Vec::new(),
            }
        })
        .collect()
}

fn parse_associations(schema: &roxmltree::Node) -> Vec<Association> {
    children_by_tag(schema, "Association")
        .into_iter()
        .map(|node| {
            let name = node.attribute("Name").unwrap_or("").to_string();
            let ends = children_by_tag(&node, "End")
                .into_iter()
                .map(|e| AssociationEnd {
                    entity_type: e.attribute("Type").unwrap_or("").to_string(),
                    multiplicity: e.attribute("Multiplicity").unwrap_or("").to_string(),
                    role: e.attribute("Role").unwrap_or("").to_string(),
                })
                .collect();
            Association { name, ends }
        })
        .collect()
}

fn parse_entity_container(
    schema: &roxmltree::Node,
    version: ODataVersion,
) -> (Vec<EntitySet>, Vec<FunctionImport>) {
    let container = match children_by_tag(schema, "EntityContainer")
        .into_iter()
        .next()
    {
        Some(c) => c,
        None => return (vec![], vec![]),
    };

    let entity_sets = children_by_tag(&container, "EntitySet")
        .into_iter()
        .map(|es| EntitySet {
            name: es.attribute("Name").unwrap_or("").to_string(),
            entity_type: es.attribute("EntityType").unwrap_or("").to_string(),
        })
        .collect();

    // V2: FunctionImport, V4: ActionImport / FunctionImport
    let mut function_imports: Vec<FunctionImport> = children_by_tag(&container, "FunctionImport")
        .into_iter()
        .map(|fi| parse_function_import(&fi, version))
        .collect();

    // V4 also has ActionImport
    if version == ODataVersion::V4 {
        let action_imports: Vec<FunctionImport> = children_by_tag(&container, "ActionImport")
            .into_iter()
            .map(|ai| FunctionImport {
                name: ai.attribute("Name").unwrap_or("").to_string(),
                http_method: "POST".to_string(),
                return_type: ai.attribute("Action").map(String::from),
                parameters: vec![],
            })
            .collect();
        function_imports.extend(action_imports);
    }

    (entity_sets, function_imports)
}

fn parse_function_import(fi: &roxmltree::Node, version: ODataVersion) -> FunctionImport {
    let parameters = children_by_tag(fi, "Parameter")
        .into_iter()
        .map(|p| FunctionParameter {
            name: p.attribute("Name").unwrap_or("").to_string(),
            edm_type: p.attribute("Type").unwrap_or("").to_string(),
            mode: p.attribute("Mode").unwrap_or("In").to_string(),
        })
        .collect();

    let http_method = match version {
        ODataVersion::V2 => fi
            .attribute((
                "http://schemas.microsoft.com/ado/2007/08/dataservices/metadata",
                "HttpMethod",
            ))
            .unwrap_or("GET")
            .to_string(),
        ODataVersion::V4 => "GET".to_string(),
    };

    FunctionImport {
        name: fi.attribute("Name").unwrap_or("").to_string(),
        http_method,
        return_type: fi.attribute("ReturnType").map(String::from),
        parameters,
    }
}

// ── Annotation collection (thin slice: raw terms only, no typed views yet) ──

const SAP_DATA_NS: &str = "http://www.sap.com/Protocols/SAPData";

/// Collect V4 annotations from `<Annotations Target="...">` blocks at the
/// schema level. Inline `<Annotation>` children directly under `<Property>`
/// or `<EntityType>` aren't handled in this first slice — SAP services
/// typically put everything under explicit `<Annotations>` blocks.
fn parse_v4_annotations(schema: &roxmltree::Node, alias: &str) -> Vec<RawAnnotation> {
    let mut out = Vec::new();
    for annots_node in children_by_tag(schema, "Annotations") {
        let raw_target = annots_node.attribute("Target").unwrap_or("");
        let target = strip_alias_prefix(raw_target, alias).to_string();
        for annot in children_by_tag(&annots_node, "Annotation") {
            let term = match annot.attribute("Term") {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => continue,
            };
            let namespace = extract_annotation_namespace(&term);
            let value = annot
                .attribute("String")
                .or_else(|| annot.attribute("Bool"))
                .or_else(|| annot.attribute("Int"))
                .or_else(|| annot.attribute("Decimal"))
                .or_else(|| annot.attribute("EnumMember"))
                .or_else(|| annot.attribute("Path"))
                .map(String::from);
            let qualifier = annot.attribute("Qualifier").map(String::from);
            out.push(RawAnnotation {
                term,
                namespace,
                target: target.clone(),
                value,
                qualifier,
            });
        }
    }
    out
}

/// Collect V2 SAP inline annotations — attributes in the SAP data-service
/// namespace (`sap:label`, `sap:filterable`, `sap:creatable`, etc.) on
/// entity types, properties, navigation properties, entity container,
/// entity sets, and function imports.
fn parse_v2_sap_annotations(schema: &roxmltree::Node) -> Vec<RawAnnotation> {
    let mut out = Vec::new();

    for et in children_by_tag(schema, "EntityType") {
        let et_name = et.attribute("Name").unwrap_or("").to_string();
        push_sap_attrs(&mut out, &et, &et_name);
        for prop in children_by_tag(&et, "Property") {
            let name = prop.attribute("Name").unwrap_or("");
            push_sap_attrs(&mut out, &prop, &format!("{et_name}/{name}"));
        }
        for np in children_by_tag(&et, "NavigationProperty") {
            let name = np.attribute("Name").unwrap_or("");
            push_sap_attrs(&mut out, &np, &format!("{et_name}/{name}"));
        }
    }

    for container in children_by_tag(schema, "EntityContainer") {
        let cn = container.attribute("Name").unwrap_or("").to_string();
        push_sap_attrs(&mut out, &container, &cn);
        for es in children_by_tag(&container, "EntitySet") {
            let n = es.attribute("Name").unwrap_or("");
            push_sap_attrs(&mut out, &es, &format!("{cn}/{n}"));
        }
        for fi in children_by_tag(&container, "FunctionImport") {
            let n = fi.attribute("Name").unwrap_or("");
            push_sap_attrs(&mut out, &fi, &format!("{cn}/{n}"));
        }
    }

    out
}

/// Second pass over `<Annotations Target>` blocks that populates typed
/// accessors on the parsed entity types. Handles `UI.HeaderInfo`,
/// `Common.Text`, `UI.Criticality`, `Measures.Unit`, `Measures.ISOCurrency`
/// (all targeted at entity types / properties), and the entity-set–targeted
/// `Capabilities.FilterRestrictions` / `SortRestrictions` /
/// `InsertRestrictions` / `UpdateRestrictions` property lists.
/// Each additional vocabulary term gets its own branch here, keeping
/// complex Record/Collection parsing in one place instead of spilling
/// into callers.
fn apply_v4_typed_annotations(
    entity_types: &mut [EntityType],
    entity_sets: &[EntitySet],
    schema: &roxmltree::Node,
    alias: &str,
) {
    for annots_node in children_by_tag(schema, "Annotations") {
        let raw_target = annots_node.attribute("Target").unwrap_or("");
        let target = strip_alias_prefix(raw_target, alias);

        for annot in children_by_tag(&annots_node, "Annotation") {
            let term = annot.attribute("Term").unwrap_or("");
            // Normalize: strip the SAP__ alias prefix and lowercase so the
            // match works for `UI.HeaderInfo`, `SAP__UI.HeaderInfo`,
            // `com.sap.vocabularies.UI.v1.HeaderInfo`, etc.
            let lower = term.trim_start_matches("SAP__").to_ascii_lowercase();

            if lower.ends_with(".headerinfo") && !target.contains('/') {
                if let Some(et) = entity_types.iter_mut().find(|e| e.name == target) {
                    if let Some(info) = parse_header_info_record(&annot) {
                        et.header_info = Some(info);
                    }
                }
            } else if lower.ends_with(".text") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if prop.text_path.is_none() {
                                prop.text_path = annot.attribute("Path").map(String::from);
                            }
                        }
                    }
                }
            } else if lower.ends_with(".criticality") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if let Some(c) = parse_criticality(&annot) {
                                prop.criticality = Some(c);
                            }
                        }
                    }
                }
            } else if lower.ends_with(".unit") && !lower.ends_with(".isunit") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if prop.unit_path.is_none() {
                                prop.unit_path = annot.attribute("Path").map(String::from);
                            }
                        }
                    }
                }
            } else if lower.ends_with(".isocurrency") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            prop.iso_currency_path = annot.attribute("Path").map(String::from);
                        }
                    }
                }
            } else if lower.ends_with(".filterrestrictions")
                || lower.ends_with(".sortrestrictions")
                || lower.ends_with(".insertrestrictions")
                || lower.ends_with(".updaterestrictions")
            {
                // Entity-set–scoped capability restriction.
                // Target looks like "Container/EntitySetName".
                if let Some((_, set_name)) = target.split_once('/') {
                    if let Some(type_ref) =
                        entity_sets.iter().find(|s| s.name == set_name)
                    {
                        let type_name = extract_type_name(&type_ref.entity_type).to_string();
                        if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                            apply_capability_restriction(&lower, &annot, &mut et.properties);
                        }
                    }
                }
            } else if lower.ends_with(".valuelist")
                && !lower.ends_with(".valuelistreferences")
                && !lower.ends_with(".valuelistmapping")
                && !lower.ends_with(".valuelistwithfixedvalues")
            {
                // Common.ValueList targets a specific property. We take
                // the first (unqualified) match; qualified variants
                // (e.g. multiple value helps for the same field) are
                // left for a later pass once the picker exposes them.
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if prop.value_list.is_none() {
                                if let Some(vl) = parse_value_list_record(&annot) {
                                    prop.value_list = Some(vl);
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".valuelistreferences") {
                // Common.ValueListReferences carries a Collection of
                // String URLs that point at separate value-help services.
                // Each URL resolves (relative to the current service) to
                // an F4 service whose `$metadata` contains the real
                // `Common.ValueList` mapping. Capture all URLs so the
                // frontend can try multiple references if needed.
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            for coll in children_by_tag(&annot, "Collection") {
                                for s in children_by_tag(&coll, "String") {
                                    if let Some(text) = s.text() {
                                        let trimmed = text.trim();
                                        if !trimmed.is_empty()
                                            && !prop.value_list_references.iter().any(|u| u == trimmed)
                                        {
                                            prop.value_list_references.push(trimmed.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".valuelistwithfixedvalues") {
                // Marker-only annotation — flips a boolean on the
                // property. The term is almost always written as an
                // empty `<Annotation .../>`; we don't inspect any
                // attributes.
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            prop.value_list_fixed = true;
                        }
                    }
                }
            } else if lower.ends_with(".lineitem") {
                // UI.LineItem = Collection(UI.DataField). Mirrors
                // SelectionFields' target-resolution pattern: either
                // the entity set (Container/SetName) or the type name.
                let fields = parse_line_item_collection(&annot);
                if fields.is_empty() {
                    continue;
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name {
                    if let Some(et) = entity_types.iter_mut().find(|t| t.name == name) {
                        // Only the default (no qualifier) wins, so we
                        // don't let a "Simplified" variant overwrite.
                        if et.line_item.is_empty() {
                            et.line_item = fields;
                        }
                    }
                }
            } else if lower.ends_with(".selectionfields") {
                // UI.SelectionFields can target either the entity set
                // ("Container/SetName") or the entity type directly
                // ("TypeName"). SAP-generated services put it on the type;
                // hand-written ones often put it on the set. Handle both.
                let paths = parse_property_path_collection(&annot);
                if paths.is_empty() {
                    continue;
                }
                let type_name = if let Some((_, set_name)) = target.split_once('/') {
                    entity_sets
                        .iter()
                        .find(|s| s.name == set_name)
                        .map(|s| extract_type_name(&s.entity_type).to_string())
                } else {
                    Some(target.to_string())
                };
                if let Some(name) = type_name {
                    if let Some(et) = entity_types.iter_mut().find(|t| t.name == name) {
                        et.selection_fields = paths;
                    }
                }
            }
        }
    }
}

/// Apply a single `Capabilities.*Restrictions` record to an entity type's
/// property list. Pulls out the collection-of-PropertyPath payloads for
/// the five lists we care about and flips the relevant `Option<bool>`
/// on each listed property. Unknown fields are ignored.
fn apply_capability_restriction(
    lower_term: &str,
    annot: &roxmltree::Node,
    properties: &mut [Property],
) {
    let record = match children_by_tag(annot, "Record").into_iter().next() {
        Some(r) => r,
        None => return,
    };
    for pv in children_by_tag(&record, "PropertyValue") {
        let prop_name = match pv.attribute("Property") {
            Some(n) => n,
            None => continue,
        };
        let paths = parse_property_path_collection(&pv);
        let (field_name, value) = match (lower_term, prop_name) {
            (t, "NonFilterableProperties") if t.ends_with(".filterrestrictions") => {
                ("filterable", Some(false))
            }
            (t, "RequiredProperties") if t.ends_with(".filterrestrictions") => {
                ("required_in_filter", Some(true))
            }
            (t, "NonSortableProperties") if t.ends_with(".sortrestrictions") => {
                ("sortable", Some(false))
            }
            (t, "NonInsertableProperties") if t.ends_with(".insertrestrictions") => {
                ("creatable", Some(false))
            }
            (t, "NonUpdatableProperties") if t.ends_with(".updaterestrictions") => {
                ("updatable", Some(false))
            }
            _ => continue,
        };
        for path in paths {
            if let Some(prop) = properties.iter_mut().find(|p| p.name == path) {
                match field_name {
                    "filterable" => prop.filterable = value,
                    "sortable" => prop.sortable = value,
                    "creatable" => prop.creatable = value,
                    "updatable" => prop.updatable = value,
                    "required_in_filter" => prop.required_in_filter = value,
                    _ => {}
                }
            }
        }
    }
}

/// Parse the `Record` inside a `Common.ValueList` annotation. Returns
/// `None` if the record is missing, has no `CollectionPath`, or has an
/// empty `Parameters` collection — any of those make the value help
/// unusable for the picker. Skips parameter records whose Type we
/// don't recognize rather than inventing a mapping we can't drive.
fn parse_value_list_record(annot: &roxmltree::Node) -> Option<ValueList> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut collection_path: Option<String> = None;
    let mut label: Option<String> = None;
    let mut search_supported: Option<bool> = None;
    let mut parameters: Vec<ValueListParameter> = Vec::new();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("CollectionPath") => {
                collection_path = pv.attribute("String").map(String::from);
            }
            Some("Label") => {
                label = pv.attribute("String").map(String::from);
            }
            Some("SearchSupported") => {
                search_supported = pv.attribute("Bool").map(|v| v == "true");
            }
            Some("Parameters") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for param_rec in children_by_tag(&coll, "Record") {
                        if let Some(p) = parse_value_list_parameter(&param_rec) {
                            parameters.push(p);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let collection_path = collection_path?;
    if parameters.is_empty() {
        return None;
    }
    Some(ValueList { collection_path, label, search_supported, parameters })
}

/// Parse one `<Record>` inside `Common.ValueList.Parameters`. Returns
/// `None` for unrecognized record types — skipping beats inventing.
fn parse_value_list_parameter(record: &roxmltree::Node) -> Option<ValueListParameter> {
    let type_attr = record.attribute("Type")?;
    // Accept SAP__-aliased, fully qualified, or bare names. The leaf is
    // the segment after the last dot.
    let normalized = type_attr.trim_start_matches("SAP__");
    let leaf = normalized.rsplit('.').next().unwrap_or(normalized);
    let kind = match leaf {
        "ValueListParameterIn" => ValueListParameterKind::In,
        "ValueListParameterOut" => ValueListParameterKind::Out,
        "ValueListParameterInOut" => ValueListParameterKind::InOut,
        "ValueListParameterDisplayOnly" => ValueListParameterKind::DisplayOnly,
        "ValueListParameterConstant" => ValueListParameterKind::Constant,
        _ => return None,
    };
    let mut local_property: Option<String> = None;
    let mut value_list_property: Option<String> = None;
    let mut constant: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("LocalDataProperty") => {
                local_property = pv.attribute("PropertyPath").map(String::from);
            }
            Some("ValueListProperty") => {
                value_list_property = pv.attribute("String").map(String::from);
            }
            Some("Constant") => {
                // Spec allows several primitive-typed attrs; SAP most
                // often emits String=. Fall through to Int/Bool for the
                // rarer numeric/boolean constants.
                constant = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    let value_list_property = value_list_property?;
    Some(ValueListParameter { kind, local_property, value_list_property, constant })
}

/// Walk a `UI.LineItem` annotation's `<Collection>` of `<Record>`s and
/// return one `LineItemField` per `UI.DataField` record that has a
/// `Value Path="..."`. Records whose `Type` is `UI.DataFieldFor*`
/// (Action, Annotation, IntentBasedNavigation, ...) are skipped — they
/// don't map to `$select`-able columns.
fn parse_line_item_collection(annot: &roxmltree::Node) -> Vec<LineItemField> {
    let mut out = Vec::new();
    let collection = match children_by_tag(annot, "Collection").into_iter().next() {
        Some(c) => c,
        None => return out,
    };
    for record in children_by_tag(&collection, "Record") {
        // Record Type defaults to "UI.DataField" when omitted. Accept
        // the bare name, the SAP__ alias, and the fully-qualified form.
        let type_attr = record.attribute("Type").unwrap_or("UI.DataField");
        let normalized = type_attr.trim_start_matches("SAP__");
        let leaf = normalized.rsplit('.').next().unwrap_or(normalized);
        if leaf != "DataField" {
            continue;
        }
        let mut value_path: Option<String> = None;
        let mut label: Option<String> = None;
        let mut importance: Option<String> = None;
        for pv in children_by_tag(&record, "PropertyValue") {
            match pv.attribute("Property") {
                Some("Value") => value_path = pv.attribute("Path").map(String::from),
                Some("Label") => label = pv.attribute("String").map(String::from),
                Some("Importance") => {
                    if let Some(em) = pv.attribute("EnumMember") {
                        importance = Some(
                            em.rsplit('/').next().unwrap_or(em).to_string(),
                        );
                    }
                }
                _ => {}
            }
        }
        if let Some(vp) = value_path {
            out.push(LineItemField { value_path: vp, label, importance });
        }
    }
    out
}

/// Read a `<PropertyValue>` whose content is a `<Collection>` of
/// `<PropertyPath>` text nodes. Returns the path strings in source order.
fn parse_property_path_collection(pv: &roxmltree::Node) -> Vec<String> {
    let mut out = Vec::new();
    for coll in children_by_tag(pv, "Collection") {
        for pp in children_by_tag(&coll, "PropertyPath") {
            if let Some(text) = pp.text() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
        }
    }
    out
}

/// Extract a `UI.Criticality` value from a `<Annotation>` node. Accepts
/// the three common inline forms: `Path="..."`, `Int="n"`, or
/// `EnumMember="UI.CriticalityType/Positive"`. Anything else (nested
/// records, expressions) is ignored for now.
fn parse_criticality(annot: &roxmltree::Node) -> Option<Criticality> {
    if let Some(path) = annot.attribute("Path") {
        return Some(Criticality::Path(path.to_string()));
    }
    if let Some(int_str) = annot.attribute("Int") {
        if let Ok(n) = int_str.parse::<u8>() {
            return Some(Criticality::Fixed(n));
        }
    }
    if let Some(em) = annot.attribute("EnumMember") {
        // "UI.CriticalityType/Positive" (or SAP-aliased). Take the last segment.
        let name = em.rsplit('/').next().unwrap_or(em);
        let level = match name {
            "Neutral" | "VeryNegative" => 0, // SAP "VeryNegative" is a legacy alias; treat as 0 to avoid misclassification.
            "Negative" => 1,
            "Critical" => 2,
            "Positive" | "VeryPositive" => 3,
            "Information" => 5,
            _ => return None,
        };
        return Some(Criticality::Fixed(level));
    }
    None
}

/// Read a V2 `sap:<name>` boolean attribute off a node. Returns `None` if
/// the attribute isn't present so the caller can distinguish "unspecified"
/// from "explicitly false".
fn parse_sap_bool(node: &roxmltree::Node, name: &str) -> Option<bool> {
    node.attribute((SAP_DATA_NS, name)).map(|v| v == "true")
}

/// Parse the `Record` inside a `UI.HeaderInfo` annotation. Returns `None`
/// if the annotation is missing its Record child or has no useful fields.
fn parse_header_info_record(annot: &roxmltree::Node) -> Option<HeaderInfo> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut info = HeaderInfo::default();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("TypeName") => info.type_name = pv.attribute("String").map(String::from),
            Some("TypeNamePlural") => {
                info.type_name_plural = pv.attribute("String").map(String::from);
            }
            Some("Title") => {
                // Title is itself a Record { Value: Path } (UI.DataField shape).
                if let Some(title_rec) = children_by_tag(&pv, "Record").into_iter().next() {
                    for title_pv in children_by_tag(&title_rec, "PropertyValue") {
                        if title_pv.attribute("Property") == Some("Value") {
                            info.title_path = title_pv.attribute("Path").map(String::from);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if info.type_name.is_none() && info.type_name_plural.is_none() && info.title_path.is_none() {
        None
    } else {
        Some(info)
    }
}

fn push_sap_attrs(out: &mut Vec<RawAnnotation>, node: &roxmltree::Node, target: &str) {
    for attr in node.attributes() {
        if attr.namespace() == Some(SAP_DATA_NS) {
            out.push(RawAnnotation {
                term: format!("sap:{}", attr.name()),
                namespace: "SAP".to_string(),
                target: target.to_string(),
                value: Some(attr.value().to_string()),
                qualifier: None,
            });
        }
    }
}

/// Derive a display-friendly vocabulary name from an annotation term.
/// Examples: `Common.Label` → "Common", `SAP__common.Label` → "Common",
/// `UI.LineItem` → "UI", `Org.OData.Measures.V1.ISOCurrency` → "Measures".
/// Scan an F4 service's `$metadata` XML for a `Common.ValueListMapping`
/// annotation that targets `local_property` (matched on the trailing
/// path segment — the target's namespace-qualified entity type doesn't
/// have to match). Returns the first match as a `ValueList` — the same
/// record shape our inline-`Common.ValueList` parser already handles.
///
/// This is the bridge between the parent service (which only carries
/// `Common.ValueListReferences` URLs) and the F4 service (which carries
/// the actual `Common.ValueListMapping` record). Callers resolve the
/// reference URL, fetch the F4 `$metadata`, then hand it to this helper
/// along with the parent property name.
pub fn parse_value_list_mapping_xml(xml: &str, local_property: &str) -> Option<ValueList> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let schema_nodes: Vec<_> = doc
        .descendants()
        .filter(|n| n.has_tag_name("Schema"))
        .collect();
    let suffix = format!("/{local_property}");
    for schema in schema_nodes {
        for annotations_block in children_by_tag(&schema, "Annotations") {
            let target = match annotations_block.attribute("Target") {
                Some(t) => t,
                None => continue,
            };
            if !target.ends_with(&suffix) {
                continue;
            }
            for annot in children_by_tag(&annotations_block, "Annotation") {
                let term = match annot.attribute("Term") {
                    Some(t) => t,
                    None => continue,
                };
                let lower = term.trim_start_matches("SAP__").to_ascii_lowercase();
                if !lower.ends_with(".valuelistmapping") {
                    continue;
                }
                if let Some(vl) = parse_value_list_record(&annot) {
                    return Some(vl);
                }
            }
        }
    }
    None
}

fn extract_annotation_namespace(term: &str) -> String {
    let t = term.strip_prefix("SAP__").unwrap_or(term);
    let t = t.strip_prefix("Org.OData.").unwrap_or(t);
    // Strip the last dot-segment, which is the term name itself.
    let ns = match t.rsplit_once('.') {
        Some((ns, _)) => ns,
        None => return capitalize_first(t),
    };
    // The first segment of what remains is the vocabulary name
    // (version segments like `V1` sit after it and can be ignored for grouping).
    let primary = ns.split('.').next().unwrap_or(ns);
    capitalize_first(primary)
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

// ── V4 Annotation label parsing ──

/// Parse V4 `<Annotations Target="...">` blocks and extract labels.
/// Returns a map: "EntityTypeName/PropertyName" → label string.
fn parse_v4_annotation_labels(schema: &roxmltree::Node) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    let alias = schema.attribute("Alias").unwrap_or("");

    for annots_node in children_by_tag(schema, "Annotations") {
        let target = annots_node.attribute("Target").unwrap_or("");
        // Target is like "SAP__self.WarehouseType/EWMWarehouse" or "SAP__self.WarehouseType"
        // Normalize: strip the alias prefix
        let target = strip_alias_prefix(target, alias);

        for annot in children_by_tag(&annots_node, "Annotation") {
            let term = annot.attribute("Term").unwrap_or("");
            // Look for common.Label or SAP__common.Label
            if term.ends_with(".Label") && (term.contains("common") || term.contains("Common")) {
                if let Some(value) = annot.attribute("String") {
                    labels.insert(target.to_string(), value.to_string());
                }
            }
        }
    }

    labels
}

/// Strip alias prefix: "SAP__self.TypeName" → "TypeName", "SAP__self.TypeName/Prop" → "TypeName/Prop"
fn strip_alias_prefix<'a>(target: &'a str, alias: &str) -> &'a str {
    if !alias.is_empty() {
        if let Some(rest) = target.strip_prefix(alias) {
            return rest.strip_prefix('.').unwrap_or(rest);
        }
    }
    // Fallback: strip first dot-segment if it looks like a namespace
    if let Some(dot_pos) = target.find('.') {
        &target[dot_pos + 1..]
    } else {
        target
    }
}

/// Apply annotation labels to entity type properties.
fn apply_annotation_labels(
    entity_types: &mut [EntityType],
    labels: &HashMap<String, String>,
    _namespace: &str,
) {
    for et in entity_types.iter_mut() {
        // Entity-level label: "TypeName" → label
        // We don't have a field for entity-level label yet, but could add one.

        // Property-level label: "TypeName/PropertyName" → label
        for prop in et.properties.iter_mut() {
            if prop.label.is_none() {
                let key = format!("{}/{}", et.name, prop.name);
                // Try with type suffix stripped
                if let Some(label) = labels.get(&key) {
                    prop.label = Some(label.clone());
                } else {
                    // Also try with "Type" suffix stripped (e.g., WarehouseStorageTypeType → WarehouseStorageTypeType)
                    // The annotation uses the full name
                    let key_with_suffix = format!("{}Type/{}", et.name, prop.name);
                    if let Some(label) = labels.get(&key_with_suffix) {
                        prop.label = Some(label.clone());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── V2 tests ──

    const TEST_METADATA_V2: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="1.0" xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx">
  <edmx:DataServices m:DataServiceVersion="2.0" xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
    <Schema Namespace="ZSALES_SRV" xmlns="http://schemas.microsoft.com/ado/2008/09/edm" xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="Customer">
        <Key>
          <PropertyRef Name="CustomerID"/>
        </Key>
        <Property Name="CustomerID" Type="Edm.String" Nullable="false" MaxLength="10" sap:label="Customer"/>
        <Property Name="CustomerName" Type="Edm.String" MaxLength="40"/>
        <Property Name="City" Type="Edm.String" MaxLength="30"/>
        <NavigationProperty Name="ToOrders" Relationship="ZSALES_SRV.CustomerOrder" FromRole="Customer" ToRole="Order"/>
      </EntityType>
      <EntityType Name="Order">
        <Key>
          <PropertyRef Name="OrderID"/>
        </Key>
        <Property Name="OrderID" Type="Edm.String" Nullable="false" MaxLength="10"/>
        <Property Name="CustomerID" Type="Edm.String" MaxLength="10"/>
        <Property Name="Amount" Type="Edm.Decimal" Precision="15" Scale="2"/>
      </EntityType>
      <Association Name="CustomerOrder">
        <End Type="ZSALES_SRV.Customer" Multiplicity="1" Role="Customer"/>
        <End Type="ZSALES_SRV.Order" Multiplicity="*" Role="Order"/>
      </Association>
      <EntityContainer Name="ZSALES_SRV_Entities" m:IsDefaultEntityContainer="true">
        <EntitySet Name="CustomerSet" EntityType="ZSALES_SRV.Customer"/>
        <EntitySet Name="OrderSet" EntityType="ZSALES_SRV.Order"/>
        <FunctionImport Name="GetTopCustomers" m:HttpMethod="GET" ReturnType="Collection(ZSALES_SRV.Customer)">
          <Parameter Name="TopN" Type="Edm.Int32" Mode="In"/>
        </FunctionImport>
      </EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    #[test]
    fn test_parse_v2_metadata() {
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        assert_eq!(meta.version, ODataVersion::V2);
        assert_eq!(meta.schema_namespace, "ZSALES_SRV");
        assert_eq!(meta.entity_types.len(), 2);
        assert_eq!(meta.associations.len(), 1);
        assert_eq!(meta.entity_sets.len(), 2);
        assert_eq!(meta.function_imports.len(), 1);
    }

    #[test]
    fn test_v2_entity_type_properties() {
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        let customer = meta.find_entity_type("Customer").unwrap();
        assert_eq!(customer.keys, vec!["CustomerID"]);
        assert_eq!(customer.properties.len(), 3);
        assert_eq!(customer.nav_properties.len(), 1);
        assert!(!customer.properties[0].nullable);
        assert_eq!(customer.properties[0].label.as_deref(), Some("Customer"));
    }

    #[test]
    fn test_v2_entity_type_for_set() {
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        let et = meta.entity_type_for_set("CustomerSet").unwrap();
        assert_eq!(et.name, "Customer");
    }

    #[test]
    fn test_v2_nav_targets() {
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        let customer = meta.find_entity_type("Customer").unwrap();
        let targets = meta.nav_targets(customer);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "ToOrders");
        assert_eq!(targets[0].1, "Order");
        assert_eq!(targets[0].2, "*");
    }

    #[test]
    fn test_v2_function_import() {
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        let fi = &meta.function_imports[0];
        assert_eq!(fi.name, "GetTopCustomers");
        assert_eq!(fi.http_method, "GET");
        assert_eq!(fi.parameters.len(), 1);
        assert_eq!(fi.parameters[0].name, "TopN");
    }

    // ── V4 tests ──

    const TEST_METADATA_V4: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="com.sap.gateway.srvd_a2x.api_warehouse_2.v0001" Alias="SAP__self">
      <EntityType Name="WarehouseStorageTypeType">
        <Key>
          <PropertyRef Name="EWMWarehouse"/>
          <PropertyRef Name="EWMStorageType"/>
        </Key>
        <Property Name="EWMWarehouse" Type="Edm.String" Nullable="false" MaxLength="4"/>
        <Property Name="EWMStorageType" Type="Edm.String" Nullable="false" MaxLength="4"/>
        <NavigationProperty Name="_Warehouse" Type="com.sap.gateway.srvd_a2x.api_warehouse_2.v0001.WarehouseType" Nullable="false" Partner="_WarehouseStorageType">
          <ReferentialConstraint Property="EWMWarehouse" ReferencedProperty="EWMWarehouse"/>
        </NavigationProperty>
      </EntityType>
      <EntityType Name="WarehouseType">
        <Key>
          <PropertyRef Name="EWMWarehouse"/>
        </Key>
        <Property Name="EWMWarehouse" Type="Edm.String" Nullable="false" MaxLength="4"/>
        <NavigationProperty Name="_WarehouseStorageType" Type="Collection(com.sap.gateway.srvd_a2x.api_warehouse_2.v0001.WarehouseStorageTypeType)" Partner="_Warehouse">
          <OnDelete Action="Cascade"/>
        </NavigationProperty>
      </EntityType>
      <EntityContainer Name="Container">
        <EntitySet Name="Warehouse" EntityType="com.sap.gateway.srvd_a2x.api_warehouse_2.v0001.WarehouseType">
          <NavigationPropertyBinding Path="_WarehouseStorageType" Target="WarehouseStorageType"/>
        </EntitySet>
        <EntitySet Name="WarehouseStorageType" EntityType="com.sap.gateway.srvd_a2x.api_warehouse_2.v0001.WarehouseStorageTypeType">
          <NavigationPropertyBinding Path="_Warehouse" Target="Warehouse"/>
        </EntitySet>
      </EntityContainer>
      <Annotations Target="SAP__self.WarehouseStorageTypeType/EWMWarehouse">
        <Annotation Term="SAP__common.Label" String="Warehouse Number"/>
      </Annotations>
      <Annotations Target="SAP__self.WarehouseStorageTypeType/EWMStorageType">
        <Annotation Term="SAP__common.Label" String="Storage Type"/>
      </Annotations>
      <Annotations Target="SAP__self.WarehouseType/EWMWarehouse">
        <Annotation Term="SAP__common.Label" String="Warehouse Number"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    #[test]
    fn test_parse_v4_metadata() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        assert_eq!(meta.version, ODataVersion::V4);
        assert_eq!(meta.entity_types.len(), 2);
        assert_eq!(meta.entity_sets.len(), 2);
        assert_eq!(meta.associations.len(), 0); // V4 has no associations
    }

    #[test]
    fn test_v4_entity_type_for_set() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let et = meta.entity_type_for_set("Warehouse").unwrap();
        assert_eq!(et.name, "WarehouseType");
        assert_eq!(et.keys, vec!["EWMWarehouse"]);
    }

    #[test]
    fn test_v4_nav_properties() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let wh = meta.find_entity_type("WarehouseType").unwrap();
        assert_eq!(wh.nav_properties.len(), 1);
        let nav = &wh.nav_properties[0];
        assert_eq!(nav.name, "_WarehouseStorageType");
        assert!(nav.target_type.contains("WarehouseStorageTypeType"));
        assert_eq!(nav.partner, "_Warehouse");
    }

    #[test]
    fn test_v4_nav_targets() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let wh = meta.find_entity_type("WarehouseType").unwrap();
        let targets = meta.nav_targets(wh);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "_WarehouseStorageType");
        assert_eq!(targets[0].1, "WarehouseStorageTypeType");
        assert_eq!(targets[0].2, "*"); // Collection = many
    }

    #[test]
    fn test_v4_nav_targets_single() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let st = meta.find_entity_type("WarehouseStorageTypeType").unwrap();
        let targets = meta.nav_targets(st);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].0, "_Warehouse");
        assert_eq!(targets[0].1, "WarehouseType");
        assert_eq!(targets[0].2, "1"); // Not a collection = single
    }

    #[test]
    fn test_v2_captures_sap_annotations() {
        // The V2 fixture has one sap:label on Property — plus whatever test
        // authoring evolves. We just assert we find at least that one and
        // that the SAP namespace is used.
        let meta = parse_metadata(TEST_METADATA_V2).unwrap();
        assert!(
            !meta.annotations.is_empty(),
            "V2 sap:* attributes must surface as RawAnnotation"
        );
        let has_label = meta
            .annotations
            .iter()
            .any(|a| a.term == "sap:label" && a.namespace == "SAP");
        assert!(has_label, "expected at least one sap:label annotation");
    }

    #[test]
    fn test_v4_captures_common_label_annotations() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        // Fixture has three Common.Label annotations in explicit <Annotations> blocks.
        assert_eq!(meta.annotations.len(), 3);
        for ann in &meta.annotations {
            assert_eq!(ann.namespace, "Common");
            assert_eq!(ann.term, "SAP__common.Label");
            assert!(ann.value.is_some(), "String-valued annotations must carry value");
        }
    }

    #[test]
    fn annotation_summary_groups_by_namespace() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let summary = meta.annotation_summary();
        assert_eq!(summary.total, 3);
        assert_eq!(summary.by_namespace.get("Common").copied(), Some(3));
    }

    #[test]
    fn extract_annotation_namespace_handles_common_shapes() {
        assert_eq!(extract_annotation_namespace("Common.Label"), "Common");
        assert_eq!(extract_annotation_namespace("SAP__common.Label"), "Common");
        assert_eq!(extract_annotation_namespace("UI.LineItem"), "UI");
        assert_eq!(
            extract_annotation_namespace("Capabilities.FilterRestrictions"),
            "Capabilities"
        );
        assert_eq!(
            extract_annotation_namespace("Org.OData.Measures.V1.ISOCurrency"),
            "Measures"
        );
    }

    const TEST_METADATA_V4_TYPED: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="com.example.warehouse" Alias="SAP__self">
      <EntityType Name="WarehouseOrderType">
        <Key><PropertyRef Name="OrderNumber"/></Key>
        <Property Name="OrderNumber" Type="Edm.String" Nullable="false" MaxLength="10"/>
        <Property Name="MaterialID" Type="Edm.String" MaxLength="18"/>
        <Property Name="MaterialDescription" Type="Edm.String" MaxLength="40"/>
      </EntityType>
      <EntityContainer Name="Container">
        <EntitySet Name="WarehouseOrder" EntityType="com.example.warehouse.WarehouseOrderType"/>
      </EntityContainer>
      <Annotations Target="SAP__self.WarehouseOrderType">
        <Annotation Term="UI.HeaderInfo">
          <Record>
            <PropertyValue Property="TypeName" String="Warehouse Order"/>
            <PropertyValue Property="TypeNamePlural" String="Warehouse Orders"/>
            <PropertyValue Property="Title">
              <Record>
                <PropertyValue Property="Value" Path="OrderNumber"/>
              </Record>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.WarehouseOrderType/MaterialID">
        <Annotation Term="Common.Text" Path="MaterialDescription"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    #[test]
    fn test_v4_parses_header_info_onto_entity_type() {
        let meta = parse_metadata(TEST_METADATA_V4_TYPED).unwrap();
        let et = meta.find_entity_type("WarehouseOrderType").unwrap();
        let info = et
            .header_info
            .as_ref()
            .expect("UI.HeaderInfo should be parsed onto entity type");
        assert_eq!(info.type_name.as_deref(), Some("Warehouse Order"));
        assert_eq!(info.type_name_plural.as_deref(), Some("Warehouse Orders"));
        assert_eq!(info.title_path.as_deref(), Some("OrderNumber"));
    }

    #[test]
    fn test_v4_parses_common_text_onto_property() {
        let meta = parse_metadata(TEST_METADATA_V4_TYPED).unwrap();
        let et = meta.find_entity_type("WarehouseOrderType").unwrap();
        let prop = et
            .properties
            .iter()
            .find(|p| p.name == "MaterialID")
            .unwrap();
        assert_eq!(prop.text_path.as_deref(), Some("MaterialDescription"));

        // Sibling property has no Common.Text — text_path must stay None.
        let order = et
            .properties
            .iter()
            .find(|p| p.name == "OrderNumber")
            .unwrap();
        assert!(order.text_path.is_none());
    }

    #[test]
    fn test_v2_property_flags() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="1.0" xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx">
  <edmx:DataServices m:DataServiceVersion="2.0" xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
    <Schema Namespace="ZT" xmlns="http://schemas.microsoft.com/ado/2008/09/edm" xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="Mat">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" sap:filterable="false" sap:sortable="false" sap:creatable="false" sap:updatable="false" sap:required-in-filter="true"/>
        <Property Name="Plain" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="C"><EntitySet Name="Mats" EntityType="ZT.Mat"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let mat = meta.find_entity_type("Mat").unwrap();
        let id = mat.properties.iter().find(|p| p.name == "ID").unwrap();
        assert_eq!(id.filterable, Some(false));
        assert_eq!(id.sortable, Some(false));
        assert_eq!(id.creatable, Some(false));
        assert_eq!(id.updatable, Some(false));
        assert_eq!(id.required_in_filter, Some(true));

        let plain = mat.properties.iter().find(|p| p.name == "Plain").unwrap();
        // Unspecified sap:* must stay None so callers can distinguish default
        // from "explicitly false".
        assert_eq!(plain.filterable, None);
        assert_eq!(plain.sortable, None);
    }

    #[test]
    fn test_v4_criticality_enum_and_path() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="T">
        <Key><PropertyRef Name="K"/></Key>
        <Property Name="K" Type="Edm.String" Nullable="false"/>
        <Property Name="Status" Type="Edm.String"/>
        <Property Name="Overall" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="C"><EntitySet Name="Ts" EntityType="n.T"/></EntityContainer>
      <Annotations Target="SAP__self.T/Status">
        <Annotation Term="SAP__UI.Criticality" EnumMember="SAP__UI.CriticalityType/Positive"/>
      </Annotations>
      <Annotations Target="SAP__self.T/Overall">
        <Annotation Term="UI.Criticality" Path="StatusCriticality"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let t = meta.find_entity_type("T").unwrap();
        let status = t.properties.iter().find(|p| p.name == "Status").unwrap();
        assert!(matches!(status.criticality, Some(Criticality::Fixed(3))));
        let overall = t.properties.iter().find(|p| p.name == "Overall").unwrap();
        match &overall.criticality {
            Some(Criticality::Path(p)) => assert_eq!(p, "StatusCriticality"),
            other => panic!("expected Path criticality, got {other:?}"),
        }
    }

    #[test]
    fn test_v2_sap_text_lands_on_property() {
        // Minimal V2 schema with a sap:text attribute — the code path that
        // picks up older-style text associations.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="1.0" xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx">
  <edmx:DataServices m:DataServiceVersion="2.0" xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
    <Schema Namespace="ZT" xmlns="http://schemas.microsoft.com/ado/2008/09/edm" xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="Mat">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" sap:text="Description"/>
        <Property Name="Description" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="C"><EntitySet Name="Mats" EntityType="ZT.Mat"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let mat = meta.find_entity_type("Mat").unwrap();
        let id_prop = mat.properties.iter().find(|p| p.name == "ID").unwrap();
        assert_eq!(id_prop.text_path.as_deref(), Some("Description"));
    }

    #[test]
    fn test_v4_measures_unit_and_isocurrency() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="NetAmount" Type="Edm.Decimal"/>
        <Property Name="TransactionCurrency" Type="Edm.String"/>
        <Property Name="NetWeight" Type="Edm.Decimal"/>
        <Property Name="WeightUnit" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="C"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/NetAmount">
        <Annotation Term="SAP__measures.ISOCurrency" Path="TransactionCurrency"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/NetWeight">
        <Annotation Term="SAP__measures.Unit" Path="WeightUnit"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let amount = ot.properties.iter().find(|p| p.name == "NetAmount").unwrap();
        assert_eq!(amount.iso_currency_path.as_deref(), Some("TransactionCurrency"));
        let weight = ot.properties.iter().find(|p| p.name == "NetWeight").unwrap();
        assert_eq!(weight.unit_path.as_deref(), Some("WeightUnit"));
    }

    #[test]
    fn test_v4_capabilities_restrictions_flip_property_flags() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="StockType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Quantity" Type="Edm.Decimal"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="StorageType" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Stocks" EntityType="n.StockType"/></EntityContainer>
      <Annotations Target="SAP__self.Container/Stocks">
        <Annotation Term="SAP__capabilities.FilterRestrictions">
          <Record>
            <PropertyValue Property="NonFilterableProperties">
              <Collection>
                <PropertyPath>Quantity</PropertyPath>
              </Collection>
            </PropertyValue>
            <PropertyValue Property="RequiredProperties">
              <Collection>
                <PropertyPath>Warehouse</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="SAP__capabilities.SortRestrictions">
          <Record>
            <PropertyValue Property="NonSortableProperties">
              <Collection>
                <PropertyPath>StorageType</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="Capabilities.UpdateRestrictions">
          <Record>
            <PropertyValue Property="NonUpdatableProperties">
              <Collection>
                <PropertyPath>ID</PropertyPath>
                <PropertyPath>Warehouse</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let st = meta.find_entity_type("StockType").unwrap();
        let qty = st.properties.iter().find(|p| p.name == "Quantity").unwrap();
        assert_eq!(qty.filterable, Some(false));
        let wh = st.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        assert_eq!(wh.required_in_filter, Some(true));
        assert_eq!(wh.updatable, Some(false));
        let st_type = st.properties.iter().find(|p| p.name == "StorageType").unwrap();
        assert_eq!(st_type.sortable, Some(false));
        let id = st.properties.iter().find(|p| p.name == "ID").unwrap();
        assert_eq!(id.updatable, Some(false));
        // Untouched properties stay at None.
        assert_eq!(id.filterable, None);
        assert_eq!(qty.sortable, None);
    }

    #[test]
    fn test_v4_selection_fields_on_entity_type_target() {
        // SAP-generated services typically target the entity type directly.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="StockType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Stocks" EntityType="n.StockType"/></EntityContainer>
      <Annotations Target="SAP__self.StockType">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let st = meta.find_entity_type("StockType").unwrap();
        assert_eq!(st.selection_fields, vec!["Warehouse"]);
    }

    #[test]
    fn test_v4_selection_fields_lands_on_entity_type() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="WarehouseType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="Language" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Warehouses" EntityType="n.WarehouseType"/></EntityContainer>
      <Annotations Target="SAP__self.Container/Warehouses">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
            <PropertyPath>Language</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let wh = meta.find_entity_type("WarehouseType").unwrap();
        assert_eq!(wh.selection_fields, vec!["Warehouse", "Language"]);
    }

    #[test]
    fn test_v4_common_value_list_full_record() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="Plant" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="SAP__common.ValueList">
          <Record>
            <PropertyValue Property="Label" String="Warehouse F4"/>
            <PropertyValue Property="CollectionPath" String="WarehouseValueHelp"/>
            <PropertyValue Property="SearchSupported" Bool="true"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="Common.ValueListParameterInOut">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Warehouse"/>
                </Record>
                <Record Type="Common.ValueListParameterIn">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Plant"/>
                  <PropertyValue Property="ValueListProperty" String="Plant"/>
                </Record>
                <Record Type="Common.ValueListParameterDisplayOnly">
                  <PropertyValue Property="ValueListProperty" String="Description"/>
                </Record>
                <Record Type="Common.ValueListParameterConstant">
                  <PropertyValue Property="Constant" String="EN"/>
                  <PropertyValue Property="ValueListProperty" String="Language"/>
                </Record>
                <Record Type="Common.ValueListParameterExotic">
                  <PropertyValue Property="ValueListProperty" String="SkipMe"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let wh = ot.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        let vl = wh.value_list.as_ref().expect("value_list should be parsed");
        assert_eq!(vl.collection_path, "WarehouseValueHelp");
        assert_eq!(vl.label.as_deref(), Some("Warehouse F4"));
        assert_eq!(vl.search_supported, Some(true));
        // Unknown Type "Exotic" should have been skipped.
        assert_eq!(vl.parameters.len(), 4);
        assert!(matches!(vl.parameters[0].kind, ValueListParameterKind::InOut));
        assert_eq!(vl.parameters[0].local_property.as_deref(), Some("Warehouse"));
        assert_eq!(vl.parameters[0].value_list_property, "Warehouse");
        assert!(matches!(vl.parameters[1].kind, ValueListParameterKind::In));
        assert_eq!(vl.parameters[1].local_property.as_deref(), Some("Plant"));
        assert!(matches!(vl.parameters[2].kind, ValueListParameterKind::DisplayOnly));
        assert!(vl.parameters[2].local_property.is_none());
        assert_eq!(vl.parameters[2].value_list_property, "Description");
        assert!(matches!(vl.parameters[3].kind, ValueListParameterKind::Constant));
        assert_eq!(vl.parameters[3].constant.as_deref(), Some("EN"));
        assert_eq!(vl.parameters[3].value_list_property, "Language");
    }

    #[test]
    fn test_v4_common_value_list_missing_collection_path_is_skipped() {
        // Without CollectionPath the picker can't do anything — drop it.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="Common.ValueList">
          <Record>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="Common.ValueListParameterInOut">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Warehouse"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let wh = ot.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        assert!(wh.value_list.is_none());
    }

    #[test]
    fn test_parse_value_list_mapping_xml_finds_mapping() {
        // Shape of an F4 service's $metadata — the mapping targets the
        // *parent* service's property via the SAP__ParentService alias.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="f4" Alias="SAP__f4">
      <EntityType Name="WarehouseVHType">
        <Key><PropertyRef Name="EWMWarehouse"/></Key>
        <Property Name="EWMWarehouse" Type="Edm.String" Nullable="false"/>
        <Property Name="EWMWarehouse_Text" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="I_EWM_WarehouseNumberVH" EntityType="f4.WarehouseVHType"/></EntityContainer>
      <Annotations Target="SAP__ParentService.OrderType/EWMWarehouse">
        <Annotation Term="SAP__common.ValueListMapping">
          <Record>
            <PropertyValue Property="Label" String="Warehouse Number"/>
            <PropertyValue Property="CollectionPath" String="I_EWM_WarehouseNumberVH"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="SAP__common.ValueListParameterInOut">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="EWMWarehouse"/>
                  <PropertyValue Property="ValueListProperty" String="EWMWarehouse"/>
                </Record>
                <Record Type="SAP__common.ValueListParameterDisplayOnly">
                  <PropertyValue Property="ValueListProperty" String="EWMWarehouse_Text"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let vl = parse_value_list_mapping_xml(xml, "EWMWarehouse")
            .expect("mapping should parse");
        assert_eq!(vl.collection_path, "I_EWM_WarehouseNumberVH");
        assert_eq!(vl.label.as_deref(), Some("Warehouse Number"));
        assert_eq!(vl.parameters.len(), 2);
        assert!(parse_value_list_mapping_xml(xml, "SomeOtherProperty").is_none());
    }

    #[test]
    fn test_v4_value_list_references_and_fixed_values_are_captured() {
        // Shape observed on HA9 (UI_PHYSSTOCKPROD_1): properties carry
        // Common.ValueListReferences (relative URL) plus the marker-only
        // Common.ValueListWithFixedValues.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="StockType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="EWMWarehouse" Type="Edm.String"/>
        <Property Name="StockDocumentCategory" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Stocks" EntityType="n.StockType"/></EntityContainer>
      <Annotations Target="SAP__self.StockType/EWMWarehouse">
        <Annotation Term="SAP__common.ValueListReferences">
          <Collection>
            <String>../../../../srvd_f4/sap/i_ewm_warehousenumbervh/0001/$metadata</String>
          </Collection>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.StockType/StockDocumentCategory">
        <Annotation Term="SAP__common.ValueListReferences">
          <Collection>
            <String>../../../../srvd_f4/sap/c_ewm_stockdoccategoryvh/0001/$metadata</String>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__common.ValueListWithFixedValues"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let st = meta.find_entity_type("StockType").unwrap();
        let wh = st.properties.iter().find(|p| p.name == "EWMWarehouse").unwrap();
        assert!(wh.value_list.is_none());
        assert_eq!(wh.value_list_references.len(), 1);
        assert!(wh.value_list_references[0].contains("i_ewm_warehousenumbervh"));
        assert!(!wh.value_list_fixed);
        let sdc = st.properties.iter().find(|p| p.name == "StockDocumentCategory").unwrap();
        assert_eq!(sdc.value_list_references.len(), 1);
        assert!(sdc.value_list_fixed);
    }

    #[test]
    fn test_v4_common_value_list_references_is_not_confused() {
        // Common.ValueListReferences shares a prefix with ValueList but
        // is a different mechanism we don't support yet. Must NOT parse
        // as a ValueList.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="Common.ValueListReferences">
          <Collection>
            <String>https://example.com/external-vh</String>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let wh = ot.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        assert!(wh.value_list.is_none());
    }

    #[test]
    fn test_v4_line_item_on_entity_type_target() {
        // Typical SAP-generated shape: UI.LineItem on the EntityType,
        // mixing DataFields with a DataFieldForAction that we should skip.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="OrderNumber" Type="Edm.String"/>
        <Property Name="Status" Type="Edm.String"/>
        <Property Name="NetAmount" Type="Edm.Decimal"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.LineItem">
          <Collection>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="OrderNumber"/>
              <PropertyValue Property="Label" String="Order"/>
              <PropertyValue Property="Importance" EnumMember="UI.ImportanceType/High"/>
            </Record>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="Status"/>
            </Record>
            <Record Type="UI.DataFieldForAction">
              <PropertyValue Property="Action" String="n.Cancel"/>
            </Record>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="NetAmount"/>
              <PropertyValue Property="Importance" EnumMember="UI.ImportanceType/Low"/>
            </Record>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        assert_eq!(ot.line_item.len(), 3);
        assert_eq!(ot.line_item[0].value_path, "OrderNumber");
        assert_eq!(ot.line_item[0].label.as_deref(), Some("Order"));
        assert_eq!(ot.line_item[0].importance.as_deref(), Some("High"));
        assert_eq!(ot.line_item[1].value_path, "Status");
        assert!(ot.line_item[1].label.is_none());
        assert!(ot.line_item[1].importance.is_none());
        assert_eq!(ot.line_item[2].value_path, "NetAmount");
        assert_eq!(ot.line_item[2].importance.as_deref(), Some("Low"));
    }

    #[test]
    fn test_v4_line_item_on_entity_set_target() {
        // Some hand-written services put UI.LineItem on the EntitySet.
        // Default (no Type attribute) Record should be treated as UI.DataField.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="OrderNumber" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.Container/Orders">
        <Annotation Term="UI.LineItem">
          <Collection>
            <Record>
              <PropertyValue Property="Value" Path="OrderNumber"/>
            </Record>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        assert_eq!(ot.line_item.len(), 1);
        assert_eq!(ot.line_item[0].value_path, "OrderNumber");
    }

    #[test]
    fn test_v4_annotation_labels() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let st = meta.find_entity_type("WarehouseStorageTypeType").unwrap();
        let wh_prop = st
            .properties
            .iter()
            .find(|p| p.name == "EWMWarehouse")
            .unwrap();
        assert_eq!(wh_prop.label.as_deref(), Some("Warehouse Number"));
        let st_prop = st
            .properties
            .iter()
            .find(|p| p.name == "EWMStorageType")
            .unwrap();
        assert_eq!(st_prop.label.as_deref(), Some("Storage Type"));
    }
}
