//! `$metadata` parser entry point and orchestration.
//!
//! Public API: `parse_metadata(xml)` and the model types re-exported from
//! `model`. The XML walking, V2/V4 dispatch, raw-annotation collection,
//! and typed-annotation pass all live here for now. A follow-up split
//! peels the typed-annotation pass into its own `annotations` submodule.

mod annotations;
mod model;

pub use annotations::parse_value_list_mapping_xml;
pub use model::*;

use std::collections::HashMap;

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
                annotations::apply_v4_typed_annotations(
                    &mut entity_types,
                    &entity_sets,
                    schema_node,
                    alias,
                );
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

pub(super) fn children_by_tag<'a>(
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
                    semantic_object: None,
                    masked: false,
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
                semantic_keys: Vec::new(),
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

/// Read a SAP-namespaced boolean attribute (e.g. `sap:filterable="false"`).
/// Returns `None` when the attribute isn't present so the caller can
/// distinguish "unspecified" from "explicitly false".
fn parse_sap_bool(node: &roxmltree::Node, name: &str) -> Option<bool> {
    node.attribute((SAP_DATA_NS, name)).map(|v| v == "true")
}

/// Append every `sap:*` attribute on `node` to the raw annotation list,
/// targeted at `target`. V2-only — V4 services use `<Annotations>` blocks
/// instead.
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
pub(super) fn strip_alias_prefix<'a>(target: &'a str, alias: &str) -> &'a str {
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
            assert!(
                ann.value.is_some(),
                "String-valued annotations must carry value"
            );
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
        let amount = ot
            .properties
            .iter()
            .find(|p| p.name == "NetAmount")
            .unwrap();
        assert_eq!(
            amount.iso_currency_path.as_deref(),
            Some("TransactionCurrency")
        );
        let weight = ot
            .properties
            .iter()
            .find(|p| p.name == "NetWeight")
            .unwrap();
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
        let wh = st
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
        assert_eq!(wh.required_in_filter, Some(true));
        assert_eq!(wh.updatable, Some(false));
        let st_type = st
            .properties
            .iter()
            .find(|p| p.name == "StorageType")
            .unwrap();
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
        let vl = wh.value_list.as_ref().expect("value_list should be parsed");
        assert_eq!(vl.collection_path, "WarehouseValueHelp");
        assert_eq!(vl.label.as_deref(), Some("Warehouse F4"));
        assert_eq!(vl.search_supported, Some(true));
        // Unknown Type "Exotic" should have been skipped.
        assert_eq!(vl.parameters.len(), 4);
        assert!(matches!(
            vl.parameters[0].kind,
            ValueListParameterKind::InOut
        ));
        assert_eq!(
            vl.parameters[0].local_property.as_deref(),
            Some("Warehouse")
        );
        assert_eq!(vl.parameters[0].value_list_property, "Warehouse");
        assert!(matches!(vl.parameters[1].kind, ValueListParameterKind::In));
        assert_eq!(vl.parameters[1].local_property.as_deref(), Some("Plant"));
        assert!(matches!(
            vl.parameters[2].kind,
            ValueListParameterKind::DisplayOnly
        ));
        assert!(vl.parameters[2].local_property.is_none());
        assert_eq!(vl.parameters[2].value_list_property, "Description");
        assert!(matches!(
            vl.parameters[3].kind,
            ValueListParameterKind::Constant
        ));
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
        // Default (no-qualifier) surfaces on `value_list`.
        let default = wh.value_list.as_ref().expect("default variant expected");
        assert!(default.qualifier.is_none());
        assert_eq!(default.collection_path, "WarehouseByKey");
        // Both variants are captured on value_list_variants.
        assert_eq!(wh.value_list_variants.len(), 2);
        assert!(wh.value_list_variants[0].qualifier.is_none());
        assert_eq!(wh.value_list_variants[0].collection_path, "WarehouseByKey");
        assert_eq!(
            wh.value_list_variants[1].qualifier.as_deref(),
            Some("ByDescription")
        );
        assert_eq!(
            wh.value_list_variants[1].collection_path,
            "WarehouseByDescription"
        );
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
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
        let vl = parse_value_list_mapping_xml(xml, "EWMWarehouse").expect("mapping should parse");
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
        let wh = st
            .properties
            .iter()
            .find(|p| p.name == "EWMWarehouse")
            .unwrap();
        assert!(wh.value_list.is_none());
        assert_eq!(wh.value_list_references.len(), 1);
        assert!(wh.value_list_references[0].contains("i_ewm_warehousenumbervh"));
        assert!(!wh.value_list_fixed);
        let sdc = st
            .properties
            .iter()
            .find(|p| p.name == "StockDocumentCategory")
            .unwrap();
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
        // Warehouse had no override → picks up the type default.
        assert_eq!(wh.text_arrangement, Some(TextArrangement::TextFirst));
    }

    #[test]
    fn test_v4_semantic_key_object_and_masked() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="CustomerType">
        <Key><PropertyRef Name="CustomerUUID"/></Key>
        <Property Name="CustomerUUID" Type="Edm.Guid" Nullable="false"/>
        <Property Name="CustomerID" Type="Edm.String"/>
        <Property Name="TaxNumber" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Customers" EntityType="n.CustomerType"/></EntityContainer>
      <Annotations Target="SAP__self.CustomerType">
        <Annotation Term="SAP__common.SemanticKey">
          <Collection>
            <PropertyPath>CustomerID</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.CustomerType/CustomerID">
        <Annotation Term="SAP__common.SemanticObject" String="Customer"/>
      </Annotations>
      <Annotations Target="SAP__self.CustomerType/TaxNumber">
        <Annotation Term="SAP__common.Masked"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ct = meta.find_entity_type("CustomerType").unwrap();
        assert_eq!(ct.semantic_keys, vec!["CustomerID"]);
        let id = ct
            .properties
            .iter()
            .find(|p| p.name == "CustomerID")
            .unwrap();
        assert_eq!(id.semantic_object.as_deref(), Some("Customer"));
        let tax = ct
            .properties
            .iter()
            .find(|p| p.name == "TaxNumber")
            .unwrap();
        assert!(tax.masked);
        let uuid = ct
            .properties
            .iter()
            .find(|p| p.name == "CustomerUUID")
            .unwrap();
        assert!(uuid.semantic_object.is_none());
        assert!(!uuid.masked);
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
        let wh = ot
            .properties
            .iter()
            .find(|p| p.name == "Warehouse")
            .unwrap();
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
        assert!(matches!(
            status.field_control,
            Some(FieldControl::Mandatory)
        ));
        let internal = ot
            .properties
            .iter()
            .find(|p| p.name == "InternalCode")
            .unwrap();
        assert!(internal.hidden);
        let aux = ot.properties.iter().find(|p| p.name == "AuxKey").unwrap();
        assert!(aux.hidden_filter);
        let dyn_ctrl = ot
            .properties
            .iter()
            .find(|p| p.name == "DynControl")
            .unwrap();
        match dyn_ctrl.field_control.as_ref() {
            Some(FieldControl::Path(p)) => assert_eq!(p, "SomeStatus"),
            other => panic!("expected Path, got {:?}", other),
        }
        let changed = ot
            .properties
            .iter()
            .find(|p| p.name == "ChangedAt")
            .unwrap();
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
        assert_eq!(
            ot.selection_variants[0].qualifier.as_deref(),
            Some("Pending")
        );
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
        assert_eq!(
            ot.request_at_least,
            vec!["WarehouseTimeZone", "EWMWarehouse"]
        );
        assert_eq!(ot.sort_order.len(), 2);
        assert_eq!(ot.sort_order[0].property, "Product");
        assert!(!ot.sort_order[0].descending);
        assert_eq!(ot.sort_order[1].property, "EWMStorageBin");
        assert!(ot.sort_order[1].descending);
    }

    #[test]
    fn test_v4_line_item_prefers_unqualified_regardless_of_order() {
        // Qualified variant appears FIRST in the XML but the unqualified
        // one should still win. Previously (pre-fix) first-wins would
        // have stored the "Simplified" columns.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="A" Type="Edm.String"/>
        <Property Name="B" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.LineItem" Qualifier="Simplified">
          <Collection>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="A"/></Record>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.LineItem">
          <Collection>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="A"/></Record>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="B"/></Record>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        assert_eq!(
            ot.line_item.len(),
            2,
            "unqualified variant should win even when it comes second"
        );
        assert_eq!(ot.line_item[0].value_path, "A");
        assert_eq!(ot.line_item[1].value_path, "B");
    }

    #[test]
    fn test_v4_selection_fields_prefers_unqualified_regardless_of_order() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="A" Type="Edm.String"/>
        <Property Name="B" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection><PropertyPath>A</PropertyPath></Collection>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionFields" Qualifier="Extended">
          <Collection>
            <PropertyPath>A</PropertyPath>
            <PropertyPath>B</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let ot = meta.find_entity_type("OrderType").unwrap();
        // Unqualified came first; the qualified "Extended" must NOT
        // overwrite (pre-fix last-wins would have stored [A, B]).
        assert_eq!(ot.selection_fields, vec!["A"]);
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
