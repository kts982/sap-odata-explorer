use serde::{Deserialize, Serialize};

/// Authentication configuration for SAP system connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AuthConfig {
    /// Basic username/password authentication.
    Basic { username: String, password: String },
    /// SSO via SPNEGO/Negotiate (Windows SSPI). No credentials needed.
    Sso,
    /// Browser-based SSO (Azure AD / SAP IAS / SAML). Session cookies are injected at runtime.
    Browser,
}

/// SAP system connection parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SapConnection {
    /// Base URL of the SAP system (e.g., https://myhost:44300)
    pub base_url: String,
    /// SAP client number (e.g., "100")
    pub client: String,
    /// Language key (e.g., "EN")
    #[serde(default = "default_language")]
    pub language: String,
    /// Authentication configuration
    pub auth: AuthConfig,
    /// Disable TLS certificate verification (for self-signed certs)
    #[serde(default)]
    pub insecure_tls: bool,
    /// When `auth` is `Sso`, allow the SAP server to impersonate the user to
    /// downstream services (Kerberos constrained delegation). Off by default —
    /// only enable for landscapes that need multi-hop auth (reverse-proxy
    /// fronted Gateway → backend R/3).
    #[serde(default)]
    pub sso_delegate: bool,
}

fn default_language() -> String {
    "EN".to_string()
}

impl SapConnection {
    pub fn service_url(&self, service_path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = service_path.trim_start_matches('/');
        format!("{base}/{path}")
    }
}
