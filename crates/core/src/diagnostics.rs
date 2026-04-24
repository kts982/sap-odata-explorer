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
    let sensitive = match (kind, name.to_ascii_lowercase().as_str()) {
        (HeaderKind::Request, "authorization") => true,
        (HeaderKind::Request, "cookie") => true,
        (HeaderKind::Response, "set-cookie") => true,
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
}
