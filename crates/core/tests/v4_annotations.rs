//! V4 CSDL annotation parser coverage.
//!
//! Drives the parser through the network layer using the wiremock harness
//! and asserts each annotation lands on the right field of the parsed
//! `ServiceMetadata` / `EntityType` / `Property`. Each `#[tokio::test]`
//! focuses on one annotation aspect so a parser regression names the
//! exact thing that broke.
//!
//! The fixture (`fixtures/v4/warehouse.xml`) is shaped after real SAP
//! services like UI_PHYSSTOCKPROD_1 — header info + line item +
//! selection variants on the entity, capabilities + criticality + value
//! list refs scattered across properties.

mod common;

use sap_odata_core::metadata::{
    Criticality, FieldControl, ODataVersion, SelectionOption, SelectionSign, ServiceMetadata,
};

const SERVICE_PATH: &str = "/sap/opu/odata4/warehouse/SrvD";
const WAREHOUSE_V4: &str = include_str!("fixtures/v4/warehouse.xml");

async fn warehouse_metadata() -> ServiceMetadata {
    common::fetch_parsed_metadata(SERVICE_PATH, WAREHOUSE_V4).await
}

fn warehouse(meta: &ServiceMetadata) -> &sap_odata_core::metadata::EntityType {
    meta.entity_types
        .iter()
        .find(|e| e.name == "Warehouse")
        .expect("Warehouse entity type")
}

fn property<'a>(
    et: &'a sap_odata_core::metadata::EntityType,
    name: &str,
) -> &'a sap_odata_core::metadata::Property {
    et.properties
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("property {name} not found"))
}

#[tokio::test]
async fn parses_v4_basics() {
    let meta = warehouse_metadata().await;
    assert!(matches!(meta.version, ODataVersion::V4));
    assert_eq!(meta.schema_namespace, "WarehouseService");

    let wh = warehouse(&meta);
    assert_eq!(wh.keys, vec!["WarehouseId"]);
    let prop_names: Vec<&str> = wh.properties.iter().map(|p| p.name.as_str()).collect();
    assert!(prop_names.contains(&"WarehouseId"));
    assert!(prop_names.contains(&"Quantity"));
    assert!(prop_names.contains(&"Amount"));

    assert!(meta.entity_sets.iter().any(|s| s.name == "Warehouses"));
}

#[tokio::test]
async fn parses_ui_header_info() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    let hi = wh.header_info.as_ref().expect("UI.HeaderInfo parsed");
    assert_eq!(hi.type_name.as_deref(), Some("Warehouse"));
    assert_eq!(hi.type_name_plural.as_deref(), Some("Warehouses"));
    assert_eq!(hi.title_path.as_deref(), Some("WarehouseName"));
}

#[tokio::test]
async fn parses_ui_line_item_in_order_skipping_non_data_fields() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    // 3 DataField records — DataFieldForAction must be skipped.
    assert_eq!(
        wh.line_item.len(),
        3,
        "expected 3 DataField entries (DataFieldForAction must be skipped), got {:?}",
        wh.line_item
    );

    // Source order preserved.
    assert_eq!(wh.line_item[0].value_path, "WarehouseId");
    assert_eq!(wh.line_item[1].value_path, "WarehouseName");
    assert_eq!(wh.line_item[2].value_path, "ShippingCountry");

    // Label override on the second entry.
    assert_eq!(wh.line_item[0].label, None);
    assert_eq!(wh.line_item[1].label.as_deref(), Some("Name"));

    // Importance EnumMember on the third entry.
    assert_eq!(wh.line_item[2].importance.as_deref(), Some("High"));
}

#[tokio::test]
async fn parses_ui_selection_fields() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    assert_eq!(wh.selection_fields, vec!["WarehouseId", "ShippingCountry"]);
}

#[tokio::test]
async fn parses_ui_presentation_variant() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(wh.request_at_least, vec!["LastChangedAt"]);
    assert_eq!(wh.sort_order.len(), 1);
    assert_eq!(wh.sort_order[0].property, "LastChangedAt");
    assert!(wh.sort_order[0].descending);
}

#[tokio::test]
async fn parses_ui_selection_variant_with_qualifier_and_range() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(wh.selection_variants.len(), 1);
    let sv = &wh.selection_variants[0];
    assert_eq!(sv.qualifier.as_deref(), Some("Active"));
    assert_eq!(sv.text.as_deref(), Some("Active warehouses"));
    assert_eq!(sv.parameters.len(), 0);

    // One SelectOption on StatusCode with one Include/EQ/1 range.
    assert_eq!(sv.select_options.len(), 1);
    let opt = &sv.select_options[0];
    assert_eq!(opt.property_name, "StatusCode");
    assert_eq!(opt.ranges.len(), 1);
    assert_eq!(opt.ranges[0].sign, SelectionSign::I);
    assert_eq!(opt.ranges[0].option, SelectionOption::Eq);
    assert_eq!(opt.ranges[0].low, "1");
    assert!(opt.ranges[0].high.is_none());
}

#[tokio::test]
async fn parses_common_semantic_key() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    assert_eq!(wh.semantic_keys, vec!["WarehouseId"]);
}

#[tokio::test]
async fn parses_common_label_per_property() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(
        property(wh, "WarehouseName").label.as_deref(),
        Some("Warehouse Name")
    );
    assert_eq!(
        property(wh, "ShippingCountry").label.as_deref(),
        Some("Country")
    );
    // Properties without Common.Label have label = None.
    assert_eq!(property(wh, "WarehouseId").label, None);
}

#[tokio::test]
async fn parses_value_list_references() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    let country = property(wh, "ShippingCountry");
    assert_eq!(
        country.value_list_references,
        vec!["../../CountryVH/$metadata"]
    );
    // Inline value_list isn't declared on this property — only references.
    assert!(country.value_list.is_none());
    assert!(country.value_list_variants.is_empty());
}

#[tokio::test]
async fn parses_ui_criticality_path_and_field_control_mandatory() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    let status = property(wh, "StatusCode");

    match &status.criticality {
        Some(Criticality::Path(p)) => assert_eq!(p, "StatusCode"),
        other => panic!("expected Criticality::Path, got {:?}", other),
    }

    assert!(matches!(
        status.field_control,
        Some(FieldControl::Mandatory)
    ));
}

#[tokio::test]
async fn parses_measures_unit_and_iso_currency() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(
        property(wh, "Quantity").unit_path.as_deref(),
        Some("UnitOfMeasure")
    );
    assert_eq!(
        property(wh, "Amount").iso_currency_path.as_deref(),
        Some("Currency")
    );
}

#[tokio::test]
async fn parses_ui_hidden_marker() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);
    assert!(property(wh, "LastChangedAt").hidden);
    assert!(!property(wh, "WarehouseId").hidden);
}

#[tokio::test]
async fn parses_capabilities_filter_restrictions() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    // RequiredProperties → required_in_filter = Some(true)
    assert_eq!(
        property(wh, "WarehouseId").required_in_filter,
        Some(true),
        "WarehouseId is in Capabilities.FilterRestrictions.RequiredProperties"
    );

    // NonFilterableProperties → filterable = Some(false)
    assert_eq!(
        property(wh, "LastChangedAt").filterable,
        Some(false),
        "LastChangedAt is in Capabilities.FilterRestrictions.NonFilterableProperties"
    );

    // Properties not listed don't have the flag explicitly set.
    assert_eq!(property(wh, "WarehouseName").required_in_filter, None);
    assert_eq!(property(wh, "WarehouseName").filterable, None);
}

#[tokio::test]
async fn parses_capabilities_sort_restrictions() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(
        property(wh, "StatusCode").sortable,
        Some(false),
        "StatusCode is in Capabilities.SortRestrictions.NonSortableProperties"
    );
    assert_eq!(property(wh, "WarehouseId").sortable, None);
}

#[tokio::test]
async fn parses_capabilities_search_and_count_restrictions() {
    let meta = warehouse_metadata().await;
    let wh = warehouse(&meta);

    assert_eq!(wh.searchable, Some(true));
    assert_eq!(wh.countable, Some(false));
    // Top/Skip/Expand were not declared — None means "use OData default".
    assert_eq!(wh.top_supported, None);
    assert_eq!(wh.skip_supported, None);
    assert_eq!(wh.expandable, None);
}

#[tokio::test]
async fn raw_annotations_flat_list_populated() {
    // Every entry that ends up typed should also be present on the flat
    // annotations list. Lets the inspector show "what's actually declared"
    // even when a term hasn't been hoisted to a typed accessor yet.
    let meta = warehouse_metadata().await;
    let terms: Vec<&str> = meta.annotations.iter().map(|a| a.term.as_str()).collect();
    for expected in [
        "UI.HeaderInfo",
        "UI.LineItem",
        "UI.SelectionFields",
        "UI.PresentationVariant",
        "UI.SelectionVariant",
        "Common.SemanticKey",
        "Common.Label",
        "Common.ValueListReferences",
        "UI.Criticality",
        "Common.FieldControl",
        "Measures.Unit",
        "Measures.ISOCurrency",
        "UI.Hidden",
        "Capabilities.FilterRestrictions",
        "Capabilities.SortRestrictions",
        "Capabilities.SearchRestrictions",
        "Capabilities.CountRestrictions",
    ] {
        assert!(
            terms.iter().any(|t| t.ends_with(expected)),
            "expected term {expected} in annotations list (got {:?})",
            terms
        );
    }
}
