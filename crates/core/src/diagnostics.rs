use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

const MAX_TRACE_ENTRIES: usize = 64;
/// Upper bound on the response body we keep per trace entry. Large
/// enough to capture verbose SAP OData error payloads in full (~tens
/// of KB is typical, 100+ KB for batch error dumps) without unbounded
/// growth. The frontend's Inspector shows the first ~4 KB by default
/// and exposes an "expand" action that reveals up to this cap.
const MAX_BODY_PREVIEW_CHARS: usize = 256_000;
const MAX_HEADER_VALUE_CHARS: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHeaderView {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTraceEntry {
    pub id: u64,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<HttpHeaderView>,
    pub request_body_preview: Option<String>,
    pub status: Option<u16>,
    pub response_headers: Vec<HttpHeaderView>,
    pub response_body_preview: Option<String>,
    pub duration_ms: u64,
    pub redirect_location: Option<String>,
    pub error: Option<String>,
    /// Raw values of sensitive request headers (Authorization, Cookie,
    /// X-CSRF-Token) captured at trace-push time. Used to defensively
    /// redact accidental occurrences in body previews — e.g. an error
    /// response that echoes a header back. Never serialised, so it
    /// can't cross the IPC / display boundary.
    #[serde(skip)]
    pub(crate) sensitive_values: Vec<String>,
}

#[derive(Default)]
pub(crate) struct DiagnosticsStore {
    next_id: AtomicU64,
    entries: Mutex<Vec<HttpTraceEntry>>,
}

impl DiagnosticsStore {
    pub(crate) fn push(&self, mut entry: HttpTraceEntry) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        entry.id = id;

        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);
        if entries.len() > MAX_TRACE_ENTRIES {
            let overflow = entries.len() - MAX_TRACE_ENTRIES;
            entries.drain(0..overflow);
        }
        id
    }

    pub(crate) fn set_response_body_preview(
        &self,
        id: u64,
        content_type: Option<&str>,
        body: &str,
    ) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
            let preview = body_preview_for_diagnostics(content_type, body, &entry.sensitive_values);
            entry.response_body_preview = preview;
        }
    }

    pub(crate) fn snapshot(&self) -> Vec<HttpTraceEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub(crate) fn clear(&self) {
        self.entries.lock().unwrap().clear();
    }
}

pub(crate) fn request_headers_for_diagnostics(headers: &HeaderMap) -> Vec<HttpHeaderView> {
    headers_for_diagnostics(headers, HeaderKind::Request)
}

/// Pull the raw values of sensitive request headers so we can defensively
/// redact them from response body previews. Must be called before the
/// request is consumed by the HTTP client.
///
/// For Cookie headers we also split out each `name=value` pair, because a
/// response body could echo just a single cookie even though the full
/// `Cookie:` header carries several.
pub(crate) fn extract_sensitive_request_values(headers: &HeaderMap) -> Vec<String> {
    const SENSITIVE: &[&str] = &["authorization", "cookie", "x-csrf-token"];
    let mut out = Vec::new();
    for (name, value) in headers.iter() {
        let lower = name.as_str().to_ascii_lowercase();
        if SENSITIVE.contains(&lower.as_str())
            && let Ok(v) = value.to_str()
            && !v.is_empty()
        {
            out.push(v.to_string());
            if lower == "cookie" {
                for piece in v.split(';') {
                    let piece = piece.trim();
                    if !piece.is_empty() && piece != v {
                        out.push(piece.to_string());
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn response_headers_for_diagnostics(headers: &HeaderMap) -> Vec<HttpHeaderView> {
    headers_for_diagnostics(headers, HeaderKind::Response)
}

fn headers_for_diagnostics(headers: &HeaderMap, kind: HeaderKind) -> Vec<HttpHeaderView> {
    headers
        .iter()
        .map(|(name, value)| HttpHeaderView {
            name: name.as_str().to_string(),
            value: redact_header(name.as_str(), value.to_str().unwrap_or("<non-utf8>"), kind),
        })
        .collect()
}

fn redact_header(name: &str, value: &str, kind: HeaderKind) -> String {
    // X-CSRF-Token is session-equivalent for SAP — leaking one in a
    // copy-as-curl export is as bad as leaking the cookie.
    let sensitive = matches!(
        (kind, name.to_ascii_lowercase().as_str()),
        (HeaderKind::Request, "authorization")
            | (HeaderKind::Request, "cookie")
            | (HeaderKind::Request, "x-csrf-token")
            | (HeaderKind::Response, "set-cookie")
            | (HeaderKind::Response, "x-csrf-token")
    );

    if sensitive {
        return "<redacted>".to_string();
    }

    truncate(value, MAX_HEADER_VALUE_CHARS)
}

/// Minimum length of a sensitive value we'll bother substring-replacing
/// in a body preview. Short values (a 4-char token, an empty header)
/// would risk mangling unrelated content.
const MIN_REDACTABLE_SECRET_LEN: usize = 8;

fn body_preview_for_diagnostics(
    content_type: Option<&str>,
    body: &str,
    sensitive_values: &[String],
) -> Option<String> {
    if body.is_empty() {
        return None;
    }

    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.starts_with("text/html") {
        return Some("<HTML body omitted from diagnostics>".to_string());
    }

    let mut preview = truncate(body, MAX_BODY_PREVIEW_CHARS);
    for secret in sensitive_values {
        if secret.len() >= MIN_REDACTABLE_SECRET_LEN {
            preview = preview.replace(secret.as_str(), "<redacted>");
        }
    }
    Some(preview)
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, ch) in value.chars().enumerate() {
        if count == max_chars {
            out.push_str("... <truncated>");
            return out;
        }
        out.push(ch);
    }
    out
}

#[derive(Copy, Clone)]
enum HeaderKind {
    Request,
    Response,
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, HeaderValue, SET_COOKIE};

    #[test]
    fn request_headers_redact_auth_and_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        headers.insert(COOKIE, HeaderValue::from_static("MYSAPSSO2=secret"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let rendered = request_headers_for_diagnostics(&headers);
        assert_eq!(rendered[0].value, "<redacted>");
        assert_eq!(rendered[1].value, "<redacted>");
        assert_eq!(rendered[2].value, "application/json");
    }

    #[test]
    fn response_headers_redact_set_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(SET_COOKIE, HeaderValue::from_static("MYSAPSSO2=secret"));

        let rendered = response_headers_for_diagnostics(&headers);
        assert_eq!(rendered[0].value, "<redacted>");
    }

    #[test]
    fn html_body_preview_is_omitted() {
        let preview = body_preview_for_diagnostics(Some("text/html"), "<html>secret</html>", &[]);
        assert_eq!(
            preview.as_deref(),
            Some("<HTML body omitted from diagnostics>")
        );
    }

    #[test]
    fn body_preview_redacts_echoed_authorization_value() {
        // Some servers (or misbehaving proxies) echo request headers back in
        // error responses. Defend against it: any sensitive value we sent is
        // scrubbed from the body preview.
        let secrets = vec!["Bearer Bearer-token-must-not-leak-1234".to_string()];
        let body = r#"{"error":"forbidden","echoed":"Bearer Bearer-token-must-not-leak-1234"}"#;
        let preview =
            body_preview_for_diagnostics(Some("application/json"), body, &secrets).unwrap();
        assert!(!preview.contains("Bearer-token-must-not-leak-1234"));
        assert!(preview.contains("<redacted>"));
    }

    #[test]
    fn body_preview_redacts_echoed_csrf_token() {
        let secrets = vec!["csrf-token-must-not-leak-5678".to_string()];
        let body =
            r#"<error><message>token csrf-token-must-not-leak-5678 expired</message></error>"#;
        let preview =
            body_preview_for_diagnostics(Some("application/xml"), body, &secrets).unwrap();
        assert!(!preview.contains("csrf-token-must-not-leak-5678"));
        assert!(preview.contains("<redacted>"));
    }

    #[test]
    fn body_preview_passes_through_normal_content_unchanged() {
        // Negative test: when sensitive values aren't echoed, the body is
        // shown verbatim. Business data must not be silently mangled.
        let secrets = vec!["Bearer some-token-not-in-body".to_string()];
        let body = r#"{"d":{"results":[{"Material":"MK01-RM-01","Description":"Raw material"}]}}"#;
        let preview =
            body_preview_for_diagnostics(Some("application/json"), body, &secrets).unwrap();
        assert_eq!(preview, body);
    }

    #[test]
    fn body_preview_does_not_redact_short_secrets() {
        // Don't replace 4-char strings in bodies — false-positive risk on
        // ordinary text (e.g. matching "abcd" inside random JSON content).
        // The header redactor still scrubs the header itself; this is just
        // about the substring-in-body fallback.
        let secrets = vec!["abc".to_string(), "xyz1".to_string()];
        let body = r#"{"name":"abc-product","tag":"xyz1"}"#;
        let preview =
            body_preview_for_diagnostics(Some("application/json"), body, &secrets).unwrap();
        assert_eq!(preview, body);
    }

    #[test]
    fn extract_sensitive_request_values_collects_auth_cookie_and_csrf() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer xxx"));
        headers.insert(COOKIE, HeaderValue::from_static("session=yyy"));
        headers.insert("x-csrf-token", HeaderValue::from_static("zzz"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let values = extract_sensitive_request_values(&headers);
        assert_eq!(values.len(), 3);
        assert!(values.contains(&"Bearer xxx".to_string()));
        assert!(values.contains(&"session=yyy".to_string()));
        assert!(values.contains(&"zzz".to_string()));
        assert!(!values.iter().any(|v| v == "application/json"));
    }

    #[test]
    fn request_redacts_x_csrf_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-csrf-token", HeaderValue::from_static("ABC123XYZ"));
        let rendered = request_headers_for_diagnostics(&headers);
        assert_eq!(rendered[0].value, "<redacted>");
    }

    #[test]
    fn response_redacts_x_csrf_token() {
        let mut headers = HeaderMap::new();
        headers.insert("x-csrf-token", HeaderValue::from_static("ABC123XYZ"));
        let rendered = response_headers_for_diagnostics(&headers);
        assert_eq!(rendered[0].value, "<redacted>");
    }

    #[test]
    fn header_redaction_is_case_insensitive() {
        // SAP servers send X-CSRF-Token, X-Csrf-Token, x-csrf-token in the wild.
        // reqwest normalises to lowercase but verify our match is case-insensitive
        // independent of that, so a future change in the HTTP layer doesn't quietly
        // open a leak.
        for name in ["AUTHORIZATION", "Authorization", "authorization"] {
            let v = redact_header(name, "Bearer secret", HeaderKind::Request);
            assert_eq!(v, "<redacted>", "request header {name}");
        }
        for name in ["X-CSRF-TOKEN", "X-Csrf-Token", "x-csrf-token"] {
            let v = redact_header(name, "abc", HeaderKind::Request);
            assert_eq!(v, "<redacted>", "request header {name}");
        }
    }

    #[test]
    fn cookie_with_multiple_values_is_fully_redacted() {
        // Make sure we replace the entire Cookie header value, not just one cookie.
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("sap-usercontext=sap-client=100; MYSAPSSO2=secret; foo=bar"),
        );
        let rendered = request_headers_for_diagnostics(&headers);
        assert_eq!(rendered[0].value, "<redacted>");
        assert!(!rendered[0].value.contains("MYSAPSSO2"));
        assert!(!rendered[0].value.contains("secret"));
    }

    #[test]
    fn various_auth_schemes_all_redacted() {
        for scheme in [
            "Basic dXNlcjpwYXNz",
            "Bearer eyJhbGc",
            "Negotiate YIIH",
            "Kerberos abc",
        ] {
            let v = redact_header("authorization", scheme, HeaderKind::Request);
            assert_eq!(v, "<redacted>", "scheme {scheme}");
            assert!(!v.contains(scheme), "leaked: {scheme}");
        }
    }

    #[test]
    fn non_sensitive_headers_pass_through_unchanged() {
        // Negative test — make sure the redactor isn't over-eager.
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert("accept", HeaderValue::from_static("application/json"));
        headers.insert("sap-client", HeaderValue::from_static("100"));
        headers.insert(
            "x-requested-with",
            HeaderValue::from_static("XMLHttpRequest"),
        );

        let rendered = request_headers_for_diagnostics(&headers);
        let by_name: std::collections::HashMap<_, _> = rendered
            .iter()
            .map(|h| (h.name.as_str(), h.value.as_str()))
            .collect();
        assert_eq!(by_name["content-type"], "application/json");
        assert_eq!(by_name["accept"], "application/json");
        assert_eq!(by_name["sap-client"], "100");
        assert_eq!(by_name["x-requested-with"], "XMLHttpRequest");
    }

    #[test]
    fn full_trace_entry_does_not_leak_sensitive_substrings() {
        // End-to-end: build a realistic request/response with every credential
        // surface populated *including the body echoing the auth header back*,
        // render through the diagnostics pipeline, then scan every rendered
        // field for the secrets. Catches accidental new leak paths if the
        // structure of HttpTraceEntry ever grows fields that bypass redaction.
        const SECRETS: &[&str] = &[
            "Bearer-token-must-not-leak-1234",
            "MYSAPSSO2=secret-cookie-must-not-leak",
            "csrf-token-must-not-leak-5678",
            "set-cookie-must-not-leak-9012",
        ];

        let mut req = HeaderMap::new();
        req.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer Bearer-token-must-not-leak-1234"),
        );
        req.insert(
            COOKIE,
            HeaderValue::from_static("MYSAPSSO2=secret-cookie-must-not-leak; foo=bar"),
        );
        req.insert(
            "x-csrf-token",
            HeaderValue::from_static("csrf-token-must-not-leak-5678"),
        );

        let mut resp = HeaderMap::new();
        resp.insert(
            SET_COOKIE,
            HeaderValue::from_static("MYSAPSSO2=set-cookie-must-not-leak-9012; HttpOnly"),
        );
        resp.insert(
            "x-csrf-token",
            HeaderValue::from_static("csrf-token-must-not-leak-5678"),
        );
        resp.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let sensitive_values = extract_sensitive_request_values(&req);

        // Simulate a misbehaving server that echoes the request's auth header
        // back in the JSON error body — exactly the threat model the body
        // redaction layer is meant to defend against.
        let evil_body = r#"{
  "error": {
    "code": "AUTH_FAILED",
    "message": "received header Bearer Bearer-token-must-not-leak-1234",
    "csrf": "csrf-token-must-not-leak-5678",
    "session": "MYSAPSSO2=secret-cookie-must-not-leak"
  }
}"#;

        let entry = HttpTraceEntry {
            id: 1,
            method: "GET".to_string(),
            url: "https://example/sap/opu/odata4/foo".to_string(),
            request_headers: request_headers_for_diagnostics(&req),
            request_body_preview: None,
            status: Some(403),
            response_headers: response_headers_for_diagnostics(&resp),
            response_body_preview: body_preview_for_diagnostics(
                Some("application/json"),
                evil_body,
                &sensitive_values,
            ),
            duration_ms: 42,
            redirect_location: None,
            error: None,
            sensitive_values,
        };

        let serialised = serde_json::to_string(&entry).expect("serialise trace entry");
        for secret in SECRETS {
            assert!(
                !serialised.contains(secret),
                "secret leaked through diagnostics pipeline: {secret}\nrendered: {serialised}"
            );
        }
    }

    #[test]
    fn sensitive_values_field_is_skipped_in_serialisation() {
        // Belt-and-suspenders: if someone removes the #[serde(skip)] attribute
        // by accident, this test fails immediately. The sensitive_values field
        // must never cross the IPC boundary.
        let entry = HttpTraceEntry {
            id: 1,
            method: "GET".to_string(),
            url: "https://example/sap/opu/odata4/foo".to_string(),
            request_headers: vec![],
            request_body_preview: None,
            status: Some(200),
            response_headers: vec![],
            response_body_preview: None,
            duration_ms: 0,
            redirect_location: None,
            error: None,
            sensitive_values: vec!["this-must-not-appear-in-json".to_string()],
        };
        let serialised = serde_json::to_string(&entry).expect("serialise");
        assert!(!serialised.contains("this-must-not-appear-in-json"));
        assert!(!serialised.contains("sensitive_values"));
    }
}
