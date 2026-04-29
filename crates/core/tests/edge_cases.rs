//! Edge cases: SAP error envelopes, invalid XML, unusual key shapes,
//! function imports.
//!
//! Each scenario inlines a small XML/JSON fixture rather than spinning up
//! a dedicated file under `fixtures/` — these payloads are small and the
//! parser-shape under test is the focus, not the realism of the metadata.

mod common;

use sap_odata_core::error::ODataError;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SERVICE_PATH: &str = "/sap/opu/odata/test";

// ── SAP error envelopes ───────────────────────────────────────────────────

#[tokio::test]
async fn extracts_message_from_v4_json_error_envelope() {
    // OData V4/V2 error JSON shape: { "error": { "message": { "value": "..." } } }
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(ResponseTemplate::new(500).set_body_raw(
            r#"{"error":{"code":"BACKEND_ERROR","message":{"lang":"en","value":"Service implementation raised exception"}}}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let err = client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect_err("500 should fail");
    match err {
        ODataError::ResponseParse(msg) => {
            assert!(
                msg.contains("Service implementation raised exception"),
                "expected SAP error to be surfaced, got: {msg}"
            );
        }
        other => panic!("expected ResponseParse, got {other:?}"),
    }
}

#[tokio::test]
async fn extracts_message_from_xml_error_envelope() {
    // Some V2 services emit XML errors:
    // <error><code>...</code><message xml:lang="en">...</message></error>
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(ResponseTemplate::new(500).set_body_raw(
            r#"<?xml version="1.0"?>
<error xmlns="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
  <code>SY/530</code>
  <message xml:lang="en">Resource not found for the segment 'foo'</message>
</error>"#,
            "application/xml",
        ))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let err = client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect_err("500 with XML error should fail");
    match err {
        ODataError::ResponseParse(msg) => {
            assert!(
                msg.contains("Resource not found for the segment 'foo'"),
                "expected XML <message> to be extracted, got: {msg}"
            );
        }
        other => panic!("expected ResponseParse, got {other:?}"),
    }
}

#[tokio::test]
async fn http_404_surfaces_as_service_not_found() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let err = client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect_err("404 should fail");
    assert!(
        matches!(err, ODataError::ServiceNotFound(_)),
        "expected ServiceNotFound, got {err:?}"
    );
}

// ── Malformed metadata ────────────────────────────────────────────────────

#[tokio::test]
async fn invalid_xml_in_metadata_surfaces_as_metadata_parse_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            // Unclosed tag → not valid XML
            "<edmx:Edmx xmlns:edmx=\"http://docs.oasis-open.org/odata/ns/edmx\" Version=\"4.0\"><broken",
            "application/xml",
        ))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let err = client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect_err("invalid XML should fail");
    assert!(
        matches!(err, ODataError::MetadataParse(_)),
        "expected MetadataParse, got {err:?}"
    );
}

// ── Unusual key shapes ────────────────────────────────────────────────────

const COMPOSITE_KEY_V4: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="CompositeKey">
      <EntityType Name="OrderLine">
        <Key>
          <PropertyRef Name="OrderID"/>
          <PropertyRef Name="LineNumber"/>
        </Key>
        <Property Name="OrderID" Type="Edm.String" Nullable="false"/>
        <Property Name="LineNumber" Type="Edm.Int32" Nullable="false"/>
        <Property Name="Material" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="C">
        <EntitySet Name="OrderLines" EntityType="CompositeKey.OrderLine"/>
      </EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

#[tokio::test]
async fn parses_composite_key_in_source_order() {
    let meta = common::fetch_parsed_metadata(SERVICE_PATH, COMPOSITE_KEY_V4).await;
    let line = meta
        .entity_types
        .iter()
        .find(|e| e.name == "OrderLine")
        .expect("OrderLine entity type");
    assert_eq!(
        line.keys,
        vec!["OrderID", "LineNumber"],
        "composite key must preserve source order"
    );
}

const KEYLESS_V4: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="Keyless">
      <!-- Function-result entity type — V4 allows entity types without
           a <Key> when used only as the return type of a function. The
           parser must not crash. -->
      <EntityType Name="Summary">
        <Property Name="TotalCount" Type="Edm.Int64"/>
        <Property Name="GeneratedAt" Type="Edm.DateTimeOffset"/>
      </EntityType>
      <EntityContainer Name="C"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

#[tokio::test]
async fn parses_entity_type_without_keys_without_panicking() {
    let meta = common::fetch_parsed_metadata(SERVICE_PATH, KEYLESS_V4).await;
    let summary = meta
        .entity_types
        .iter()
        .find(|e| e.name == "Summary")
        .expect("Summary entity type");
    assert!(
        summary.keys.is_empty(),
        "keyless EntityType must yield empty keys, got {:?}",
        summary.keys
    );
    assert_eq!(summary.properties.len(), 2);
}

// ── Function imports ──────────────────────────────────────────────────────

const V2_FUNCTION_IMPORT: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx Version="1.0" xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx">
  <edmx:DataServices m:DataServiceVersion="2.0"
                     xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
    <Schema Namespace="ZF" xmlns="http://schemas.microsoft.com/ado/2008/09/edm"
            xmlns:sap="http://www.sap.com/Protocols/SAPData">
      <EntityType Name="Order">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="C" m:IsDefaultEntityContainer="true">
        <EntitySet Name="OrderSet" EntityType="ZF.Order"/>
        <FunctionImport Name="GetTopOrders" m:HttpMethod="GET" ReturnType="Collection(ZF.Order)">
          <Parameter Name="TopN" Type="Edm.Int32" Mode="In"/>
          <Parameter Name="MinAmount" Type="Edm.Decimal" Mode="In" Nullable="true"/>
        </FunctionImport>
      </EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

#[tokio::test]
async fn parses_v2_function_import_with_parameters() {
    let meta = common::fetch_parsed_metadata(SERVICE_PATH, V2_FUNCTION_IMPORT).await;
    assert_eq!(meta.function_imports.len(), 1);
    let fi = &meta.function_imports[0];
    assert_eq!(fi.name, "GetTopOrders");
    assert_eq!(fi.http_method, "GET");
    assert_eq!(fi.return_type.as_deref(), Some("Collection(ZF.Order)"));
    assert_eq!(fi.parameters.len(), 2);
    assert_eq!(fi.parameters[0].name, "TopN");
    assert_eq!(fi.parameters[1].name, "MinAmount");
}
