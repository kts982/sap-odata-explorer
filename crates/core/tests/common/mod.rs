//! Shared helpers for integration tests.
//!
//! Built on `wiremock` — each test calls `MockServer::start().await` to get
//! its own local server (fresh port, fresh state). The helpers here just
//! glue a `SapClient` to that server's `uri()` so test bodies can focus on
//! the scenario being mocked.

use sap_odata_core::auth::{AuthConfig, SapConnection};
use sap_odata_core::client::SapClient;
use sap_odata_core::metadata::ServiceMetadata;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a `SapConnection` pointed at a wiremock server, using basic auth
/// with throwaway credentials. wiremock doesn't enforce the Authorization
/// header by default, so the password value is irrelevant — tests that
/// care about auth headers should match on them explicitly.
pub fn basic_connection(server: &MockServer) -> SapConnection {
    SapConnection {
        base_url: server.uri(),
        client: "100".to_string(),
        language: "EN".to_string(),
        auth: AuthConfig::Basic {
            username: "test".to_string(),
            password: "test".to_string(),
        },
        insecure_tls: false,
        sso_delegate: false,
    }
}

/// Convenience: build a `SapClient` already pointed at the mock server.
pub fn basic_client(server: &MockServer) -> SapClient {
    SapClient::new(basic_connection(server)).expect("SapClient should build from test connection")
}

/// Mount the two mocks needed to make `fetch_metadata(service_path)` succeed:
///   1. The session-establishment GET on the service root
///      (`ensure_session()` does this once with `X-CSRF-Token: Fetch`).
///   2. The `$metadata` GET that returns `xml_body` with content-type
///      `application/xml`.
///
/// Returned mocks are mounted directly on `server`; the caller doesn't need
/// to do anything else. Used by every annotation-coverage test that doesn't
/// need to constrain auth/CSRF/redirect details.
pub async fn mount_metadata(server: &MockServer, service_path: &str, xml_body: &'static str) {
    Mock::given(method("GET"))
        .and(path(service_path.to_string()))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{service_path}/$metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(xml_body),
        )
        .mount(server)
        .await;
}

/// Spin up a fresh mock server, mount metadata-fetch mocks for the given
/// fixture content, and return the parsed `ServiceMetadata`. Tests that
/// only care about the parsed shape (not the request/response mechanics)
/// use this to skip boilerplate.
///
/// `service_path` should be a leading-slash absolute path (e.g.
/// `/sap/opu/odata4/test/SrvD`).
pub async fn fetch_parsed_metadata(service_path: &str, xml_fixture: &'static str) -> ServiceMetadata {
    let server = MockServer::start().await;
    mount_metadata(&server, service_path, xml_fixture).await;
    let client = basic_client(&server);
    client
        .fetch_metadata(service_path)
        .await
        .expect("metadata should parse")
}
