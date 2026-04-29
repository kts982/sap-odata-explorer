//! Smoke test for the wiremock-based integration test harness.
//!
//! Validates that we can spin up a mock server, point a `SapClient` at it,
//! mock the two requests `fetch_metadata` performs (session establishment
//! GET on the service root, then the `$metadata` GET), and successfully
//! parse the served document into a `ServiceMetadata`. Future fixtures
//! and scenarios layer on top of this same harness.

mod common;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use sap_odata_core::metadata::ODataVersion;

const TINY_V4_METADATA: &str = include_str!("fixtures/v4/tiny.xml");

#[tokio::test]
async fn fetch_metadata_parses_minimal_v4() {
    let server = MockServer::start().await;

    // ensure_session() does a GET on the service root with X-CSRF-Token: Fetch
    // before any other request. We don't constrain the header here — the
    // smoke test cares about the happy path, not auth/CSRF mechanics
    // (those land in 4d).
    Mock::given(method("GET"))
        .and(path("/sap/opu/odata4/test/SrvD"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/sap/opu/odata4/test/SrvD/$metadata"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(TINY_V4_METADATA),
        )
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let metadata = client
        .fetch_metadata("/sap/opu/odata4/test/SrvD")
        .await
        .expect("metadata should parse");

    assert!(matches!(metadata.version, ODataVersion::V4));
    assert_eq!(metadata.schema_namespace, "TestService");
    assert!(
        metadata
            .entity_types
            .iter()
            .any(|e| e.name == "TestEntity"),
        "expected TestEntity in {:?}",
        metadata.entity_types.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    assert!(
        metadata
            .entity_sets
            .iter()
            .any(|s| s.name == "TestEntities"),
        "expected TestEntities entity set in {:?}",
        metadata.entity_sets.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}
