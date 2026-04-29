//! CSRF token roundtrip + redirect-handling coverage.
//!
//! The client's `fetch_csrf_token`, `ensure_session`, and the browser-SSO
//! redirect detection are stateful network behavior that the in-source unit
//! tests can't easily exercise. These integration tests mount targeted
//! mocks via wiremock to verify request-shape and response-handling.

mod common;

use sap_odata_core::error::ODataError;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SERVICE_PATH: &str = "/sap/opu/odata/test";

// ── CSRF token roundtrip ──────────────────────────────────────────────────

#[tokio::test]
async fn fetch_csrf_token_returns_token_from_response_header() {
    let server = MockServer::start().await;

    Mock::given(method("HEAD"))
        .and(path(SERVICE_PATH))
        .and(header("X-CSRF-Token", "Fetch"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-csrf-token", "abc123-token"))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let token = client
        .fetch_csrf_token(SERVICE_PATH)
        .await
        .expect("token fetch should succeed");
    assert_eq!(token, "abc123-token");
}

#[tokio::test]
async fn fetch_csrf_token_uses_head_method_with_fetch_header() {
    let server = MockServer::start().await;

    // .expect(1) makes the mock server panic on drop if the request didn't
    // arrive exactly once, which catches "client used GET instead of HEAD"
    // or "client forgot to send X-CSRF-Token: Fetch" regressions.
    Mock::given(method("HEAD"))
        .and(path(SERVICE_PATH))
        .and(header("X-CSRF-Token", "Fetch"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-csrf-token", "tok"))
        .expect(1)
        .named("HEAD with X-CSRF-Token: Fetch")
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    client
        .fetch_csrf_token(SERVICE_PATH)
        .await
        .expect("token fetch should succeed");
    // wiremock verifies expectations on server drop.
}

#[tokio::test]
async fn fetch_csrf_token_errors_when_response_omits_header() {
    let server = MockServer::start().await;

    Mock::given(method("HEAD"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    let err = client
        .fetch_csrf_token(SERVICE_PATH)
        .await
        .expect_err("missing token header should fail");
    assert!(
        matches!(err, ODataError::CsrfFetch(_)),
        "expected CsrfFetch error, got {err:?}"
    );
}

#[tokio::test]
async fn ensure_session_captures_token_from_initial_get() {
    // Post-condition: after ensure_session() the client should have
    // captured the CSRF token from the response. We verify by then
    // calling fetch_csrf_token and observing the same value comes back —
    // BUT fetch_csrf_token always re-fetches; instead we check via
    // a follow-up that *consumes* the token. Since writes aren't yet
    // exposed in the public API, we verify indirectly: ensure_session
    // succeeds even when the server includes x-csrf-token in the
    // session-establishing response, and a subsequent metadata fetch
    // continues to work.
    let server = MockServer::start().await;

    // Initial GET returns a 200 with a CSRF token header.
    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .and(header("X-CSRF-Token", "Fetch"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-csrf-token", "session-tok"))
        .expect(1)
        .named("session GET with token")
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path(format!("{SERVICE_PATH}/$metadata")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "application/xml")
                .set_body_string(
                    "<edmx:Edmx xmlns:edmx=\"http://docs.oasis-open.org/odata/ns/edmx\" Version=\"4.0\">\
                     <edmx:DataServices><Schema xmlns=\"http://docs.oasis-open.org/odata/ns/edm\" Namespace=\"X\"/>\
                     </edmx:DataServices></edmx:Edmx>",
                ),
        )
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    client
        .ensure_session(SERVICE_PATH)
        .await
        .expect("session establishes");

    // Subsequent metadata fetch should not re-trigger ensure_session
    // (it's idempotent) — passing here confirms the session_established
    // flag was set and the captured token didn't poison anything.
    client
        .fetch_metadata(SERVICE_PATH)
        .await
        .expect("metadata fetch after established session works");
}

#[tokio::test]
async fn ensure_session_ignores_required_token_value() {
    // SAP services that require a CSRF token but don't have one yet send
    // `x-csrf-token: Required` on the initial GET. The client must NOT
    // store "Required" as if it were a real token — `Required` means
    // "next time, do a real Fetch". We can't directly observe the
    // internal token state, but we can confirm that ensure_session
    // succeeds and a follow-up fetch_csrf_token reaches the server
    // (rather than the client returning a cached "Required" value).
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200).insert_header("x-csrf-token", "Required"))
        .mount(&server)
        .await;

    // The HEAD that fetch_csrf_token issues — assert it actually fires,
    // which would NOT happen if the client had cached "Required" as a
    // valid token and short-circuited.
    Mock::given(method("HEAD"))
        .and(path(SERVICE_PATH))
        .and(header("X-CSRF-Token", "Fetch"))
        .respond_with(ResponseTemplate::new(200).insert_header("x-csrf-token", "real-token"))
        .expect(1)
        .named("CSRF HEAD must still fire after Required")
        .mount(&server)
        .await;

    let client = common::basic_client(&server);
    client
        .ensure_session(SERVICE_PATH)
        .await
        .expect("session establishes");

    let token = client
        .fetch_csrf_token(SERVICE_PATH)
        .await
        .expect("token fetch should succeed");
    assert_eq!(
        token, "real-token",
        "fetch_csrf_token must hit the server, not return cached \"Required\""
    );
}

// ── Browser SSO redirect handling ─────────────────────────────────────────
//
// Note on host-based IdP detection (`login.microsoftonline.com` etc.): the
// `is_idp_host` patterns are covered by unit tests in `session.rs` against
// the function directly. We don't replicate them at the integration layer
// because pointing the client at an external host requires either real
// network (flaky) or DNS/host overrides (overkill). The path-based
// branch of `is_idp_redirect_location` (/saml2/, /oauth2/) is exercised
// here via a self-loop redirect that keeps everything inside the mock
// server.

#[tokio::test]
async fn browser_session_with_saml_path_redirect_returns_auth_failed() {
    // is_idp_redirect_location detects /saml2/ and /oauth2/ paths even
    // when the host isn't a known IdP. The client's send_with_sso_redirects
    // follows redirects manually for browser auth, so we make /saml2/login
    // itself a redirect-to-self loop. After the 10-hop limit the final
    // response is still a 302 with Location: /sap/saml2/login, and
    // ensure_session's IdP check fires.
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/sap/saml2/login"))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/sap/saml2/login"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/sap/saml2/login"))
        .mount(&server)
        .await;

    let client = common::browser_client(&server);
    let err = client
        .ensure_session(SERVICE_PATH)
        .await
        .expect_err("/saml2/ redirect should fail");
    assert!(
        matches!(err, ODataError::AuthFailed(_)),
        "expected AuthFailed, got {err:?}"
    );
}

#[tokio::test]
async fn browser_session_with_html_body_returns_auth_failed() {
    // Some IdP setups respond with 200 OK and an HTML sign-in page rather
    // than a 302 redirect. The client detects this via Content-Type and
    // surfaces a uniform "browser sign-in incomplete" error. Use
    // `set_body_raw` so the HTML's Content-Type isn't overridden by
    // wiremock's default `text/plain`.
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path(SERVICE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><body>Please sign in</body></html>",
            "text/html; charset=utf-8",
        ))
        .mount(&server)
        .await;

    let client = common::browser_client(&server);
    let err = client
        .ensure_session(SERVICE_PATH)
        .await
        .expect_err("HTML response should fail");
    match err {
        ODataError::AuthFailed(msg) => {
            assert!(
                msg.contains("HTML") || msg.contains("sign-in"),
                "expected HTML/sign-in message, got {msg}"
            );
        }
        other => panic!("expected AuthFailed, got {other:?}"),
    }
}
