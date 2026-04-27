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
        let preview = body_preview_for_diagnostics(content_type, body);
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
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
    let sensitive = match (kind, name.to_ascii_lowercase().as_str()) {
        (HeaderKind::Request, "authorization") => true,
        (HeaderKind::Request, "cookie") => true,
        (HeaderKind::Request, "x-csrf-token") => true,
        (HeaderKind::Response, "set-cookie") => true,
        (HeaderKind::Response, "x-csrf-token") => true,
        _ => false,
    };

    if sensitive {
        return "<redacted>".to_string();
    }

    truncate(value, MAX_HEADER_VALUE_CHARS)
}

fn body_preview_for_diagnostics(content_type: Option<&str>, body: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }

    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();
    if content_type.starts_with("text/html") {
        return Some("<HTML body omitted from diagnostics>".to_string());
    }

    Some(truncate(body, MAX_BODY_PREVIEW_CHARS))
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in value.chars() {
        if count == max_chars {
            out.push_str("... <truncated>");
            return out;
        }
        out.push(ch);
        count += 1;
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
        let preview = body_preview_for_diagnostics(Some("text/html"), "<html>secret</html>");
        assert_eq!(
            preview.as_deref(),
            Some("<HTML body omitted from diagnostics>")
        );
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
        // surface populated, render through the diagnostics pipeline, then scan
        // every rendered field for the secrets. Catches accidental new leak
        // paths if the structure of HttpTraceEntry ever grows fields that bypass
        // redaction.
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

        let entry = HttpTraceEntry {
            id: 1,
            method: "GET".to_string(),
            url: "https://example/sap/opu/odata4/foo".to_string(),
            request_headers: request_headers_for_diagnostics(&req),
            request_body_preview: None,
            status: Some(200),
            response_headers: response_headers_for_diagnostics(&resp),
            response_body_preview: body_preview_for_diagnostics(
                Some("application/json"),
                "{\"d\":{\"results\":[]}}",
            ),
            duration_ms: 42,
            redirect_location: None,
            error: None,
        };

        let serialised = serde_json::to_string(&entry).expect("serialise trace entry");
        for secret in SECRETS {
            assert!(
                !serialised.contains(secret),
                "secret leaked through diagnostics pipeline: {secret}\nrendered: {serialised}"
            );
        }
    }
}
