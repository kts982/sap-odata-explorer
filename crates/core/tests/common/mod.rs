//! Shared helpers for integration tests.
//!
//! Built on `wiremock` — each test calls `MockServer::start().await` to get
//! its own local server (fresh port, fresh state). The helpers here just
//! glue a `SapClient` to that server's `uri()` so test bodies can focus on
//! the scenario being mocked.

use sap_odata_core::auth::{AuthConfig, SapConnection};
use sap_odata_core::client::SapClient;
use wiremock::MockServer;

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
