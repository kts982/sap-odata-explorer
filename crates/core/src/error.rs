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

    #[error("response parse error: {0}")]
    ResponseParse(String),

    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
}
