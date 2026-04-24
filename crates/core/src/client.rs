use reqwest::{
    Client, Response, StatusCode,
    cookie::Jar,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use std::{sync::Arc, time::Instant};
use tokio::sync::Mutex;
use tracing::{debug, instrument};

use crate::auth::{AuthConfig, SapConnection};
use crate::diagnostics::{
    DiagnosticsStore, HttpTraceEntry, request_headers_for_diagnostics,
    response_headers_for_diagnostics,
};
use crate::error::ODataError;
use crate::metadata::{self, ServiceMetadata};
use crate::query::ODataQuery;

/// SAP OData HTTP client handling authentication, CSRF tokens, and SAP-specific headers.
pub struct SapClient {
    connection: SapConnection,
    http: Client,
    cookie_jar: Arc<Jar>,
    diagnostics: Arc<DiagnosticsStore>,
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
            diagnostics: Arc::new(DiagnosticsStore::default()),
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

    /// Return the current request/response diagnostics snapshot for this client.
    pub fn diagnostics_snapshot(&self) -> Vec<HttpTraceEntry> {
        self.diagnostics.snapshot()
    }

    /// Clear any diagnostics currently stored for this client.
    pub fn clear_diagnostics(&self) {
        self.diagnostics.clear();
    }

    /// Try to load a persisted Browser SSO session from the OS keyring
    /// and inject its cookies into this client. Returns true if a session
    /// was loaded, false if none is stored or the stored session belongs to
    /// a different SAP target (stale — automatically cleared).
    pub fn try_load_persisted_session(&self, profile_name: &str) -> Result<bool, ODataError> {
        if !matches!(&self.connection.auth, AuthConfig::Browser) {
            return Ok(false);
        }
        let fingerprint = crate::session::connection_fingerprint(
            &self.connection.base_url,
            &self.connection.client,
            &self.connection.language,
        );
        match crate::session::load_for_connection(profile_name, &fingerprint) {
            Ok(Some(session)) => {
                self.import_cookie_strings(&session.request_url, &session.cookies)?;
                debug!(
                    "Loaded persisted Browser SSO session for '{}' ({} cookies)",
                    profile_name,
                    session.cookies.len()
                );
                Ok(true)
            }
            Ok(None) => Ok(false),
            Err(e) => Err(ODataError::AuthFailed(format!(
                "failed to load persisted session: {e}"
            ))),
        }
    }

    /// Fetch a CSRF token from the server (required for modifying requests).
    #[instrument(skip(self))]
    pub async fn fetch_csrf_token(&self, service_path: &str) -> Result<String, ODataError> {
        let url = self.build_url(service_path);
        debug!("Fetching CSRF token from {url}");

        let req = self.http.head(&url).header("X-CSRF-Token", "Fetch");
        let req = self.apply_auth(req)?;

        let (resp, _) = self.send_with_sso_redirects(req).await?;

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

        let req = self.http.get(&url).header("X-CSRF-Token", "Fetch");
        let req = self.apply_auth(req)?;

        let (resp, trace_id) = self.send_with_sso_redirects(req).await?;
        let status = resp.status();
        let response_url = resp.url().to_string();
        let response_content_type = response_content_type(&resp);

        if status.is_redirection() && matches!(&self.connection.auth, AuthConfig::Browser) {
            // Check if redirect goes to an IdP (sign-in needed) vs normal SAP redirect
            let location = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            let is_idp_redirect = crate::session::is_idp_redirect_location(location);
            if is_idp_redirect {
                return Err(ODataError::AuthFailed(
                    "browser sign-in required or session expired".to_string(),
                ));
            }
            // Otherwise it might be a normal SAP redirect — follow it
            debug!(
                "Browser auth: following non-IdP redirect to {}",
                &location[..location.len().min(100)]
            );
        }

        // Store CSRF token if returned
        if let Some(token) = resp
            .headers()
            .get("x-csrf-token")
            .and_then(|v| v.to_str().ok())
        {
            if token != "Required" {
                *self.csrf_token.lock().await = Some(token.to_string());
                debug!("Got CSRF token");
            }
        }

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let body = self
                .read_response_text(trace_id, resp)
                .await
                .unwrap_or_default();
            return Err(ODataError::AuthFailed(auth_error_detail(
                status,
                &response_url,
                response_content_type.as_deref(),
                &body,
                &self.connection.auth,
            )));
        }

        if matches!(&self.connection.auth, AuthConfig::Browser) {
            let is_html = response_content_type
                .as_deref()
                .is_some_and(|v| v.starts_with("text/html"));
            if is_html {
                // Consume the body so the preview is captured in the trace,
                // then surface a uniform message. The captured preview is
                // stripped for HTML (see diagnostics::body_preview_for_diagnostics)
                // so there's nothing useful to interpolate into the error here.
                let _ = self.read_response_text(trace_id, resp).await;
                return Err(ODataError::AuthFailed(
                    "browser sign-in incomplete; SAP or the IdP returned HTML instead of OData. \
                     Re-run sign-in from the desktop app and check the HTTP inspector for redirects."
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

        let req = self.http.get(&url).header(ACCEPT, "application/xml");
        let req = self.apply_auth(req)?;

        let (resp, trace_id) = self.send_with_sso_redirects(req).await?;
        let xml = self.read_text_response(resp, trace_id).await?;
        metadata::parse_metadata(&xml)
    }

    /// Execute an OData query and return the response body as JSON.
    pub async fn query_json(
        &self,
        service_path: &str,
        query: &ODataQuery,
    ) -> Result<serde_json::Value, ODataError> {
        self.ensure_session(service_path).await?;
        let query_path = format!("{}/{}", service_path.trim_end_matches('/'), query.build());
        let url = self.build_url(&query_path);
        debug!("Executing query: {url}");

        let req = self.http.get(&url);
        let req = self.apply_auth(req)?;

        let (resp, trace_id) = self.send_with_sso_redirects(req).await?;
        let text = self.read_text_response(resp, trace_id).await?;
        serde_json::from_str(&text)
            .map_err(|e| ODataError::ResponseParse(format!("invalid JSON response: {e}")))
    }

    /// Get a raw URL and return the response body as text.
    pub async fn get_raw(&self, service_path: &str, path: &str) -> Result<String, ODataError> {
        self.ensure_session(service_path).await?;
        let url = self.build_url(path);
        let req = self.http.get(&url);
        let req = self.apply_auth(req)?;
        let (resp, trace_id) = self.send_with_sso_redirects(req).await?;
        self.read_text_response(resp, trace_id).await
    }

    /// For SSO: send a request and follow redirects manually with Negotiate auth.
    async fn send_with_sso_redirects(
        &self,
        req: reqwest::RequestBuilder,
    ) -> Result<(Response, u64), ODataError> {
        let request = req.build()?;
        let (mut resp, mut last_trace_id) = self.execute_traced_request(request).await?;

        if !matches!(&self.connection.auth, AuthConfig::Sso | AuthConfig::Browser) {
            return Ok((resp, last_trace_id));
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
                    redirect_req =
                        redirect_req.header("Authorization", format!("Negotiate {token}"));
                }
            }

            let request = redirect_req.build()?;
            let (next_resp, trace_id) = self.execute_traced_request(request).await?;
            resp = next_resp;
            last_trace_id = trace_id;
        }

        Ok((resp, last_trace_id))
    }

    pub fn connection(&self) -> &SapConnection {
        &self.connection
    }

    async fn execute_traced_request(
        &self,
        request: reqwest::Request,
    ) -> Result<(Response, u64), ODataError> {
        let method = request.method().to_string();
        let url = request.url().to_string();
        let request_headers = request_headers_for_diagnostics(request.headers());
        let started = Instant::now();

        match self.http.execute(request).await {
            Ok(resp) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                let redirect_location = resp
                    .headers()
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .map(String::from);
                let trace_id = self.diagnostics.push(HttpTraceEntry {
                    id: 0,
                    method,
                    url,
                    request_headers,
                    request_body_preview: None,
                    status: Some(resp.status().as_u16()),
                    response_headers: response_headers_for_diagnostics(resp.headers()),
                    response_body_preview: None,
                    duration_ms,
                    redirect_location,
                    error: None,
                });
                Ok((resp, trace_id))
            }
            Err(err) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                self.diagnostics.push(HttpTraceEntry {
                    id: 0,
                    method,
                    url,
                    request_headers,
                    request_body_preview: None,
                    status: None,
                    response_headers: Vec::new(),
                    response_body_preview: None,
                    duration_ms,
                    redirect_location: None,
                    error: Some(err.to_string()),
                });
                Err(err.into())
            }
        }
    }

    async fn read_text_response(
        &self,
        resp: Response,
        trace_id: u64,
    ) -> Result<String, ODataError> {
        let status = resp.status();
        let url = resp.url().to_string();
        let content_type = response_content_type(&resp);
        let text = self.read_response_text(trace_id, resp).await?;

        if status.is_success() {
            return Ok(text);
        }

        Err(response_error(
            status,
            &url,
            content_type.as_deref(),
            &text,
            &self.connection.auth,
        ))
    }

    async fn read_response_text(
        &self,
        trace_id: u64,
        resp: Response,
    ) -> Result<String, ODataError> {
        let content_type = response_content_type(&resp);
        let text = resp.text().await?;
        self.diagnostics
            .set_response_body_preview(trace_id, content_type.as_deref(), &text);
        Ok(text)
    }
}

fn response_content_type(resp: &Response) -> Option<String> {
    resp.headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(String::from)
}

fn response_error(
    status: StatusCode,
    url: &str,
    content_type: Option<&str>,
    body: &str,
    auth: &AuthConfig,
) -> ODataError {
    if status == StatusCode::NOT_FOUND {
        let path = url::Url::parse(url)
            .ok()
            .map(|parsed| parsed.path().to_string())
            .unwrap_or_else(|| url.to_string());
        let hint = response_hint(status, url, content_type, body, auth);
        return ODataError::ServiceNotFound(match hint {
            Some(hint) => format!("{path}\n  Hint: {hint}"),
            None => path,
        });
    }

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return ODataError::AuthFailed(auth_error_detail(status, url, content_type, body, auth));
    }

    let mut detail = format!("server returned {status}");
    if let Some(msg) = extract_sap_error(body) {
        detail.push_str(&format!("\n  SAP error: {msg}"));
    }
    if let Some(hint) = response_hint(status, url, content_type, body, auth) {
        detail.push_str(&format!("\n  Hint: {hint}"));
    }
    ODataError::ResponseParse(detail)
}

fn auth_error_detail(
    status: StatusCode,
    url: &str,
    content_type: Option<&str>,
    body: &str,
    auth: &AuthConfig,
) -> String {
    let mut detail = format!("server returned {status}");
    if let Some(msg) = extract_sap_error(body) {
        detail.push_str(&format!("\n  SAP error: {msg}"));
    }
    if let Some(hint) = response_hint(status, url, content_type, body, auth) {
        detail.push_str(&format!("\n  Hint: {hint}"));
    }
    detail
}

fn response_hint(
    status: StatusCode,
    url: &str,
    content_type: Option<&str>,
    body: &str,
    auth: &AuthConfig,
) -> Option<String> {
    let path = url::Url::parse(url)
        .ok()
        .map(|parsed| parsed.path().to_ascii_lowercase())
        .unwrap_or_else(|| url.to_ascii_lowercase());
    let content_type = content_type.unwrap_or_default().to_ascii_lowercase();

    if status == StatusCode::NOT_FOUND {
        if path.contains("/iwfnd/catalogservice") {
            return Some(
                "SAP Gateway catalog service was not found. Check SICF activation for /IWFND/CATALOGSERVICE and that the base URL points at the right system."
                    .to_string(),
            );
        }
        if path.contains("/sap/opu/odata") || path.contains("/$metadata") {
            return Some(
                "The OData service path may be wrong, inactive, or not registered in /IWFND/MAINT_SERVICE."
                    .to_string(),
            );
        }
    }

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        if matches!(auth, AuthConfig::Browser) && content_type.starts_with("text/html") {
            return Some(
                "Browser SSO usually returned a login page instead of an OData response. Re-run sign-in from the desktop app and inspect the request log for redirects."
                    .to_string(),
            );
        }
        if path.contains("/iwfnd/catalogservice") {
            return Some(
                "Access to the SAP Gateway catalog was denied. Check SAP authorizations for catalog discovery and that the user can reach /IWFND/CATALOGSERVICE."
                    .to_string(),
            );
        }
        if matches!(auth, AuthConfig::Basic { .. }) {
            return Some(
                "Basic authentication was accepted by the HTTP stack but SAP rejected the request. Check the user, password, and service authorization."
                    .to_string(),
            );
        }
        return Some(
            "Authentication succeeded at the transport layer but SAP denied access. Check the active session, assigned roles, and any front-door SSO policy."
                .to_string(),
        );
    }

    if status.is_server_error() {
        if path.contains("/sap/opu/odata") {
            return Some(
                "SAP returned a server-side error. Check /IWFND/ERROR_LOG, ST22, and backend application logs for the failing service."
                    .to_string(),
            );
        }
        if !body.is_empty() && content_type.starts_with("text/html") {
            return Some(
                "An HTML error page was returned instead of OData. This often means a reverse proxy, SSO gateway, or SAP ICF error page intercepted the request."
                    .to_string(),
            );
        }
    }

    None
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
