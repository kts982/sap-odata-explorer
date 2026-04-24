use std::collections::HashMap;
use serde::Serialize;

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
}

#[derive(Debug, Clone, Serialize)]
pub struct EntityType {
    pub name: String,
    pub keys: Vec<String>,
    pub properties: Vec<Property>,
    pub nav_properties: Vec<NavigationProperty>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Property {
    pub name: String,
    pub edm_type: String,
    pub nullable: bool,
    pub max_length: Option<u32>,
    pub label: Option<String>,
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
        return Err(crate::error::ODataError::MetadataParse("no Schema element found".into()));
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

    // Merge data from all schemas
    for schema_node in &schema_nodes {
        entity_types.extend(parse_entity_types(schema_node, version));
        associations.extend(parse_associations(schema_node));

        let (sets, funcs) = parse_entity_container(schema_node, version);
        entity_sets.extend(sets);
        function_imports.extend(funcs);

        if version == ODataVersion::V4 {
            annotation_labels.extend(parse_v4_annotation_labels(schema_node));
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
    let container = match children_by_tag(schema, "EntityContainer").into_iter().next() {
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
    fn test_v4_annotation_labels() {
        let meta = parse_metadata(TEST_METADATA_V4).unwrap();
        let st = meta.find_entity_type("WarehouseStorageTypeType").unwrap();
        let wh_prop = st.properties.iter().find(|p| p.name == "EWMWarehouse").unwrap();
        assert_eq!(wh_prop.label.as_deref(), Some("Warehouse Number"));
        let st_prop = st.properties.iter().find(|p| p.name == "EWMStorageType").unwrap();
        assert_eq!(st_prop.label.as_deref(), Some("Storage Type"));
    }
}
