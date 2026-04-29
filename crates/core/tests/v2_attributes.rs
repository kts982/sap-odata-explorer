//! V2 EDMX `sap:*` inline-attribute parser coverage.
//!
//! V2 services carry the SAP semantics V4 packs into `<Annotations>` blocks
//! as inline attributes on `<Property>` / `<EntityType>` / `<EntitySet>`.
//! Tests here drive a richly-attributed V2 fixture through the wiremock
//! harness and assert each `sap:*` attribute lands on the right field of
//! the parsed `Property`.

mod common;

use sap_odata_core::metadata::{ODataVersion, ServiceMetadata};

const SERVICE_PATH: &str = "/sap/opu/odata/sap/ZSALES_ORDER_SRV";
const SALES_ORDER_V2: &str = include_str!("fixtures/v2/sales_order.xml");

async fn sales_order_metadata() -> ServiceMetadata {
    common::fetch_parsed_metadata(SERVICE_PATH, SALES_ORDER_V2).await
}

fn sales_order(meta: &ServiceMetadata) -> &sap_odata_core::metadata::EntityType {
    meta.entity_types
        .iter()
        .find(|e| e.name == "SalesOrder")
        .expect("SalesOrder entity type")
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
async fn parses_v2_basics() {
    let meta = sales_order_metadata().await;
    assert!(matches!(meta.version, ODataVersion::V2));
    assert_eq!(meta.schema_namespace, "ZSALES_ORDER_SRV");

    let so = sales_order(&meta);
    assert_eq!(so.keys, vec!["OrderID"]);
    assert!(meta.entity_sets.iter().any(|s| s.name == "SalesOrderSet"));
}

#[tokio::test]
async fn parses_sap_label() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(property(so, "OrderID").label.as_deref(), Some("Order"));
    assert_eq!(
        property(so, "MaterialCode").label.as_deref(),
        Some("Material")
    );
    assert_eq!(
        property(so, "DocumentDate").label.as_deref(),
        Some("Doc. Date")
    );
}

#[tokio::test]
async fn parses_sap_text_as_text_path() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(
        property(so, "MaterialCode").text_path.as_deref(),
        Some("MaterialDescription")
    );
    // Properties without sap:text leave text_path = None.
    assert_eq!(property(so, "OrderID").text_path, None);
}

#[tokio::test]
async fn parses_sap_unit_as_unit_path() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(
        property(so, "Quantity").unit_path.as_deref(),
        Some("QuantityUnit")
    );
    // Properties without sap:unit leave unit_path = None.
    assert_eq!(property(so, "MaterialCode").unit_path, None);
}

#[tokio::test]
async fn parses_sap_display_format() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(
        property(so, "DocumentDate").display_format.as_deref(),
        Some("Date")
    );
    assert_eq!(
        property(so, "NetAmount").display_format.as_deref(),
        Some("NonNegative")
    );
    // Properties without the attribute leave display_format = None.
    assert_eq!(property(so, "MaterialCode").display_format, None);
}

#[tokio::test]
async fn parses_sap_value_list_marker() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(
        property(so, "MaterialCode").sap_value_list.as_deref(),
        Some("standard")
    );
    assert_eq!(
        property(so, "CustomerID").sap_value_list.as_deref(),
        Some("standard")
    );
    assert_eq!(
        property(so, "StatusCode").sap_value_list.as_deref(),
        Some("fixed-values")
    );
    assert_eq!(property(so, "OrderID").sap_value_list, None);
}

#[tokio::test]
async fn parses_sap_required_in_filter() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);
    assert_eq!(
        property(so, "CustomerID").required_in_filter,
        Some(true),
        "sap:required-in-filter=\"true\" should set required_in_filter to Some(true)"
    );
    // Properties without the attribute leave it None.
    assert_eq!(property(so, "OrderID").required_in_filter, None);
}

#[tokio::test]
async fn parses_sap_filterable_sortable_creatable_updatable() {
    let meta = sales_order_metadata().await;
    let so = sales_order(&meta);

    // LastChangedAt has every restriction explicitly set to false.
    let lc = property(so, "LastChangedAt");
    assert_eq!(lc.filterable, Some(false));
    assert_eq!(lc.sortable, Some(false));
    assert_eq!(lc.creatable, Some(false));
    assert_eq!(lc.updatable, Some(false));

    // MaterialDescription only flips creatable/updatable.
    let md = property(so, "MaterialDescription");
    assert_eq!(md.creatable, Some(false));
    assert_eq!(md.updatable, Some(false));
    assert_eq!(md.filterable, None, "sap:filterable not set → leave as None");
    assert_eq!(md.sortable, None, "sap:sortable not set → leave as None");

    // DocumentDate has both filterable and sortable explicitly true.
    let dd = property(so, "DocumentDate");
    assert_eq!(dd.filterable, Some(true));
    assert_eq!(dd.sortable, Some(true));

    // OrderID has none of these flags — all four stay None.
    let id = property(so, "OrderID");
    assert_eq!(id.filterable, None);
    assert_eq!(id.sortable, None);
    assert_eq!(id.creatable, None);
    assert_eq!(id.updatable, None);
}

#[tokio::test]
async fn captures_sap_attributes_on_raw_annotation_list() {
    // V2 inline attributes also surface on the raw annotations list as
    // `sap:label` etc. with namespace="SAP". Verify the channel exists for
    // tooling (annotation inspector, lint integrity rules) that walks the
    // flat list rather than typed Property fields.
    let meta = sales_order_metadata().await;
    let has_label = meta
        .annotations
        .iter()
        .any(|a| a.term == "sap:label" && a.namespace == "SAP");
    assert!(
        has_label,
        "expected at least one sap:label entry on the raw annotations list"
    );
}
