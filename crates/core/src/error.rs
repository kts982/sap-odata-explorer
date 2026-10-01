use thiserror::Error;

#[derive(Debug, Error)]
pub enum ODataError {
    #[error("HTTP error: {}", describe_http_error(.0))]
    Http(#[from] reqwest::Error),

    #[error("metadata parse error: {0}")]
    MetadataParse(String),

    #[error("CSRF token fetch failed: {0}")]
    CsrfFetch(String),

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("entity not found: {0}")]
    EntityNotFound(String),

    #[error("service not found: {0}")]
    ServiceNotFound(String),

    /// Non-success HTTP status ("server returned 400 Bad Request …") or
    /// an unparseable body ("invalid JSON response: …"). The message is
    /// self-describing; no prefix, since most of these are not parse
    /// failures at all.
    #[error("{0}")]
    ResponseParse(String),

    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
}

/// reqwest's own Display stops at "error sending request for url (…)",
/// hiding *why*. Append the innermost cause (connection refused, DNS
/// failure, certificate problem …) and say so plainly on timeouts.
fn describe_http_error(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut root: &dyn std::error::Error = e;
    while let Some(next) = root.source() {
        root = next;
    }
    let cause = root.to_string();
    if !std::ptr::addr_eq(root, e as &dyn std::error::Error) && !text.contains(&cause) {
        text.push_str(": ");
        text.push_str(&cause);
    }
    if e.is_timeout() {
        text.push_str(&format!(
            " — no response from the server in time (set {} to raise the read timeout, 0 disables it)",
            crate::client::READ_TIMEOUT_ENV
        ));
    }
    text
}
