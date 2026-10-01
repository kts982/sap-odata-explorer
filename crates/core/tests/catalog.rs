//! Catalog resolution against mocked SAP Gateway catalogs: resolving a
//! service by technical name through the V2 and V4 catalogs.

mod common;

use sap_odata_core::catalog::{fetch_service_catalog, resolve_service_by_name};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const V2_CATALOG: &str = "/sap/opu/odata/IWFND/CATALOGSERVICE;v=2";
const V4_CATALOG: &str = "/sap/opu/odata4/iwfnd/config/default/iwfnd/catalog/0002";
const ORDERS_PATH: &str = "/sap/opu/odata4/zns/ui_orders_o4/srvd/zns/ui_orders_o4/0001";

async fn mount_json(server: &MockServer, at: String, body: serde_json::Value) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Session probes for both catalogs plus an empty V2 service list.
async fn mount_catalog_roots(server: &MockServer) {
    for root in [V2_CATALOG, V4_CATALOG] {
        Mock::given(method("GET"))
            .and(path(root))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
    mount_json(
        server,
        format!("{V2_CATALOG}/ServiceCollection"),
        serde_json::json!({ "d": { "results": [] } }),
    )
    .await;
}

#[tokio::test]
async fn resolves_namespaced_v4_group_from_the_expanded_catalog() {
    let server = MockServer::start().await;
    mount_catalog_roots(&server).await;
    mount_json(
        &server,
        format!("{V4_CATALOG}/ServiceGroups"),
        serde_json::json!({ "value": [{
            "GroupId": "/ZNS/UI_ORDERS_O4",
            "Description": "Orders",
            "DefaultSystem": { "Services": [{ "ServiceUrl": format!("{ORDERS_PATH}/") }] }
        }]}),
    )
    .await;

    let client = common::basic_client(&server);
    let resolved = resolve_service_by_name(&client, "/zns/ui_orders_o4")
        .await
        .expect("namespaced V4 group must resolve");
    assert_eq!(resolved, ORDERS_PATH);
}

#[tokio::test]
async fn per_group_lookup_percent_encodes_the_namespaced_key() {
    // Catalog entry without an expanded ServiceUrl forces the per-group
    // request. Before the fix it sent `ServiceGroups('/ZNS/…')` raw, which
    // SAP rejected with 400.
    let server = MockServer::start().await;
    mount_catalog_roots(&server).await;
    mount_json(
        &server,
        format!("{V4_CATALOG}/ServiceGroups"),
        serde_json::json!({ "value": [{
            "GroupId": "/ZNS/UI_ORDERS_O4",
            "Description": "Orders",
            "DefaultSystem": { "Services": [] }
        }]}),
    )
    .await;
    mount_json(
        &server,
        format!("{V4_CATALOG}/ServiceGroups('%2FZNS%2FUI_ORDERS_O4')/DefaultSystem/Services"),
        serde_json::json!({ "value": [{ "ServiceUrl": ORDERS_PATH }] }),
    )
    .await;

    let client = common::basic_client(&server);
    let resolved = resolve_service_by_name(&client, "/ZNS/UI_ORDERS_O4")
        .await
        .expect("encoded per-group lookup must resolve");
    assert_eq!(resolved, ORDERS_PATH);
}

#[tokio::test]
async fn failed_group_lookup_is_reported_not_swallowed() {
    let server = MockServer::start().await;
    mount_catalog_roots(&server).await;
    mount_json(
        &server,
        format!("{V4_CATALOG}/ServiceGroups"),
        serde_json::json!({ "value": [{
            "GroupId": "ZGROUP",
            "Description": "x",
            "DefaultSystem": { "Services": [] }
        }]}),
    )
    .await;
    // No mock for the per-group request → wiremock answers 404.

    let client = common::basic_client(&server);
    let err = resolve_service_by_name(&client, "ZGROUP")
        .await
        .expect_err("no URL anywhere must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("V4 group 'ZGROUP'"),
        "lookup failure must be named: {msg}"
    );
}

#[tokio::test]
async fn catalog_listing_survives_multibyte_text() {
    // Localized descriptions with multi-byte characters around the
    // debug-log truncation point must not panic the listing.
    let server = MockServer::start().await;
    for root in [V2_CATALOG, V4_CATALOG] {
        Mock::given(method("GET"))
            .and(path(root))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    let long_title = format!("{}ß Lagerverwaltung", "a".repeat(450));
    mount_json(
        &server,
        format!("{V2_CATALOG}/ServiceCollection"),
        serde_json::json!({ "d": { "results": [{
            "Title": long_title,
            "TechnicalServiceName": "ZLAGER_SRV",
            "TechnicalServiceVersion": "1",
            "Description": "Übersicht",
            "ServiceUrl": "/sap/opu/odata/sap/ZLAGER_SRV"
        }]}}),
    )
    .await;

    let client = common::basic_client(&server);
    let result = fetch_service_catalog(&client).await.unwrap();
    assert_eq!(result.entries.len(), 1);
    assert_eq!(result.entries[0].description, "Übersicht");
}
