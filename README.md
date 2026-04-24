# sap-odata-explorer

> A fast, SAP-aware OData explorer that removes the pain of Gateway Client and generic API testers.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/Rust-1.85+-orange.svg)](https://rustup.rs)
[![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux%20%7C%20macOS-lightgrey)](#)

A CLI tool and desktop app for exploring and testing SAP OData services against real customer systems. Supports **OData V2 and V4**, SAP Gateway catalog discovery, and all three common authentication modes: basic, Windows SSO (Kerberos), and browser-based SSO (Azure AD / SAP IAS / SAML).

> [!NOTE]
> Early-stage project. Core features work end-to-end on real SAP systems but polish and distribution are ongoing. Feedback and contributions welcome.

## Why

SAP developers and consultants waste a lot of time fighting tools that aren't built for SAP OData:

- **SAP Gateway Client** (`/IWFND/GW_CLIENT`) is cumbersome, ugly, and single-system-bound.
- **Postman / Insomnia / Bruno** are great general API clients but have no SAP intelligence — no CSRF handling, no `sap-client`, no metadata browsing, no V4 catalog discovery.
- **SAP Business Accelerator Hub** only covers standard APIs, not custom services.
- **SAP Business Application Studio** is a cloud IDE, not a focused OData explorer.

This tool fills that gap.

## Features

- **Service discovery** — browse V2 and V4 services from SAP Gateway catalogs with search
- **Entity explorer** — list entity sets, see properties, keys, navigation properties, labels
- **Visual query builder** — click properties to add to `$select`, nav props to `$expand`, build `$filter`/`$orderby`/`$top`/`$skip`
- **Results grid** — data table with expandable nested data from `$expand`
- **Connection profiles** — save SAP systems, passwords in OS keyring (Windows Credential Manager / macOS Keychain / Linux Secret Service)
- **Three auth modes** — basic, Windows SSO (SPNEGO), browser SSO (Azure AD / SAP IAS SAML chain)
- **Service aliases** — short names for long V4 paths
- **Auto resolution** — type `API_WAREHOUSE_2`, the tool looks it up in the catalog
- **Tabs** — multiple independent workspaces in the desktop app
- **Favorites and history** — star services, replay past queries
- **Copy helpers** — clipboard buttons for rows, columns, query URLs
- **Filter helper** — click any cell value to filter by it
- **Single binary** — no runtime dependencies, cross-platform (Windows, Linux, macOS)

## Installation

Until signed releases exist, build from source (see [CONTRIBUTING.md](CONTRIBUTING.md)) or download unsigned releases from GitHub once published.

> [!WARNING]
> Windows SmartScreen may show an "unrecognized app" warning when launching unsigned releases downloaded from the internet. Click **More info → Run anyway**. Reputation and/or code signing is planned.

## Quick start

### Desktop app

1. Launch `sap-odata-explorer-app.exe`
2. Click `+` next to the profile dropdown to add a system
3. Choose auth mode (Basic / Windows SSO / Browser SSO) and save
4. Click **Search** to browse services, pick one
5. Click an entity set in the sidebar → click property names to build a query → **Run**

### CLI

```bash
# Save a system once — password goes to OS keyring
sap-odata profile add DEV --url https://myhost:44300 --client 100 --user myuser --password 'mypass'

# Or Windows SSO (no password)
sap-odata profile add PRD --url https://prdhost:44300 --client 100 --sso

# Find a service
sap-odata -p DEV services -f warehouse

# Explore and query — just use the service name
sap-odata -p DEV -s API_WAREHOUSE_2 entities
sap-odata -p DEV -s API_WAREHOUSE_2 describe Warehouse
sap-odata -p DEV -s API_WAREHOUSE_2 run Warehouse --top 5
```

## CLI commands

| Command | Purpose |
|---|---|
| `profile list/add/remove/test/where` | Manage saved SAP systems |
| `alias add/list/remove` | Per-profile short names for services |
| `services` | List available OData services from the catalog |
| `entities` | List entity sets in a service |
| `describe <set>` | Show properties, keys, nav properties, labels |
| `functions` | List function imports / actions |
| `build <set> [query]` | Dry-run: print the OData URL, no HTTP call |
| `run <set> [query]` | Execute query, show results as table |
| `metadata` | Dump raw `$metadata` XML |

See [CLI-REFERENCE.md](docs/CLI-REFERENCE.md) for all options (or run `sap-odata <command> --help`).

## Authentication

- **Basic** — username/password, stored in OS keyring
- **Windows SSO** — SPNEGO/Kerberos via Windows SSPI, no credentials needed (domain-joined machines)
- **Browser SSO** — for SAP systems behind SAML chains like Azure AD + SAP IAS. Opens a webview to complete the sign-in flow and captures the session cookies.

## Security notes

- **TLS verification** is enabled by default. For self-signed SAP certs, set `insecure_tls = true` in the profile's `connections.toml`.
- **Passwords** stored in OS keyring, never in plaintext by default.
- **CSP** enforced in the Tauri app — no external CDNs, all assets bundled locally.
- **Browser SSO sessions** are in-memory only (re-authenticate after app restart).

## How it compares

| Tool | Verdict |
|---|---|
| **SAP Gateway Client** | We win on usability, cross-platform, catalog discovery, modern UI. Gateway Client is still better for backend QA replay/simulation. |
| **SAP Business Accelerator Hub** | We win for real customer systems, custom services, live queries. Hub wins for official API browsing and SDK downloads. |
| **Postman / Insomnia / Bruno / Hoppscotch** | We win for SAP-specific needs (CSRF, sap-client, V4 catalog, metadata). They win on breadth, collections, team features, protocols beyond HTTP. |
| **OData MCP bridges** | Complementary — they expose OData to AI agents. This is a human-first tool. |

**Best fit:** *"I need to quickly understand and query this SAP OData service."*

## Project structure

```
sap-odata-explorer/
├── crates/
│   ├── core/       # Shared library (auth, HTTP, metadata, query, catalog, config, SSO)
│   └── cli/        # CLI binary
└── tauri-app/
    ├── src/        # Frontend (HTML + Tailwind + vanilla JS)
    └── src-tauri/  # Tauri commands wrapping core
```

The `sap-odata-core` crate holds all protocol logic. CLI and Tauri are thin wrappers. This makes it easy to add a third frontend later (MCP server, web app, etc.).

## Roadmap

Short term:
- [ ] Write operations (POST / PATCH / DELETE)
- [ ] Saved requests / collections
- [ ] Import/export to Postman / Bruno / curl / OpenAPI
- [ ] Raw request/response inspector panel

Later:
- [ ] MCP server (expose core to AI agents)
- [ ] Schema diff between systems (DEV vs QAS vs PRD)
- [ ] Export metadata to JSON Schema / TypeScript types
- [ ] Code signing and auto-update

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports, PRs, and discussions welcome.

## License

MIT — see [LICENSE](LICENSE).
