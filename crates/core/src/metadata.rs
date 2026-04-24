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
    /// Property paths listed under `UI.PresentationVariant.RequestAtLeast`
    /// — fields Fiori silently appends to `$select` regardless of which
    /// columns the user picks, typically technical supports like time
    /// zones and key descriptions. Used to augment the "Fiori cols"
    /// quick-action. Empty when the service doesn't declare any.
    pub request_at_least: Vec<String>,
    /// `UI.PresentationVariant.SortOrder` entries — the default ordering
    /// Fiori would apply to the list. Each entry points at a property
    /// with a direction flag. Order matches the source collection.
    /// Consumed by the "Fiori cols" action to also fill `$orderby`.
    pub sort_order: Vec<SortOrder>,
    /// Declared `UI.SelectionVariant` records on this entity — Fiori's
    /// "filter variants" (e.g. "Pending", "Completed"). Includes the
    /// default (no-qualifier) variant and any qualified ones. Used by
    /// the desktop's "Fiori filter" button to drop pre-declared
    /// filter defaults into `$filter`. Empty when the service declares
    /// no variants.
    pub selection_variants: Vec<SelectionVariant>,
    /// `Capabilities.SearchRestrictions.Searchable` — whether the set
    /// accepts `$search`. `None` means the service didn't declare it
    /// (callers assume `true` per OData default).
    pub searchable: Option<bool>,
    /// `Capabilities.CountRestrictions.Countable` — whether `$count`
    /// and `$inlinecount` are honored. `None` = unspecified.
    pub countable: Option<bool>,
    /// `Capabilities.TopSupported` — `$top` honored. `None` = unspecified.
    pub top_supported: Option<bool>,
    /// `Capabilities.SkipSupported` — `$skip` honored. `None` = unspecified.
    pub skip_supported: Option<bool>,
    /// Navigation property names listed under
    /// `Capabilities.ExpandRestrictions.NonExpandableProperties` — the
    /// server rejects `$expand` referencing any of these. Empty when
    /// the service declares no restrictions or marks the whole set
    /// non-expandable (see `expandable` for the global flag).
    pub non_expandable_properties: Vec<String>,
    /// `Capabilities.ExpandRestrictions.Expandable` — `$expand` is
    /// supported at all on this set. `None` = unspecified (assume true).
    pub expandable: Option<bool>,
}

/// One `UI.SelectionVariant` record — the declarative shape of a
/// Fiori filter variant. Contains single-valued `Parameters` (always
/// equality) and multi-valued `SelectOptions` (SELECT-OPTIONS-style
/// ranges with sign and option code).
#[derive(Debug, Clone, Serialize)]
pub struct SelectionVariant {
    /// Optional `Qualifier` on the annotation — distinguishes variants
    /// on the same entity (e.g. `Qualifier="Pending"`). `None` for the
    /// default variant.
    pub qualifier: Option<String>,
    /// `Text` property — human-readable variant name (e.g.
    /// `"Pending Orders"`). `None` if not declared.
    pub text: Option<String>,
    /// Fixed `property = value` assignments fed as filter constraints.
    pub parameters: Vec<SelectionParameter>,
    /// Per-property lists of SELECT-OPTIONS-style ranges.
    pub select_options: Vec<SelectOption>,
}

/// Single-valued parameter in a `UI.SelectionVariant.Parameters`.
/// Shape: `<Record Type="UI.ParameterType">` with `PropertyName` +
/// `PropertyValue`. Always interpreted as `property eq value`.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionParameter {
    pub property_name: String,
    pub property_value: String,
}

/// One property's worth of SELECT-OPTIONS ranges inside
/// `UI.SelectionVariant.SelectOptions`. Multiple ranges on the same
/// property are OR-joined when building `$filter`.
#[derive(Debug, Clone, Serialize)]
pub struct SelectOption {
    pub property_name: String,
    pub ranges: Vec<SelectionRange>,
}

/// One `<Record Type="UI.SelectionRangeType">` inside a `SelectOption`.
/// Mirrors ABAP SELECT-OPTIONS semantics: Include/Exclude × operator.
#[derive(Debug, Clone, Serialize)]
pub struct SelectionRange {
    /// `I` (include, normal comparison) or `E` (exclude, negated).
    pub sign: SelectionSign,
    /// Comparison operator — `EQ`/`NE`/`GT`/`GE`/`LT`/`LE`/`BT`/`NB`/`CP`/`NP`.
    pub option: SelectionOption,
    /// Primary value. For `BT`/`NB` this is the lower bound.
    pub low: String,
    /// Upper bound — only present for `BT`/`NB`. `None` otherwise.
    pub high: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionSign {
    I,
    E,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SelectionOption {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Bt,
    Nb,
    Cp,
    Np,
}

/// One entry inside `UI.PresentationVariant.SortOrder`. Maps directly
/// to an `$orderby` clause: `property [asc|desc]`.
#[derive(Debug, Clone, Serialize)]
pub struct SortOrder {
    pub property: String,
    pub descending: bool,
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
    /// The "default" value help for this property — the first parsed
    /// `Common.ValueList` annotation (no-qualifier preferred, then
    /// qualified in source order). Retained for callers that just want
    /// one value help; the full set lives in `value_list_variants`.
    /// `None` when the service declares no inline value list.
    pub value_list: Option<ValueList>,
    /// All parsed `Common.ValueList` annotations for this property,
    /// including the one surfaced on `value_list`. When a property
    /// declares multiple qualified variants (e.g. "ByKey" vs
    /// "ByDescription"), consumers iterate this list to offer a
    /// variant picker in the F4 UI. Empty when there's no inline
    /// value list.
    pub value_list_variants: Vec<ValueList>,
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
    /// How an ID column and its `Common.Text` companion should be
    /// displayed together. Source: `UI.TextArrangement` (per-property
    /// when nested inside `Common.Text`, or entity-type-level as the
    /// default for every text-bearing property in the type). `None`
    /// means "no explicit arrangement" — callers default to
    /// `TextFirst` per Fiori convention.
    pub text_arrangement: Option<TextArrangement>,
    /// `Common.FieldControl` — write/display control signal. Can be a
    /// fixed enum value (`Mandatory` / `Optional` / `ReadOnly` /
    /// `Inapplicable` / `Hidden`) or a path to a sibling property whose
    /// runtime value supplies the control code (1/3/7 = Read-only /
    /// Optional / Mandatory, per the SAP vocabulary numeric codes).
    pub field_control: Option<FieldControl>,
    /// `UI.Hidden` marker — Fiori should never show this property in
    /// any list/detail UI. We still render it in the describe panel
    /// (for transparency) but flag it clearly.
    pub hidden: bool,
    /// `UI.HiddenFilter` marker — property can be shown but must not be
    /// offered as a filter control. Signals "server knows about this
    /// but Fiori hides it from the filter bar".
    pub hidden_filter: bool,
    /// V2 `sap:display-format` attribute — presentation hint like
    /// `Date`, `NonNegative`, `UpperCase`. Stored verbatim (lowercased
    /// or not — SAP emits mixed case historically). Mainly useful for
    /// the describe overlay and future results-grid formatting.
    pub display_format: Option<String>,
    /// V2 `sap:value-list` attribute — marker that the property has a
    /// value help in this service (typical values: `"standard"` or
    /// `"fixed-values"`). V2 doesn't carry a mapping record inside
    /// `$metadata` (unlike V4's `Common.ValueList`), so this is hint-
    /// only: there's no picker target to open. Stored verbatim.
    pub sap_value_list: Option<String>,
}

/// `Common.FieldControl` value. Fixed numeric codes map to the SAP
/// vocabulary's enum (7 = Mandatory, 3 = Optional, 1 = ReadOnly,
/// 0 = Inapplicable, 5 = Hidden); anything else parses as `Path`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum FieldControl {
    Mandatory,
    Optional,
    ReadOnly,
    Inapplicable,
    Hidden,
    /// Dynamic control — path to a sibling property whose runtime
    /// value (one of the numeric codes above) drives the state.
    Path(String),
}

/// `UI.TextArrangement` — how a coded-value column combines with its
/// `Common.Text` description when rendered.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TextArrangement {
    /// Description first, then the code in parens — "English (EN)".
    TextFirst,
    /// Code first, then the description in parens — "EN (English)".
    TextLast,
    /// Keep the two columns separate — no combination.
    TextSeparate,
    /// Description only; hide the code.
    TextOnly,
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
    /// Optional `Qualifier` on the annotation — distinguishes multiple
    /// value helps on the same property (e.g. `Qualifier="ByKey"` vs
    /// `Qualifier="ByDescription"`). `None` for the default variant.
    pub qualifier: Option<String>,
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
                    value_list_variants: Vec::new(),
                    value_list_references: Vec::new(),
                    value_list_fixed: false,
                    text_arrangement: None,
                    field_control: None,
                    hidden: false,
                    hidden_filter: false,
                    display_format: p
                        .attribute(("http://www.sap.com/Protocols/SAPData", "display-format"))
                        .map(String::from),
                    sap_value_list: p
                        .attribute(("http://www.sap.com/Protocols/SAPData", "value-list"))
                        .map(String::from),
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
                request_at_least: Vec::new(),
                sort_order: Vec::new(),
                selection_variants: Vec::new(),
                searchable: None,
                countable: None,
                top_supported: None,
                skip_supported: None,
                non_expandable_properties: Vec::new(),
                expandable: None,
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
            } else if lower.ends_with(".text") && !lower.ends_with(".textarrangement") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if prop.text_path.is_none() {
                                prop.text_path = annot.attribute("Path").map(String::from);
                            }
                            // A `UI.TextArrangement` nested inside the
                            // `Common.Text` annotation is the per-property
                            // override — takes priority over any type-level
                            // default set later.
                            for nested in children_by_tag(&annot, "Annotation") {
                                let n_term = nested.attribute("Term").unwrap_or("");
                                let n_lower = n_term.trim_start_matches("SAP__").to_ascii_lowercase();
                                if n_lower.ends_with(".textarrangement") {
                                    if let Some(ta) = parse_text_arrangement(&nested) {
                                        prop.text_arrangement = Some(ta);
                                    }
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".textarrangement") {
                // Entity-type-level default: apply to every property that
                // already has a `text_path` and no per-property override.
                // (We parse annotations in file order, and `Common.Text`
                // is typically declared *before* a standalone
                // `UI.TextArrangement` on the type — but we guard against
                // either order.)
                if !target.contains('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == target) {
                        if let Some(ta) = parse_text_arrangement(&annot) {
                            for prop in et.properties.iter_mut() {
                                if prop.text_arrangement.is_none() {
                                    prop.text_arrangement = Some(ta);
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".fieldcontrol") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if let Some(fc) = parse_field_control(&annot) {
                                prop.field_control = Some(fc);
                            }
                        }
                    }
                }
            } else if lower.ends_with(".hidden") && !lower.ends_with(".hiddenfilter") {
                // UI.Hidden can be marker-only (`<Annotation .../>`) or
                // carry a `Bool` / `Path` — Fiori convention treats a
                // missing value as `true`. Path variants are
                // runtime-evaluated; we don't resolve them per row, so
                // a Path here still flips the static marker on.
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            let explicit = annot.attribute("Bool").map(|v| v == "true");
                            prop.hidden = explicit.unwrap_or(true);
                        }
                    }
                }
            } else if lower.ends_with(".hiddenfilter") {
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            let explicit = annot.attribute("Bool").map(|v| v == "true");
                            prop.hidden_filter = explicit.unwrap_or(true);
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
            } else if lower.ends_with(".searchrestrictions")
                || lower.ends_with(".countrestrictions")
                || lower.ends_with(".expandrestrictions")
            {
                // Entity-set–scoped. These set flat Option<bool>s and/or
                // a nav-path list on the EntityType so the frontend
                // pre-flight validator can catch queries that will 500.
                if let Some((_, set_name)) = target.split_once('/') {
                    if let Some(type_ref) =
                        entity_sets.iter().find(|s| s.name == set_name)
                    {
                        let type_name = extract_type_name(&type_ref.entity_type).to_string();
                        if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                            apply_entity_set_capability(&lower, &annot, et);
                        }
                    }
                }
            } else if lower.ends_with(".topsupported")
                || lower.ends_with(".skipsupported")
            {
                // Standalone `<Annotation ... Bool="false"/>` on an entity
                // set. Default is `true`, so only `false` is informative
                // (but we store whatever was declared for transparency).
                if let Some((_, set_name)) = target.split_once('/') {
                    if let Some(type_ref) =
                        entity_sets.iter().find(|s| s.name == set_name)
                    {
                        let type_name = extract_type_name(&type_ref.entity_type).to_string();
                        if let Some(et) = entity_types.iter_mut().find(|t| t.name == type_name) {
                            let val = annot.attribute("Bool").map(|v| v == "true");
                            if lower.ends_with(".topsupported") {
                                et.top_supported = val;
                            } else {
                                et.skip_supported = val;
                            }
                        }
                    }
                }
            } else if lower.ends_with(".valuelist")
                && !lower.ends_with(".valuelistreferences")
                && !lower.ends_with(".valuelistmapping")
                && !lower.ends_with(".valuelistwithfixedvalues")
            {
                // Common.ValueList targets a specific property. We keep
                // every parsed variant — the no-qualifier one lands on
                // `value_list` (first-fill) for consumers that want a
                // single default; all are pushed into
                // `value_list_variants` so the picker can offer a
                // variant switcher. Dedupe by qualifier so a repeated
                // annotation on the same target doesn't double-count.
                if let Some((et_name, prop_name)) = target.split_once('/') {
                    if let Some(et) = entity_types.iter_mut().find(|e| e.name == et_name) {
                        if let Some(prop) = et.properties.iter_mut().find(|p| p.name == prop_name) {
                            if let Some(mut vl) = parse_value_list_record(&annot) {
                                vl.qualifier = annot.attribute("Qualifier").map(String::from);
                                let already_have = prop
                                    .value_list_variants
                                    .iter()
                                    .any(|existing| existing.qualifier == vl.qualifier);
                                if !already_have {
                                    if prop.value_list.is_none() {
                                        prop.value_list = Some(vl.clone());
                                    }
                                    prop.value_list_variants.push(vl);
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
            } else if lower.ends_with(".selectionpresentationvariant") {
                // UI.SelectionPresentationVariant wraps a named view:
                // Text + ID + (SelectionVariant inline Record OR Path)
                // + (PresentationVariant inline Record OR Path). Path
                // references point at peer qualified annotations that
                // our standalone SV/PV dispatches already capture, so
                // we only inspect inline records here. Inline records
                // are unusual in SAP-CDS output but supported by the
                // vocabulary — this keeps services that emit them
                // (hand-written CDS, tests) working.
                let qualifier = annot.attribute("Qualifier").map(String::from);
                let record = match children_by_tag(&annot, "Record").into_iter().next() {
                    Some(r) => r,
                    None => continue,
                };
                let mut spv_text: Option<String> = None;
                let mut inline_sv: Option<roxmltree::Node> = None;
                let mut inline_pv: Option<roxmltree::Node> = None;
                for pv in children_by_tag(&record, "PropertyValue") {
                    match pv.attribute("Property") {
                        Some("Text") => spv_text = pv.attribute("String").map(String::from),
                        Some("SelectionVariant") => {
                            if pv.attribute("Path").is_none() {
                                inline_sv = children_by_tag(&pv, "Record").into_iter().next();
                            }
                        }
                        Some("PresentationVariant") => {
                            if pv.attribute("Path").is_none() {
                                inline_pv = children_by_tag(&pv, "Record").into_iter().next();
                            }
                        }
                        _ => {}
                    }
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
                        // Inline SelectionVariant → materialize and add,
                        // deduped by qualifier against existing variants.
                        if let Some(sv_record) = inline_sv {
                            let already_have = qualifier.is_some()
                                && et
                                    .selection_variants
                                    .iter()
                                    .any(|v| v.qualifier == qualifier);
                            if !already_have {
                                let fake_annot = sv_record.parent().unwrap_or(sv_record);
                                // Build a variant from the inline record
                                // directly. We can't reuse
                                // parse_selection_variant_record since
                                // it expects the outer <Annotation> as
                                // input — just walk the Record here.
                                let mut variant = SelectionVariant {
                                    qualifier: qualifier.clone(),
                                    text: spv_text.clone(),
                                    parameters: Vec::new(),
                                    select_options: Vec::new(),
                                };
                                for pv in children_by_tag(&sv_record, "PropertyValue") {
                                    match pv.attribute("Property") {
                                        Some("Text") if variant.text.is_none() => {
                                            variant.text = pv.attribute("String").map(String::from);
                                        }
                                        Some("Parameters") => {
                                            for coll in children_by_tag(&pv, "Collection") {
                                                for rec in children_by_tag(&coll, "Record") {
                                                    if let Some(p) = parse_selection_parameter(&rec) {
                                                        variant.parameters.push(p);
                                                    }
                                                }
                                            }
                                        }
                                        Some("SelectOptions") => {
                                            for coll in children_by_tag(&pv, "Collection") {
                                                for rec in children_by_tag(&coll, "Record") {
                                                    if let Some(o) = parse_select_option(&rec) {
                                                        variant.select_options.push(o);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                if qualifier.is_none() {
                                    et.selection_variants.insert(0, variant);
                                } else {
                                    et.selection_variants.push(variant);
                                }
                                let _ = fake_annot; // silence unused-hint lints
                            }
                        }
                        // Inline PresentationVariant → reuse existing
                        // parser on the outer-ish record by synthesizing
                        // an annotation wrapping it. Simpler path: walk
                        // the record directly since we control the shape.
                        if let Some(pv_record) = inline_pv {
                            for pv in children_by_tag(&pv_record, "PropertyValue") {
                                match pv.attribute("Property") {
                                    Some("RequestAtLeast") if et.request_at_least.is_empty() => {
                                        for coll in children_by_tag(&pv, "Collection") {
                                            for pp in children_by_tag(&coll, "PropertyPath") {
                                                if let Some(text) = pp.text() {
                                                    let trimmed = text.trim();
                                                    if !trimmed.is_empty()
                                                        && !et
                                                            .request_at_least
                                                            .iter()
                                                            .any(|x| x == trimmed)
                                                    {
                                                        et.request_at_least
                                                            .push(trimmed.to_string());
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Some("SortOrder") if et.sort_order.is_empty() => {
                                        for coll in children_by_tag(&pv, "Collection") {
                                            for rec in children_by_tag(&coll, "Record") {
                                                if let Some(so) = parse_sort_order_record(&rec) {
                                                    et.sort_order.push(so);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            } else if lower.ends_with(".selectionvariant") {
                // UI.SelectionVariant = declared Fiori filter variant.
                // Multiple qualifiers are allowed (one per named variant),
                // so we push every match rather than keeping just the
                // first. Targets the entity type or the entity set.
                let variant_opt = parse_selection_variant_record(&annot);
                if let Some(mut variant) = variant_opt {
                    variant.qualifier = annot.attribute("Qualifier").map(String::from);
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
                            // Default (no-qualifier) variant goes first so
                            // the frontend's "pick one" logic can grab it
                            // with .first(). Qualified variants preserve
                            // their parse order after.
                            if variant.qualifier.is_none() {
                                et.selection_variants.insert(0, variant);
                            } else {
                                et.selection_variants.push(variant);
                            }
                        }
                    }
                }
            } else if lower.ends_with(".presentationvariant") {
                // UI.PresentationVariant = Record with RequestAtLeast /
                // SortOrder / Visualizations / ... We lift the two
                // actionable pieces — `$select` augment paths and the
                // default `$orderby` order. Targets the entity type or
                // the entity set; resolve both.
                let (paths, sort) = parse_presentation_variant(&annot);
                if paths.is_empty() && sort.is_empty() {
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
                        if !paths.is_empty() && et.request_at_least.is_empty() {
                            et.request_at_least = paths;
                        }
                        if !sort.is_empty() && et.sort_order.is_empty() {
                            et.sort_order = sort;
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

/// Apply an entity-set–scoped capability annotation that lives on
/// the EntityType itself (not on individual properties).
/// Handles `Capabilities.SearchRestrictions`, `CountRestrictions`,
/// and `ExpandRestrictions`. The outer record's `Property` values are
/// the interesting payloads: `Searchable` / `Countable` / `Expandable`
/// booleans, and `NonExpandableProperties` as a NavigationPropertyPath
/// collection. Unknown fields are ignored so new Capability sub-terms
/// don't break the parser.
fn apply_entity_set_capability(
    lower_term: &str,
    annot: &roxmltree::Node,
    et: &mut EntityType,
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
        match (lower_term, prop_name) {
            (t, "Searchable") if t.ends_with(".searchrestrictions") => {
                et.searchable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "Countable") if t.ends_with(".countrestrictions") => {
                et.countable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "Expandable") if t.ends_with(".expandrestrictions") => {
                et.expandable = pv.attribute("Bool").map(|v| v == "true");
            }
            (t, "NonExpandableProperties") if t.ends_with(".expandrestrictions") => {
                // NavigationPropertyPath collection — same shape as
                // PropertyPath. Reuse the existing helper rather than
                // duplicating the walk.
                for coll in children_by_tag(&pv, "Collection") {
                    for npp in children_by_tag(&coll, "NavigationPropertyPath") {
                        if let Some(text) = npp.text() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty()
                                && !et.non_expandable_properties.iter().any(|x| x == trimmed)
                            {
                                et.non_expandable_properties.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
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
    Some(ValueList { qualifier: None, collection_path, label, search_supported, parameters })
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

/// Parse the `Record` inside a `UI.SelectionVariant` annotation into
/// a typed variant. Qualifier is stitched on by the caller (it's on
/// the outer `<Annotation Qualifier="...">`, not on the record). Returns
/// `None` only when the annotation has no inner `<Record>`.
fn parse_selection_variant_record(annot: &roxmltree::Node) -> Option<SelectionVariant> {
    let record = children_by_tag(annot, "Record").into_iter().next()?;
    let mut text: Option<String> = None;
    let mut parameters: Vec<SelectionParameter> = Vec::new();
    let mut select_options: Vec<SelectOption> = Vec::new();
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Text") => text = pv.attribute("String").map(String::from),
            Some("Parameters") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for param_rec in children_by_tag(&coll, "Record") {
                        if let Some(p) = parse_selection_parameter(&param_rec) {
                            parameters.push(p);
                        }
                    }
                }
            }
            Some("SelectOptions") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for opt_rec in children_by_tag(&coll, "Record") {
                        if let Some(o) = parse_select_option(&opt_rec) {
                            select_options.push(o);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(SelectionVariant {
        qualifier: None,
        text,
        parameters,
        select_options,
    })
}

/// Parse a `<Record Type="UI.ParameterType">` — a single-valued filter
/// constraint with a literal `PropertyValue`.
fn parse_selection_parameter(record: &roxmltree::Node) -> Option<SelectionParameter> {
    let mut property_name: Option<String> = None;
    let mut property_value: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("PropertyName") => {
                property_name = pv.attribute("PropertyPath").map(String::from);
            }
            Some("PropertyValue") => {
                // Parameters take a literal primitive value. SAP emits
                // `String=`, `Int=`, or `Bool=`; fall through so we
                // don't lose numeric/bool defaults.
                property_value = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    Some(SelectionParameter {
        property_name: property_name?,
        property_value: property_value?,
    })
}

/// Parse a `<Record Type="UI.SelectOptionType">` — one property's
/// SELECT-OPTIONS list, with zero or more ranges inside.
fn parse_select_option(record: &roxmltree::Node) -> Option<SelectOption> {
    let mut property_name: Option<String> = None;
    let mut ranges: Vec<SelectionRange> = Vec::new();
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("PropertyName") => {
                property_name = pv.attribute("PropertyPath").map(String::from);
            }
            Some("Ranges") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for range_rec in children_by_tag(&coll, "Record") {
                        if let Some(r) = parse_selection_range(&range_rec) {
                            ranges.push(r);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let property_name = property_name?;
    if ranges.is_empty() {
        return None;
    }
    Some(SelectOption { property_name, ranges })
}

/// Parse a `<Record Type="UI.SelectionRangeType">` — one range with
/// sign, operator, and one or two bounds. Unknown enum members short-
/// circuit the whole range (better to drop than to mis-interpret as EQ).
fn parse_selection_range(record: &roxmltree::Node) -> Option<SelectionRange> {
    let mut sign: Option<SelectionSign> = None;
    let mut option: Option<SelectionOption> = None;
    let mut low: Option<String> = None;
    let mut high: Option<String> = None;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Sign") => {
                if let Some(em) = pv.attribute("EnumMember") {
                    let leaf = em.rsplit('/').next().unwrap_or(em);
                    sign = match leaf {
                        "I" => Some(SelectionSign::I),
                        "E" => Some(SelectionSign::E),
                        _ => None,
                    };
                }
            }
            Some("Option") => {
                if let Some(em) = pv.attribute("EnumMember") {
                    let leaf = em.rsplit('/').next().unwrap_or(em);
                    option = match leaf {
                        "EQ" => Some(SelectionOption::Eq),
                        "NE" => Some(SelectionOption::Ne),
                        "GT" => Some(SelectionOption::Gt),
                        "GE" => Some(SelectionOption::Ge),
                        "LT" => Some(SelectionOption::Lt),
                        "LE" => Some(SelectionOption::Le),
                        "BT" => Some(SelectionOption::Bt),
                        "NB" => Some(SelectionOption::Nb),
                        "CP" => Some(SelectionOption::Cp),
                        "NP" => Some(SelectionOption::Np),
                        _ => None,
                    };
                }
            }
            Some("Low") => {
                low = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            Some("High") => {
                high = pv
                    .attribute("String")
                    .or_else(|| pv.attribute("Int"))
                    .or_else(|| pv.attribute("Bool"))
                    .map(String::from);
            }
            _ => {}
        }
    }
    Some(SelectionRange {
        sign: sign?,
        option: option?,
        low: low?,
        high,
    })
}

/// Pull the interesting fields out of a `UI.PresentationVariant`
/// record: `RequestAtLeast` (property paths added to `$select`) and
/// `SortOrder` (default `$orderby` clauses). Empty tuple when the
/// record is missing or both collections are unset.
fn parse_presentation_variant(
    annot: &roxmltree::Node,
) -> (Vec<String>, Vec<SortOrder>) {
    let mut request_at_least: Vec<String> = Vec::new();
    let mut sort_order: Vec<SortOrder> = Vec::new();
    let record = match children_by_tag(annot, "Record").into_iter().next() {
        Some(r) => r,
        None => return (request_at_least, sort_order),
    };
    for pv in children_by_tag(&record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("RequestAtLeast") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for pp in children_by_tag(&coll, "PropertyPath") {
                        if let Some(text) = pp.text() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty()
                                && !request_at_least.iter().any(|x: &String| x == trimmed)
                            {
                                request_at_least.push(trimmed.to_string());
                            }
                        }
                    }
                }
            }
            Some("SortOrder") => {
                for coll in children_by_tag(&pv, "Collection") {
                    for sort_rec in children_by_tag(&coll, "Record") {
                        if let Some(so) = parse_sort_order_record(&sort_rec) {
                            sort_order.push(so);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (request_at_least, sort_order)
}

/// Parse one `<Record Type="Common.SortOrderType">` (or untyped) from
/// the `SortOrder` collection. Shape: `Property` (PropertyPath) +
/// optional `Descending` (Bool, default false).
fn parse_sort_order_record(record: &roxmltree::Node) -> Option<SortOrder> {
    let mut property: Option<String> = None;
    let mut descending = false;
    for pv in children_by_tag(record, "PropertyValue") {
        match pv.attribute("Property") {
            Some("Property") => {
                property = pv.attribute("PropertyPath").map(String::from);
            }
            Some("Descending") => {
                descending = pv.attribute("Bool").map(|v| v == "true").unwrap_or(false);
            }
            _ => {}
        }
    }
    Some(SortOrder { property: property?, descending })
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

/// Extract a `Common.FieldControl` value from an `<Annotation>` node.
/// Accepts `EnumMember` for the five named states, `Path=` for dynamic
/// (per-row) control, and the legacy `Int=` form that SAP-generated
/// services still sometimes emit (7=Mandatory, 3=Optional, 1=ReadOnly,
/// 0=Inapplicable, 5=Hidden).
fn parse_field_control(annot: &roxmltree::Node) -> Option<FieldControl> {
    if let Some(path) = annot.attribute("Path") {
        return Some(FieldControl::Path(path.to_string()));
    }
    if let Some(em) = annot.attribute("EnumMember") {
        let leaf = em.rsplit('/').next().unwrap_or(em);
        return match leaf {
            "Mandatory" => Some(FieldControl::Mandatory),
            "Optional" => Some(FieldControl::Optional),
            "ReadOnly" => Some(FieldControl::ReadOnly),
            "Inapplicable" => Some(FieldControl::Inapplicable),
            "Hidden" => Some(FieldControl::Hidden),
            _ => None,
        };
    }
    if let Some(int_str) = annot.attribute("Int") {
        if let Ok(n) = int_str.parse::<u8>() {
            return match n {
                7 => Some(FieldControl::Mandatory),
                3 => Some(FieldControl::Optional),
                1 => Some(FieldControl::ReadOnly),
                0 => Some(FieldControl::Inapplicable),
                5 => Some(FieldControl::Hidden),
                _ => None,
            };
        }
    }
    None
}

/// Extract a `UI.TextArrangement` value from an `<Annotation>` node.
/// Accepts the `EnumMember="UI.TextArrangementType/TextFirst"` form
/// (with or without the `SAP__` namespace alias) and the SAP-shorthand
/// `TextFirst` unqualified. Unknown variants return `None` rather than
/// guessing — we'd rather fall back to Fiori's default than mis-render.
fn parse_text_arrangement(annot: &roxmltree::Node) -> Option<TextArrangement> {
    let em = annot.attribute("EnumMember")?;
    let leaf = em.rsplit('/').next().unwrap_or(em);
    match leaf {
        "TextFirst" => Some(TextArrangement::TextFirst),
        "TextLast" => Some(TextArrangement::TextLast),
        "TextSeparate" => Some(TextArrangement::TextSeparate),
        "TextOnly" => Some(TextArrangement::TextOnly),
        _ => None,
    }
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
    fn test_v4_common_value_list_multiple_qualified_variants() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="SAP__common.ValueList">
          <Record>
            <PropertyValue Property="CollectionPath" String="WarehouseByKey"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="Common.ValueListParameterInOut">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Key"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="SAP__common.ValueList" Qualifier="ByDescription">
          <Record>
            <PropertyValue Property="CollectionPath" String="WarehouseByDescription"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="Common.ValueListParameterInOut">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Description"/>
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
        // Default (no-qualifier) surfaces on `value_list`.
        let default = wh.value_list.as_ref().expect("default variant expected");
        assert!(default.qualifier.is_none());
        assert_eq!(default.collection_path, "WarehouseByKey");
        // Both variants are captured on value_list_variants.
        assert_eq!(wh.value_list_variants.len(), 2);
        assert!(wh.value_list_variants[0].qualifier.is_none());
        assert_eq!(wh.value_list_variants[0].collection_path, "WarehouseByKey");
        assert_eq!(wh.value_list_variants[1].qualifier.as_deref(), Some("ByDescription"));
        assert_eq!(wh.value_list_variants[1].collection_path, "WarehouseByDescription");
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
    fn test_v4_text_arrangement_nested_in_common_text() {
        // Per-property override: `@UI.textArrangement` inside the CDS
        // annotation on a single field becomes a nested
        // `<Annotation Term="UI.TextArrangement"/>` inside the
        // `<Annotation Term="Common.Text"/>` in $metadata.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Product" Type="Edm.String"/>
        <Property Name="ProductDescription" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Product">
        <Annotation Term="SAP__common.Text" Path="ProductDescription">
          <Annotation Term="SAP__UI.TextArrangement" EnumMember="UI.TextArrangementType/TextSeparate"/>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let p = ot.properties.iter().find(|p| p.name == "Product").unwrap();
        assert_eq!(p.text_path.as_deref(), Some("ProductDescription"));
        assert_eq!(p.text_arrangement, Some(TextArrangement::TextSeparate));
    }

    #[test]
    fn test_v4_text_arrangement_on_entity_type_is_default() {
        // Type-level default applies to every text-bearing property that
        // didn't override it. Properties without a `Common.Text` still
        // get the arrangement stamped (harmless — they have no text to
        // arrange, the frontend ignores it).
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Product" Type="Edm.String"/>
        <Property Name="ProductDescription" Type="Edm.String"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="WarehouseDescription" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Product">
        <Annotation Term="SAP__common.Text" Path="ProductDescription">
          <Annotation Term="SAP__UI.TextArrangement" EnumMember="UI.TextArrangementType/TextLast"/>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="SAP__common.Text" Path="WarehouseDescription"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.TextArrangement" EnumMember="UI.TextArrangementType/TextFirst"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let prod = ot.properties.iter().find(|p| p.name == "Product").unwrap();
        // Per-property override wins.
        assert_eq!(prod.text_arrangement, Some(TextArrangement::TextLast));
        let wh = ot.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        // Warehouse had no override → picks up the type default.
        assert_eq!(wh.text_arrangement, Some(TextArrangement::TextFirst));
    }

    #[test]
    fn test_v2_sap_value_list_marker() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx" Version="1.0">
  <edmx:DataServices xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata" m:DataServiceVersion="2.0">
    <Schema xmlns="http://schemas.microsoft.com/ado/2008/09/edm" Namespace="n" xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String" sap:value-list="standard"/>
        <Property Name="Category" Type="Edm.String" sap:value-list="fixed-values"/>
        <Property Name="Plain" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let wh = ot.properties.iter().find(|p| p.name == "Warehouse").unwrap();
        assert_eq!(wh.sap_value_list.as_deref(), Some("standard"));
        let cat = ot.properties.iter().find(|p| p.name == "Category").unwrap();
        assert_eq!(cat.sap_value_list.as_deref(), Some("fixed-values"));
        let plain = ot.properties.iter().find(|p| p.name == "Plain").unwrap();
        assert!(plain.sap_value_list.is_none());
    }

    #[test]
    fn test_v4_field_control_hidden_hidden_filter_and_v2_display_format() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self" xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="ChangedAt" Type="Edm.DateTime" sap:display-format="Date"/>
        <Property Name="Amount" Type="Edm.Decimal" sap:display-format="NonNegative"/>
        <Property Name="Status" Type="Edm.String"/>
        <Property Name="InternalCode" Type="Edm.String"/>
        <Property Name="AuxKey" Type="Edm.String"/>
        <Property Name="DynControl" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Status">
        <Annotation Term="SAP__common.FieldControl" EnumMember="Common.FieldControlType/Mandatory"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/InternalCode">
        <Annotation Term="SAP__UI.Hidden"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/AuxKey">
        <Annotation Term="SAP__UI.HiddenFilter"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/DynControl">
        <Annotation Term="SAP__common.FieldControl" Path="SomeStatus"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        let status = ot.properties.iter().find(|p| p.name == "Status").unwrap();
        assert!(matches!(status.field_control, Some(FieldControl::Mandatory)));
        let internal = ot.properties.iter().find(|p| p.name == "InternalCode").unwrap();
        assert!(internal.hidden);
        let aux = ot.properties.iter().find(|p| p.name == "AuxKey").unwrap();
        assert!(aux.hidden_filter);
        let dyn_ctrl = ot.properties.iter().find(|p| p.name == "DynControl").unwrap();
        match dyn_ctrl.field_control.as_ref() {
            Some(FieldControl::Path(p)) => assert_eq!(p, "SomeStatus"),
            other => panic!("expected Path, got {:?}", other),
        }
        let changed = ot.properties.iter().find(|p| p.name == "ChangedAt").unwrap();
        assert_eq!(changed.display_format.as_deref(), Some("Date"));
        let amount = ot.properties.iter().find(|p| p.name == "Amount").unwrap();
        assert_eq!(amount.display_format.as_deref(), Some("NonNegative"));
    }

    #[test]
    fn test_v4_extended_capabilities_on_entity_set() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <NavigationProperty Name="_Items" Type="Collection(n.OrderItemType)"/>
        <NavigationProperty Name="_Serial" Type="Collection(n.SerialType)"/>
      </EntityType>
      <EntityType Name="OrderItemType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityType Name="SerialType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.Container/Orders">
        <Annotation Term="SAP__capabilities.SearchRestrictions">
          <Record>
            <PropertyValue Property="Searchable" Bool="false"/>
          </Record>
        </Annotation>
        <Annotation Term="SAP__capabilities.CountRestrictions">
          <Record>
            <PropertyValue Property="Countable" Bool="false"/>
          </Record>
        </Annotation>
        <Annotation Term="SAP__capabilities.ExpandRestrictions">
          <Record>
            <PropertyValue Property="Expandable" Bool="true"/>
            <PropertyValue Property="NonExpandableProperties">
              <Collection>
                <NavigationPropertyPath>_Serial</NavigationPropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="SAP__capabilities.TopSupported" Bool="false"/>
        <Annotation Term="SAP__capabilities.SkipSupported" Bool="false"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        assert_eq!(ot.searchable, Some(false));
        assert_eq!(ot.countable, Some(false));
        assert_eq!(ot.expandable, Some(true));
        assert_eq!(ot.top_supported, Some(false));
        assert_eq!(ot.skip_supported, Some(false));
        assert_eq!(ot.non_expandable_properties, vec!["_Serial"]);
    }

    #[test]
    fn test_v4_selection_presentation_variant_extracts_inline_sv_and_pv() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Status" Type="Edm.String"/>
        <Property Name="CreatedAt" Type="Edm.DateTimeOffset"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionPresentationVariant" Qualifier="Pending">
          <Record>
            <PropertyValue Property="Text" String="Pending Orders"/>
            <PropertyValue Property="SelectionVariant">
              <Record>
                <PropertyValue Property="SelectOptions">
                  <Collection>
                    <Record Type="SAP__UI.SelectOptionType">
                      <PropertyValue Property="PropertyName" PropertyPath="Status"/>
                      <PropertyValue Property="Ranges">
                        <Collection>
                          <Record Type="SAP__UI.SelectionRangeType">
                            <PropertyValue Property="Sign" EnumMember="SAP__UI.SelectionRangeSignType/I"/>
                            <PropertyValue Property="Option" EnumMember="SAP__UI.SelectionRangeOptionType/EQ"/>
                            <PropertyValue Property="Low" String="PENDING"/>
                          </Record>
                        </Collection>
                      </PropertyValue>
                    </Record>
                  </Collection>
                </PropertyValue>
              </Record>
            </PropertyValue>
            <PropertyValue Property="PresentationVariant">
              <Record>
                <PropertyValue Property="SortOrder">
                  <Collection>
                    <Record>
                      <PropertyValue Property="Property" PropertyPath="CreatedAt"/>
                      <PropertyValue Property="Descending" Bool="true"/>
                    </Record>
                  </Collection>
                </PropertyValue>
              </Record>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        // SPV's inline SV should have been added as a qualified variant.
        assert_eq!(ot.selection_variants.len(), 1);
        let v = &ot.selection_variants[0];
        assert_eq!(v.qualifier.as_deref(), Some("Pending"));
        assert_eq!(v.text.as_deref(), Some("Pending Orders"));
        assert_eq!(v.select_options.len(), 1);
        assert_eq!(v.select_options[0].property_name, "Status");
        // SPV's inline PV should have fed SortOrder.
        assert_eq!(ot.sort_order.len(), 1);
        assert_eq!(ot.sort_order[0].property, "CreatedAt");
        assert!(ot.sort_order[0].descending);
    }

    #[test]
    fn test_v4_selection_presentation_variant_path_refs_are_skipped() {
        // When SPV only references peer annotations via Path, we don't
        // double-count — the standalone SV dispatch already picks them
        // up.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Status" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionPresentationVariant" Qualifier="Pending">
          <Record>
            <PropertyValue Property="SelectionVariant" Path="@SAP__UI.SelectionVariant#Pending"/>
            <PropertyValue Property="PresentationVariant" Path="@SAP__UI.PresentationVariant#Pending"/>
          </Record>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionVariant" Qualifier="Pending">
          <Record>
            <PropertyValue Property="Text" String="Pending"/>
            <PropertyValue Property="SelectOptions">
              <Collection>
                <Record Type="SAP__UI.SelectOptionType">
                  <PropertyValue Property="PropertyName" PropertyPath="Status"/>
                  <PropertyValue Property="Ranges">
                    <Collection>
                      <Record Type="SAP__UI.SelectionRangeType">
                        <PropertyValue Property="Sign" EnumMember="SAP__UI.SelectionRangeSignType/I"/>
                        <PropertyValue Property="Option" EnumMember="SAP__UI.SelectionRangeOptionType/EQ"/>
                        <PropertyValue Property="Low" String="PENDING"/>
                      </Record>
                    </Collection>
                  </PropertyValue>
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
        // Only the one standalone SV should be present — SPV's path
        // references don't spawn a duplicate.
        assert_eq!(ot.selection_variants.len(), 1);
        assert_eq!(ot.selection_variants[0].qualifier.as_deref(), Some("Pending"));
    }

    #[test]
    fn test_v4_selection_variant_default_and_qualified() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="Status" Type="Edm.String"/>
        <Property Name="NetAmount" Type="Edm.Decimal"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionVariant" Qualifier="Pending">
          <Record>
            <PropertyValue Property="Text" String="Pending Orders"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="UI.ParameterType">
                  <PropertyValue Property="PropertyName" PropertyPath="Warehouse"/>
                  <PropertyValue Property="PropertyValue" String="HB01"/>
                </Record>
              </Collection>
            </PropertyValue>
            <PropertyValue Property="SelectOptions">
              <Collection>
                <Record Type="UI.SelectOptionType">
                  <PropertyValue Property="PropertyName" PropertyPath="Status"/>
                  <PropertyValue Property="Ranges">
                    <Collection>
                      <Record Type="UI.SelectionRangeType">
                        <PropertyValue Property="Sign" EnumMember="UI.SelectionRangeSignType/I"/>
                        <PropertyValue Property="Option" EnumMember="UI.SelectionRangeOptionType/EQ"/>
                        <PropertyValue Property="Low" String="PENDING"/>
                      </Record>
                      <Record Type="UI.SelectionRangeType">
                        <PropertyValue Property="Sign" EnumMember="UI.SelectionRangeSignType/I"/>
                        <PropertyValue Property="Option" EnumMember="UI.SelectionRangeOptionType/EQ"/>
                        <PropertyValue Property="Low" String="HOLD"/>
                      </Record>
                    </Collection>
                  </PropertyValue>
                </Record>
                <Record Type="UI.SelectOptionType">
                  <PropertyValue Property="PropertyName" PropertyPath="NetAmount"/>
                  <PropertyValue Property="Ranges">
                    <Collection>
                      <Record Type="UI.SelectionRangeType">
                        <PropertyValue Property="Sign" EnumMember="UI.SelectionRangeSignType/I"/>
                        <PropertyValue Property="Option" EnumMember="UI.SelectionRangeOptionType/BT"/>
                        <PropertyValue Property="Low" Int="100"/>
                        <PropertyValue Property="High" Int="1000"/>
                      </Record>
                    </Collection>
                  </PropertyValue>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionVariant">
          <Record>
            <PropertyValue Property="Text" String="All"/>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        // Default variant (no qualifier) is first.
        assert_eq!(ot.selection_variants.len(), 2);
        assert!(ot.selection_variants[0].qualifier.is_none());
        assert_eq!(ot.selection_variants[0].text.as_deref(), Some("All"));
        let v = &ot.selection_variants[1];
        assert_eq!(v.qualifier.as_deref(), Some("Pending"));
        assert_eq!(v.text.as_deref(), Some("Pending Orders"));
        assert_eq!(v.parameters.len(), 1);
        assert_eq!(v.parameters[0].property_name, "Warehouse");
        assert_eq!(v.parameters[0].property_value, "HB01");
        assert_eq!(v.select_options.len(), 2);
        let status = &v.select_options[0];
        assert_eq!(status.property_name, "Status");
        assert_eq!(status.ranges.len(), 2);
        assert_eq!(status.ranges[0].sign, SelectionSign::I);
        assert_eq!(status.ranges[0].option, SelectionOption::Eq);
        assert_eq!(status.ranges[0].low, "PENDING");
        let amount = &v.select_options[1];
        assert_eq!(amount.property_name, "NetAmount");
        assert_eq!(amount.ranges[0].option, SelectionOption::Bt);
        assert_eq!(amount.ranges[0].low, "100");
        assert_eq!(amount.ranges[0].high.as_deref(), Some("1000"));
    }

    #[test]
    fn test_v4_presentation_variant_request_at_least_and_sort_order() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="WarehouseTimeZone" Type="Edm.String"/>
        <Property Name="EWMWarehouse" Type="Edm.String"/>
        <Property Name="Product" Type="Edm.String"/>
        <Property Name="EWMStorageBin" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.PresentationVariant">
          <Record>
            <PropertyValue Property="RequestAtLeast">
              <Collection>
                <PropertyPath>WarehouseTimeZone</PropertyPath>
                <PropertyPath>EWMWarehouse</PropertyPath>
              </Collection>
            </PropertyValue>
            <PropertyValue Property="SortOrder">
              <Collection>
                <Record>
                  <PropertyValue Property="Property" PropertyPath="Product"/>
                </Record>
                <Record>
                  <PropertyValue Property="Property" PropertyPath="EWMStorageBin"/>
                  <PropertyValue Property="Descending" Bool="true"/>
                </Record>
              </Collection>
            </PropertyValue>
            <PropertyValue Property="Visualizations">
              <Collection>
                <AnnotationPath>@UI.LineItem</AnnotationPath>
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
        assert_eq!(ot.request_at_least, vec!["WarehouseTimeZone", "EWMWarehouse"]);
        assert_eq!(ot.sort_order.len(), 2);
        assert_eq!(ot.sort_order[0].property, "Product");
        assert!(!ot.sort_order[0].descending);
        assert_eq!(ot.sort_order[1].property, "EWMStorageBin");
        assert!(ot.sort_order[1].descending);
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
