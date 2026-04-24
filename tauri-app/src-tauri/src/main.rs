#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    catalog,
    client::SapClient,
    config,
    diagnostics::HttpTraceEntry,
    metadata::{
        self, AnnotationSummary, Criticality, FieldControl, HeaderInfo, LineItemField,
        SelectionVariant, TextArrangement, ValueList,
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
    base_url: String,
    client: String,
    username: String,
    password_source: String,
    auth_mode: String,
    aliases: Vec<AliasInfo>,
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
    selection_variants: Vec<SelectionVariant>,
    searchable: Option<bool>,
    countable: Option<bool>,
    top_supported: Option<bool>,
    skip_supported: Option<bool>,
    expandable: Option<bool>,
    non_expandable_properties: Vec<String>,
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
    value_list_references: Vec<String>,
    value_list_fixed: bool,
    text_arrangement: Option<TextArrangement>,
    field_control: Option<FieldControl>,
    hidden: bool,
    hidden_filter: bool,
    display_format: Option<String>,
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
            if was_at_idp && is_browser_auth_complete(url, &expected_url) {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(Ok(()));
                }
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
            ) {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(Err("Sign-in cancelled".to_string()));
                }
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

    Ok(cfg
        .connections
        .iter()
        .map(|(name, profile)| {
            let password_source = if profile.browser_sso {
                "browser".to_string()
            } else if profile.sso {
                "none".to_string()
            } else if profile.password.is_some() {
                "config".to_string()
            } else if config::get_password_from_keyring(name, &profile.username).is_some() {
                "keyring".to_string()
            } else {
                "none".to_string()
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
                base_url: profile.base_url.clone(),
                client: profile.client.clone(),
                username: profile.username.clone(),
                password_source,
                auth_mode: auth_mode_for_profile(profile).to_string(),
                aliases,
            }
        })
        .collect())
}

#[tauri::command]
async fn get_services(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    search: Option<String>,
    v4_only: bool,
) -> CmdResult<Vec<ServiceInfo>> {
    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    let result = catalog::fetch_service_catalog(&client)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("Catalog error: {e}")))?;

    if result.entries.is_empty() && !result.warnings.is_empty() {
        return Err(CommandError::with_client(
            &client,
            format!("No services found. Warnings: {}", result.warnings.join("; ")),
        ));
    }

    let data = result
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
        data,
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

#[tauri::command]
async fn get_entities(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
) -> CmdResult<EntityListResponse> {
    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("Metadata error: {e}")))?;

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
        trace: client.diagnostics_snapshot(),
    })
}

#[tauri::command]
async fn describe_entity(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    entity_set: String,
) -> CmdResult<EntityTypeInfo> {
    let client = client_from_profile(&profile_name, &state).map_err(CommandError::msg)?;
    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| CommandError::with_client(&client, format!("Metadata error: {e}")))?;

    let et = meta.entity_type_for_set(&entity_set).ok_or_else(|| {
        CommandError::with_client(&client, format!("Entity set '{}' not found", entity_set))
    })?;

    let nav_targets = meta.nav_targets(et);

    let data = EntityTypeInfo {
        name: et.name.clone(),
        keys: et.keys.clone(),
        header_info: et.header_info.clone(),
        selection_fields: et.selection_fields.clone(),
        line_item: et.line_item.clone(),
        request_at_least: et.request_at_least.clone(),
        selection_variants: et.selection_variants.clone(),
        searchable: et.searchable,
        countable: et.countable,
        top_supported: et.top_supported,
        skip_supported: et.skip_supported,
        expandable: et.expandable,
        non_expandable_properties: et.non_expandable_properties.clone(),
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
                value_list_references: p.value_list_references.clone(),
                value_list_fixed: p.value_list_fixed,
                text_arrangement: p.text_arrangement,
                field_control: p.field_control.clone(),
                hidden: p.hidden,
                hidden_filter: p.hidden_filter,
                display_format: p.display_format.clone(),
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
    Ok(CommandOk {
        data,
        trace: client.diagnostics_snapshot(),
    })
}

#[tauri::command]
async fn run_query(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    params: QueryParams,
) -> CmdResult<serde_json::Value> {
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
fn add_profile(
    name: String,
    base_url: String,
    client: String,
    language: String,
    auth_mode: String,
    username: String,
    password: String,
) -> Result<String, String> {
    let (mut cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

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
        aliases: existing_aliases,
    };

    if auth_mode == "basic" && !password.is_empty() {
        // Store password in keyring
        match config::set_password_in_keyring(&name, &username, &password) {
            Ok(()) => {}
            Err(_) => {
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

#[tauri::command]
fn remove_profile(state: tauri::State<'_, AppState>, name: String) -> Result<String, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

    let profile = cfg
        .connections
        .remove(&name)
        .ok_or_else(|| format!("Profile '{}' not found", name))?;

    let mut warnings: Vec<String> = Vec::new();
    if !profile.sso && !profile.browser_sso && !profile.username.is_empty() {
        if let Err(e) = config::delete_password_from_keyring(&name, &profile.username) {
            warnings.push(format!("password (user: {}) — {e}", profile.username));
        }
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

#[tauri::command]
async fn test_connection(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    base_url: String,
    client: String,
    language: String,
    auth_mode: String,
    username: String,
    password: String,
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
                if let Some(domain) = c.domain() {
                    if sap_odata_core::session::is_idp_host(domain) {
                        to_delete.push(c);
                    }
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
    if vl.search_supported.is_none() {
        if let Ok(f4_meta) = metadata::parse_metadata(&xml) {
            if let Some(et) = f4_meta.entity_type_for_set(&vl.collection_path) {
                if et.searchable == Some(true) {
                    vl.search_supported = Some(true);
                }
            }
        }
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
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_profiles,
            get_services,
            resolve_service,
            get_entities,
            describe_entity,
            run_query,
            resolve_value_list_reference,
            add_profile,
            remove_profile,
            test_connection,
            browser_sign_in_profile,
            sign_out_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
