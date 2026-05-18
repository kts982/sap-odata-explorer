use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sap_odata_core::{
    auth::{AuthConfig, SapConnection},
    client::SapClient,
    config::{self, ConnectionProfile},
    diagnostics::HttpTraceEntry,
    lint::{self, LintSeverity},
    metadata::ODataVersion,
    offline::{
        self, ImportOptions, SaveOptions, SaveOutcome, auto_offline_profile_name, current_iso8601,
    },
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
    #[command(
        long_about = "Build and display the OData query URL without sending it.\n\
        \n\
        Common flags (also accepted by `run`):\n  \
            --select FIELDS        comma-separated $select list\n  \
            --filter EXPR          raw OData $filter expression\n  \
            --in PROP V1 V2 ...    OR'd equality shortcut: builds (PROP eq 'V1' or PROP eq 'V2' ...)\n  \
            --expand NAVS          comma-separated $expand list\n  \
            --orderby CLAUSES      comma-separated $orderby clauses (e.g. \"Name asc,Date desc\")\n  \
            --top N                $top\n  \
            --skip N               $skip\n  \
            --key K                single-entity read: CustomerSet(K)\n  \
            --count                request inline count\n  \
            --json                 emit URL as a JSON string instead of plain text"
    )]
    Build {
        /// Entity set name
        entity_set: String,

        /// Fields to select (comma-separated)
        #[arg(long)]
        select: Option<String>,

        /// Filter expression
        #[arg(long)]
        filter: Option<String>,

        /// OR'd equality shortcut. Usage: `--in PROP V1 V2 V3` builds
        /// `(PROP eq 'V1' or PROP eq 'V2' or PROP eq 'V3')`. Combines with
        /// `--filter` via AND. Saves PowerShell-escaping pain when filtering
        /// against a small set of values. String values only; quoting is
        /// applied automatically.
        #[arg(long = "in", num_args = 2.., value_name = "PROP VALUES...")]
        in_values: Vec<String>,

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
    #[command(long_about = "Execute the OData query and render the results.\n\
        \n\
        Common flags:\n  \
            --select FIELDS        comma-separated $select list\n  \
            --filter EXPR          raw OData $filter expression\n  \
            --in PROP V1 V2 ...    OR'd equality shortcut: builds (PROP eq 'V1' or PROP eq 'V2' ...)\n  \
            --expand NAVS          comma-separated $expand list\n  \
            --orderby CLAUSES      comma-separated $orderby clauses\n  \
            --top N                $top (default: server-side limit)\n  \
            --skip N               $skip (pagination)\n  \
            --key K                single-entity read\n  \
            --count                request inline count\n  \
            --json                 emit the raw response JSON instead of the table\n\
        \n\
        Use `build` instead to print the URL without sending the request.")]
    Run {
        /// Entity set name
        entity_set: String,

        /// Fields to select (comma-separated)
        #[arg(long)]
        select: Option<String>,

        /// Filter expression
        #[arg(long)]
        filter: Option<String>,

        /// OR'd equality shortcut. Usage: `--in PROP V1 V2 V3` builds
        /// `(PROP eq 'V1' or PROP eq 'V2' or PROP eq 'V3')`. Combines with
        /// `--filter` via AND.
        #[arg(long = "in", num_args = 2.., value_name = "PROP VALUES...")]
        in_values: Vec<String>,

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

    /// Smoke-test every entity set in a service. Issues a small
    /// `$top` GET against each set and reports OK / FAIL with row
    /// counts. Exit code is 0 when every probe succeeded, 1 otherwise
    /// — intended as a CI- and agent-friendly health check.
    #[command(long_about = "Smoke-test every entity set in a service.\n\
        \n\
        Issues a small `$top` GET against each set and reports OK / FAIL\n\
        with row counts. SAP V4 framework sets (SAP__/__ prefixed) are\n\
        skipped because they're runtime plumbing, not user data.\n\
        \n\
        Flags:\n  \
            --quick                only list entity sets, no per-set GET\n  \
            --top N                $top per probe (default 1)\n  \
            --json                 emit results as a JSON array\n\
        \n\
        Exit code: 0 when every probe succeeded, 1 if any probe failed.")]
    Verify {
        /// Just list entity sets, skip the per-set probe.
        #[arg(long)]
        quick: bool,

        /// $top to use on each probe. Smaller is faster; the default
        /// of 1 is enough to confirm the set is reachable.
        #[arg(long, default_value_t = 1)]
        top: u32,
    },

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

        /// Override the auto-detected lint profile. Useful when the
        /// heuristics misread a service (e.g. a list-report-shaped
        /// entity that lacks the naming convention) or to ask "how
        /// list-report-ready is this value-help service?". Named
        /// `--lint-profile` to avoid clashing with the global
        /// connection-profile flag (`-p, --profile`).
        #[arg(long = "lint-profile", value_enum)]
        lint_profile: Option<LintProfileArg>,

        /// Exit non-zero if any finding at or above this severity is
        /// present. Intended for CI use. Independent of `--min-severity`,
        /// which only filters what's displayed.
        #[arg(long, value_enum)]
        fail_on: Option<FailOnArg>,
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

    /// Manage the offline EDMX library (save / list / import services
    /// for offline browsing).
    Offline {
        #[command(subcommand)]
        action: OfflineAction,
    },
}

#[derive(Subcommand)]
enum OfflineAction {
    /// Save the current connection's `$metadata` for a service into the
    /// offline library. Requires `-p PROFILE -s SERVICE`. The bytes are
    /// written under `{config}/offline/`, attributed to the connected
    /// profile; subsequent re-saves overwrite the file in place under
    /// the same stable `service_id`.
    Save {
        /// Override the offline-profile bucket name. Defaults to
        /// `<connected> (offline)`. If the chosen name doesn't yet
        /// exist as an offline profile, it's created; the name must
        /// not collide with an existing connected profile.
        #[arg(long)]
        offline_profile: Option<String>,

        /// Override the auto-derived display label. The label-derivation
        /// rule for V4 SAP services produces e.g. `UI_PHYSSTOCKPROD_1`
        /// from the schema namespace; pass `--label` to override.
        #[arg(long)]
        label: Option<String>,

        /// Optional free-form note to attach to the saved entry.
        #[arg(long)]
        note: Option<String>,
    },

    /// Import an EDMX file from disk into the offline library. No
    /// connected profile needed — the file is read, validated, and
    /// indexed. Suitable for files pulled from API Hub, exported via
    /// `/IWFND/GW_CLIENT` "Save Response", or downloaded with
    /// `curl <base>/$metadata > out.xml`.
    Import {
        /// Path to the EDMX file on disk. Must be a regular file,
        /// UTF-8 encoded, ≤ 10 MB.
        file: std::path::PathBuf,

        /// Target offline-profile bucket. Defaults to `Imported` —
        /// the catch-all where mixed-source captures coexist.
        #[arg(long)]
        offline_profile: Option<String>,

        /// Override the auto-derived display label. Auto-derivation
        /// reads the file's `Schema Namespace`; for unknown shapes
        /// the filename stem is used as a fallback.
        #[arg(long)]
        label: Option<String>,

        /// Optional free-form note to attach to the imported entry.
        #[arg(long)]
        note: Option<String>,
    },

    /// List all services in an offline-profile bucket. With no
    /// `--profile` argument, lists all offline profiles instead.
    List {
        /// Offline-profile bucket name to list services from. Omit
        /// to list the buckets themselves.
        #[arg(long)]
        profile: Option<String>,
    },
}

/// Lint profile override flag values. Mirrors `lint::LintProfile`
/// but is decoupled so clap's value-enum derive owns the kebab-case
/// CLI surface (`list-report`, `object-page`, …) without leaking
/// clap into the core crate.
#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum LintProfileArg {
    ListReport,
    ObjectPage,
    ValueHelp,
    Analytical,
    Transactional,
}

impl From<LintProfileArg> for lint::LintProfile {
    fn from(value: LintProfileArg) -> Self {
        match value {
            LintProfileArg::ListReport => lint::LintProfile::ListReport,
            LintProfileArg::ObjectPage => lint::LintProfile::ObjectPage,
            LintProfileArg::ValueHelp => lint::LintProfile::ValueHelp,
            LintProfileArg::Analytical => lint::LintProfile::Analytical,
            LintProfileArg::Transactional => lint::LintProfile::Transactional,
        }
    }
}

/// `--fail-on` accepts the actionable severities only — `pass` would
/// fire on every entity and isn't a useful CI gate.
#[derive(Clone, Copy, Debug, ValueEnum)]
#[value(rename_all = "lowercase")]
enum FailOnArg {
    Warn,
    Miss,
}

impl From<FailOnArg> for LintSeverity {
    fn from(value: FailOnArg) -> Self {
        match value {
            FailOnArg::Warn => LintSeverity::Warn,
            FailOnArg::Miss => LintSeverity::Miss,
        }
    }
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
    // `offline import` reads a local file and doesn't talk to SAP —
    // skip the connection-resolution step that follows so the user
    // can import without any profile / URL / credentials configured.
    if let Commands::Offline {
        action:
            OfflineAction::Import {
                file,
                offline_profile,
                label,
                note,
            },
    } = &cli.command
    {
        return cmd_offline_import(
            file.clone(),
            offline_profile.clone(),
            label.clone(),
            note.clone(),
            cli.json,
        );
    }
    // `offline list` reads only the local TOML index — no network.
    if let Commands::Offline {
        action: OfflineAction::List { profile },
    } = &cli.command
    {
        return cmd_offline_list(profile.clone(), cli.json);
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
                in_values,
                expand,
                orderby,
                top,
                skip,
                key,
                count,
            } => {
                let svc = require_service()?;
                let combined = combine_filter_with_in(filter.clone(), in_values)?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    combined,
                    expand.clone(),
                    orderby.clone(),
                    *top,
                    *skip,
                    key.clone(),
                    *count,
                    None,
                );
                let full_url = format!("{}/{}", svc.trim_end_matches('/'), query.build());
                if cli.json {
                    println!("{}", serde_json::to_string(&full_url)?);
                } else {
                    println!("{full_url}");
                }
                Ok(())
            }
            Commands::Run {
                entity_set,
                select,
                filter,
                in_values,
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
                let combined = combine_filter_with_in(filter.clone(), in_values)?;
                let query = build_query(
                    entity_set,
                    select.clone(),
                    combined,
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
            Commands::Verify { quick, top } => {
                let svc = require_service()?;
                cmd_verify(&sap_client, &svc, *quick, *top, cli.json).await
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
                lint_profile,
                fail_on,
            } => {
                let svc = require_service()?;
                cmd_lint(
                    &sap_client,
                    &svc,
                    cli.json,
                    entity.clone(),
                    min_severity.clone(),
                    *lint_profile,
                    *fail_on,
                )
                .await
            }
            Commands::Offline { action } => match action {
                OfflineAction::Save {
                    offline_profile,
                    label,
                    note,
                } => {
                    let svc = require_service()?;
                    let connected_profile = cli.profile.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "`offline save` requires `-p PROFILE` so the captured bytes can be attributed to a connected system"
                        )
                    })?;
                    cmd_offline_save(
                        &sap_client,
                        &connected_profile,
                        &svc,
                        offline_profile.clone(),
                        label.clone(),
                        note.clone(),
                        cli.json,
                    )
                    .await
                }
                OfflineAction::Import { .. } | OfflineAction::List { .. } => {
                    // Both handled in the early-exit block above. If
                    // we reach here, the early-exit was skipped —
                    // that's a logic bug, not user error.
                    unreachable!(
                        "offline import/list should have early-exited before connection resolution"
                    );
                }
            },
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
                    eprintln!("  Resolved alias '{}' -> {}", raw, alias_path);
                    return Ok(Some(alias_path.clone()));
                }
            }
        }
    }

    // Try catalog resolution (searches V2 + V4 by technical name).
    // Banner goes to stderr so `metadata > out.xml` and similar
    // redirects capture only the data on stdout.
    eprintln!("  Resolving service '{}'...", raw);
    match sap_odata_core::catalog::resolve_service_by_name(sap_client, &raw).await {
        Ok(path) => {
            eprintln!("  Resolved '{}' -> {}", raw, path);
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

    // Global name-uniqueness check (step-7 hardening). Run BEFORE any
    // keyring writes / further prompts so we don't leave an orphan
    // credential entry under a name we're about to reject. Same shared
    // helper as `cmd_profile_add` and Tauri `add_profile`.
    {
        let (cfg_for_check, _) =
            config::load_config().context("failed to load config for name check")?;
        if let Err(msg) = offline::check_connected_profile_name_available(&name, &cfg_for_check) {
            anyhow::bail!("{msg}");
        }
    }

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
        // Order matches Tauri's get_profiles: browser_sso and sso both have
        // empty usernames and no Basic password, so they must short-circuit
        // before the keyring lookup — otherwise Entry::new(target, "") can
        // surface a misleading "keyring error" on a profile that legitimately
        // has no password at all.
        let auth_info = if profile.browser_sso {
            "Browser SSO".to_string()
        } else if profile.sso {
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
            Cell::new(if profile.browser_sso {
                "(Browser)"
            } else if profile.sso {
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

    // Global name-uniqueness check (step 7's connected-side enforcement).
    // Shared core helper so this path and `cmd_setup_wizard` and the
    // Tauri `add_profile` all enforce identical semantics.
    if let Err(msg) = offline::check_connected_profile_name_available(name, &cfg) {
        anyhow::bail!("{msg}");
    }

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

/// Combine the raw `--filter` expression with a `--in PROP V1 V2 ...`
/// shortcut. Returns the merged $filter expression, or the original
/// filter / None when `--in` is empty.
///
/// `--in` builds `(PROP eq 'V1' or PROP eq 'V2' or ...)` with each value
/// quoted as an OData string literal (apostrophes doubled per spec —
/// `O'Brien` → `'O''Brien'`). Combined with an explicit `--filter` by
/// wrapping both in parens and joining with `and`, so operator
/// precedence stays intact. String literals only — numeric/date typing
/// would need property metadata.
fn combine_filter_with_in(filter: Option<String>, in_values: &[String]) -> Result<Option<String>> {
    if in_values.is_empty() {
        return Ok(filter);
    }
    // clap's `num_args = 2..` already enforces this, but guard for
    // defensive callers (and to keep the error specific).
    if in_values.len() < 2 {
        anyhow::bail!(
            "--in needs a property name followed by one or more values (got {} arg(s))",
            in_values.len()
        );
    }
    let prop = &in_values[0];
    let in_expr = in_values[1..]
        .iter()
        .map(|v| format!("{prop} eq '{}'", v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(" or ");
    let in_expr = format!("({in_expr})");

    Ok(Some(match filter {
        Some(f) => format!("({f}) and {in_expr}"),
        None => in_expr,
    }))
}

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

        // Hide SAP V4 system entity sets from the table; --json (handled
        // above) still prints them. Re-number with `enumerate` after the
        // filter so the displayed index is 1..N over what's shown.
        for (i, es) in meta
            .entity_sets
            .iter()
            .filter(|es| !is_sap_system_field(&es.name))
            .enumerate()
        {
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

    // Hide SAP V4 system fields (`SAP__Messages`, `__OperationControl`,
    // ...) from the property table. --json output above keeps the full
    // entity type intact so scripted consumers can still see them.
    for p in et
        .properties
        .iter()
        .filter(|p| !is_sap_system_field(&p.name))
    {
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

    let nav_targets: Vec<_> = meta
        .nav_targets(et)
        .into_iter()
        .filter(|(name, _, _)| !is_sap_system_field(name))
        .collect();
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
    // Skip SAP V4 system fields when picking example $select columns —
    // showing `SAP__Messages` in the example query would mislead the user.
    let select_fields: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| !is_sap_system_field(&p.name))
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

/// `verify` — issue a small `$top` GET against every entity set in
/// the service and report OK / FAIL per set. Returns Err only on
/// fatal errors (metadata fetch failed); per-set failures are
/// reported in the table and surface via the exit code.
async fn cmd_verify(
    client: &SapClient,
    service: &str,
    quick: bool,
    top: u32,
    json: bool,
) -> Result<()> {
    let meta = client
        .fetch_metadata(service)
        .await
        .context("failed to fetch metadata")?;
    let version = meta.version;

    // Skip SAP V4 framework sets (SAP__Messages, __OperationControl,
    // ...). Same rule cmd_entities uses for its table view — they're
    // runtime plumbing, not part of the data model under test.
    let sets: Vec<&str> = meta
        .entity_sets
        .iter()
        .map(|es| es.name.as_str())
        .filter(|name| !is_sap_system_field(name))
        .collect();

    if sets.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("No entity sets to verify in this service.");
        }
        return Ok(());
    }

    let mut results: Vec<VerifyResult> = Vec::with_capacity(sets.len());
    for set in &sets {
        if quick {
            results.push(VerifyResult {
                set: (*set).to_string(),
                status: VerifyStatus::Ok,
                rows: None,
                note: Some("not probed (--quick)".to_string()),
            });
            continue;
        }
        let query = ODataQuery::new(set)
            .top(top)
            .format("json")
            .version(version);
        match client.query_json(service, &query).await {
            Ok(data) => {
                let rows = match extract_results(&data) {
                    Some(serde_json::Value::Array(arr)) => Some(arr.len() as u32),
                    Some(serde_json::Value::Object(_)) => Some(1),
                    _ => None,
                };
                results.push(VerifyResult {
                    set: (*set).to_string(),
                    status: VerifyStatus::Ok,
                    rows,
                    note: None,
                });
            }
            Err(e) => {
                let note = truncate_for_table(&e.to_string(), 120);
                results.push(VerifyResult {
                    set: (*set).to_string(),
                    status: VerifyStatus::Fail,
                    rows: None,
                    note: Some(note),
                });
            }
        }
    }

    let any_fail = results
        .iter()
        .any(|r| matches!(r.status, VerifyStatus::Fail));

    if json {
        let payload: Vec<_> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "set": r.set,
                    "status": match r.status {
                        VerifyStatus::Ok => "ok",
                        VerifyStatus::Fail => "fail",
                    },
                    "rows": r.rows,
                    "note": r.note,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL);
        table.set_header(vec![
            Cell::new("Entity Set").fg(Color::DarkCyan),
            Cell::new("Status").fg(Color::DarkCyan),
            Cell::new("Rows").fg(Color::DarkCyan),
            Cell::new("Note").fg(Color::DarkCyan),
        ]);
        for r in &results {
            let status_cell = match r.status {
                VerifyStatus::Ok => Cell::new("OK").fg(Color::Green),
                VerifyStatus::Fail => Cell::new("FAIL").fg(Color::Red),
            };
            let rows_cell = match r.rows {
                Some(n) => Cell::new(n),
                None => Cell::new("-"),
            };
            table.add_row(vec![
                Cell::new(&r.set),
                status_cell,
                rows_cell,
                Cell::new(r.note.as_deref().unwrap_or("")),
            ]);
        }
        println!("{table}");

        let ok = results
            .iter()
            .filter(|r| matches!(r.status, VerifyStatus::Ok))
            .count();
        let fail = results.len() - ok;
        eprintln!(
            "\n  {} OK · {} FAIL ({} set{} probed)",
            ok,
            fail,
            results.len(),
            if results.len() == 1 { "" } else { "s" }
        );
    }

    if any_fail {
        // Bail (rather than `process::exit`) so the caller's post-
        // command paths still run — most importantly `emit_http_trace`
        // under `--verbose`, which is exactly when failure diagnostics
        // are useful. The terse message is one line under the table
        // and doesn't compete with the per-row failure details.
        let fail = results
            .iter()
            .filter(|r| matches!(r.status, VerifyStatus::Fail))
            .count();
        anyhow::bail!("{fail} entity set(s) failed verification");
    }
    Ok(())
}

/// Per-set outcome captured during `cmd_verify`. Kept inside the
/// fn module so the JSON/table renderers can share it; not exported.
struct VerifyResult {
    set: String,
    status: VerifyStatus,
    rows: Option<u32>,
    note: Option<String>,
}

enum VerifyStatus {
    Ok,
    Fail,
}

/// Trim a multi-line error message down to the first line and cap
/// at `max_len` chars so the verify table stays scannable.
fn truncate_for_table(s: &str, max_len: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    if first_line.chars().count() <= max_len {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(max_len).collect();
        format!("{truncated}…")
    }
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
    profile_override: Option<LintProfileArg>,
    fail_on: Option<FailOnArg>,
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
    let fail_on_sev: Option<LintSeverity> = fail_on.map(Into::into);

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

    // Top-level profile fields are intentionally separate from the
    // per-finding banner: programmatic consumers (CI, agents) read the
    // structured fields, while the banner stays in `findings` so the
    // text-rendering path keeps its current shape.
    //
    // `detected_profile` is always the heuristic auto-detection — it
    // reflects what the linter would pick if no override were supplied.
    // `effective_profile` is what the rules actually ran against. Without
    // `--profile` they match. With `--profile`, callers can compare the
    // two to report "we auto-detected X but you forced Y" without losing
    // either signal.
    #[derive(serde::Serialize)]
    struct Report<'a> {
        entity: &'a str,
        detected_profile: lint::LintProfile,
        effective_profile: lint::LintProfile,
        findings: Vec<lint::LintFinding>,
    }
    let mut reports: Vec<Report> = Vec::new();
    let mut fail_on_tripped = false;
    for name in &targets {
        let et = meta
            .find_entity_type(name)
            .ok_or_else(|| anyhow::anyhow!("entity '{name}' vanished"))?;
        let detected_profile = lint::detect_profile(et);
        let effective_profile = profile_override.map(Into::into).unwrap_or(detected_profile);
        let mut findings = lint::evaluate_entity_type_with_profile(et, effective_profile);
        // Compute fail-on hit BEFORE the min-severity retain — fail-on
        // is an exit-code gate, not a display gate, and a user who runs
        // `--min-severity miss --fail-on warn` still wants warns to
        // count toward the exit code even if they're filtered out of
        // the table.
        if let Some(threshold) = fail_on_sev
            && findings
                .iter()
                .any(|f| f.code != "profile" && severity_at_least(f.severity, threshold))
        {
            fail_on_tripped = true;
        }
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
                detected_profile,
                effective_profile,
                findings,
            });
        }
    }

    if as_json {
        println!("{}", serde_json::to_string_pretty(&reports)?);
        if fail_on_tripped {
            // Even in JSON mode, --fail-on still drives the exit code.
            // We've already emitted the report so consumers can read
            // the findings; the bail just sets exit != 0.
            anyhow::bail!("lint findings at or above --fail-on threshold are present",);
        }
        return Ok(());
    }

    if reports.is_empty() {
        println!("No findings at or above the requested severity.");
        if fail_on_tripped {
            anyhow::bail!(
                "lint findings at or above --fail-on threshold are present (filtered out by --min-severity)",
            );
        }
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
    if fail_on_tripped {
        anyhow::bail!("lint findings at or above --fail-on threshold are present");
    }
    Ok(())
}

/// Capture the bytes of a connected service's `$metadata` into the
/// offline library. Mirrors the Tauri command but is a separate
/// implementation because the CLI flow operates without `AppState`:
/// load config from disk → call the core save function → write back.
/// The core function does the atomic EDMX + TOML write internally.
async fn cmd_offline_save(
    client: &SapClient,
    connected_profile_name: &str,
    service_path: &str,
    offline_profile_name: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
    json: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("loading config")?;
    let base_url = cfg
        .connections
        .get(connected_profile_name)
        .map(|p| p.base_url.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Profile '{}' not found in connections.toml",
                connected_profile_name
            )
        })?;
    let svc_trimmed = service_path.trim_start_matches('/').trim_end_matches('/');
    let source_url = format!(
        "{}/{}/$metadata",
        base_url.trim_end_matches('/'),
        svc_trimmed
    );

    let opts = SaveOptions {
        offline_profile_name: offline_profile_name
            .or_else(|| Some(auto_offline_profile_name(connected_profile_name))),
        source_profile_for_new_bucket: Some(connected_profile_name.to_string()),
        label_override,
        note,
        now_iso: current_iso8601(),
    };

    let outcome = offline::save_service_offline(
        client,
        service_path,
        source_url,
        &mut cfg,
        &config_dir.path,
        opts,
    )
    .await
    .context("save_service_offline")?;

    if json {
        let s = serde_json::to_string_pretty(&outcome)?;
        println!("{s}");
        return Ok(());
    }

    render_offline_save_outcome(&outcome);
    Ok(())
}

fn render_offline_save_outcome(outcome: &SaveOutcome) {
    use offline::SaveKind;
    let header = match outcome.kind {
        SaveKind::NewService => "Saved offline (new entry)",
        SaveKind::OverwriteUpdatedBytes => "Saved offline (updated existing entry)",
        SaveKind::SkippedByteIdentical => "Skipped — bytes identical to the existing entry",
    };
    println!("{header}");
    println!("  profile:        {}", outcome.offline_profile_name);
    println!("  service id:     {}", outcome.service_id);
    println!("  edmx file:      {}", outcome.edmx_file);
    println!("  odata version:  {:?}", outcome.odata_version);
    println!("  sha256:         {}", outcome.sha256);
    println!("  size:           {} bytes", outcome.size_bytes);
    if outcome.created_new_offline_profile {
        println!("  (created new offline profile bucket)");
    }
}

/// Path B: import an EDMX file from disk. No SapClient — the file
/// path is read directly and run through the import-validation
/// pipeline. Reuses the same atomic-write / lockfile primitives as
/// `cmd_offline_save`.
fn cmd_offline_import(
    file: std::path::PathBuf,
    target_offline_profile: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
    json: bool,
) -> Result<()> {
    let (mut cfg, config_dir) = config::load_config().context("loading config")?;
    let opts = ImportOptions {
        file_path: file,
        target_offline_profile,
        label_override,
        note,
        now_iso: current_iso8601(),
    };
    let outcome =
        offline::import_edmx_file(&mut cfg, &config_dir.path, opts).context("import_edmx_file")?;
    if json {
        let s = serde_json::to_string_pretty(&outcome)?;
        println!("{s}");
        return Ok(());
    }
    render_offline_save_outcome(&outcome);
    Ok(())
}

/// List offline profiles (when `profile` is None) or services within
/// an offline profile. Reads only the local TOML index — no network.
fn cmd_offline_list(profile: Option<String>, json: bool) -> Result<()> {
    let (cfg, _) = config::load_config().context("loading config")?;
    match profile {
        None => render_offline_profiles(&cfg, json),
        Some(name) => render_offline_services(&cfg, &name, json),
    }
}

fn render_offline_profiles(cfg: &config::ConfigFile, json: bool) -> Result<()> {
    if json {
        let entries: Vec<_> = cfg
            .offline_profiles
            .iter()
            .map(|(name, p)| {
                serde_json::json!({
                    "name": name,
                    "source_profile": p.source_profile,
                    "created_at": p.created_at,
                    "service_count": cfg
                        .offline_services
                        .iter()
                        .filter(|s| &s.profile == name)
                        .count(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }
    if cfg.offline_profiles.is_empty() {
        println!(
            "No offline profiles. Use `sap-odata offline save` (from a connected profile) or `sap-odata offline import <file>` to create one."
        );
        return Ok(());
    }
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_header(vec!["Profile", "Source", "Services", "Created"]);
    for (name, p) in &cfg.offline_profiles {
        let count = cfg
            .offline_services
            .iter()
            .filter(|s| &s.profile == name)
            .count();
        let source = if p.source_profile.is_empty() {
            "—"
        } else {
            &p.source_profile
        };
        table.add_row(vec![
            Cell::new(name),
            Cell::new(source),
            Cell::new(count.to_string()),
            Cell::new(&p.created_at),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn render_offline_services(cfg: &config::ConfigFile, profile: &str, json: bool) -> Result<()> {
    if !cfg.offline_profiles.contains_key(profile) {
        anyhow::bail!(
            "Offline profile '{profile}' not found. Run `sap-odata offline list` to see available profiles."
        );
    }
    let services: Vec<_> = cfg
        .offline_services
        .iter()
        .filter(|s| s.profile == profile)
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&services)?);
        return Ok(());
    }
    if services.is_empty() {
        println!("Offline profile '{profile}' has no services yet.");
        return Ok(());
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        "Service ID",
        "Label",
        "Version",
        "Size",
        "Source",
    ]);
    for s in services {
        let source = s
            .source_service_path
            .clone()
            .or_else(|| s.original_filename.clone())
            .unwrap_or_else(|| "—".to_string());
        table.add_row(vec![
            Cell::new(&s.id),
            Cell::new(&s.label),
            Cell::new(&s.odata_version),
            Cell::new(format!("{} B", s.size_bytes)),
            Cell::new(truncate_for_table(&source, 60)),
        ]);
    }
    println!("{table}");
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

/// Whether a metadata identifier (entity-set name, property name, response
/// column) belongs to the SAP V4 framework's reserved-prefix family. The
/// `SAP__` and `__` double-underscore prefixes are owned by the V4
/// runtime — `SAP__Messages` (transient message container),
/// `SAP__Currencies`, `SAP__UnitsOfMeasure`, `__OperationControl` — none
/// of which are part of the developer's data model. Table output filters
/// these out for a cleaner read; `--json` output is left untouched so
/// scripted consumers still see the raw service truth. The same rule
/// lives in the desktop frontend (services.js / describe.js / results.js).
fn is_sap_system_field(name: &str) -> bool {
    name.starts_with("SAP__") || name.starts_with("__")
}

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
                && !is_sap_system_field(k)
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
