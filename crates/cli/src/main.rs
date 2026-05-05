use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    client::SapClient,
    config::{self, ConnectionProfile},
    diagnostics::HttpTraceEntry,
    lint::{self, LintSeverity},
    metadata::ODataVersion,
    query::ODataQuery,
};

#[derive(Parser)]
#[command(
    name = "sap-odata",
    version,
    about = "SAP OData Service Explorer — browse entities, build queries, test services"
)]
struct Cli {
    /// Connection profile name (from connections.toml)
    #[arg(long, short = 'p', global = true)]
    profile: Option<String>,

    /// SAP system base URL (overrides profile)
    #[arg(long, env = "SAP_BASE_URL")]
    url: Option<String>,

    /// SAP client number (overrides profile)
    #[arg(long, env = "SAP_CLIENT")]
    client: Option<String>,

    /// Language key (overrides profile)
    #[arg(long, env = "SAP_LANGUAGE")]
    language: Option<String>,

    /// Username for basic auth (overrides profile)
    #[arg(long, env = "SAP_USER")]
    user: Option<String>,

    /// Password for basic auth (overrides profile)
    #[arg(long, env = "SAP_PASSWORD")]
    password: Option<String>,

    /// OData service path (e.g., /sap/opu/odata/sap/ZMY_SERVICE_SRV).
    /// Required for all commands except 'services' and 'profile'.
    #[arg(long, short = 's')]
    service: Option<String>,

    /// Output as JSON instead of table
    #[arg(long, global = true)]
    json: bool,

    /// Show debug logs
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interactive wizard for adding a new SAP system
    Setup,

    /// Clear persisted Browser SSO session for a profile
    Signout {
        /// Profile name
        name: String,
    },

    /// Manage connection profiles
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },

    /// Manage service aliases for a profile (requires -p)
    Alias {
        #[command(subcommand)]
        action: AliasAction,
    },

    /// List all available OData services from the SAP Gateway catalog
    Services {
        /// Filter services by name, title, or description (case-insensitive)
        #[arg(long, short = 'f')]
        filter: Option<String>,

        /// Show only V2 services
        #[arg(long)]
        v2: bool,

        /// Show only V4 services
        #[arg(long)]
        v4: bool,

        /// Max number of services to display
        #[arg(long)]
        top: Option<usize>,
    },

    /// List all entity sets in the service
    Entities,

    /// Describe an entity type — show properties, keys, and navigation properties
    Describe {
        /// Entity set name (e.g., CustomerSet)
        entity_set: String,
    },

    /// List function imports in the service
    Functions,

    /// Build and display an OData query URL (dry-run)
    Build {
        /// Entity set name
        entity_set: String,

        /// Fields to select (comma-separated)
        #[arg(long)]
        select: Option<String>,

        /// Filter expression
        #[arg(long)]
        filter: Option<String>,

        /// Navigation properties to expand (comma-separated)
        #[arg(long)]
        expand: Option<String>,

        /// Order by clause (comma-separated, e.g. "Name asc,Date desc")
        #[arg(long)]
        orderby: Option<String>,

        /// Max number of results
        #[arg(long)]
        top: Option<u32>,

        /// Number of results to skip
        #[arg(long)]
        skip: Option<u32>,

        /// Entity key (e.g., "'CUST01'" or "CustomerID='CUST01',CompanyCode='1000'")
        #[arg(long)]
        key: Option<String>,

        /// Request inline count
        #[arg(long)]
        count: bool,
    },

    /// Execute an OData query and display the results
    Run {
        /// Entity set name
        entity_set: String,

        /// Fields to select (comma-separated)
        #[arg(long)]
        select: Option<String>,

        /// Filter expression
        #[arg(long)]
        filter: Option<String>,

        /// Navigation properties to expand (comma-separated)
        #[arg(long)]
        expand: Option<String>,

        /// Order by clause
        #[arg(long)]
        orderby: Option<String>,

        /// Max number of results
        #[arg(long)]
        top: Option<u32>,

        /// Number of results to skip
        #[arg(long)]
        skip: Option<u32>,

        /// Entity key
        #[arg(long)]
        key: Option<String>,

        /// Request inline count
        #[arg(long)]
        count: bool,
    },

    /// Fetch and display the raw $metadata XML
    Metadata,

    /// Fiori-readiness checklist for an entity. Evaluates the
    /// service's annotations against what a Fiori list-report /
    /// object-page app would normally expect (HeaderInfo, LineItem,
    /// SelectionFields, Text/Unit pairings, Capabilities...). Scope
    /// it to one entity or omit the positional arg to scan the whole
    /// service.
    Lint {
        /// Entity type / entity set name. Either works — we resolve to
        /// the type. Omit to lint every entity in the service.
        entity: Option<String>,

        /// Only show findings at or above this severity level
        /// (pass / warn / miss). Defaults to showing everything.
        #[arg(long)]
        min_severity: Option<String>,
    },

    /// List every SAP/UI5 annotation parsed from `$metadata`, grouped
    /// by vocabulary namespace. Useful for answering "does this
    /// service declare X?" without re-reading the raw XML.
    Annotations {
        /// Filter to a specific namespace (e.g. "UI", "Common",
        /// "Capabilities"). Case-insensitive. Omit to list all.
        #[arg(long)]
        namespace: Option<String>,

        /// Substring match applied to Term + Target + Value +
        /// Qualifier (case-insensitive).
        #[arg(long)]
        filter: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProfileAction {
    /// List all saved connection profiles
    List,

    /// Add or update a connection profile
    Add {
        /// Profile name (e.g., DEV, QAS, PRD)
        name: String,

        /// SAP system base URL
        #[arg(long)]
        url: String,

        /// SAP client number
        #[arg(long, default_value = "100")]
        client: String,

        /// Language key
        #[arg(long, default_value = "EN")]
        language: String,

        /// Username (not needed for --sso)
        #[arg(long, default_value = "")]
        user: String,

        /// Password (not needed for --sso; stored in OS keyring by default)
        #[arg(long, default_value = "")]
        password: String,

        /// Use Windows SSO (SPNEGO/Kerberos) — no username/password needed
        #[arg(long)]
        sso: bool,

        /// Allow Kerberos delegation when --sso is set (server can impersonate
        /// the user to downstream backends). Off by default; only enable for
        /// reverse-proxy → Gateway → backend R/3 setups that need multi-hop auth.
        #[arg(long)]
        sso_delegate: bool,

        /// Store password in config file instead of OS keyring (not recommended)
        #[arg(long)]
        plaintext: bool,

        /// Create portable config next to the executable
        #[arg(long)]
        portable: bool,
    },

    /// Remove a connection profile
    Remove {
        /// Profile name to remove
        name: String,
    },

    /// Test a connection profile
    Test {
        /// Profile name to test
        name: String,
    },

    /// Show where the config file is located
    Where,
}

#[derive(Subcommand)]
enum AliasAction {
    /// List all aliases for a profile
    List,

    /// Add or update a service alias
    Add {
        /// Short name (e.g., warehouse, orders)
        name: String,

        /// Full OData service path
        path: String,
    },

    /// Remove a service alias
    Remove {
        /// Alias name to remove
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose {
        "sap_odata=debug"
    } else {
        "sap_odata=warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(log_level.parse().unwrap()),
        )
        .with_target(false)
        .init();

    // Handle commands that don't need a SAP connection
    if let Commands::Profile { action } = &cli.command {
        return handle_profile_command(action).await;
    }
    if let Commands::Alias { action } = &cli.command {
        return handle_alias_command(action, cli.profile.as_deref());
    }
    if matches!(&cli.command, Commands::Setup) {
        return cmd_setup_wizard().await;
    }
    if let Commands::Signout { name } = &cli.command {
        return cmd_signout(name);
    }

    // Resolve connection: profile → env vars → CLI flags
    let connection = resolve_connection(&cli)?;
    let uses_browser_sso = matches!(&connection.auth, AuthConfig::Browser);
    let sap_client = SapClient::new(connection).context("failed to create SAP client")?;

    // For Browser SSO profiles, try to load persisted cookies from keyring.
    // If none are present, fail early with a clear message.
    if uses_browser_sso && let Some(ref profile_name) = cli.profile {
        let loaded = sap_client
            .try_load_persisted_session(profile_name)
            .context("failed to load persisted session")?;
        if !loaded {
            anyhow::bail!(
                "Browser SSO profile '{}' has no active session.\n\
                     Open the desktop app, select this profile and click 'Sign In',\n\
                     then re-run your command.",
                profile_name
            );
        }
    }

    let result = async {
        // Resolve service path: alias -> catalog lookup -> raw path
        let resolved_service = resolve_service_path(&cli, &sap_client).await?;

        let require_service = || -> Result<String> {
            resolved_service
                .clone()
                .ok_or_else(|| anyhow::anyhow!("--service / -s is required for this command"))
        };

        match &cli.command {
            Commands::Profile { .. }
            | Commands::Alias { .. }
            | Commands::Setup
            | Commands::Signout { .. } => unreachable!(),
            Commands::Services {
                filter,
                v2,
                v4,
                top,
            } => cmd_services(&sap_client, filter.as_deref(), *v2, *v4, *top, cli.json).await,
            Commands::Entities => {
                let svc = require_service()?;
                cmd_entities(&sap_client, &svc, cli.json).await
            }
            Commands::Describe { entity_set } => {
                let svc = require_service()?;
                cmd_describe(&sap_client, &svc, entity_set, cli.json).await
            }
            Commands::Functions => {
                let svc = require_service()?;
                cmd_functions(&sap_client, &svc, cli.json).await
            }
            Commands::Build {
                entity_set,
                select,
                filter,
                expand,
                orderby,
                top,
                skip,
                key,
                count,
            } => {
                let svc = require_service()?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    filter.clone(),
                    expand.clone(),
                    orderby.clone(),
                    *top,
                    *skip,
                    key.clone(),
                    *count,
                    None,
                );
                let full_url = format!("{}/{}", svc.trim_end_matches('/'), query.build());
                println!("{full_url}");
                Ok(())
            }
            Commands::Run {
                entity_set,
                select,
                filter,
                expand,
                orderby,
                top,
                skip,
                key,
                count,
            } => {
                let svc = require_service()?;
                let meta = sap_client
                    .fetch_metadata(&svc)
                    .await
                    .context("failed to fetch metadata")?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    filter.clone(),
                    expand.clone(),
                    orderby.clone(),
                    *top,
                    *skip,
                    key.clone(),
                    *count,
                    Some(meta.version),
                );
                cmd_run(&sap_client, &svc, &query, cli.json).await
            }
            Commands::Metadata => {
                let svc = require_service()?;
                cmd_metadata(&sap_client, &svc).await
            }
            Commands::Annotations { namespace, filter } => {
                let svc = require_service()?;
                cmd_annotations(
                    &sap_client,
                    &svc,
                    cli.json,
                    namespace.clone(),
                    filter.clone(),
                )
                .await
            }
            Commands::Lint {
                entity,
                min_severity,
            } => {
                let svc = require_service()?;
                cmd_lint(
                    &sap_client,
                    &svc,
                    cli.json,
                    entity.clone(),
                    min_severity.clone(),
                )
                .await
            }
        }
    }
    .await;

    if cli.verbose {
        emit_http_trace(&sap_client);
    }

    result
}

fn emit_http_trace(client: &SapClient) {
    let trace = client.diagnostics_snapshot();
    if trace.is_empty() {
        return;
    }

    eprintln!();
    eprintln!("HTTP trace ({} exchange(s)):", trace.len());
    for entry in &trace {
        emit_http_trace_entry(entry);
    }
}

fn emit_http_trace_entry(entry: &HttpTraceEntry) {
    let status = entry
        .status
        .map(|status| status.to_string())
        .unwrap_or_else(|| "ERR".to_string());

    eprintln!();
    eprintln!("[{}] {} {}", entry.id, entry.method, entry.url);
    eprintln!("  status: {status}    time: {}ms", entry.duration_ms);

    if let Some(location) = &entry.redirect_location {
        eprintln!("  redirect: {location}");
    }
    if let Some(error) = &entry.error {
        eprintln!("  transport error: {error}");
    }

    if !entry.request_headers.is_empty() {
        eprintln!("  request headers:");
        for header in &entry.request_headers {
            eprintln!("    {}: {}", header.name, header.value);
        }
    }

    if !entry.response_headers.is_empty() {
        eprintln!("  response headers:");
        for header in &entry.response_headers {
            eprintln!("    {}: {}", header.name, header.value);
        }
    }

    if let Some(body) = entry.response_body_preview.as_deref() {
        eprintln!("  response preview:");
        for line in body.lines() {
            eprintln!("    {line}");
        }
    }
}

/// Resolve connection from profile, env vars, or CLI flags.
/// Priority: CLI flags > env vars > profile.
fn resolve_connection(cli: &Cli) -> Result<SapConnection> {
    // If a profile is specified, load it as the base
    let (profile_conn, profile_name) = if let Some(ref name) = cli.profile {
        let (cfg, _) = config::load_config().context("failed to load config")?;
        let profile = cfg.connections.get(name).ok_or_else(|| {
            anyhow::anyhow!(
                "profile '{}' not found. Run 'sap-odata profile list' to see available profiles.",
                name
            )
        })?;
        let conn = config::resolve_connection(name, profile)
            .with_context(|| format!("failed to resolve profile '{name}'"))?;
        (Some(conn), Some(name.clone()))
    } else {
        (None, None)
    };

    // Build final connection, CLI/env overriding profile
    let base_url = cli
        .url
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.base_url.clone()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no SAP URL specified. Use --url, SAP_BASE_URL env, or --profile.{}",
                if profile_name.is_none() {
                    " Run 'sap-odata profile list' to see saved profiles."
                } else {
                    ""
                }
            )
        })?;

    let client = cli
        .client
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.client.clone()))
        .unwrap_or_else(|| "100".to_string());

    let language = cli
        .language
        .clone()
        .or_else(|| profile_conn.as_ref().map(|c| c.language.clone()))
        .unwrap_or_else(|| "EN".to_string());

    // Check if profile uses a non-password auth mode
    let uses_non_basic_auth = profile_conn
        .as_ref()
        .is_some_and(|c| matches!(&c.auth, AuthConfig::Sso | AuthConfig::Browser));

    let insecure_tls = profile_conn.as_ref().is_some_and(|c| c.insecure_tls);

    if uses_non_basic_auth && cli.user.is_none() && cli.password.is_none() {
        let auth = profile_conn
            .as_ref()
            .map(|c| c.auth.clone())
            .unwrap_or(AuthConfig::Sso);
        let sso_delegate = profile_conn.as_ref().is_some_and(|c| c.sso_delegate);
        return Ok(SapConnection {
            base_url,
            client,
            language,
            auth,
            insecure_tls,
            sso_delegate,
        });
    }

    let username = cli
        .user
        .clone()
        .or_else(|| {
            profile_conn.as_ref().and_then(|c| match &c.auth {
                AuthConfig::Basic { username, .. } => Some(username.clone()),
                AuthConfig::Sso | AuthConfig::Browser => None,
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!("no username specified. Use --user, SAP_USER env, --profile, or --sso.")
        })?;

    let password = cli
        .password
        .clone()
        .or_else(|| {
            profile_conn.as_ref().and_then(|c| match &c.auth {
                AuthConfig::Basic { password, .. } => Some(password.clone()),
                AuthConfig::Sso | AuthConfig::Browser => None,
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no password specified. Use --password, SAP_PASSWORD env, --profile, or --sso."
            )
        })?;

    Ok(SapConnection {
        base_url,
        client,
        language,
        auth: AuthConfig::Basic { username, password },
        insecure_tls,
        sso_delegate: false,
    })
}

/// Resolve the -s flag: alias → catalog lookup → raw path.
/// Returns None if no -s was given (OK for services/profile commands).
async fn resolve_service_path(cli: &Cli, sap_client: &SapClient) -> Result<Option<String>> {
    let raw = match &cli.service {
        Some(s) => s.clone(),
        None => return Ok(None),
    };

    // If it looks like an SAP OData service path (`/sap/...`), use it verbatim.
    // Anything else rooted at `/` — notably SAP namespace-prefixed catalog
    // names like `/NAMESPACE/SERVICE_NAME` — still needs catalog resolution.
    if raw.starts_with("/sap/") {
        return Ok(Some(raw));
    }

    // Try profile aliases first
    if let Some(ref profile_name) = cli.profile {
        let (cfg, _) = config::load_config().context("failed to load config")?;
        if let Some(profile) = cfg.connections.get(profile_name) {
            // Case-insensitive alias lookup
            let alias_lower = raw.to_lowercase();
            for (alias_name, alias_path) in &profile.aliases {
                if alias_name.to_lowercase() == alias_lower {
                    println!("  Resolved alias '{}' -> {}", raw, alias_path);
                    return Ok(Some(alias_path.clone()));
                }
            }
        }
    }

    // Try catalog resolution (searches V2 + V4 by technical name)
    println!("  Resolving service '{}'...", raw);
    match sap_odata_core::catalog::resolve_service_by_name(sap_client, &raw).await {
        Ok(path) => {
            println!("  Resolved '{}' -> {}", raw, path);
            Ok(Some(path))
        }
        Err(e) => Err(anyhow::anyhow!(
            "could not resolve service '{}': {}. Use a full path starting with / or add an alias.",
            raw,
            e
        )),
    }
}

// ── Alias commands ──

fn handle_alias_command(action: &AliasAction, profile_name: Option<&str>) -> Result<()> {
    let profile_name = profile_name
        .ok_or_else(|| anyhow::anyhow!("--profile / -p is required for alias commands"))?;

    match action {
        AliasAction::List => cmd_alias_list(profile_name),
        AliasAction::Add { name, path } => cmd_alias_add(profile_name, name, path),
        AliasAction::Remove { name } => cmd_alias_remove(profile_name, name),
    }
}

fn cmd_alias_list(profile_name: &str) -> Result<()> {
    let (cfg, _) = config::load_config().context("failed to load config")?;
    let profile = cfg
        .connections
        .get(profile_name)
        .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", profile_name))?;

    if profile.aliases.is_empty() {
        println!(
            "No aliases for profile '{}'. Use 'sap-odata -p {} alias add <name> <path>' to create one.",
            profile_name, profile_name
        );
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("Alias").fg(Color::DarkCyan),
        Cell::new("Service Path").fg(Color::DarkCyan),
    ]);

    for (name, path) in &profile.aliases {
        table.add_row(vec![Cell::new(name), Cell::new(path)]);
    }

    println!("{table}");
    Ok(())
}

fn cmd_alias_add(profile_name: &str, alias_name: &str, service_path: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("failed to load config")?;
    let profile = cfg
        .connections
        .get_mut(profile_name)
        .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", profile_name))?;

    profile
        .aliases
        .insert(alias_name.to_string(), service_path.to_string());
    config::save_config(&cfg, &config_dir.path)?;
    println!(
        "  Alias '{}' -> {} saved for profile '{}'.",
        alias_name, service_path, profile_name
    );
    Ok(())
}

fn cmd_alias_remove(profile_name: &str, alias_name: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("failed to load config")?;
    let profile = cfg
        .connections
        .get_mut(profile_name)
        .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", profile_name))?;

    if profile.aliases.remove(alias_name).is_none() {
        anyhow::bail!(
            "alias '{}' not found in profile '{}'",
            alias_name,
            profile_name
        );
    }

    config::save_config(&cfg, &config_dir.path)?;
    println!(
        "  Alias '{}' removed from profile '{}'.",
        alias_name, profile_name
    );
    Ok(())
}

// ── Profile commands ──

// ══════════════════════════════════════════════════════════════
// ── SETUP WIZARD ──
// ══════════════════════════════════════════════════════════════

async fn cmd_setup_wizard() -> Result<()> {
    use dialoguer::{Confirm, Input, Password, Select, theme::ColorfulTheme};

    let theme = ColorfulTheme::default();

    println!();
    println!("SAP OData Explorer — profile setup wizard");
    println!("─────────────────────────────────────────");
    println!();

    // Profile name
    let name: String = Input::with_theme(&theme)
        .with_prompt("Profile name (e.g. DEV, QAS, PRD)")
        .validate_with(|input: &String| -> std::result::Result<(), &str> {
            if input.trim().is_empty() {
                Err("Name cannot be empty")
            } else if input.contains(|c: char| c.is_whitespace() || c == '/' || c == '\\') {
                Err("Name cannot contain whitespace or slashes")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    let name = name.trim().to_string();

    // Base URL
    let url: String = Input::with_theme(&theme)
        .with_prompt("SAP base URL (e.g. https://sap.example.com or https://host:44300)")
        .validate_with(|input: &String| -> std::result::Result<(), &str> {
            if !input.starts_with("http://") && !input.starts_with("https://") {
                Err("URL must start with http:// or https://")
            } else {
                Ok(())
            }
        })
        .interact_text()?;
    let url = url.trim().trim_end_matches('/').to_string();

    // Client
    let client: String = Input::with_theme(&theme)
        .with_prompt("SAP client")
        .default("100".to_string())
        .interact_text()?;

    // Language
    let language: String = Input::with_theme(&theme)
        .with_prompt("Language key")
        .default("EN".to_string())
        .interact_text()?;

    // Auth method
    let auth_modes = &[
        "Basic (username + password)",
        "Windows SSO (Kerberos / SPNEGO)",
        "Browser SSO (Azure AD / SAP IAS / SAML)",
    ];
    let auth_idx = Select::with_theme(&theme)
        .with_prompt("Authentication method")
        .items(auth_modes)
        .default(0)
        .interact()?;

    let (username, password, sso, browser_sso) = match auth_idx {
        0 => {
            let user: String = Input::with_theme(&theme)
                .with_prompt("Username")
                .interact_text()?;
            let pass = Password::with_theme(&theme)
                .with_prompt("Password")
                .interact()?;
            (user.trim().to_string(), pass, false, false)
        }
        1 => (String::new(), String::new(), true, false),
        2 => {
            println!();
            println!(
                "  Browser SSO needs a one-time sign-in from the desktop app or VS Code extension."
            );
            println!("  After signing in there once, this CLI profile will work too —");
            println!("  cookies are stored securely in the OS keyring.");
            println!();
            (String::new(), String::new(), false, true)
        }
        _ => unreachable!(),
    };

    // Build profile — preserve existing aliases if updating
    let existing_aliases = config::load_config()
        .ok()
        .and_then(|(cfg, _)| cfg.connections.get(&name).map(|p| p.aliases.clone()))
        .unwrap_or_default();

    let profile = ConnectionProfile {
        base_url: url.clone(),
        client: client.clone(),
        language: language.clone(),
        username: username.clone(),
        password: None, // goes to keyring
        sso,
        browser_sso,
        insecure_tls: false,
        sso_delegate: false,
        aliases: existing_aliases,
    };

    // Store password in keyring if Basic auth
    if auth_idx == 0 {
        match config::set_password_in_keyring(&name, &username, &password) {
            Ok(()) => println!("  Password stored in OS keyring."),
            Err(e) => {
                println!("  Warning: could not store password in OS keyring: {e}");
                println!("  (Re-run the wizard or set SAP_PASSWORD env var to use this profile.)");
            }
        }
    }

    // Save config
    let (mut cfg, config_dir) = config::load_config().context("failed to load config")?;

    // If the connection fingerprint changed (url/client/language), discard any
    // persisted Browser SSO session so we don't replay cookies to a different target.
    let old_profile = cfg.connections.get(&name).cloned();
    match config::clear_session_if_connection_changed(
        &name,
        old_profile.as_ref(),
        &url,
        &client,
        &language,
    ) {
        Ok(true) => println!("  Connection changed — cleared stale Browser SSO session."),
        Ok(false) => {}
        Err(e) => println!(
            "  Warning: connection changed but could not clear stale session ({e}). \
             Re-sign-in may be required; consider 'sap-odata signout {name}'."
        ),
    }

    cfg.connections.insert(name.clone(), profile);
    let save_path = if config_dir.path.as_os_str().is_empty() {
        let dir = config::get_or_create_config_dir()?;
        config::save_config(&cfg, &dir.path)?
    } else {
        config::save_config(&cfg, &config_dir.path)?
    };

    println!();
    println!("  Profile '{}' saved to {}", name, save_path.display());

    // Offer to test the connection (except for Browser SSO which needs a webview)
    if !browser_sso {
        let do_test = Confirm::with_theme(&theme)
            .with_prompt("Test connection now?")
            .default(true)
            .interact()?;
        if do_test {
            println!("  Testing connection...");
            let conn = SapConnection {
                base_url: url,
                client,
                language,
                auth: if sso {
                    AuthConfig::Sso
                } else {
                    AuthConfig::Basic { username, password }
                },
                insecure_tls: false,
                sso_delegate: false,
            };
            match SapClient::new(conn) {
                Ok(client) => match client
                    .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
                    .await
                {
                    Ok(()) => println!("  Connection OK."),
                    Err(e) => println!("  Connection test failed: {e}"),
                },
                Err(e) => println!("  Could not create client: {e}"),
            }
        }
    } else {
        println!();
        println!("  Next step: open the desktop app, select this profile, and click Sign In.");
        println!("  After signing in there, CLI commands like");
        println!("    sap-odata -p {name} services");
        println!("  will work from this terminal too.");
    }

    println!();
    Ok(())
}

// ══════════════════════════════════════════════════════════════
// ── SIGNOUT ──
// ══════════════════════════════════════════════════════════════

fn cmd_signout(name: &str) -> Result<()> {
    sap_odata_core::session::clear(name)
        .with_context(|| format!("failed to clear session for '{name}'"))?;
    println!("  Cleared persisted Browser SSO session for profile '{name}'.");
    Ok(())
}

async fn handle_profile_command(action: &ProfileAction) -> Result<()> {
    match action {
        ProfileAction::List => cmd_profile_list(),
        ProfileAction::Add {
            name,
            url,
            client,
            language,
            user,
            password,
            sso,
            sso_delegate,
            plaintext,
            portable,
        } => cmd_profile_add(
            name,
            url,
            client,
            language,
            user,
            password,
            *sso,
            *sso_delegate,
            *plaintext,
            *portable,
        ),
        ProfileAction::Remove { name } => cmd_profile_remove(name),
        ProfileAction::Test { name } => cmd_profile_test(name).await,
        ProfileAction::Where => cmd_profile_where(),
    }
}

fn cmd_profile_list() -> Result<()> {
    let (cfg, config_dir) = config::load_config().context("failed to load config")?;

    if cfg.connections.is_empty() {
        println!("No profiles saved yet. Use 'sap-odata profile add' to create one.");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("Profile").fg(Color::DarkCyan),
        Cell::new("URL").fg(Color::DarkCyan),
        Cell::new("Client").fg(Color::DarkCyan),
        Cell::new("User").fg(Color::DarkCyan),
        Cell::new("Password").fg(Color::DarkCyan),
    ]);

    for (name, profile) in &cfg.connections {
        let auth_info = if profile.sso {
            "SSO (Windows)".to_string()
        } else if profile.password.is_some() {
            "config (plaintext)".to_string()
        } else {
            match config::try_get_password_from_keyring(name, &profile.username) {
                Ok(Some(_)) => "OS keyring".to_string(),
                Ok(None) => "NOT SET".to_string(),
                Err(config::KeyringReadError::Locked(_)) => "keyring locked".to_string(),
                Err(config::KeyringReadError::Corrupt(_)) => "keyring corrupt".to_string(),
                Err(config::KeyringReadError::Backend(_)) => "keyring error".to_string(),
            }
        };

        table.add_row(vec![
            Cell::new(name),
            Cell::new(&profile.base_url),
            Cell::new(&profile.client),
            Cell::new(if profile.sso {
                "(SSO)"
            } else {
                &profile.username
            }),
            Cell::new(auth_info),
        ]);
    }

    println!("{table}");
    let mode = if config_dir.is_portable {
        "portable"
    } else {
        "user"
    };
    println!("\n  Config: {} ({})\n", config_dir.path.display(), mode);

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn cmd_profile_add(
    name: &str,
    url: &str,
    client: &str,
    language: &str,
    user: &str,
    password: &str,
    sso: bool,
    sso_delegate: bool,
    plaintext: bool,
    portable: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("failed to load config")?;

    let existing_aliases = cfg
        .connections
        .get(name)
        .map(|p| p.aliases.clone())
        .unwrap_or_default();

    // If the connection fingerprint changed (url/client/language), discard any
    // persisted Browser SSO session so we don't replay cookies to a different target.
    let old_profile = cfg.connections.get(name).cloned();
    match config::clear_session_if_connection_changed(
        name,
        old_profile.as_ref(),
        url,
        client,
        language,
    ) {
        Ok(true) => println!("  Connection changed — cleared stale Browser SSO session."),
        Ok(false) => {}
        Err(e) => println!(
            "  Warning: connection changed but could not clear stale session ({e}). \
             Re-sign-in may be required; consider 'sap-odata signout {name}'."
        ),
    }

    let profile = ConnectionProfile {
        base_url: url.to_string(),
        client: client.to_string(),
        language: language.to_string(),
        username: user.to_string(),
        password: if plaintext && !sso {
            Some(password.to_string())
        } else {
            None
        },
        sso,
        browser_sso: false,
        insecure_tls: false,
        sso_delegate: sso && sso_delegate,
        aliases: existing_aliases,
    };

    // SSO profiles don't need password storage
    if sso {
        cfg.connections.insert(name.to_string(), profile);
        let save_path = if portable {
            config::init_portable_config(&cfg)?
        } else if config_dir.path.as_os_str().is_empty() {
            let dir = config::get_or_create_config_dir()?;
            config::save_config(&cfg, &dir.path)?
        } else {
            config::save_config(&cfg, &config_dir.path)?
        };
        println!(
            "  Profile '{}' saved with SSO to {}",
            name,
            save_path.display()
        );
        return Ok(());
    }

    // Store password in keyring unless plaintext requested.
    // Fail closed: do NOT silently fall back to plaintext on keyring failure —
    // that's exactly the kind of surprise an enterprise security review rejects.
    // The user has to explicitly opt in with --plaintext.
    if !plaintext {
        if let Err(e) = config::set_password_in_keyring(name, user, password) {
            anyhow::bail!(
                "could not store password in OS keyring: {e}\n\
                 Re-run with --plaintext to store the password in the config file instead \
                 (not recommended), or fix the keyring backend and retry."
            );
        }
        println!("  Password stored in OS keyring.");
    }

    cfg.connections.insert(name.to_string(), profile);

    let save_path = if portable {
        config::init_portable_config(&cfg)?
    } else if config_dir.path.as_os_str().is_empty() {
        let dir = config::get_or_create_config_dir()?;
        config::save_config(&cfg, &dir.path)?
    } else {
        config::save_config(&cfg, &config_dir.path)?
    };

    println!("  Profile '{}' saved to {}", name, save_path.display());
    Ok(())
}

fn cmd_profile_remove(name: &str) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("failed to load config")?;

    let profile = cfg.connections.remove(name);
    if profile.is_none() {
        anyhow::bail!("profile '{}' not found", name);
    }

    let profile = profile.unwrap();

    // Remove password from keyring (basic auth). If the keyring is unavailable
    // the credential stays stored — warn the user so they can clean it up.
    let mut warnings: Vec<String> = Vec::new();
    if !profile.username.is_empty()
        && let Err(e) = config::delete_password_from_keyring(name, &profile.username)
    {
        warnings.push(format!("password (user: {}) — {e}", profile.username));
    }
    // Remove any persisted Browser SSO session — critical so that recreating
    // a profile with the same name doesn't resurrect the old session.
    if let Err(e) = sap_odata_core::session::clear(name) {
        warnings.push(format!("Browser SSO session — {e}"));
    }

    config::save_config(&cfg, &config_dir.path)?;

    if warnings.is_empty() {
        println!(
            "  Profile '{}' removed (credentials and session cleared).",
            name
        );
    } else {
        println!(
            "  Profile '{}' removed from config, but these items could NOT be cleared:",
            name
        );
        for w in &warnings {
            println!("    - {w}");
        }
        println!("  You may need to remove them manually from the OS credential store.");
    }
    Ok(())
}

async fn cmd_profile_test(name: &str) -> Result<()> {
    let (cfg, _) = config::load_config().context("failed to load config")?;
    let profile = cfg
        .connections
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", name))?;

    let connection = config::resolve_connection(name, profile)
        .with_context(|| format!("failed to resolve profile '{name}'"))?;

    println!(
        "  Testing connection to {} (client {})...",
        connection.base_url, connection.client
    );

    let client = SapClient::new(connection).context("failed to create SAP client")?;

    // Try to establish a session
    client
        .ensure_session("/sap/opu/odata/IWFND/CATALOGSERVICE;v=2")
        .await
        .context("connection test failed")?;

    println!("  Connection successful!");
    Ok(())
}

fn cmd_profile_where() -> Result<()> {
    match config::find_config_dir() {
        Some(dir) => {
            let mode = if dir.is_portable { "portable" } else { "user" };
            println!("  Config directory: {} ({})", dir.path.display(), mode);
        }
        None => {
            // Show where it would be created
            let dir = config::get_or_create_config_dir()?;
            let mode = if dir.is_portable { "portable" } else { "user" };
            println!(
                "  Config directory: {} ({}, will be created on first use)",
                dir.path.display(),
                mode
            );
        }
    }
    Ok(())
}

// ── Query builder ──

#[allow(clippy::too_many_arguments)]
fn build_query(
    entity_set: &str,
    select: Option<String>,
    filter: Option<String>,
    expand: Option<String>,
    orderby: Option<String>,
    top: Option<u32>,
    skip: Option<u32>,
    key: Option<String>,
    count: bool,
    version: Option<ODataVersion>,
) -> ODataQuery {
    let mut q = ODataQuery::new(entity_set).format("json");
    if let Some(v) = version {
        q = q.version(v);
    }

    if let Some(s) = select {
        let fields: Vec<&str> = s.split(',').map(str::trim).collect();
        q = q.select(&fields);
    }
    if let Some(f) = filter {
        q = q.filter(&f);
    }
    if let Some(e) = expand {
        let navs: Vec<&str> = e.split(',').map(str::trim).collect();
        q = q.expand(&navs);
    }
    if let Some(o) = orderby {
        let clauses: Vec<&str> = o.split(',').map(str::trim).collect();
        q = q.orderby(&clauses);
    }
    if let Some(t) = top {
        q = q.top(t);
    }
    if let Some(s) = skip {
        q = q.skip(s);
    }
    if let Some(k) = key {
        q = q.key(&k);
    }
    if count {
        q = q.count();
    }
    q
}

// ── Service commands ──

async fn cmd_services(
    client: &SapClient,
    search: Option<&str>,
    v2_only: bool,
    v4_only: bool,
    top: Option<usize>,
    json: bool,
) -> Result<()> {
    let result = sap_odata_core::catalog::fetch_service_catalog(client)
        .await
        .context("failed to fetch service catalog")?;

    for w in &result.warnings {
        eprintln!("  Warning: {w}");
    }

    let mut filtered: Vec<_> = result
        .entries
        .iter()
        .filter(|e| {
            if v2_only && e.is_v4 {
                return false;
            }
            if v4_only && !e.is_v4 {
                return false;
            }
            if let Some(s) = search {
                return e.matches(s);
            }
            true
        })
        .collect();

    let total = filtered.len();
    if let Some(n) = top {
        filtered.truncate(n);
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("No services found.");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(vec![
        Cell::new("#").fg(Color::DarkCyan),
        Cell::new("Ver").fg(Color::DarkCyan),
        Cell::new("Technical Name").fg(Color::DarkCyan),
        Cell::new("Title").fg(Color::DarkCyan),
        Cell::new("Service URL").fg(Color::DarkCyan),
    ]);

    for (i, entry) in filtered.iter().enumerate() {
        table.add_row(vec![
            Cell::new(i + 1),
            Cell::new(entry.version_label()),
            Cell::new(&entry.technical_name),
            Cell::new(&entry.title),
            Cell::new(&entry.service_url),
        ]);
    }

    println!("{table}");
    if filtered.len() < total {
        println!("\n  Showing {} of {} service(s)\n", filtered.len(), total);
    } else {
        println!("\n  {} service(s) found\n", total);
    }

    Ok(())
}

async fn cmd_entities(client: &SapClient, service: &str, json: bool) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;

    if json {
        let sets: Vec<_> = meta
            .entity_sets
            .iter()
            .map(|es| {
                serde_json::json!({
                    "name": es.name,
                    "entity_type": es.entity_type,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&sets)?);
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.set_header(vec![
            Cell::new("#").fg(Color::DarkCyan),
            Cell::new("Entity Set").fg(Color::DarkCyan),
            Cell::new("Entity Type").fg(Color::DarkCyan),
            Cell::new("Keys").fg(Color::DarkCyan),
        ]);

        for (i, es) in meta.entity_sets.iter().enumerate() {
            let et = meta.entity_type_for_set(&es.name);
            let keys = et.map(|t| t.keys.join(", ")).unwrap_or_default();
            table.add_row(vec![
                Cell::new(i + 1),
                Cell::new(&es.name),
                Cell::new(&es.entity_type),
                Cell::new(keys),
            ]);
        }

        println!("{table}");
    }
    Ok(())
}

async fn cmd_describe(
    client: &SapClient,
    service: &str,
    entity_set: &str,
    json: bool,
) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;

    let et = meta
        .entity_type_for_set(entity_set)
        .ok_or_else(|| anyhow::anyhow!("entity set '{entity_set}' not found"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&et)?);
        return Ok(());
    }

    println!("\n  Entity Type: {}", et.name);
    println!("  Keys: {}\n", et.keys.join(", "));

    let mut prop_table = Table::new();
    prop_table.load_preset(UTF8_FULL);
    prop_table.set_header(vec![
        Cell::new("Property").fg(Color::DarkCyan),
        Cell::new("Type").fg(Color::DarkCyan),
        Cell::new("Key").fg(Color::DarkCyan),
        Cell::new("Nullable").fg(Color::DarkCyan),
        Cell::new("MaxLen").fg(Color::DarkCyan),
        Cell::new("Label").fg(Color::DarkCyan),
    ]);

    for p in &et.properties {
        let is_key = if et.keys.contains(&p.name) { "*" } else { "" };
        prop_table.add_row(vec![
            Cell::new(&p.name),
            Cell::new(&p.edm_type),
            Cell::new(is_key),
            Cell::new(if p.nullable { "yes" } else { "no" }),
            Cell::new(p.max_length.map(|l| l.to_string()).unwrap_or_default()),
            Cell::new(p.label.as_deref().unwrap_or("")),
        ]);
    }
    println!("{prop_table}");

    let nav_targets = meta.nav_targets(et);
    if !nav_targets.is_empty() {
        println!("\n  Navigation Properties:\n");
        let mut nav_table = Table::new();
        nav_table.load_preset(UTF8_FULL);
        nav_table.set_header(vec![
            Cell::new("Nav Property").fg(Color::DarkCyan),
            Cell::new("Target Type").fg(Color::DarkCyan),
            Cell::new("Multiplicity").fg(Color::DarkCyan),
        ]);
        for (name, target, mult) in &nav_targets {
            nav_table.add_row(vec![Cell::new(name), Cell::new(target), Cell::new(mult)]);
        }
        println!("{nav_table}");
    }

    println!("\n  Example query:");
    let select_fields: Vec<&str> = et
        .properties
        .iter()
        .take(3)
        .map(|p| p.name.as_str())
        .collect();
    println!(
        "  sap-odata ... run {entity_set} --select {} --top 10\n",
        select_fields.join(",")
    );
    println!(
        "  URL: {}/{}\n",
        service,
        ODataQuery::new(entity_set)
            .select(&select_fields)
            .top(10)
            .format("json")
            .build()
    );

    Ok(())
}

async fn cmd_functions(client: &SapClient, service: &str, json: bool) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;

    if meta.function_imports.is_empty() {
        println!("No function imports in this service.");
        return Ok(());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&meta.function_imports)?);
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.set_header(vec![
            Cell::new("Function").fg(Color::DarkCyan),
            Cell::new("Method").fg(Color::DarkCyan),
            Cell::new("Return Type").fg(Color::DarkCyan),
            Cell::new("Parameters").fg(Color::DarkCyan),
        ]);

        for fi in &meta.function_imports {
            let params: Vec<String> = fi
                .parameters
                .iter()
                .map(|p| format!("{}: {}", p.name, p.edm_type))
                .collect();
            table.add_row(vec![
                Cell::new(&fi.name),
                Cell::new(&fi.http_method),
                Cell::new(fi.return_type.as_deref().unwrap_or("-")),
                Cell::new(params.join(", ")),
            ]);
        }
        println!("{table}");
    }
    Ok(())
}

async fn cmd_run(
    client: &SapClient,
    service: &str,
    query: &ODataQuery,
    json_output: bool,
) -> Result<()> {
    let data = client
        .query_json(service, query)
        .await
        .context("query execution failed")?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&data)?);
        return Ok(());
    }

    let results = extract_results(&data);

    match results {
        Some(serde_json::Value::Array(rows)) if !rows.is_empty() => {
            print_results_table(&rows);
        }
        Some(obj @ serde_json::Value::Object(_)) => {
            print_results_table(&[obj]);
        }
        _ => {
            println!("{}", serde_json::to_string_pretty(&data)?);
        }
    }

    Ok(())
}

async fn cmd_metadata(client: &SapClient, service: &str) -> Result<()> {
    let xml = client
        .fetch_metadata_xml(service)
        .await
        .context("failed to fetch metadata")?;
    println!("{xml}");
    Ok(())
}

async fn cmd_annotations(
    client: &SapClient,
    service: &str,
    as_json: bool,
    namespace: Option<String>,
    filter: Option<String>,
) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;

    // Optional filters applied in order: namespace (exact-ish match) +
    // free-text substring across term/target/value/qualifier.
    let ns_needle = namespace.as_deref().map(|s| s.to_ascii_lowercase());
    let text_needle = filter.as_deref().map(|s| s.to_ascii_lowercase());
    let filtered: Vec<_> = meta
        .annotations
        .into_iter()
        .filter(|a| {
            if let Some(ns) = &ns_needle
                && a.namespace.to_ascii_lowercase() != *ns
            {
                return false;
            }
            if let Some(n) = &text_needle {
                let hay = format!(
                    "{} {} {} {}",
                    a.term,
                    a.target,
                    a.value.as_deref().unwrap_or(""),
                    a.qualifier.as_deref().unwrap_or(""),
                )
                .to_ascii_lowercase();
                if !hay.contains(n) {
                    return false;
                }
            }
            true
        })
        .collect();

    if as_json {
        println!("{}", serde_json::to_string_pretty(&filtered)?);
        return Ok(());
    }

    if filtered.is_empty() {
        println!("No annotations match the filters.");
        return Ok(());
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        Cell::new("Namespace").fg(Color::Cyan),
        Cell::new("Term").fg(Color::Cyan),
        Cell::new("Target").fg(Color::Cyan),
        Cell::new("Value").fg(Color::Cyan),
        Cell::new("Qualifier").fg(Color::Cyan),
    ]);
    for a in &filtered {
        table.add_row(vec![
            Cell::new(&a.namespace),
            Cell::new(&a.term),
            Cell::new(&a.target),
            Cell::new(a.value.as_deref().unwrap_or("—")),
            Cell::new(a.qualifier.as_deref().unwrap_or("")),
        ]);
    }
    println!("{table}");
    println!("\n{} annotation(s) shown.", filtered.len());
    Ok(())
}

async fn cmd_lint(
    client: &SapClient,
    service: &str,
    as_json: bool,
    entity: Option<String>,
    min_severity: Option<String>,
) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;

    let min_sev = match min_severity.as_deref() {
        Some("pass") | None => None,
        Some("warn") => Some(LintSeverity::Warn),
        Some("miss") => Some(LintSeverity::Miss),
        Some(other) => {
            anyhow::bail!("unknown severity '{other}' — expected pass, warn, or miss");
        }
    };

    // Resolve the scope: one entity (accept either type name or set
    // name), or every entity type in the schema.
    let targets: Vec<&str> = match entity.as_deref() {
        Some(name) => {
            if meta.find_entity_type(name).is_some() {
                vec![name]
            } else if let Some(et) = meta.entity_type_for_set(name) {
                vec![et.name.as_str()]
            } else {
                anyhow::bail!("entity type or set '{name}' not found");
            }
        }
        None => meta
            .entity_types
            .iter()
            .map(|et| et.name.as_str())
            .collect(),
    };

    #[derive(serde::Serialize)]
    struct Report<'a> {
        entity: &'a str,
        findings: Vec<lint::LintFinding>,
    }
    let mut reports: Vec<Report> = Vec::new();
    for name in &targets {
        let et = meta
            .find_entity_type(name)
            .ok_or_else(|| anyhow::anyhow!("entity '{name}' vanished"))?;
        let mut findings = lint::evaluate_entity_type(et);
        if let Some(s) = min_sev {
            // Keep the "profile" banner regardless of severity so the
            // user still sees which profile was assumed when they ask
            // for only warns/misses. Every other Pass is filtered out
            // as intended.
            findings.retain(|f| f.code == "profile" || severity_at_least(f.severity, s));
        }
        // A lone "profile" banner with nothing else at the requested
        // severity isn't informative — it just says "shape=X, no
        // warns/misses". Suppress to keep the output clean; users
        // see the banner when they run without --min-severity.
        let has_actionable = findings.iter().any(|f| f.code != "profile");
        if has_actionable {
            reports.push(Report {
                entity: name,
                findings,
            });
        }
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        return Ok(());
    }

    if reports.is_empty() {
        println!("No findings at or above the requested severity.");
        return Ok(());
    }

    for r in &reports {
        println!("\n── {} ──", r.entity);
        let mut table = Table::new();
        table.load_preset(UTF8_FULL).set_header(vec![
            Cell::new("").fg(Color::Cyan),
            Cell::new("Category").fg(Color::Cyan),
            Cell::new("Check").fg(Color::Cyan),
            Cell::new("Message").fg(Color::Cyan),
        ]);
        for f in &r.findings {
            let (symbol, color) = match f.severity {
                LintSeverity::Pass => ("✓", Color::Green),
                LintSeverity::Warn => ("!", Color::Yellow),
                LintSeverity::Miss => ("✗", Color::Red),
            };
            // Inline the ABAP CDS hint under the message so the user
            // sees both without a separate column. Keeps the table
            // from blowing out horizontally on actionable findings.
            let mut msg = f.message.clone();
            if let Some(cds) = f.suggested_cds {
                msg.push_str("\nABAP CDS: ");
                msg.push_str(cds);
            }
            if let Some(why) = f.why_in_fiori {
                msg.push_str("\n→ ");
                msg.push_str(why);
            }
            table.add_row(vec![
                Cell::new(symbol).fg(color),
                Cell::new(format!("{:?}", f.category).to_lowercase()),
                Cell::new(f.code),
                Cell::new(msg),
            ]);
        }
        println!("{table}");
    }
    Ok(())
}

/// Severity ranking helper: Pass < Warn < Miss. Returns true if
/// `actual` is at or above `threshold`.
fn severity_at_least(actual: LintSeverity, threshold: LintSeverity) -> bool {
    fn rank(s: LintSeverity) -> u8 {
        match s {
            LintSeverity::Pass => 0,
            LintSeverity::Warn => 1,
            LintSeverity::Miss => 2,
        }
    }
    rank(actual) >= rank(threshold)
}

// ── Helpers ──

fn extract_results(data: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(d) = data.get("d") {
        if let Some(results) = d.get("results") {
            return Some(results.clone());
        }
        return Some(d.clone());
    }
    if let Some(value) = data.get("value") {
        return Some(value.clone());
    }
    None
}

fn print_results_table(rows: &[serde_json::Value]) {
    let Some(first) = rows.first() else { return };
    let Some(obj) = first.as_object() else {
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_default()
        );
        return;
    };

    let columns: Vec<&String> = obj
        .keys()
        .filter(|k| {
            !k.starts_with('@')
                && *k != "__metadata"
                && !matches!(
                    obj.get(*k),
                    Some(serde_json::Value::Object(_) | serde_json::Value::Array(_))
                )
        })
        .collect();

    let mut table = Table::new();
    table.load_preset(UTF8_FULL);
    table.set_header(
        columns
            .iter()
            .map(|c| Cell::new(c.as_str()).fg(Color::DarkCyan)),
    );

    for row in rows {
        let cells: Vec<Cell> = columns
            .iter()
            .map(|col| {
                let val = row.get(*col);
                let text = match val {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(serde_json::Value::Null) => String::new(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                Cell::new(text)
            })
            .collect();
        table.add_row(cells);
    }

    println!("{table}");
    println!("\n  {} row(s) returned\n", rows.len());
}
