use reqwest::{
    Client, Response, StatusCode,
    cookie::Jar,
    header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderValue},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tracing::{debug, instrument};

use crate::auth::{AuthConfig, SapConnection};
use crate::diagnostics::{
    DiagnosticsStore, HttpTraceEntry, extract_sensitive_request_values,
    request_headers_for_diagnostics, response_headers_for_diagnostics,
};
use crate::error::ODataError;
use crate::metadata::{self, ServiceMetadata};
use crate::query::ODataQuery;

/// Timeout for the connect phase (TCP + TLS handshake to the SAP host).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Default idle timeout between reads of a response, including the wait
/// for the first byte. Bounds a hung work process or a proxy that accepts
/// the connection and never answers, without capping long downloads that
/// keep making progress (it resets after every successful read). Set
/// above SAP ICM's default 60 s processing timeout.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(300);

/// Environment variable overriding [`DEFAULT_READ_TIMEOUT`], in seconds.
/// `0` disables the read timeout.
pub const READ_TIMEOUT_ENV: &str = "SAP_ODATA_READ_TIMEOUT_SECS";

/// Read timeout from [`READ_TIMEOUT_ENV`], falling back to the default.
/// `None` means no read timeout.
pub fn read_timeout_from_env() -> Option<Duration> {
    match std::env::var(READ_TIMEOUT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
    {
        Some(0) => None,
        Some(secs) => Some(Duration::from_secs(secs)),
        None => Some(DEFAULT_READ_TIMEOUT),
    }
}

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
    /// Create a new SAP client from connection parameters. The read
    /// timeout comes from [`read_timeout_from_env`].
    pub fn new(connection: SapConnection) -> Result<Self, ODataError> {
        Self::with_read_timeout(connection, read_timeout_from_env())
    }

    /// Like [`SapClient::new`] with an explicit read timeout (`None` = no
    /// read timeout). The connect timeout is always [`CONNECT_TIMEOUT`].
    pub fn with_read_timeout(
        connection: SapConnection,
        read_timeout: Option<Duration>,
    ) -> Result<Self, ODataError> {
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
        let mut builder = Client::builder()
            .default_headers(default_headers)
            .cookie_provider(cookie_jar.clone())
            .danger_accept_invalid_certs(connection.insecure_tls)
            .redirect(redirect_policy)
            .connect_timeout(CONNECT_TIMEOUT);
        if let Some(timeout) = read_timeout {
            builder = builder.read_timeout(timeout);
        }
        let http = builder.build()?;

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
                let token =
                    crate::sspi::generate_negotiate_token(&host, self.connection.sso_delegate)
                        .map_err(ODataError::AuthFailed)?;
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
                    "browser sign-in required or session expired — run `signout <profile>` and re-sign-in from the desktop app".to_string(),
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
            && token != "Required"
        {
            *self.csrf_token.lock().await = Some(token.to_string());
            debug!("Got CSRF token");
        }

        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            let body = self
                .read_response_text(trace_id, resp)
                .await
                .unwrap_or_default();
            return Err(response_error(
                status,
                &response_url,
                response_content_type.as_deref(),
                &body,
                &self.connection.auth,
            ));
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
                     The session likely expired — run `signout <profile>` and re-sign-in from the desktop app, \
                     then inspect the HTTP trace for redirects."
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
        let xml = self.fetch_metadata_xml(service_path).await?;
        metadata::parse_metadata(&xml)
    }

    /// Fetch `$metadata` as raw XML. Same session/auth handling as
    /// [`fetch_metadata`], but skips parsing — used by diagnostics and by
    /// the CLI's `metadata` command to inspect raw annotation structure.
    pub async fn fetch_metadata_xml(&self, service_path: &str) -> Result<String, ODataError> {
        self.ensure_session(service_path).await?;

        let metadata_path = format!("{}/$metadata", service_path.trim_end_matches('/'));
        let url = self.build_url(&metadata_path);
        debug!("Fetching metadata from {url}");

        let req = self.http.get(&url).header(ACCEPT, "application/xml");
        let req = self.apply_auth(req)?;

        let (resp, trace_id) = self.send_with_sso_redirects(req).await?;
        self.read_text_response(resp, trace_id).await
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
            if matches!(&self.connection.auth, AuthConfig::Sso)
                && !redirect_host.is_empty()
                && let Ok(token) = crate::sspi::generate_negotiate_token(
                    &redirect_host,
                    self.connection.sso_delegate,
                )
            {
                debug!("SSO: Negotiate -> {redirect_host}");
                redirect_req = redirect_req.header("Authorization", format!("Negotiate {token}"));
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
        // Capture raw sensitive values BEFORE the redactor runs and BEFORE
        // self.http.execute(request) consumes the request. These get stored
        // on the trace entry (skip-serialised) so the body preview can
        // defensively redact any echoes of them.
        let sensitive_values = extract_sensitive_request_values(request.headers());
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
                    sensitive_values,
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
                    sensitive_values,
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

/// Browser SSO got an HTML page (the IdP / SAP logon page) instead of
/// OData: the session is gone, whatever the status code says.
fn is_browser_login_page(auth: &AuthConfig, content_type: Option<&str>) -> bool {
    matches!(auth, AuthConfig::Browser)
        && content_type.is_some_and(|ct| ct.to_ascii_lowercase().starts_with("text/html"))
}

/// SADL message for a direct read of an entity set the service exposes
/// only through navigation or `$expand` (RAP's `I_DraftAdministrativeData`,
/// metadata `ReadRestrictions/Readable=false`). V4 sends this message id;
/// V2 sends exception class `CX_SADL_PRIVILEGE_VIOLATION`, which is too
/// broad to key a hint on.
const SADL_NAVIGATION_ONLY: &str = "SADL_GW/012";

/// SAP Gateway answers a request for an unregistered service (wrong
/// path, wrong version, not activated) with 403 and "No service found for
/// namespace …" — not an authorization problem, despite the status.
fn is_unknown_service_403(status: StatusCode, body: &str) -> bool {
    status == StatusCode::FORBIDDEN
        && extract_sap_error(body).is_some_and(|m| m.contains("No service found for namespace"))
}

fn response_error(
    status: StatusCode,
    url: &str,
    content_type: Option<&str>,
    body: &str,
    auth: &AuthConfig,
) -> ODataError {
    if status == StatusCode::NOT_FOUND || is_unknown_service_403(status, body) {
        let path = url::Url::parse(url)
            .ok()
            .map(|parsed| parsed.path().to_string())
            .unwrap_or_else(|| url.to_string());
        let mut detail = path;
        if status == StatusCode::FORBIDDEN
            && let Some(msg) = extract_sap_error(body)
        {
            detail.push_str(&format!("\n  SAP error: {msg}"));
        }
        if let Some(hint) = response_hint(status, url, content_type, body, auth) {
            detail.push_str(&format!("\n  Hint: {hint}"));
        }
        return ODataError::ServiceNotFound(detail);
    }

    // 401 is a refused logon. 403 is only an auth failure when Browser SSO
    // got a login page back; otherwise SAP answers 403 for many reasons
    // that are not about the user's credentials or roles (a set readable
    // only via navigation, an unknown service path, CSRF), and its message
    // says which. Those take the generic path below.
    if status == StatusCode::UNAUTHORIZED
        || (status == StatusCode::FORBIDDEN && is_browser_login_page(auth, content_type))
    {
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

/// Map an HTTP response to a SAP-specific troubleshooting hint, or
/// `None` when the status code doesn't warrant one.
///
/// Public so the desktop / Tauri layer can call it directly on its
/// own error paths without round-tripping through this client's
/// error type — both surfaces stay consistent that way.
///
/// The function is best-effort: it inspects the URL path for SAP
/// SICF nodes, the response content-type for HTML interception, and
/// the body for SAP `<innererror>` codes. None of that is required —
/// missing inputs just narrow what can be suggested.
pub fn response_hint(
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
        if path.contains("/sap/opu/odata4") {
            return Some(
                "V4 OData lookup failed. The service path may be wrong, the entity set may not exist on this service (`entities -s <svc>` lists valid sets), or the V4 catalog SICF node may be unpublished on this system — try pasting the full /sap/opu/odata4/<vendor>/<svc>/<version>/ path instead of the short name. Use `services` to list what's exposed."
                    .to_string(),
            );
        }
        if path.contains("/sap/opu/odata") || path.contains("/$metadata") {
            return Some(
                "The OData service path may be wrong, inactive, or not registered in /IWFND/MAINT_SERVICE. If the service exists, the entity set name may not — use `entities -s <svc>` to list valid sets."
                    .to_string(),
            );
        }
    }

    if is_unknown_service_403(status, body) {
        return Some(
            "SAP Gateway has no service registered under this path. Check the path and its version (`services` lists registered service URLs) and that the service is activated — /IWFND/MAINT_SERVICE for V2, /IWFND/V4_ADMIN for V4. This 403 is not an authorization problem."
                .to_string(),
        );
    }

    let browser = matches!(auth, AuthConfig::Browser);
    if (status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN)
        && browser
        && content_type.starts_with("text/html")
    {
        return Some(
            "Browser SSO usually returned a login page instead of an OData response. Run `signout <profile>` and re-sign-in from the desktop app, then inspect the request log for redirects."
                .to_string(),
        );
    }

    // 401: SAP refused the logon itself, before any authorization check.
    if status == StatusCode::UNAUTHORIZED {
        let hint = match auth {
            AuthConfig::Browser => {
                "Browser SSO session likely expired. Run `signout <profile>` and re-sign-in from the desktop app."
            }
            AuthConfig::Basic { .. } => {
                "SAP refused the logon. Check the profile's user, password and client (`profile add` stores a new password), and that the user is not locked in that client."
            }
            AuthConfig::Sso => {
                "SAP refused the Windows SSO (SPNEGO) logon. `-v` shows what the server answered."
            }
        };
        return Some(hint.to_string());
    }

    // 403: the logon worked, SAP refused this request. The reason is in
    // SAP's message, so no hint second-guesses it with a cause that isn't
    // certain (missing roles / SU53, a wrong password).
    if status == StatusCode::FORBIDDEN {
        if extract_sap_error_code(body).as_deref() == Some(SADL_NAVIGATION_ONLY) {
            return Some(
                "This entity set can only be read through navigation or $expand from its parent entity; the service's metadata marks it as not directly readable. It is not an authorization problem."
                    .to_string(),
            );
        }
        if extract_sap_error(body).is_none() {
            if path.contains("/iwfnd/config/") {
                return Some(
                    "The V4 service catalog refused the request. This is typical when the /IWFND/CONFIG V4 catalog node isn't active; V4 services may still be reachable — pass the full /sap/opu/odata4/... path with -s."
                        .to_string(),
                );
            }
            if browser {
                return Some(
                    "Browser SSO session likely expired. Run `signout <profile>` and re-sign-in from the desktop app."
                        .to_string(),
                );
            }
            return Some(
                "SAP refused the request without an OData error message, so the response doesn't say why. `-v` prints the full exchange."
                    .to_string(),
            );
        }
        // SAP's message is shown with the error; the code hint below
        // points at its long text.
    }

    // 4xx with a SAP error code in the body — point at where the message
    // is maintained. Only fires when there's a parseable code; the
    // generic `server returned 4xx` message already carries the
    // SAP-supplied message text via `extract_sap_error`.
    if status.is_client_error()
        && !status.is_success()
        && let Some(code) = extract_sap_error_code(body)
    {
        if !code.contains('/') && code.starts_with("CX_") {
            return Some(format!(
                "SAP error code {code} is an exception class. SE24 (or ADT) shows its texts and where it is raised."
            ));
        }
        return Some(format!(
            "SAP error code {code}. Look the message up in SE91 (or your IDE's SAP message viewer) for the full text and where-used."
        ));
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

/// Try to extract a SAP message code (e.g. `SY/530`, `/IWBEP/CM_MGW_BUSI/107`)
/// from an OData error envelope. Returns the first code found in either
/// the top-level `error.code` or under `error.innererror`. Codes are
/// strings like `<class>/<number>` — used to suggest SE91 lookups.
fn extract_sap_error_code(body: &str) -> Option<String> {
    // Placeholder codes SAP emits when there's no meaningful diagnostic
    // identifier — pointing users at "SE91 /0" would be worse than
    // showing no hint at all, so treat them as misses.
    fn is_useful_code(code: &str) -> bool {
        !code.is_empty() && code != "/0" && code != "0"
    }

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        let err = json.get("error")?;
        if let Some(code) = err.get("code").and_then(|v| v.as_str())
            && is_useful_code(code)
        {
            return Some(code.to_string());
        }
        // SAP's `innererror.errordetails` is a list — the most actionable
        // code isn't always at index 0 (the first entry can be the
        // placeholder `/0` while a real `SY/530` sits further down).
        // Walk the list and return the first useful code.
        if let Some(details) = err
            .get("innererror")
            .and_then(|i| i.get("errordetails"))
            .and_then(|d| d.as_array())
        {
            for entry in details {
                if let Some(code) = entry.get("code").and_then(|v| v.as_str())
                    && is_useful_code(code)
                {
                    return Some(code.to_string());
                }
            }
        }
    }
    // XML form: <code>SY/530</code> inside <error>.
    if let Ok(doc) = roxmltree::Document::parse(body) {
        for node in doc.descendants() {
            if node.has_tag_name("code")
                && let Some(text) = node.text()
                && is_useful_code(text)
                && text.contains('/')
            {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Try to extract a meaningful error message from a SAP error response body.
fn extract_sap_error(body: &str) -> Option<String> {
    // JSON envelopes:
    // - V2: { "error": { "message": { "lang": "en", "value": "..." } } }
    // - V4: { "error": { "code": "...", "message": "...", "details": [ { "message": "..." } ] } }
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(err) = json.get("error")
    {
        fn non_empty(v: Option<&serde_json::Value>) -> Option<&str> {
            v?.as_str().map(str::trim).filter(|t| !t.is_empty())
        }
        let message = err.get("message");
        let text = non_empty(message)
            .or_else(|| non_empty(message.and_then(|m| m.get("value"))))
            .or_else(|| {
                err.get("details")?
                    .as_array()?
                    .iter()
                    .find_map(|e| non_empty(e.get("message")))
            });
        if let Some(text) = text {
            return Some(text.to_string());
        }
    }
    // Try XML: look for <message>...</message>
    if let Ok(doc) = roxmltree::Document::parse(body) {
        for node in doc.descendants() {
            if node.has_tag_name("message")
                && let Some(text) = node.text()
            {
                return Some(text.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_auth() -> AuthConfig {
        AuthConfig::Basic {
            username: "u".into(),
            password: "p".into(),
        }
    }

    #[test]
    fn hint_404_v4_mentions_unpublished_sicf_and_entity_listing() {
        // Single combined V4 hint covers both service-level and
        // entity-set-level 404s — the URL alone can't reliably
        // distinguish them, so we surface every angle the user can
        // act on.
        let h = response_hint(
            StatusCode::NOT_FOUND,
            "https://example.com/sap/opu/odata4/sap/zsvc/srvd/sap/zsvc/0001/",
            None,
            "",
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("V4"));
        assert!(h.contains("SICF"));
        assert!(h.contains("entities -s"));
    }

    #[test]
    fn hint_404_v2_mentions_entity_set_listing() {
        let h = response_hint(
            StatusCode::NOT_FOUND,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC/SomeBogusSet",
            None,
            "",
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(
            h.contains("entities -s"),
            "expected entity-set listing pointer, got: {h}"
        );
    }

    #[test]
    fn hint_401_basic_points_at_the_logon_not_authorizations() {
        let h = response_hint(
            StatusCode::UNAUTHORIZED,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC",
            None,
            "",
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("password"), "{h}");
        assert!(
            !h.contains("SU53"),
            "a refused logon never reaches an authorization check: {h}"
        );
    }

    /// No 403 hint may name a cause SAP didn't state: not missing roles
    /// (SU53), not the password. SAP answers 403 for structural reasons
    /// too, and its message says which.
    fn assert_no_guessed_cause(h: &str) {
        assert!(!h.contains("SU53"), "{h}");
        assert!(!h.contains("password"), "{h}");
        assert!(!h.to_ascii_lowercase().contains("role"), "{h}");
    }

    #[test]
    fn hint_403_navigation_only_set() {
        // Verbatim shape of a RAP V4 service's answer to a direct read of
        // I_DraftAdministrativeData (with SAP_ALL, so not an authorization).
        let body = r#"{"error":{"code":"SADL_GW/012","message":"The entity I_DRAFTADMINISTRATIVEDATA can only be accessed by navigation or expand","@SAP__common.ExceptionCategory":"Authorization_Error"}}"#;
        let h = response_hint(
            StatusCode::FORBIDDEN,
            "https://example.com/sap/opu/odata4/x/y/srvd/x/y/0001/I_DraftAdministrativeData",
            Some("application/json"),
            body,
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("$expand"), "{h}");
        assert!(h.contains("not an authorization problem"), "{h}");
        assert_no_guessed_cause(&h);
    }

    #[test]
    fn hint_403_with_sap_message_points_at_the_message_only() {
        let body =
            r#"{"error":{"code":"ZCM_ORDERS/042","message":{"lang":"en","value":"Some reason"}}}"#;
        for auth in [basic_auth(), AuthConfig::Sso, AuthConfig::Browser] {
            let h = response_hint(
                StatusCode::FORBIDDEN,
                "https://example.com/sap/opu/odata/sap/ZMY_SVC/Orders",
                Some("application/json"),
                body,
                &auth,
            )
            .expect("expected a hint");
            assert!(h.contains("SE91"), "{h}");
            assert!(
                !h.contains("signout"),
                "SAP answered, the session works: {h}"
            );
            assert_no_guessed_cause(&h);
        }
    }

    #[test]
    fn hint_403_without_sap_message_is_neutral() {
        let h = response_hint(
            StatusCode::FORBIDDEN,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC/Orders",
            Some("text/plain"),
            "Forbidden",
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("-v"), "{h}");
        assert_no_guessed_cause(&h);
    }

    #[test]
    fn hint_403_on_v4_catalog_mentions_the_catalog_node() {
        let h = response_hint(
            StatusCode::FORBIDDEN,
            "https://example.com/sap/opu/odata4/iwfnd/config/default/iwfnd/catalog/0002/ServiceGroups",
            Some("text/html"),
            "<html>Forbidden</html>",
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("/IWFND/CONFIG"), "{h}");
        assert_no_guessed_cause(&h);
    }

    #[test]
    fn hint_exception_class_code_points_at_se24() {
        let body = r#"{"error":{"code":"CX_SADL_PRIVILEGE_VIOLATION","message":{"lang":"en","value":"x"}}}"#;
        let h = response_hint(
            StatusCode::FORBIDDEN,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC/X",
            Some("application/json"),
            body,
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("SE24"), "{h}");
        assert!(!h.contains("SE91"), "{h}");
    }

    #[test]
    fn hint_401_browser_sso_mentions_signout() {
        let h = response_hint(
            StatusCode::UNAUTHORIZED,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC",
            None,
            "",
            &AuthConfig::Browser,
        )
        .expect("expected a hint");
        assert!(
            h.contains("signout"),
            "expected browser-SSO signout pointer, got: {h}"
        );
    }

    #[test]
    fn hint_4xx_with_sap_code_points_at_se91() {
        let body = r#"{"error":{"code":"SY/530","message":{"value":"Something went wrong"}}}"#;
        let h = response_hint(
            StatusCode::BAD_REQUEST,
            "https://example.com/sap/opu/odata/sap/ZMY_SVC/X",
            None,
            body,
            &basic_auth(),
        )
        .expect("expected a hint");
        assert!(h.contains("SY/530"));
        assert!(h.contains("SE91"));
    }

    #[test]
    fn extract_sap_error_code_handles_innererror_details() {
        let body = r#"{"error":{"code":"","innererror":{"errordetails":[{"code":"/IWBEP/CM_MGW/123","message":"x"}]}}}"#;
        assert_eq!(
            extract_sap_error_code(body).as_deref(),
            Some("/IWBEP/CM_MGW/123")
        );
    }

    #[test]
    fn extract_sap_error_code_ignores_placeholder_zero() {
        // Some SAP error envelopes carry `code: "/0"` when there's no
        // meaningful code — skip it rather than suggesting `SE91 /0`.
        let body = r#"{"error":{"code":"/0","message":{"value":"x"}}}"#;
        assert_eq!(extract_sap_error_code(body), None);
    }

    #[test]
    fn extract_sap_error_code_walks_past_placeholder_in_errordetails() {
        // SAP often returns a leading `/0` placeholder followed by the
        // real code. `.first()` would return the placeholder; we need
        // to walk the list.
        let body = r#"{"error":{"code":"","innererror":{"errordetails":[
            {"code":"/0","message":"warm-up"},
            {"code":"SY/530","message":"the real one"}
        ]}}}"#;
        assert_eq!(extract_sap_error_code(body).as_deref(), Some("SY/530"));
    }
}
