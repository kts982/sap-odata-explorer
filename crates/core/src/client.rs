use reqwest::{
    cookie::Jar,
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client, Response, StatusCode,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, instrument};

use crate::auth::{AuthConfig, SapConnection};
use crate::error::ODataError;
use crate::metadata::{self, ServiceMetadata};
use crate::query::ODataQuery;

/// SAP OData HTTP client handling authentication, CSRF tokens, and SAP-specific headers.
pub struct SapClient {
    connection: SapConnection,
    http: Client,
    cookie_jar: Arc<Jar>,
    csrf_token: Arc<Mutex<Option<String>>>,
    session_established: Arc<Mutex<bool>>,
}

impl SapClient {
    /// Create a new SAP client from connection parameters.
    pub fn new(connection: SapConnection) -> Result<Self, ODataError> {
        let mut default_headers = HeaderMap::new();
        default_headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        default_headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let redirect_policy = if matches!(&connection.auth, AuthConfig::Sso | AuthConfig::Browser) {
            // For SSO/browser auth: don't auto-follow redirects. Windows SSO
            // handles them manually with Negotiate headers, while browser SSO
            // uses redirects as the signal that the embedded login step is
            // still missing or expired.
            reqwest::redirect::Policy::none()
        } else {
            reqwest::redirect::Policy::default()
        };

        let cookie_jar = Arc::new(Jar::default());
        let http = Client::builder()
            .default_headers(default_headers)
            .cookie_provider(cookie_jar.clone())
            .danger_accept_invalid_certs(connection.insecure_tls)
            .redirect(redirect_policy)
            .build()?;

        Ok(Self {
            connection,
            http,
            cookie_jar,
            csrf_token: Arc::new(Mutex::new(None)),
            session_established: Arc::new(Mutex::new(false)),
        })
    }

    /// Build the full URL for a request, including sap-client and sap-language.
    fn build_url(&self, path: &str) -> String {
        let base_url = self.connection.service_url(path);
        let sep = if base_url.contains('?') { '&' } else { '?' };
        format!(
            "{base_url}{sep}sap-client={}&sap-language={}",
            self.connection.client, self.connection.language
        )
    }

    /// Apply authentication headers to a request builder.
    /// For SSO: only sends the Negotiate token if the session is not yet established.
    /// Once the session cookie is set (after ensure_session), cookies handle auth.
    fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, ODataError> {
        match &self.connection.auth {
            AuthConfig::Basic { username, password } => {
                Ok(builder.basic_auth(username, Some(password)))
            }
            AuthConfig::Sso => {
                // Always generate the SPNEGO token — the cookie jar will
                // also send session cookies if present, but SAP may need
                // the Negotiate header for different service paths.
                let host = extract_host(&self.connection.base_url);
                let token = crate::sspi::generate_negotiate_token(&host)
                    .map_err(|e| ODataError::AuthFailed(e))?;
                Ok(builder.header("Authorization", format!("Negotiate {token}")))
            }
            AuthConfig::Browser => Ok(builder),
        }
    }

    /// Import browser-session cookies into the internal reqwest jar.
    pub fn import_cookie_strings(
        &self,
        request_url: &str,
        cookies: &[String],
    ) -> Result<(), ODataError> {
        let url = url::Url::parse(request_url)?;
        for cookie in cookies {
            self.cookie_jar.add_cookie_str(cookie, &url);
        }
        Ok(())
    }

    /// Fetch a CSRF token from the server (required for modifying requests).
    #[instrument(skip(self))]
    pub async fn fetch_csrf_token(&self, service_path: &str) -> Result<String, ODataError> {
        let url = self.build_url(service_path);
        debug!("Fetching CSRF token from {url}");

        let req = self
            .http
            .head(&url)
            .header("X-CSRF-Token", "Fetch");
        let req = self.apply_auth(req)?;

        let resp = self.send_with_sso_redirects(req).await?;

        let token = resp
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| ODataError::CsrfFetch("no token in response headers".into()))?;

        debug!("Got CSRF token");
        *self.csrf_token.lock().await = Some(token.clone());
        Ok(token)
    }

    /// Establish a session with the SAP server (CSRF preflight + cookie).
    /// For SSO with Azure AD/SAML, follows the redirect chain manually.
    /// Skips if session is already established. Safe to call multiple times.
    #[instrument(skip(self))]
    pub async fn ensure_session(&self, service_path: &str) -> Result<(), ODataError> {
        {
            let established = self.session_established.lock().await;
            if *established {
                return Ok(());
            }
        }

        let url = self.build_url(service_path);
        debug!("Establishing session via GET {url}");

        let req = self
            .http
            .get(&url)
            .header("X-CSRF-Token", "Fetch");
        let req = self.apply_auth(req)?;

        let resp = self.send_with_sso_redirects(req).await?;
        let status = resp.status();

        if status.is_redirection() && matches!(&self.connection.auth, AuthConfig::Browser) {
            // Check if redirect goes to an IdP (sign-in needed) vs normal SAP redirect
            let location = resp.headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let is_idp_redirect = location.contains("login.microsoftonline.com")
                || location.contains("accounts.ondemand.com")
                || location.contains("/saml2/");
            if is_idp_redirect {
                return Err(ODataError::AuthFailed(
                    "browser sign-in required or session expired".to_string(),
                ));
            }
            // Otherwise it might be a normal SAP redirect — follow it
            debug!("Browser auth: following non-IdP redirect to {}", &location[..location.len().min(100)]);
        }

        // Store CSRF token if returned
        if let Some(token) = resp.headers().get("x-csrf-token").and_then(|v| v.to_str().ok()) {
            if token != "Required" {
                *self.csrf_token.lock().await = Some(token.to_string());
                debug!("Got CSRF token");
            }
        }

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            // Read the body for SAP error details
            let body = resp.text().await.unwrap_or_default();
            let detail = if body.is_empty() {
                String::new()
            } else {
                // Try to extract SAP error message from JSON or XML
                let msg = extract_sap_error(&body).unwrap_or_default();
                if msg.is_empty() { format!("\n  Response: {}", &body[..body.len().min(500)]) }
                else { format!("\n  SAP error: {msg}") }
            };
            return Err(ODataError::AuthFailed(format!(
                "server returned {status}{detail}"
            )));
        }

        if matches!(&self.connection.auth, AuthConfig::Browser) {
            let is_html = resp
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/html"));
            if is_html {
                return Err(ODataError::AuthFailed(
                    "browser sign-in incomplete; received HTML instead of an OData response"
                        .to_string(),
                ));
            }
        }

        *self.session_established.lock().await = true;
        debug!("Session established (status: {status})");
        Ok(())
    }

    /// Fetch the $metadata document for a service and parse it.
    #[instrument(skip(self))]
    pub async fn fetch_metadata(&self, service_path: &str) -> Result<ServiceMetadata, ODataError> {
        // Establish session first to avoid 403 on first request
        self.ensure_session(service_path).await?;

        let metadata_path = format!("{}/$metadata", service_path.trim_end_matches('/'));
        let url = self.build_url(&metadata_path);
        debug!("Fetching metadata from {url}");

        let req = self
            .http
            .get(&url)
            .header(ACCEPT, "application/xml");
        let req = self.apply_auth(req)?;

        let resp = self.send_with_sso_redirects(req).await?;
        check_status(&resp)?;

        let xml = resp.text().await?;
        metadata::parse_metadata(&xml)
    }

    /// Execute an OData query and return the raw response.
    #[instrument(skip(self))]
    pub async fn execute_query(
        &self,
        service_path: &str,
        query: &ODataQuery,
    ) -> Result<Response, ODataError> {
        self.ensure_session(service_path).await?;
        let query_path = format!("{}/{}", service_path.trim_end_matches('/'), query.build());
        let url = self.build_url(&query_path);
        debug!("Executing query: {url}");

        let req = self.http.get(&url);
        let req = self.apply_auth(req)?;

        let resp = self.send_with_sso_redirects(req).await?;
        check_status(&resp)?;
        Ok(resp)
    }

    /// Execute an OData query and return the response body as JSON.
    pub async fn query_json(
        &self,
        service_path: &str,
        query: &ODataQuery,
    ) -> Result<serde_json::Value, ODataError> {
        let resp = self.execute_query(service_path, query).await?;
        let json: serde_json::Value = resp.json().await?;
        Ok(json)
    }

    /// Get a raw URL and return the response body as text.
    pub async fn get_raw(&self, service_path: &str, path: &str) -> Result<String, ODataError> {
        self.ensure_session(service_path).await?;
        let url = self.build_url(path);
        let req = self.http.get(&url);
        let req = self.apply_auth(req)?;
        let resp = self.send_with_sso_redirects(req).await?;
        check_status(&resp)?;
        Ok(resp.text().await?)
    }

    /// For SSO: send a request and follow redirects manually with Negotiate auth.
    async fn send_with_sso_redirects(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<Response, ODataError> {
        let mut resp = req.send().await?;

        if !matches!(&self.connection.auth, AuthConfig::Sso | AuthConfig::Browser) {
            return Ok(resp);
        }

        for _ in 0..10 {
            if !resp.status().is_redirection() {
                break;
            }
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(String::from);

            let Some(location) = location else { break };
            debug!("SSO redirect -> {}", &location[..location.len().min(120)]);

            let redirect_url = if location.starts_with("http") {
                location
            } else {
                let base = resp.url().clone();
                base.join(&location)
                    .map(|u| u.to_string())
                    .unwrap_or(location)
            };

            let redirect_host = url::Url::parse(&redirect_url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default();

            let mut redirect_req = self.http.get(&redirect_url);

            // For SPNEGO SSO: add Negotiate token to each redirect hop
            // For Browser SSO: cookies from the jar are sent automatically
            if matches!(&self.connection.auth, AuthConfig::Sso) && !redirect_host.is_empty() {
                if let Ok(token) = crate::sspi::generate_negotiate_token(&redirect_host) {
                    debug!("SSO: Negotiate -> {redirect_host}");
                    redirect_req = redirect_req.header("Authorization", format!("Negotiate {token}"));
                }
            }

            resp = redirect_req.send().await?;
        }

        Ok(resp)
    }

    pub fn connection(&self) -> &SapConnection {
        &self.connection
    }
}

fn check_status(resp: &Response) -> Result<(), ODataError> {
    let status = resp.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ODataError::AuthFailed(format!(
            "server returned {status}"
        )));
    }
    if status == StatusCode::NOT_FOUND {
        return Err(ODataError::ServiceNotFound(
            resp.url().path().to_string(),
        ));
    }
    if status.is_client_error() || status.is_server_error() {
        return Err(ODataError::Http(
            resp.error_for_status_ref().unwrap_err(),
        ));
    }
    Ok(())
}

/// Try to extract a meaningful error message from a SAP error response body.
fn extract_sap_error(body: &str) -> Option<String> {
    // Try JSON: { "error": { "message": { "value": "..." } } }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(msg) = json
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.get("value"))
            .and_then(|v| v.as_str())
        {
            return Some(msg.to_string());
        }
    }
    // Try XML: look for <message>...</message>
    if let Ok(doc) = roxmltree::Document::parse(body) {
        for node in doc.descendants() {
            if node.has_tag_name("message") {
                if let Some(text) = node.text() {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

/// Extract hostname (without port) from a URL for SPN generation.
fn extract_host(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| u.host_str().unwrap_or(url).to_string())
        .unwrap_or_else(|_| url.to_string())
}
