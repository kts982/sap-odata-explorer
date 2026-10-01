use thiserror::Error;

#[derive(Debug, Error)]
pub enum ODataError {
    #[error("HTTP error: {0}")]
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
