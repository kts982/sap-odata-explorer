#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    catalog,
    client::SapClient,
    config,
    query::ODataQuery,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tauri::{Manager, Url, WebviewUrl, WindowEvent, webview::{Cookie, WebviewWindowBuilder}};

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

#[derive(Serialize)]
struct EntityTypeInfo {
    name: String,
    keys: Vec<String>,
    properties: Vec<PropertyInfo>,
    nav_properties: Vec<NavPropertyInfo>,
}

#[derive(Serialize)]
struct PropertyInfo {
    name: String,
    edm_type: String,
    nullable: bool,
    max_length: Option<u32>,
    label: Option<String>,
    is_key: bool,
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
        let key = browser_session_key(&connection.base_url, &connection.client, &connection.language);
        if let Some(session) = state.browser_sessions.lock().unwrap().get(&key).cloned() {
            client
                .import_cookie_strings(&session.request_url, &session.cookies)
                .map_err(|e| format!("Cookie import error: {e}"))?;
        }
    }

    Ok(client)
}

fn resolve_service_for_profile(profile_name: &str, service: &str) -> Result<String, String> {
    if service.starts_with('/') {
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
) -> Result<String, String> {
    let request_url = build_browser_probe_url(&base_url, &client, &language);
    let request_url_parsed = Url::parse(&request_url).map_err(|e| format!("Invalid sign-in URL: {e}"))?;
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
            // Detect IdP domains
            if host.contains("microsoftonline.com")
                || host.contains("ondemand.com")
                || host.contains("okta.com")
                || host.contains("auth0.com")
            {
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
    .map_err(|e| format!("Failed to open sign-in window: {e}"))?;

    auth_window.on_window_event({
        let tx = tx.clone();
        move |event| {
            if matches!(event, WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed) {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(Err("Sign-in cancelled".to_string()));
                }
            }
        }
    });

    let outcome = rx
        .await
        .map_err(|_| "Sign-in window closed unexpectedly".to_string())?;
    outcome?;

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
        return Err(format!(
            "Sign-in finished, but no SAP session cookies were found.\n\
             Tried URLs: {} and {}\n\
             The SAP system may set cookies on a different domain or path.",
            base_url_parsed, request_url_parsed
        ));
    }

    let key = browser_session_key(&base_url, &client, &language);
    state.browser_sessions.lock().unwrap().insert(
        key.clone(),
        BrowserSession {
            request_url: request_url.clone(),
            cookies: cookie_strings,
        },
    );

    let _ = auth_window.close();

    let sap_client = SapClient::new(SapConnection {
        base_url,
        client,
        language,
        auth: AuthConfig::Browser,
        insecure_tls,
    })
    .map_err(|e| format!("Client error: {e}"))?;

    if let Some(session) = state.browser_sessions.lock().unwrap().get(&key).cloned() {
        sap_client
            .import_cookie_strings(&session.request_url, &session.cookies)
            .map_err(|e| format!("Cookie import error: {e}"))?;
    }

    sap_client
        .ensure_session(browser_probe_path())
        .await
        .map_err(|e| format!("Browser sign-in succeeded, but SAP session validation failed: {e}"))?;

    Ok("Browser sign-in successful".to_string())
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
) -> Result<Vec<ServiceInfo>, String> {
    let client = client_from_profile(&profile_name, &state)?;
    let result = catalog::fetch_service_catalog(&client)
        .await
        .map_err(|e| format!("Catalog error: {e}"))?;

    // Surface warnings as part of the error if no entries found
    if result.entries.is_empty() && !result.warnings.is_empty() {
        return Err(format!("No services found. Warnings: {}", result.warnings.join("; ")));
    }

    Ok(result.entries
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
        .collect())
}

#[tauri::command]
async fn resolve_service(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service: String,
) -> Result<String, String> {
    // Try alias first
    if let Ok(path) = resolve_service_for_profile(&profile_name, &service) {
        return Ok(path);
    }
    // Try catalog
    let client = client_from_profile(&profile_name, &state)?;
    catalog::resolve_service_by_name(&client, &service)
        .await
        .map_err(|e| format!("Resolution error: {e}"))
}

#[tauri::command]
async fn get_entities(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
) -> Result<Vec<EntitySetInfo>, String> {
    let client = client_from_profile(&profile_name, &state)?;
    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| format!("Metadata error: {e}"))?;

    Ok(meta
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
        .collect())
}

#[tauri::command]
async fn describe_entity(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    entity_set: String,
) -> Result<EntityTypeInfo, String> {
    let client = client_from_profile(&profile_name, &state)?;
    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| format!("Metadata error: {e}"))?;

    let et = meta
        .entity_type_for_set(&entity_set)
        .ok_or_else(|| format!("Entity set '{}' not found", entity_set))?;

    let nav_targets = meta.nav_targets(et);

    Ok(EntityTypeInfo {
        name: et.name.clone(),
        keys: et.keys.clone(),
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
    })
}

#[tauri::command]
async fn run_query(
    state: tauri::State<'_, AppState>,
    profile_name: String,
    service_path: String,
    params: QueryParams,
) -> Result<serde_json::Value, String> {
    let client = client_from_profile(&profile_name, &state)?;

    let meta = client
        .fetch_metadata(&service_path)
        .await
        .map_err(|e| format!("Metadata error: {e}"))?;

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

    client
        .query_json(&service_path, &q)
        .await
        .map_err(|e| format!("Query error: {e}"))
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

    let sso = auth_mode == "sso";
    let browser_sso = auth_mode == "browser";
    let profile = config::ConnectionProfile {
        base_url,
        client,
        language,
        username: if browser_sso { String::new() } else { username.clone() },
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
                config::save_config(&cfg, &dir.path)
                    .map_err(|e| format!("Save error: {e}"))?;
                return Ok(format!("Profile '{}' saved (password in config file)", name));
            }
        }
    }

    cfg.connections.insert(name.clone(), profile);
    let dir = config::get_or_create_config_dir()
        .map_err(|e| format!("Config dir error: {e}"))?;
    config::save_config(&cfg, &dir.path)
        .map_err(|e| format!("Save error: {e}"))?;
    let mode = match auth_mode.as_str() {
        "sso" => " with Windows SSO",
        "browser" => " with browser SSO",
        _ => "",
    };
    Ok(format!("Profile '{}' saved{mode}", name))
}

#[tauri::command]
fn remove_profile(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<String, String> {
    let (mut cfg, config_dir) = config::load_config().map_err(|e| format!("Config error: {e}"))?;

    let profile = cfg.connections.remove(&name)
        .ok_or_else(|| format!("Profile '{}' not found", name))?;

    if !profile.sso && !profile.browser_sso && !profile.username.is_empty() {
        let _ = config::delete_password_from_keyring(&name, &profile.username);
    }
    let key = browser_session_key(&profile.base_url, &profile.client, &profile.language);
    state.browser_sessions.lock().unwrap().remove(&key);
    config::save_config(&cfg, &config_dir.path)
        .map_err(|e| format!("Save error: {e}"))?;
    Ok(format!("Profile '{}' removed", name))
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
) -> Result<String, String> {
    if auth_mode == "browser" {
        return browser_sign_in_for_connection(
            app,
            &state,
            base_url,
            client,
            language,
            false,
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

    let sap_client = SapClient::new(connection)
        .map_err(|e| format!("Client error: {e}"))?;
    sap_client
        .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;
    Ok("Connection successful".to_string())
}

#[tauri::command]
async fn browser_sign_in_profile(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_name: String,
) -> Result<String, String> {
    let (cfg, _) = config::load_config().map_err(|e| format!("Config error: {e}"))?;
    let profile = cfg
        .connections
        .get(&profile_name)
        .ok_or_else(|| format!("Profile '{}' not found", profile_name))?;

    if !profile.browser_sso {
        return Err(format!("Profile '{}' is not configured for browser SSO", profile_name));
    }

    browser_sign_in_for_connection(
        app,
        &state,
        profile.base_url.clone(),
        profile.client.clone(),
        profile.language.clone(),
        profile.insecure_tls,
    )
    .await
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
            add_profile,
            remove_profile,
            test_connection,
            browser_sign_in_profile,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
