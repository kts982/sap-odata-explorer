#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    catalog,
    client::SapClient,
    config,
    diagnostics::HttpTraceEntry,
    lint::{self, LintFinding},
    metadata::{
        self, AnnotationSummary, Criticality, FieldControl, HeaderInfo, LineItemField,
        RawAnnotation, SelectionVariant, SortOrder, TextArrangement, ValueList,
    },
    offline::{
        self, ImportOptions, SaveOptions, SaveOutcome, auto_offline_profile_name, current_iso8601,
    },
    query::ODataQuery,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{
    Manager, Url, WebviewUrl, WindowEvent,
    webview::{Cookie, WebviewWindowBuilder},
};

// ── Types for frontend communication ──

#[derive(Serialize)]
struct ProfileInfo {
    name: String,
    /// `"connected"` (live SAP profile) or `"offline"` (offline-mode
    /// EDMX bucket). The frontend's profile picker uses this to render
    /// the `OFFLINE` badge and to disable execute buttons when the
    /// active profile is offline. Backward-compatible: existing JS
    /// that doesn't read `kind` still sees normal profile entries.
    kind: String,
    base_url: String,
    client: String,
    language: String,
    username: String,
    password_source: String,
    auth_mode: String,
    sso_delegate: bool,
    aliases: Vec<AliasInfo>,
    /// Offline-profile attribution: which connected SAP system the
    /// bucket was originally captured from. `None` for connected
    /// profiles and for the catch-all `Imported` bucket.
    #[serde(skip_serializing_if = "Option::is_none")]
    source_profile: Option<String>,
    /// UTC ISO-8601 timestamp when the offline bucket was created.
    /// `None` for connected profiles.
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

#[derive(Serialize)]
struct AliasInfo {
    name: String,
    path: String,
}

#[derive(Serialize)]
struct ServiceInfo {
    technical_name: String,
    title: String,
    description: String,
    service_url: String,
    version: String,
}

#[derive(Serialize)]
struct EntitySetInfo {
    name: String,
    entity_type: String,
    keys: Vec<String>,
}

/// Shape returned by `get_entities`. Bundles the entity-set list with the
/// service-wide annotation summary so the frontend can render the footer
/// badge without a second metadata fetch.
#[derive(Serialize)]
struct EntityListResponse {
    entity_sets: Vec<EntitySetInfo>,
    annotation_summary: AnnotationSummary,
}

#[derive(Serialize)]
struct EntityTypeInfo {
    name: String,
    keys: Vec<String>,
    properties: Vec<PropertyInfo>,
    nav_properties: Vec<NavPropertyInfo>,
    header_info: Option<HeaderInfo>,
    selection_fields: Vec<String>,
    line_item: Vec<LineItemField>,
    request_at_least: Vec<String>,
    sort_order: Vec<SortOrder>,
    selection_variants: Vec<SelectionVariant>,
    searchable: Option<bool>,
    countable: Option<bool>,
    top_supported: Option<bool>,
    skip_supported: Option<bool>,
    expandable: Option<bool>,
    non_expandable_properties: Vec<String>,
    semantic_keys: Vec<String>,
    fiori_readiness: Vec<LintFinding>,
}

#[derive(Serialize)]
struct PropertyInfo {
    name: String,
    edm_type: String,
    nullable: bool,
    max_length: Option<u32>,
    label: Option<String>,
    is_key: bool,
    text_path: Option<String>,
    unit_path: Option<String>,
    iso_currency_path: Option<String>,
    filterable: Option<bool>,
    sortable: Option<bool>,
    creatable: Option<bool>,
    updatable: Option<bool>,
    required_in_filter: Option<bool>,
    criticality: Option<Criticality>,
    value_list: Option<ValueList>,
    value_list_variants: Vec<ValueList>,
    value_list_references: Vec<String>,
    value_list_fixed: bool,
    text_arrangement: Option<TextArrangement>,
    field_control: Option<FieldControl>,
    hidden: bool,
    hidden_filter: bool,
    display_format: Option<String>,
    sap_value_list: Option<String>,
    semantic_object: Option<String>,
    masked: bool,
}

#[derive(Serialize)]
struct NavPropertyInfo {
    name: String,
    target_type: String,
    multiplicity: String,
}

#[derive(Deserialize)]
struct QueryParams {
    entity_set: String,
    select: Option<String>,
    filter: Option<String>,
    expand: Option<String>,
    orderby: Option<String>,
    top: Option<u32>,
    skip: Option<u32>,
    key: Option<String>,
    count: Option<bool>,
    search: Option<String>,
}

#[derive(Clone)]
struct BrowserSession {
    request_url: String,
    cookies: Vec<String>,
}

#[derive(Default)]
struct AppState {
    browser_sessions: Mutex<HashMap<String, BrowserSession>>,
}

/// Successful command result bundled with the HTTP trace captured while it ran.
#[derive(Serialize)]
struct CommandOk<T: Serialize> {
    data: T,
    trace: Vec<HttpTraceEntry>,
}

/// Error result that still carries the trace — failures are exactly when the
/// inspector matters most.
#[derive(Serialize)]
struct CommandError {
    message: String,
    trace: Vec<HttpTraceEntry>,
}

impl CommandError {
    fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            trace: Vec::new(),
        }
    }

    fn with_client(client: &SapClient, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            trace: client.diagnostics_snapshot(),
        }
    }
}

type CmdResult<T> = std::result::Result<CommandOk<T>, CommandError>;

fn auth_mode_for_profile(profile: &config::ConnectionProfile) -> &'static str {
    if profile.browser_sso {
        "browser"
    } else if profile.sso {
        "sso"
    } else {
        "basic"
    }
}

fn browser_session_key(base_url: &str, client: &str, language: &str) -> String {
    format!(
        "{}|{}|{}",
        base_url.trim_end_matches('/').to_lowercase(),
        client.trim(),
        language.trim().to_uppercase()
    )
}

fn browser_probe_path() -> &'static str {
    "/sap/opu/odata/IWFND/CATALOGSERVICE;v=2"
}

fn build_browser_probe_url(base_url: &str, client: &str, language: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!(
        "{base}{}?sap-client={}&sap-language={}",
        browser_probe_path(),
        client,
        language
    )
}

// IMF-fixdate (RFC 7231) — the only `Expires=` format the `cookie` crate parser
// reliably round-trips. `time::Rfc2822` emits "+0000" which the parser rejects.
const IMF_FIXDATE: &[time::format_description::FormatItem<'_>] = time::macros::format_description!(
    "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT"
);

/// Probe URLs for well-known federated-SSO IdPs. Used during sign-out so we
/// don't leave Azure AD / Okta / Auth0 / SAP IAS session cookies behind that
/// would silently re-authenticate the user on the next sign-in.
///
/// This is only a *seed* list — the sign-out flow also walks the entire cookie
/// store and deletes any cookie whose domain matches `session::is_idp_host`,
/// which catches tenant-specific subdomains these fixed URLs would miss.
const IDP_PROBE_URLS: &[&str] = &[
    "https://login.microsoftonline.com/",
    "https://login.microsoft.com/",
    "https://login.windows.net/",
    "https://login.okta.com/",
    "https://login.auth0.com/",
    "https://accounts.sap.com/", // SAP IAS
];

fn is_browser_auth_complete(url: &Url, expected: &Url) -> bool {
    url.scheme() == expected.scheme()
        && url.host_str() == expected.host_str()
        && url.port_or_known_default() == expected.port_or_known_default()
        && url.path().starts_with(expected.path())
}

fn serialize_webview_cookie(cookie: &Cookie<'_>) -> String {
    let mut value = format!("{}={}", cookie.name(), cookie.value());
    if let Some(domain) = cookie.domain() {
        value.push_str(&format!("; Domain={domain}"));
    }
    if let Some(path) = cookie.path() {
        value.push_str(&format!("; Path={path}"));
    }
    // Preserve expiry — otherwise reqwest treats cookies as session-only and
    // client-side expiration is effectively disabled. Must use IMF-fixdate with
    // literal "GMT": the `cookie` crate's parser rejects RFC 2822 "+0000".
    if let Some(expires) = cookie.expires_datetime() {
        let expires_utc = expires.to_offset(time::UtcOffset::UTC);
        if let Ok(formatted) = expires_utc.format(&IMF_FIXDATE) {
            value.push_str(&format!("; Expires={formatted}"));
        }
    }
    if let Some(max_age) = cookie.max_age() {
        value.push_str(&format!("; Max-Age={}", max_age.whole_seconds()));
    }
    if let Some(same_site) = cookie.same_site() {
        value.push_str(&format!("; SameSite={same_site}"));
    }
    if cookie.secure().unwrap_or(false) {
        value.push_str("; Secure");
    }
    if cookie.http_only().unwrap_or(false) {
        value.push_str("; HttpOnly");
    }
    value
}

// ── Helper: create SapClient from profile name ──

fn client_from_profile(profile_name: &str, state: &AppState) -> Result<SapClient, String> {
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    let profile = cfg
        .connections
        .get(profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;
    let connection = config::resolve_connection(profile_name, profile)
        .map_err(|e| format!("Connection error: {e}"))?;
    let client = SapClient::new(connection.clone()).map_err(|e| format!("Client error: {e}"))?;

    if matches!(connection.auth, AuthConfig::Browser) {
        let key = browser_session_key(
            &connection.base_url,
            &connection.client,
            &connection.language,
        );
        if let Some(session) = state.browser_sessions.lock().unwrap().get(&key).cloned() {
            client
                .import_cookie_strings(&session.request_url, &session.cookies)
                .map_err(|e| format!("Cookie import error: {e}"))?;
        }
    }

    Ok(client)
}

/// Dispatch a `(profile_name, service_identifier)` pair to the right
/// metadata source — live HTTP for connected profiles, on-disk EDMX
/// read for offline profiles — and return parsed `ServiceMetadata`
/// plus a captured HTTP trace.
///
/// The trace is always returned (empty `Vec` for the offline branch)
/// so callers can keep wrapping the response in `CommandOk { data,
/// trace }` without conditional logic. Offline reads have no HTTP
/// exchanges to report; the inspector panel sees an empty trace and
/// can render a "no live activity — offline source" placeholder.
///
/// Profile-name globally unique across `connections` and
/// `offline_profiles` (enforced at add-time) means the dispatch is
/// unambiguous. If neither map contains the name, returns a typed
/// "profile not found" error.
async fn fetch_service_metadata(
    profile_name: &str,
    service_identifier: &str,
    state: &AppState,
) -> Result<(metadata::ServiceMetadata, Vec<HttpTraceEntry>), CommandError> {
    let (cfg, config_dir) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;
    let source =
        sap_odata_core::offline::MetadataSource::resolve(profile_name, service_identifier, &cfg)
            .map_err(|e| CommandError::msg(e.to_string()))?;

    match source {
        sap_odata_core::offline::MetadataSource::Connected { .. } => {
            let client = client_from_profile(profile_name, state).map_err(CommandError::msg)?;
            let meta = client
                .fetch_metadata(service_identifier)
                .await
                .map_err(|e| CommandError::with_client(&client, format!("Metadata error: {e}")))?;
            let trace = client.diagnostics_snapshot();
            Ok((meta, trace))
        }
        sap_odata_core::offline::MetadataSource::Offline { service_id, .. } => {
            let xml = sap_odata_core::offline::read_offline_metadata(
                &cfg,
                &config_dir.path,
                profile_name,
                &service_id,
            )
            .map_err(|e| CommandError::msg(format!("Offline read error: {e}")))?;
            let meta = metadata::parse_metadata(&xml)
                .map_err(|e| CommandError::msg(format!("Offline metadata parse error: {e}")))?;
            Ok((meta, Vec::new()))
        }
    }
}

fn resolve_service_for_profile(profile_name: &str, service: &str) -> Result<String, String> {
    // Only treat as a literal service path when it's rooted at `/sap/`. SAP
    // catalog technical names in a customer namespace (e.g.
    // `/NAMESPACE/SERVICE_NAME`) also start with `/` and would otherwise
    // bypass catalog resolution.
    if service.starts_with("/sap/") {
        return Ok(service.to_string());
    }
    // Check aliases
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    if let Some(profile) = cfg.connections.get(profile_name) {
        let svc_lower = service.to_lowercase();
        for (alias_name, alias_path) in &profile.aliases {
            if alias_name.to_lowercase() == svc_lower {
                return Ok(alias_path.clone());
            }
        }
    }
    // Return as-is for catalog resolution (will be done async)
    Err(format!("Service '{}' needs catalog resolution", service))
}

async fn browser_sign_in_for_connection(
    app: tauri::AppHandle,
    state: &AppState,
    base_url: String,
    client: String,
    language: String,
    insecure_tls: bool,
    profile_name: Option<String>,
) -> CmdResult<String> {
    let request_url = build_browser_probe_url(&base_url, &client, &language);
    let request_url_parsed = Url::parse(&request_url)
        .map_err(|e| CommandError::msg(format!("Invalid sign-in URL: {e}")))?;
    let label = "browser-auth";

    if let Some(existing) = app.get_webview_window(label) {
        let _ = existing.close();
    }

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let tx = Arc::new(Mutex::new(Some(tx)));
    let expected_url = request_url_parsed.clone();

    // Track whether we've left the SAP host (visited an IdP).
    // Only signal completion when we come BACK to SAP after visiting an IdP.
    let visited_idp = Arc::new(Mutex::new(false));

    let auth_window = WebviewWindowBuilder::new(
        &app,
        label,
        WebviewUrl::External(request_url_parsed.clone()),
    )
    .title("Sign In")
    .inner_size(980.0, 760.0)
    .resizable(true)
    .on_navigation({
        let tx = tx.clone();
        let visited_idp = visited_idp.clone();
        move |url| {
            let host = url.host_str().unwrap_or("");
            // Detect IdP domains — shared list with sign-out cookie sweep and
            // expired-session redirect detection so all three stay aligned.
            if sap_odata_core::session::is_idp_host(host) {
                *visited_idp.lock().unwrap() = true;
            }

            // Only signal completion if we've visited an IdP and returned to SAP
            let was_at_idp = *visited_idp.lock().unwrap();
            if was_at_idp
                && is_browser_auth_complete(url, &expected_url)
                && let Some(tx) = tx.lock().unwrap().take()
            {
                let _ = tx.send(Ok(()));
            }
            true
        }
    })
    .build()
    .map_err(|e| CommandError::msg(format!("Failed to open sign-in window: {e}")))?;

    auth_window.on_window_event({
        let tx = tx.clone();
        move |event| {
            if matches!(
                event,
                WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed
            ) && let Some(tx) = tx.lock().unwrap().take()
            {
                let _ = tx.send(Err("Sign-in cancelled".to_string()));
            }
        }
    });

    let outcome = rx
        .await
        .map_err(|_| CommandError::msg("Sign-in window closed unexpectedly"))?;
    outcome.map_err(CommandError::msg)?;

    // Wait for cookies to be fully written by the webview
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Try reading cookies for the base URL (SAP sets cookies at the root)
    let base_url_parsed = Url::parse(&format!("{}/", base_url.trim_end_matches('/')))
        .unwrap_or_else(|_| request_url_parsed.clone());

    // Try both the base URL and the probe URL to capture all cookies
    let mut all_cookies = Vec::new();
    for url in [&base_url_parsed, &request_url_parsed] {
        if let Ok(cookies) = auth_window.cookies_for_url(url.clone()) {
            for c in &cookies {
                let s = serialize_webview_cookie(c);
                if !all_cookies.contains(&s) {
                    all_cookies.push(s);
                }
            }
        }
    }

    let cookie_strings = all_cookies;
    if cookie_strings.is_empty() {
        let _ = auth_window.close();
        return Err(CommandError::msg(format!(
            "Sign-in finished, but no SAP session cookies were found.\n\
             Tried URLs: {} and {}\n\
             The SAP system may set cookies on a different domain or path.",
            base_url_parsed, request_url_parsed
        )));
    }

    let key = browser_session_key(&base_url, &client, &language);
    state.browser_sessions.lock().unwrap().insert(
        key.clone(),
        BrowserSession {
            request_url: request_url.clone(),
            cookies: cookie_strings.clone(),
        },
    );

    // Persist to OS keyring if we know the profile name (so the CLI can reuse it).
    if let Some(ref pname) = profile_name {
        let fingerprint =
            sap_odata_core::session::connection_fingerprint(&base_url, &client, &language);
        match sap_odata_core::session::save(pname, &request_url, &fingerprint, &cookie_strings) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Warning: could not persist browser session for '{pname}': {e}");
            }
        }
    }

    let _ = auth_window.close();

    let sap_client = SapClient::new(SapConnection {
        base_url,
        client,
        language,
        auth: AuthConfig::Browser,
        insecure_tls,
        sso_delegate: false,
    })
    .map_err(|e| CommandError::msg(format!("Client error: {e}")))?;

    if let Some(session) = state.browser_sessions.lock().unwrap().get(&key).cloned() {
        sap_client
            .import_cookie_strings(&session.request_url, &session.cookies)
            .map_err(|e| CommandError::msg(format!("Cookie import error: {e}")))?;
    }

    match sap_client.ensure_session(browser_probe_path()).await {
        Ok(()) => Ok(CommandOk {
            data: "Browser sign-in successful".to_string(),
            trace: sap_client.diagnostics_snapshot(),
        }),
        Err(e) => Err(CommandError::with_client(
            &sap_client,
            format!("Browser sign-in succeeded, but SAP session validation failed: {e}"),
        )),
    }
}

// ── Tauri commands ──

#[tauri::command]
fn get_profiles() -> Result<Vec<ProfileInfo>, String> {
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

    let connected = cfg.connections.iter().map(|(name, profile)| {
        let password_source = if profile.browser_sso {
            "browser".to_string()
        } else if profile.sso {
            "none".to_string()
        } else if profile.password.is_some() {
            "config".to_string()
        } else {
            match config::try_get_password_from_keyring(name, &profile.username) {
                Ok(Some(_)) => "keyring".to_string(),
                Ok(None) => "none".to_string(),
                // Distinct states so the frontend (now or later) can
                // tell users to unlock the credential store rather than
                // re-add the profile.
                Err(config::KeyringReadError::Locked(_)) => "keyring_locked".to_string(),
                Err(config::KeyringReadError::Corrupt(_)) => "keyring_corrupt".to_string(),
                Err(config::KeyringReadError::Backend(_)) => "keyring_error".to_string(),
            }
        };

        let aliases = profile
            .aliases
            .iter()
            .map(|(n, p)| AliasInfo {
                name: n.clone(),
                path: p.clone(),
            })
            .collect();

        ProfileInfo {
            name: name.clone(),
            kind: "connected".to_string(),
            base_url: profile.base_url.clone(),
            client: profile.client.clone(),
            language: profile.language.clone(),
            username: profile.username.clone(),
            password_source,
            auth_mode: auth_mode_for_profile(profile).to_string(),
            sso_delegate: profile.sso_delegate,
            aliases,
            source_profile: None,
            created_at: None,
        }
    });

    // Offline profiles share the picker with connected ones. The fields
    // that don't apply (base_url, client, language, ...) are populated
    // with empty strings so the JSON shape stays stable for the
    // frontend — `kind = "offline"` is the discriminator that tells the
    // UI to render the badge and disable execute affordances.
    let offline = cfg
        .offline_profiles
        .iter()
        .map(|(name, bucket)| ProfileInfo {
            name: name.clone(),
            kind: "offline".to_string(),
            base_url: String::new(),
            client: String::new(),
            language: String::new(),
            username: String::new(),
            password_source: "none".to_string(),
            auth_mode: "offline".to_string(),
            sso_delegate: false,
            aliases: Vec::new(),
            source_profile: if bucket.source_profile.is_empty() {
                None
            } else {
                Some(bucket.source_profile.clone())
            },
            created_at: Some(bucket.created_at.clone()),
        });

    Ok(connected.chain(offline).collect())
}

#[derive(Serialize)]
struct ServicesResponse {
    services: Vec<ServiceInfo>,
    // Per-version catalog failures (e.g., "V4 catalog: 403 Forbidden") — surfaced so
    // the UI can render a context-specific hint (paste full path) when V4 is down.
    warnings: Vec<String>,
}

#[tauri::command]
async fn get_services(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    search: Option<String>,
    v4_only: bool,
) -> CmdResult<ServicesResponse> {
    // Dispatch on profile kind. Offline profiles serve a fixed
    // services list from the TOML index rather than hitting the
    // gateway catalog; same response shape so the frontend's
    // services-rendering code path is unchanged.
    let (cfg, _) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;

    // **Fail-closed on name collisions** before branching. If the name
    // is in both `connections` and `offline_profiles`, the previous
    // "prefer offline" precedence here masked the corrupt state — the
    // picker would render cached services, but the very next call
    // (`get_entities`, `describe_entity`, etc.) would fail with
    // `MetadataSource::NameCollision` and the user would see a
    // confusing two-step error. Surface the collision at the same
    // chokepoint as the dispatcher so the picker is the place where
    // the user is told to fix `connections.toml`.
    if cfg.connections.contains_key(&profile_name)
        && cfg.offline_profiles.contains_key(&profile_name)
    {
        return Err(CommandError::msg(format!(
            "Profile name '{profile_name}' is present in BOTH connections and offline_profiles — dispatch is ambiguous. Rename or remove one of them in connections.toml."
        )));
    }

    if cfg.offline_profiles.contains_key(&profile_name) {
        let services = cfg
            .offline_services
            .iter()
            .filter(|s| s.profile == profile_name)
            .filter(|s| {
                if v4_only && s.odata_version != "V4" {
                    return false;
                }
                if let Some(ref needle) = search {
                    let needle_lc = needle.to_lowercase();
                    let hay = format!(
                        "{} {} {}",
                        s.label,
                        s.source_service_path.as_deref().unwrap_or(""),
                        s.original_filename.as_deref().unwrap_or(""),
                    )
                    .to_lowercase();
                    if !hay.contains(&needle_lc) {
                        return false;
                    }
                }
                true
            })
            .map(|s| ServiceInfo {
                // technical_name = the stable id, which is what the
                // frontend round-trips back to commands like
                // get_entities / describe_entity. `MetadataSource::resolve`
                // accepts id directly (and also accepts
                // source_service_path as a fallback for path-A rows).
                technical_name: s.id.clone(),
                title: s.label.clone(),
                description: s.note.clone(),
                service_url: s.source_service_path.clone().unwrap_or_default(),
                version: s.odata_version.clone(),
            })
            .collect();
        return Ok(CommandOk {
            data: ServicesResponse {
                services,
                warnings: Vec::new(),
            },
            trace: Vec::new(),
        });
    }

    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    let result = catalog::fetch_service_catalog(&client)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("Catalog error: {e}")))?;

    if result.entries.is_empty() && !result.warnings.is_empty() {
        return Err(CommandError::with_client(
            &client,
            format!(
                "No services found. Warnings: {}",
                result.warnings.join("; ")
            ),
        ));
    }

    let services = result
        .entries
        .iter()
        .filter(|e| {
            if v4_only && !e.is_v4 {
                return false;
            }
            if let Some(ref s) = search {
                return e.matches(s);
            }
            true
        })
        .map(|e| ServiceInfo {
            technical_name: e.technical_name.clone(),
            title: e.title.clone(),
            description: e.description.clone(),
            service_url: e.service_url.clone(),
            version: e.version_label().to_string(),
        })
        .collect();
    Ok(CommandOk {
        data: ServicesResponse {
            services,
            warnings: result.warnings,
        },
        trace: client.diagnostics_snapshot(),
    })
}

#[tauri::command]
async fn resolve_service(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service: String,
) -> CmdResult<String> {
    // Aliases bypass the network entirely, so no trace to report.
    if let Ok(path) = resolve_service_for_profile(&profile_name, &service) {
        return Ok(CommandOk {
            data: path,
            trace: Vec::new(),
        });
    }

    // **Offline-profile branch.** For an offline source, "resolving" a
    // service identifier means looking it up in the indexed
    // `offline_services` rows and returning the canonical `service_id`
    // — never building a `SapClient` or hitting the gateway catalog.
    // The frontend passes either an `id` or a `source_service_path`
    // for path-A rows; `MetadataSource::resolve` handles both forms
    // and always returns the canonical id. Name collisions surface
    // here as a hard error (same wording as the dispatcher) so the
    // user fixes `connections.toml` before any read happens.
    let (cfg, _) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;
    if cfg.offline_profiles.contains_key(&profile_name)
        || cfg.connections.contains_key(&profile_name)
    {
        // Run dispatcher unconditionally for both kinds — it's the
        // single chokepoint for the collision check too. Only the
        // offline branch returns here; connected falls through to the
        // existing live-catalog resolution below.
        match sap_odata_core::offline::MetadataSource::resolve(&profile_name, &service, &cfg) {
            Ok(sap_odata_core::offline::MetadataSource::Offline { service_id, .. }) => {
                return Ok(CommandOk {
                    data: service_id,
                    trace: Vec::new(),
                });
            }
            Ok(sap_odata_core::offline::MetadataSource::Connected { .. }) => {
                // Fall through to the live-catalog resolution.
            }
            Err(sap_odata_core::offline::MetadataSourceError::OfflineServiceNotFound {
                ..
            }) => {
                // Offline profile but the identifier doesn't match
                // any indexed row. Surface the error directly — the
                // user picked a stale/wrong identifier.
                return Err(CommandError::msg(format!(
                    "Service '{service}' is not present in offline profile '{profile_name}'."
                )));
            }
            Err(sap_odata_core::offline::MetadataSourceError::NameCollision(_)) => {
                return Err(CommandError::msg(format!(
                    "Profile name '{profile_name}' is present in BOTH connections and offline_profiles — dispatch is ambiguous. Rename or remove one of them in connections.toml."
                )));
            }
            Err(sap_odata_core::offline::MetadataSourceError::ProfileNotFound(_)) => {
                // Shouldn't happen — we just checked containment.
                // Fall through to the live-catalog path which will
                // return its own profile-not-found error.
            }
        }
    }

    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    match catalog::resolve_service_by_name(&client, &service).await {
        Ok(path) => Ok(CommandOk {
            data: path,
            trace: client.diagnostics_snapshot(),
        }),
        Err(e) => Err(CommandError::with_client(
            &client,
            format!("Resolution error: {e}"),
        )),
    }
}

/// Return the full raw annotation list for a service. Fetched lazily by
/// the desktop app's annotation inspector panel so a service with
/// hundreds of annotations doesn't bloat every catalog/entities response.
#[tauri::command]
async fn get_annotations(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
) -> CmdResult<Vec<RawAnnotation>> {
    let (meta, trace) = fetch_service_metadata(&profile_name, &service_path, &state).await?;
    Ok(CommandOk {
        data: meta.annotations,
        trace,
    })
}

#[tauri::command]
async fn get_entities(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
) -> CmdResult<EntityListResponse> {
    let (meta, trace) = fetch_service_metadata(&profile_name, &service_path, &state).await?;

    let entity_sets = meta
        .entity_sets
        .iter()
        .map(|es| {
            let keys = meta
                .entity_type_for_set(&es.name)
                .map(|et| et.keys.clone())
                .unwrap_or_default();
            EntitySetInfo {
                name: es.name.clone(),
                entity_type: es.entity_type.clone(),
                keys,
            }
        })
        .collect();
    let annotation_summary = meta.annotation_summary();
    Ok(CommandOk {
        data: EntityListResponse {
            entity_sets,
            annotation_summary,
        },
        trace,
    })
}

#[tauri::command]
async fn describe_entity(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    entity_set: String,
) -> CmdResult<EntityTypeInfo> {
    let (meta, trace) = fetch_service_metadata(&profile_name, &service_path, &state).await?;

    let et = meta
        .entity_type_for_set(&entity_set)
        .ok_or_else(|| CommandError {
            message: format!("Entity set '{}' not found", entity_set),
            trace: trace.clone(),
        })?;

    let nav_targets = meta.nav_targets(et);

    let data = EntityTypeInfo {
        name: et.name.clone(),
        keys: et.keys.clone(),
        header_info: et.header_info.clone(),
        selection_fields: et.selection_fields.clone(),
        line_item: et.line_item.clone(),
        request_at_least: et.request_at_least.clone(),
        sort_order: et.sort_order.clone(),
        selection_variants: et.selection_variants.clone(),
        searchable: et.searchable,
        countable: et.countable,
        top_supported: et.top_supported,
        skip_supported: et.skip_supported,
        expandable: et.expandable,
        non_expandable_properties: et.non_expandable_properties.clone(),
        semantic_keys: et.semantic_keys.clone(),
        fiori_readiness: lint::evaluate_entity_type(et),
        properties: et
            .properties
            .iter()
            .map(|p| PropertyInfo {
                name: p.name.clone(),
                edm_type: p.edm_type.clone(),
                nullable: p.nullable,
                max_length: p.max_length,
                label: p.label.clone(),
                is_key: et.keys.contains(&p.name),
                text_path: p.text_path.clone(),
                unit_path: p.unit_path.clone(),
                iso_currency_path: p.iso_currency_path.clone(),
                filterable: p.filterable,
                sortable: p.sortable,
                creatable: p.creatable,
                updatable: p.updatable,
                required_in_filter: p.required_in_filter,
                criticality: p.criticality.clone(),
                value_list: p.value_list.clone(),
                value_list_variants: p.value_list_variants.clone(),
                value_list_references: p.value_list_references.clone(),
                value_list_fixed: p.value_list_fixed,
                text_arrangement: p.text_arrangement,
                field_control: p.field_control.clone(),
                hidden: p.hidden,
                hidden_filter: p.hidden_filter,
                display_format: p.display_format.clone(),
                sap_value_list: p.sap_value_list.clone(),
                semantic_object: p.semantic_object.clone(),
                masked: p.masked,
            })
            .collect(),
        nav_properties: nav_targets
            .iter()
            .map(|(name, target, mult)| NavPropertyInfo {
                name: name.clone(),
                target_type: target.clone(),
                multiplicity: mult.clone(),
            })
            .collect(),
    };
    Ok(CommandOk { data, trace })
}

#[tauri::command]
async fn run_query(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    params: QueryParams,
) -> CmdResult<serde_json::Value> {
    // Query execution always requires the network. Reject early with a
    // friendly message rather than letting the offline-source caller
    // hit `client_from_profile` and get a profile-not-found error.
    let (cfg, _) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;
    let source =
        sap_odata_core::offline::MetadataSource::resolve(&profile_name, &service_path, &cfg)
            .map_err(|e| CommandError::msg(e.to_string()))?;
    source
        .assert_network_allowed()
        .map_err(|e| CommandError::msg(e.to_string()))?;

    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;

    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("Metadata error: {e}")))?;

    let mut q = ODataQuery::new(&params.entity_set)
        .format("json")
        .version(meta.version);

    if let Some(ref s) = params.select {
        let fields: Vec<&str> = s.split(',').map(str::trim).collect();
        q = q.select(&fields);
    }
    if let Some(ref f) = params.filter {
        q = q.filter(f);
    }
    if let Some(ref e) = params.expand {
        let navs: Vec<&str> = e.split(',').map(str::trim).collect();
        q = q.expand(&navs);
    }
    if let Some(ref o) = params.orderby {
        let clauses: Vec<&str> = o.split(',').map(str::trim).collect();
        q = q.orderby(&clauses);
    }
    if let Some(t) = params.top {
        q = q.top(t);
    }
    if let Some(s) = params.skip {
        q = q.skip(s);
    }
    if let Some(ref k) = params.key {
        q = q.key(k);
    }
    if params.count.unwrap_or(false) {
        q = q.count();
    }
    if let Some(ref term) = params.search {
        let trimmed = term.trim();
        if !trimmed.is_empty() {
            q = q.search(trimmed);
        }
    }

    match client.query_json(&service_path, &q).await {
        Ok(value) => Ok(CommandOk {
            data: value,
            trace: client.diagnostics_snapshot(),
        }),
        Err(e) => Err(CommandError::with_client(
            &client,
            format!("Query error: {e}"),
        )),
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn add_profile(
    name: String,
    base_url: String,
    client: String,
    language: String,
    auth_mode: String,
    username: String,
    password: String,
    // When true, allow falling back to plaintext-in-config if the OS keyring
    // rejects the password. The UI only sets this after the user explicitly
    // confirms a "store in config file instead" dialog; default false so that
    // a keyring failure surfaces an actionable error rather than silently
    // downgrading credential protection.
    #[allow(non_snake_case)] allow_plaintext_fallback: Option<bool>,
    // Opt-in to Kerberos delegation when auth_mode == "sso". Mirrors the CLI's
    // --sso-delegate flag. Option<bool> so older callers that don't pass it
    // continue to work; treated as false when absent.
    #[allow(non_snake_case)] allow_sso_delegate: Option<bool>,
) -> Result<String, String> {
    let (mut cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

    // **Global name uniqueness across connections + offline_profiles.**
    // Shared check via `offline::check_connected_profile_name_available`
    // so all three connected-add paths (this one, the CLI
    // `cmd_profile_add`, and the CLI `cmd_setup_wizard`) go through one
    // source of truth. The reverse direction is enforced inline by
    // `save_service_offline_from_bytes` / `import_edmx_file`. Upsert
    // (re-saving an existing connected profile with new settings) is
    // permitted; only cross-map collisions are rejected.
    sap_odata_core::offline::check_connected_profile_name_available(&name, &cfg)?;

    let existing_aliases = cfg
        .connections
        .get(&name)
        .map(|p| p.aliases.clone())
        .unwrap_or_default();

    // If the connection fingerprint changed (url/client/language), discard any
    // persisted Browser SSO session so we don't replay cookies to a different target.
    let old_profile = cfg.connections.get(&name).cloned();
    let mut fp_warning: Option<String> = None;
    match config::clear_session_if_connection_changed(
        &name,
        old_profile.as_ref(),
        &base_url,
        &client,
        &language,
    ) {
        Ok(_) => {}
        Err(e) => {
            fp_warning = Some(format!(
                "connection changed but could not clear stale Browser SSO session: {e}"
            ));
        }
    }

    let sso = auth_mode == "sso";
    let browser_sso = auth_mode == "browser";
    let profile = config::ConnectionProfile {
        base_url,
        client,
        language,
        username: if browser_sso {
            String::new()
        } else {
            username.clone()
        },
        password: None,
        sso,
        browser_sso,
        insecure_tls: false,
        sso_delegate: sso && allow_sso_delegate.unwrap_or(false),
        aliases: existing_aliases,
    };

    if auth_mode == "basic" && !password.is_empty() {
        // Try the OS keyring first. On failure, fail closed unless the UI
        // explicitly signalled that the user confirmed plaintext fallback.
        if let Err(e) = config::set_password_in_keyring(&name, &username, &password) {
            if allow_plaintext_fallback.unwrap_or(false) {
                let mut profile_with_pw = profile.clone();
                profile_with_pw.password = Some(password);
                cfg.connections.insert(name.clone(), profile_with_pw);
                let dir = config::get_or_create_config_dir()
                    .map_err(|e| format!("Config dir error: {e}"))?;
                config::save_config(&cfg, &dir.path).map_err(|e| format!("Save error: {e}"))?;
                let base = format!("Profile '{}' saved (password in config file)", name);
                return Ok(match fp_warning {
                    Some(w) => format!("{base}. Warning: {w}"),
                    None => base,
                });
            }
            return Err(format!(
                "KEYRING_FAILED: could not store password in OS keyring: {e}"
            ));
        }
    }

    cfg.connections.insert(name.clone(), profile);
    let dir = config::get_or_create_config_dir().map_err(|e| format!("Config dir error: {e}"))?;
    config::save_config(&cfg, &dir.path).map_err(|e| format!("Save error: {e}"))?;
    let mode = match auth_mode.as_str() {
        "sso" => " with Windows SSO",
        "browser" => " with browser SSO",
        _ => "",
    };
    let base = format!("Profile '{}' saved{mode}", name);
    Ok(match fp_warning {
        Some(w) => format!("{base}. Warning: {w}"),
        None => base,
    })
}

/// Path A: capture the bytes of a connected service's `$metadata` to the
/// offline library. Resolves the connected profile, builds a client,
/// fetches metadata, validates it, atomically writes the bytes + TOML
/// index. Returns the `SaveOutcome` describing what landed.
#[tauri::command]
async fn save_service_offline(
    state: tauri::State<'_, AppState>,
    connected_profile_name: String,
    service_path: String,
    offline_profile_name: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
) -> CmdResult<SaveOutcome> {
    let client = client_from_profile(&connected_profile_name, &state).map_err(CommandError::msg)?;
    let (mut cfg, config_dir) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;

    // Source URL for attribution: connected profile's base + service +
    // `/$metadata`. `strip_userinfo` runs inside the core fn at save
    // time so credentials never land in the offline index even if the
    // user pasted them into the connection form.
    let base_url = cfg
        .connections
        .get(&connected_profile_name)
        .map(|p| p.base_url.clone())
        .ok_or_else(|| {
            CommandError::msg(format!("Profile '{connected_profile_name}' not found"))
        })?;
    let svc = service_path.trim_start_matches('/').trim_end_matches('/');
    let source_url = format!("{}/{}/$metadata", base_url.trim_end_matches('/'), svc);

    let opts = SaveOptions {
        offline_profile_name: offline_profile_name
            .or_else(|| Some(auto_offline_profile_name(&connected_profile_name))),
        source_profile_for_new_bucket: Some(connected_profile_name.clone()),
        label_override,
        note,
        now_iso: current_iso8601(),
    };

    let outcome = offline::save_service_offline(
        &client,
        &service_path,
        source_url,
        &mut cfg,
        &config_dir.path,
        opts,
    )
    .await
    .map_err(|e| CommandError::with_client(&client, format!("Save error: {e}")))?;

    Ok(CommandOk {
        data: outcome,
        trace: client.diagnostics_snapshot(),
    })
}

/// Path B: import an EDMX file from disk into the offline library. No
/// connected profile required. The file path comes from the user (UI
/// file picker); the import-validation pipeline rejects wrong-shape
/// inputs before any state changes. Returns the `SaveOutcome` so the
/// UI can render the new-vs-overwrite-vs-skipped header.
#[tauri::command]
fn import_edmx_file(
    file_path: String,
    target_offline_profile: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
) -> Result<SaveOutcome, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    let opts = ImportOptions {
        file_path: std::path::PathBuf::from(&file_path),
        target_offline_profile,
        label_override,
        note,
        now_iso: current_iso8601(),
    };
    offline::import_edmx_file(&mut cfg, &config_dir.path, opts)
        .map_err(|e| format!("Import error: {e}"))
}

/// Bytes-based path-B import for the webview's `<input type="file">`
/// flow. The browser's file picker hands JS a `File` object whose
/// content is readable via `arrayBuffer()` but whose filesystem path
/// is not exposed (Tauri sandbox + browser policy). The frontend
/// reads the bytes and forwards them here along with the basename
/// for the `original_filename` field.
#[tauri::command]
fn import_edmx_bytes(
    bytes: Vec<u8>,
    original_filename: Option<String>,
    target_offline_profile: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
) -> Result<SaveOutcome, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    offline::import_edmx_from_bytes(
        &mut cfg,
        &config_dir.path,
        &bytes,
        original_filename,
        target_offline_profile,
        label_override,
        note,
        current_iso8601(),
    )
    .map_err(|e| format!("Import error: {e}"))
}

/// Shape returned by `list_offline_services`. Carries more attribution
/// detail than the generic `ServiceInfo` because the offline UI needs
/// to render the source profile, timestamps, original filename, etc.
/// `get_services` returns the leaner `ServiceInfo` so the existing
/// services-list rendering stays compatible.
#[derive(Serialize)]
struct OfflineServiceListEntry {
    id: String,
    profile: String,
    label: String,
    label_at_creation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_service_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    imported_at: Option<String>,
    sha256: String,
    size_bytes: u64,
    odata_version: String,
    note: String,
}

/// List all services in an offline profile bucket. Distinct from
/// `get_services` (which dispatches on profile kind and returns the
/// generic `ServiceInfo`) — this command returns the full
/// attribution shape the offline-library UI needs for rendering the
/// per-service detail panel.
#[tauri::command]
fn list_offline_services(profile_name: String) -> Result<Vec<OfflineServiceListEntry>, String> {
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    if !cfg.offline_profiles.contains_key(&profile_name) {
        return Err(format!(
            "Offline profile '{profile_name}' not found. Use `list_offline_profiles` (or `get_profiles`, filtering by `kind = \"offline\"`) to see what's available."
        ));
    }
    let entries = cfg
        .offline_services
        .iter()
        .filter(|s| s.profile == profile_name)
        .map(|s| OfflineServiceListEntry {
            id: s.id.clone(),
            profile: s.profile.clone(),
            label: s.label.clone(),
            label_at_creation: s.label_at_creation.clone(),
            source_service_path: s.source_service_path.clone(),
            source_url: s.source_url.clone(),
            original_filename: s.original_filename.clone(),
            fetched_at: s.fetched_at.clone(),
            imported_at: s.imported_at.clone(),
            sha256: s.sha256.clone(),
            size_bytes: s.size_bytes,
            odata_version: s.odata_version.clone(),
            note: s.note.clone(),
        })
        .collect();
    Ok(entries)
}

/// Dispatches on profile kind. Connected profiles go through the
/// existing keyring + session cleanup; offline profiles delegate to
/// `offline::delete_offline_profile` which removes the bucket dir +
/// every indexed service + the bucket entry, all under the shared
/// `SaveLock`. The single command keeps the frontend's Remove button
/// path-agnostic.
#[tauri::command]
fn remove_profile(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

    // Offline-profile branch.
    if cfg.offline_profiles.contains_key(&name) {
        let outcome = offline::delete_offline_profile(&mut cfg, &config_dir.path, &name)
            .map_err(|e| format!("Delete offline profile error: {e}"))?;
        return Ok(format!(
            "Offline profile '{}' removed ({} service(s), {} file(s), bucket dir {})",
            outcome.profile,
            outcome.services_removed,
            outcome.files_removed,
            if outcome.directory_removed {
                "removed"
            } else {
                "absent"
            },
        ));
    }

    // Connected-profile branch (existing logic).
    let profile = cfg
        .connections
        .remove(&name)
        .ok_or_else(|| format!("Profile '{}' not found", name))?;

    let mut warnings: Vec<String> = Vec::new();
    if !profile.sso
        && !profile.browser_sso
        && !profile.username.is_empty()
        && let Err(e) = config::delete_password_from_keyring(&name, &profile.username)
    {
        warnings.push(format!("password (user: {}) — {e}", profile.username));
    }
    let key = browser_session_key(&profile.base_url, &profile.client, &profile.language);
    state.browser_sessions.lock().unwrap().remove(&key);
    // Also clear persisted browser session from keyring
    if let Err(e) = sap_odata_core::session::clear(&name) {
        warnings.push(format!("Browser SSO session — {e}"));
    }
    config::save_config(&cfg, &config_dir.path).map_err(|e| format!("Save error: {e}"))?;
    if warnings.is_empty() {
        Ok(format!("Profile '{}' removed", name))
    } else {
        Ok(format!(
            "Profile '{}' removed from config, but these items could NOT be cleared: {}. You may need to remove them manually from the OS credential store.",
            name,
            warnings.join("; ")
        ))
    }
}

/// Delete a single offline service (one row + its EDMX file).
/// `remove_profile` handles the whole-bucket case; this is the
/// finer-grained variant the UI uses for per-row deletes from the
/// offline-services list.
#[tauri::command]
fn delete_offline_service(profile_name: String, service_id: String) -> Result<String, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    let outcome =
        offline::delete_offline_service(&mut cfg, &config_dir.path, &profile_name, &service_id)
            .map_err(|e| format!("Delete offline service error: {e}"))?;
    Ok(format!(
        "Offline service '{}' removed from profile '{}' (file {})",
        outcome.service_id,
        outcome.profile,
        if outcome.file_removed {
            "removed"
        } else {
            "already gone"
        },
    ))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn test_connection(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    base_url: String,
    client: String,
    language: String,
    auth_mode: String,
    username: String,
    password: String,
    allow_sso_delegate: Option<bool>,
) -> CmdResult<String> {
    if auth_mode == "browser" {
        return browser_sign_in_for_connection(
            app, &state, base_url, client, language, false,
            None, // pre-save test — no profile name yet
        )
        .await;
    }

    let auth = if auth_mode == "sso" {
        AuthConfig::Sso
    } else {
        AuthConfig::Basic { username, password }
    };

    let connection = SapConnection {
        base_url,
        client,
        language,
        auth,
        insecure_tls: false,
        sso_delegate: auth_mode == "sso" && allow_sso_delegate.unwrap_or(false),
    };

    let sap_client =
        SapClient::new(connection).map_err(|e| CommandError::msg(format!("Client error: {e}")))?;
    match sap_client
        .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
        .await
    {
        Ok(()) => Ok(CommandOk {
            data: "Connection successful".to_string(),
            trace: sap_client.diagnostics_snapshot(),
        }),
        Err(e) => Err(CommandError::with_client(
            &sap_client,
            format!("Connection failed: {e}"),
        )),
    }
}

#[tauri::command]
async fn browser_sign_in_profile(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_name: String,
) -> CmdResult<String> {
    let (cfg, _) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;
    let profile = cfg
        .connections
        .get(&profile_name)
        .ok_or_else(|| CommandError::msg(format!("Profile '{}' not found", profile_name)))?;

    if !profile.browser_sso {
        return Err(CommandError::msg(format!(
            "Profile '{}' is not configured for browser SSO",
            profile_name
        )));
    }

    browser_sign_in_for_connection(
        app,
        &state,
        profile.base_url.clone(),
        profile.client.clone(),
        profile.language.clone(),
        profile.insecure_tls,
        Some(profile_name),
    )
    .await
}

#[tauri::command]
async fn sign_out_profile(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_name: String,
) -> Result<String, String> {
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    let profile = cfg
        .connections
        .get(&profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    let base_url = profile.base_url.clone();
    let client = profile.client.clone();
    let language = profile.language.clone();

    // 1. Clear in-memory session
    let key = browser_session_key(&base_url, &client, &language);
    state.browser_sessions.lock().unwrap().remove(&key);

    // 2. Clear persisted session from keyring. Don't bail if this fails — we
    //    still want to attempt to clear browser-level cookies so an immediate
    //    re-sign-in still prompts for credentials. Report the partial failure.
    let persisted_err = sap_odata_core::session::clear(&profile_name).err();

    // 3. Clear cookies from the webview store itself. Covers:
    //      - SAP base URL + catalog probe URL (first-party session cookies)
    //      - Federated IdP hosts (Azure AD / Okta / Auth0 / SAP IAS). Without
    //        these, the IdP will silently re-authenticate on next sign-in.
    //    Also walks every cookie the webview currently holds and deletes any
    //    whose domain matches an IdP — catches tenant-specific subdomains
    //    (e.g. login.microsoftonline.com/<tenant-id>) that a fixed URL list
    //    would otherwise miss.
    let mut attempted = 0usize;
    let mut deleted = 0usize;
    if let Some(main_window) = app.get_webview_window("main") {
        let probe_url_str = build_browser_probe_url(&base_url, &client, &language);
        let mut probe_urls: Vec<Url> = Vec::new();
        if let Ok(u) = Url::parse(&probe_url_str) {
            probe_urls.push(u);
        }
        if let Ok(u) = Url::parse(&format!("{}/", base_url.trim_end_matches('/'))) {
            probe_urls.push(u);
        }
        for raw in IDP_PROBE_URLS {
            if let Ok(u) = Url::parse(raw) {
                probe_urls.push(u);
            }
        }

        let mut to_delete: Vec<Cookie<'static>> = Vec::new();
        for url in &probe_urls {
            if let Ok(cookies) = main_window.cookies_for_url(url.clone()) {
                for c in cookies {
                    to_delete.push(c);
                }
            }
        }
        // Also sweep the full cookie store for IdP-hosted cookies whose domain
        // we can't guess (e.g. tenant-scoped Azure AD subdomains).
        if let Ok(all) = main_window.cookies() {
            for c in all {
                if let Some(domain) = c.domain()
                    && sap_odata_core::session::is_idp_host(domain)
                {
                    to_delete.push(c);
                }
            }
        }

        attempted = to_delete.len();
        for c in to_delete {
            match main_window.delete_cookie(c) {
                Ok(()) => deleted += 1,
                Err(e) => {
                    eprintln!("sign_out: failed to delete cookie: {e}");
                }
            }
        }
    }

    let persisted_note = match &persisted_err {
        Some(e) => format!(
            " Warning: persisted session could NOT be cleared from keyring ({e}) — it may still be replayed on next app start."
        ),
        None => String::new(),
    };
    let msg = if attempted == 0 {
        format!(
            "Signed out of '{}'. No webview cookies were present.{persisted_note}",
            profile_name
        )
    } else if deleted == attempted {
        format!(
            "Signed out of '{}'. Cleared {deleted} webview cookie(s); next sign-in will prompt for credentials.{persisted_note}",
            profile_name
        )
    } else {
        format!(
            "Signed out of '{}'. Cleared {deleted}/{attempted} webview cookies — {} may remain and could allow silent re-authentication.{persisted_note}",
            profile_name,
            attempted - deleted
        )
    };
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::webview::cookie;

    #[test]
    fn serialize_cookie_expires_roundtrips_through_parser() {
        // Build a cookie with a concrete Expires timestamp, serialize it with
        // our serializer, then parse it back with the same `cookie` crate
        // reqwest uses — the Expires must survive the round-trip. Previously
        // time::Rfc2822 emitted "+0000" which the parser rejects, silently
        // turning persistent cookies into session-only cookies.
        let expires = time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let built: cookie::Cookie<'static> = cookie::Cookie::build(("MYSAPSSO2", "abc"))
            .domain("sap.corp")
            .path("/")
            .expires(expires)
            .secure(true)
            .http_only(true)
            .build();

        let serialized = serialize_webview_cookie(&built);
        assert!(
            serialized.contains("GMT"),
            "Expires must use literal 'GMT' (not +0000); got: {serialized}"
        );

        let parsed = cookie::Cookie::parse(serialized.as_str())
            .expect("serialized cookie must be parseable by cookie crate");
        let parsed_expires = parsed
            .expires_datetime()
            .expect("parsed cookie must have an Expires datetime");
        assert_eq!(
            parsed_expires.unix_timestamp(),
            expires.unix_timestamp(),
            "Expires must round-trip to the same instant"
        );
    }
}

#[derive(Serialize)]
struct ResolvedValueList {
    /// Service path to pass to `run_query` when fetching rows from the
    /// F4 service. Matrix parameters (`;ps=...;va=...`) are preserved
    /// verbatim — SAP's F4 services require them.
    resolved_service_path: String,
    /// Parsed mapping extracted from the F4 service's `$metadata`. Same
    /// shape as an inline `Common.ValueList`, so the frontend picker can
    /// drive it with the existing code path.
    value_list: ValueList,
}

/// Resolve a `Common.ValueListReferences` URL (relative to the current
/// service path) to its F4 service's `ValueListMapping`. Used by the
/// desktop picker when a property's value help is not inline but lives
/// in a separate F4 service (the S/4HANA pattern).
#[tauri::command]
async fn resolve_value_list_reference(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    reference_url: String,
    local_property: String,
) -> CmdResult<ResolvedValueList> {
    // **Fail-closed for offline sources.** The reviewer's HIGH finding
    // from earlier review passes: this command silently builds a live
    // `SapClient` for any profile name it's given, which would defeat
    // the entire offline-mode guarantee. Resolve the source first and
    // assert network is allowed before doing anything else. For v0.2
    // we return a "value-help not available offline" error; future
    // versions can grow cross-EDMX F4 resolution against another
    // cached service in the same offline profile.
    let (cfg, _) =
        config::load_config().map_err(|e| CommandError::msg(format!("Config error: {e}")))?;
    let source =
        sap_odata_core::offline::MetadataSource::resolve(&profile_name, &service_path, &cfg)
            .map_err(|e| CommandError::msg(e.to_string()))?;
    if source.assert_network_allowed().is_err() {
        return Err(CommandError::msg(format!(
            "Value-help (F4) is not available offline for profile '{profile_name}'. The referenced service '{reference_url}' would require a live fetch; cross-EDMX resolution against another cached service is planned for a future release."
        )));
    }

    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    let resolved_service_path = resolve_reference_path(&service_path, &reference_url);
    let xml = client
        .fetch_metadata_xml(&resolved_service_path)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("F4 metadata fetch error: {e}")))?;
    let mut vl = metadata::parse_value_list_mapping_xml(&xml, &local_property).ok_or_else(|| {
        CommandError::with_client(
            &client,
            format!(
                "No Common.ValueListMapping targeting '{local_property}' found in referenced service '{resolved_service_path}'"
            ),
        )
    })?;
    // SAP F4 services don't put `SearchSupported` on the ValueListMapping
    // record itself — they declare `Capabilities.SearchRestrictions.
    // Searchable` at the entity-set level on the F4 service. Parse the
    // F4 metadata once more to lift that flag onto the returned
    // `ValueList.search_supported` so the picker's $search input
    // visibility check works uniformly for both shapes. Parse errors
    // here are swallowed — they don't invalidate the mapping we already
    // parsed successfully.
    if vl.search_supported.is_none()
        && let Ok(f4_meta) = metadata::parse_metadata(&xml)
        && let Some(et) = f4_meta.entity_type_for_set(&vl.collection_path)
        && et.searchable == Some(true)
    {
        vl.search_supported = Some(true);
    }
    Ok(CommandOk {
        data: ResolvedValueList {
            resolved_service_path,
            value_list: vl,
        },
        trace: client.diagnostics_snapshot(),
    })
}

/// Resolve a relative reference URL (from `Common.ValueListReferences`)
/// against the current service path. Works on the path component only —
/// no percent-encoding so SAP's matrix parameters (`;ps='...'`) survive
/// intact. The trailing `/$metadata` is stripped so the result can be
/// passed to `fetch_metadata_xml` (which re-appends it).
fn resolve_reference_path(base: &str, relative: &str) -> String {
    let resolved = if relative.starts_with('/') {
        // Absolute path — take as-is.
        relative.to_string()
    } else {
        // Treat the base path as a directory; walk `..` up from there.
        let mut parts: Vec<&str> = base.trim_end_matches('/').split('/').collect();
        for seg in relative.split('/') {
            match seg {
                "." | "" => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        parts.join("/")
    };
    resolved
        .strip_suffix("/$metadata")
        .unwrap_or(&resolved)
        .to_string()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sap_odata=warn".parse().unwrap()),
        )
        .with_target(false)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_profiles,
            get_services,
            resolve_service,
            get_entities,
            get_annotations,
            describe_entity,
            run_query,
            resolve_value_list_reference,
            add_profile,
            save_service_offline,
            import_edmx_file,
            import_edmx_bytes,
            list_offline_services,
            delete_offline_service,
            remove_profile,
            test_connection,
            browser_sign_in_profile,
            sign_out_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
