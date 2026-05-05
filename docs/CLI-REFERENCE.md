# CLI reference

Full command reference for `sap-odata`. For a quick start, see the [main README](../README.md#cli).

- [Global flags](#global-flags)
- [Environment variables](#environment-variables)
- [Service path resolution](#service-path-resolution)
- [Output formats](#output-formats)
- [Commands](#commands)
  - [`setup`](#setup)
  - [`profile`](#profile)
  - [`alias`](#alias)
  - [`services`](#services)
  - [`entities`](#entities)
  - [`describe`](#describe)
  - [`functions`](#functions)
  - [`build`](#build)
  - [`run`](#run)
  - [`metadata`](#metadata)
  - [`signout`](#signout)
- [HTTP trace (`--verbose`)](#http-trace---verbose)
- [Configuration file](#configuration-file)
- [Troubleshooting](#troubleshooting)

All examples use fictional hostnames like `sap-dev.example.com` paired with real SAP standard service paths (`API_BUSINESS_PARTNER`, `API_SALES_ORDER_SRV`, etc.) so you can translate them directly to your system.

## Global flags

These work with any command:

| Flag | Short | Environment | Purpose |
|---|---|---|---|
| `--profile` | `-p` | — | Connection profile name from `connections.toml` |
| `--service` | `-s` | — | OData service path or alias (see [resolution rules](#service-path-resolution)) |
| `--url` | | `SAP_BASE_URL` | Override the SAP base URL |
| `--client` | | `SAP_CLIENT` | Override the `sap-client` value |
| `--language` | | `SAP_LANGUAGE` | Override the `sap-language` value |
| `--user` | | `SAP_USER` | Basic-auth username |
| `--password` | | `SAP_PASSWORD` | Basic-auth password |
| `--json` | | — | Emit JSON instead of a rendered table |
| `--verbose` | `-v` | — | Enable debug logs and print the full HTTP trace to stderr after the command runs |

CLI flags override environment variables, which override profile values.

## Environment variables

The env-var names above mirror the SAP-client conventions. They're useful for CI jobs or quick one-offs that shouldn't touch a profile:

```bash
export SAP_BASE_URL="https://sap-dev.example.com:44301"
export SAP_CLIENT="100"
export SAP_USER="DEVELOPER"
export SAP_PASSWORD="..."

sap-odata services -f business_partner
```

## Service path resolution

For commands that need `--service`, the CLI resolves the value in this order:

1. **Absolute path** — anything starting with `/` is used as-is (e.g., `-s /sap/opu/odata/sap/API_BUSINESS_PARTNER`).
2. **Alias** — if the value matches an alias on the active profile (see [`alias add`](#alias)), the alias target is used.
3. **Catalog lookup** — otherwise the CLI queries the SAP Gateway catalog (`/IWFND/CATALOGSERVICE;v=2`) and resolves by technical name, e.g. `-s API_BUSINESS_PARTNER`.

Aliases are scoped per profile, so the same short name (`bp`, `so`) can point at different services on DEV vs QAS vs PRD.

## Output formats

Default output is a compact table (via `comfy-table`). Pass `--json` to get a structured JSON dump suitable for pipelines:

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER run A_BusinessPartner --top 5 --json | jq '.d.results[].BusinessPartner'
```

For V2 services JSON results are under `d.results`; V4 services expose them under `value`. The CLI passes the server's JSON through unchanged.

## Commands

### `setup`

Interactive wizard to add a new SAP system. Walks through profile name, base URL, client, language, auth mode (basic / Windows SSO / browser SSO), and stores the profile in `connections.toml`. For basic and Windows SSO profiles it can also test the connection immediately.

```bash
sap-odata setup
```

For Browser SSO: the wizard saves the profile, but the actual interactive sign-in happens from the desktop app. After one sign-in there, the CLI reuses the persisted session automatically.

### `profile`

Manage saved connection profiles.

#### `profile list`

```bash
sap-odata profile list
```

Shows each profile with its base URL, client, language, auth mode, and the password column. The password column is one of:

- `Browser SSO` — interactive SAML / OIDC sign-in handled by the desktop app; no stored password.
- `SSO (Windows)` — Kerberos / SPNEGO; no stored password.
- `config (plaintext)` — password lives in `connections.toml`. Discouraged.
- `OS keyring` — password stored in the OS credential store.
- `NOT SET` — Basic-auth profile but no password is stored anywhere.
- `keyring locked` / `keyring corrupt` / `keyring error` — the credential store rejected the read. The fix is to unlock / repair the OS credential store, not to re-add the profile. Use `--verbose` for the underlying diagnostic.

#### `profile add`

Non-interactive alternative to `setup`. Good for scripting.

| Flag | Default | Purpose |
|---|---|---|
| `<name>` | — | Profile name (e.g., `DEV`, `QAS`, `PRD`) |
| `--url` | — | Base URL (required) |
| `--client` | `100` | SAP client |
| `--language` | `EN` | SAP language |
| `--user` | `""` | Username (empty for SSO) |
| `--password` | `""` | Password (stored in OS keyring by default) |
| `--sso` | — | Use Windows SSO (SPNEGO/Kerberos) — no user/password |
| `--sso-delegate` | — | Allow Kerberos delegation when `--sso` is set (server can impersonate the user to downstream backends). Off by default; only enable for reverse-proxy → Gateway → backend R/3 setups that need multi-hop auth. |
| `--plaintext` | — | Store password in the config file instead of the keyring (not recommended) |
| `--portable` | — | Write the config next to the executable (portable install) |

> Browser SSO isn't available on `profile add` directly — use `setup` or create the profile from the desktop app, which handles the interactive sign-in and cookie capture.
>
> `--sso-delegate` is also exposed in the desktop add-profile modal as the **Allow Kerberos delegation** checkbox (only shown when auth mode is SSO). Editing `connections.toml` directly is no longer required.

Examples:

```bash
# Basic auth, password in OS keyring
sap-odata profile add DEV \
  --url https://sap-dev.example.com:44301 \
  --client 100 --language EN \
  --user DEVELOPER --password '***'

# Windows SSO (Kerberos via SSPI) — no credentials needed
sap-odata profile add QAS \
  --url https://s4-qas.example.com:44301 \
  --client 200 --language EN --sso

# Portable: write connections.toml next to the executable (for USB-stick use)
sap-odata profile add DEV \
  --url https://sap-dev.example.com:44301 --sso --portable
```

#### `profile remove`

```bash
sap-odata profile remove DEV
```

Removes the profile from `connections.toml`. Also clears the profile's keyring password (if basic auth) and any persisted Browser SSO session cookies.

#### `profile test`

Runs the connection probe against `/sap/opu/odata/IWFND/CATALOGSERVICE;v=2` using the profile's credentials. Useful after creating the profile.

```bash
sap-odata profile test DEV
```

#### `profile where`

Prints the location of `connections.toml` — handy when support asks you to share or edit it.

```bash
sap-odata profile where
```

### `alias`

Per-profile short names for long service paths. Requires `-p`.

#### `alias add`

```bash
sap-odata -p DEV alias add bp  /sap/opu/odata/sap/API_BUSINESS_PARTNER
sap-odata -p DEV alias add so  /sap/opu/odata/sap/API_SALES_ORDER_SRV
sap-odata -p DEV alias add po  /sap/opu/odata/sap/API_PURCHASEORDER_PROCESS_SRV
sap-odata -p DEV alias add mat /sap/opu/odata/sap/API_MATERIAL_DOCUMENT_SRV
```

After adding, use the short name anywhere `-s` expects a path:

```bash
sap-odata -p DEV -s bp entities
sap-odata -p DEV -s so run A_SalesOrder --top 10
```

#### `alias list`

```bash
sap-odata -p DEV alias list
```

Shows every alias for the active profile as a table.

#### `alias remove`

```bash
sap-odata -p DEV alias remove bp
```

### `services`

List OData services exposed by the SAP Gateway catalog.

| Flag | Purpose |
|---|---|
| `-f`, `--filter <text>` | Case-insensitive substring match on name / title / description |
| `--v2` | Only V2 services |
| `--v4` | Only V4 services |
| `--top <n>` | Cap the number of rows shown |

```bash
# Full catalog on DEV
sap-odata -p DEV services

# Find anything business-partner related
sap-odata -p DEV services -f business_partner

# V4-only, top 20
sap-odata -p DEV services --v4 --top 20

# JSON for piping
sap-odata -p DEV services -f sales --json | jq -r '.[].technical_name'
```

### `entities`

List the entity sets in a service.

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER entities
sap-odata -p DEV -s /sap/opu/odata/sap/API_SALES_ORDER_SRV entities
```

### `describe`

Show an entity type's properties, keys, and navigation properties (with SAP labels where available).

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER describe A_BusinessPartner
sap-odata -p DEV -s so describe A_SalesOrder
sap-odata -p DEV -s API_MATERIAL_DOCUMENT_SRV describe A_MaterialDocumentHeader
```

With `--json`, the output also carries the parsed SAP/UI5 annotation fields the desktop app's **SAP View** overlay uses — `header_info` (UI.HeaderInfo), `selection_fields` (UI.SelectionFields), `line_item` (UI.LineItem default columns — `value_path` / `label` / `importance` per DataField), `request_at_least` (UI.PresentationVariant.RequestAtLeast — property paths Fiori silently appends to `$select`), `selection_variants` (UI.SelectionVariant — declared Fiori filter variants with `text`, `qualifier`, `parameters`, and `select_options` carrying SELECT-OPTIONS-style ranges), `searchable` / `countable` / `top_supported` / `skip_supported` / `expandable` / `non_expandable_properties` (Capabilities.Search/Count/Expand/TopSupported/SkipSupported flags applied at entity-set level), and per-property `field_control` (Common.FieldControl — `mandatory`/`optional`/`readonly`/`inapplicable`/`hidden`/`path`) / `hidden` (UI.Hidden marker) / `hidden_filter` (UI.HiddenFilter marker) / `display_format` (V2 `sap:display-format` like `Date`/`NonNegative`/`UpperCase`), and per-property `text_path` / `text_arrangement` (UI.TextArrangement — `textfirst` / `textlast` / `textseparate` / `textonly`) / `unit_path` / `iso_currency_path` / `filterable` / `sortable` / `creatable` / `updatable` / `required_in_filter` / `criticality` / `value_list` (inline Common.ValueList — `collection_path`, `label`, `search_supported`, and the In/Out/InOut/DisplayOnly/Constant `parameters` mapping) / `value_list_references` (Common.ValueListReferences — relative URLs to separate F4 services containing the actual mapping) / `value_list_fixed` (Common.ValueListWithFixedValues marker — set when the property has a small fixed value set). Handy for scripting linting or comparisons without re-parsing `$metadata` yourself.

### `functions`

List function imports / actions declared in `$metadata`.

```bash
sap-odata -p DEV -s API_SALES_ORDER_SRV functions
```

### `build`

Build and print an OData URL without issuing the request. Useful for sanity-checking a query or piping into another tool.

| Flag | Purpose |
|---|---|
| `<entity_set>` | Entity set name (positional) |
| `--select <cols>` | Comma-separated `$select` fields |
| `--filter <expr>` | `$filter` expression |
| `--expand <navs>` | Comma-separated `$expand` navigation properties |
| `--orderby <cols>` | `$orderby` clause (e.g., `"CreationDate desc,BusinessPartner asc"`) |
| `--top <n>` | `$top` |
| `--skip <n>` | `$skip` |
| `--key <keyspec>` | Entity key (single: `'1000000'`, composite: `SalesOrder='1',Item='10'`) |
| `--count` | Add `$inlinecount=allpages` (V2) / `$count=true` (V4) |

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER build A_BusinessPartner \
  --select BusinessPartner,BusinessPartnerName,BusinessPartnerCategory \
  --filter "BusinessPartnerCategory eq '1'" \
  --orderby "BusinessPartnerName asc" \
  --top 25

sap-odata -p DEV -s so build A_SalesOrder \
  --key "'0000000001'" \
  --expand "to_Item"
```

### `run`

Build the query like `build` does, then execute it and render the result.

Same flags as `build`. Add `--json` (global) to get the raw server response instead of a table.

```bash
# Default table output
sap-odata -p DEV -s API_BUSINESS_PARTNER run A_BusinessPartner \
  --select BusinessPartner,BusinessPartnerName \
  --filter "BusinessPartnerCategory eq '2'" \
  --top 10

# Single-key lookup with expansion
sap-odata -p DEV -s so run A_SalesOrder \
  --key "'0000000001'" \
  --expand to_Item \
  --json

# Count only (no rows returned, just the count in the response)
sap-odata -p DEV -s API_PURCHASEORDER_PROCESS_SRV run A_PurchaseOrder \
  --filter "CreationDate gt datetime'2026-01-01T00:00:00'" \
  --count --top 0
```

### `metadata`

Dump the raw `$metadata` XML to stdout. Useful for grepping, diffing, or piping into an XML formatter.

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER metadata > api_bp_metadata.xml
sap-odata -p DEV -s API_MATERIAL_DOCUMENT_SRV metadata | xmllint --format -
```

### `lint`

Run the Fiori-readiness checklist on one entity or the whole service. Same checks the desktop app's describe panel shows under "Fiori readiness" — `UI.HeaderInfo` present, `UI.LineItem` declared, `UI.SelectionFields` declared, decimal-looking properties paired with `Measures.Unit` / `Measures.ISOCurrency`, code-looking columns paired with `Common.Text`, `Common.SemanticKey` when the technical key is UUID-ish, and so on.

| Flag | Purpose |
|---|---|
| `<entity>` (positional, optional) | Entity type or entity set name. Omit to lint every type in the service. |
| `--min-severity <pass\|warn\|miss>` | Suppress findings below this severity in the displayed output. Default shows everything. |
| `--profile <list-report\|object-page\|value-help\|analytical\|transactional>` | Override the auto-detected lint profile. Useful when the heuristics misread a service or to ask "how list-report-ready would this value-help service be?". |
| `--fail-on <warn\|miss>` | Exit non-zero if any finding at or above this severity is present. CI-style gate. Independent of `--min-severity`, which only filters what's displayed. |
| `--json` | Dump the structured findings instead of a table. |

```bash
# Scan the whole service for anything a Fiori list-report app would miss
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 lint --min-severity warn

# One entity, table output
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 lint WarehousePhysicalStockProductsType

# Force-evaluate a value-help entity as if it were a list report
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 lint WarehouseVHType --profile list-report

# CI gate: exit non-zero on any miss, no piping required
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 lint --fail-on miss
```

JSON output is shaped per-entity as `{ entity, detected_profile, effective_profile, findings: [...] }`. `detected_profile` is always the heuristic auto-detection — what the linter would pick if no override were supplied. `effective_profile` is what the rules actually ran against. Without `--profile` the two fields match; with `--profile` they can differ, and a CI consumer can report "we auto-detected X but you forced Y" without losing either signal. `findings` still carries a `profile`-coded banner as its first entry so the text rendering stays consistent.

Findings carry a `severity` (`pass` / `warn` / `miss`), a `category` (`profile` / `identity` / `listreport` / `filtering` / `fields` / `integrity` / `capabilities`), a stable `code`, a human-readable `message`, and — for actionable warnings / misses — a `suggested_cds` token (e.g. `@UI.headerInfo`, `@ObjectModel.text.element`, `@Consumption.valueHelpDefinition`) plus a short `why_in_fiori` explanation. The table output inlines these under each message; the JSON output emits them as separate fields.

The linter is **profile-aware**: it auto-detects the entity's shape (`list_report` / `object_page` / `value_help` / `analytical` / `transactional`) from the name and declared annotations. Subsequent checks skip irrelevant ones (e.g. value-help entities aren't dinged for missing `UI.LineItem`). Use `--profile` to override the auto-detection.

Check families:

- **Presence checks** — "did you declare X?" (HeaderInfo, LineItem, SelectionFields, ...).
- **Consistency rules** — contradictions in already-declared annotations: `SelectionFields` referencing a non-filterable column, `SortOrder` referencing a non-sortable one, `UI.Hidden` columns appearing in `SelectionFields`, `Common.ValueList` without a writable (`Out`/`InOut`) parameter, `UI.TextArrangement` on a property with no `Common.Text` to arrange, `UI.SelectionVariant` referencing a hidden / non-filterable column, `Common.SemanticObject` declared without a `Common.SemanticKey`.
- **Integrity rules** — dangling references: annotations whose Path/target points at a column name that doesn't exist on the entity. Covers `Common.Text`, `Measures.Unit` / `ISOCurrency`, `UI.Criticality` (Path form), `UI.HeaderInfo.Title`, `Common.SemanticKey`, `UI.SelectionFields`, `UI.LineItem.DataField.Value`, `UI.PresentationVariant.SortOrder`. Typical cause: a column was renamed in one CDS layer without the annotation being updated.

### `annotations`

List every SAP/UI5 annotation parsed from `$metadata`, grouped by vocabulary namespace. The same data the desktop app's annotation inspector shows — handy on the command line for "does this service declare X?" grepping and for diffing annotation sets across environments.

| Flag | Purpose |
|---|---|
| `--namespace <ns>` | Filter to one vocabulary (`UI`, `Common`, `Capabilities`, `Measures`, `SAP`, ...). Case-insensitive. |
| `--filter <text>` | Substring match across Term + Target + Value + Qualifier. |
| `--json` | Dump the raw list instead of a table. |

```bash
# Everything declared on the service
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 annotations

# Only UI.LineItem / HeaderInfo / SelectionFields — the Fiori view set
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 annotations --namespace UI

# Search across namespaces for a specific property's annotations
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 annotations --filter EWMWarehouse

# Pipe the raw list through jq for scripting
sap-odata -p DEV -s UI_PHYSSTOCKPROD_1 annotations --json \
  | jq '[.[] | select(.namespace == "Capabilities")] | length'
```

### `signout`

Clear the persisted Browser SSO session for a profile, so the next run forces a fresh sign-in.

```bash
sap-odata signout DEV
```

Only meaningful for Browser SSO profiles. On Basic / Windows SSO profiles it's a no-op.

## HTTP trace (`--verbose`)

Any command accepts `-v` / `--verbose`. That does two things:

1. Turns on `sap_odata=debug` logs for the duration of the command.
2. After the command finishes (success or failure), prints the full HTTP trace to **stderr**. The trace contains one entry per exchange with method, URL, status, timing, request/response headers (Authorization / Cookie / Set-Cookie redacted), and a response body preview (HTML bodies omitted).

```bash
sap-odata -v -p DEV -s API_BUSINESS_PARTNER run A_BusinessPartner --top 1
```

Separate stdout from the trace if you want to keep the table and inspect the trace:

```bash
sap-odata -v -p DEV services -f material 2>trace.log
less trace.log
```

The desktop app shows the same data in the HTTP Inspector panel.

## Configuration file

Profiles and aliases live in `connections.toml`. The CLI resolves its location in this order:

1. **Portable mode** — if `connections.toml` exists in the same directory as the executable, that's used.
2. **OS config dir** — otherwise:
   - Windows: `%APPDATA%\sap-odata-explorer\connections.toml`
   - macOS:   `~/Library/Application Support/sap-odata-explorer/connections.toml`
   - Linux:   `~/.config/sap-odata-explorer/connections.toml`

Run `sap-odata profile where` to see the resolved path on the current machine.

Minimal schema:

```toml
[connections.DEV]
base_url     = "https://sap-dev.example.com:44301"
client       = "100"
language     = "EN"
username     = "DEVELOPER"
# password is stored in the OS keyring by default; only set this if you
# want the password in plaintext in this file (--plaintext).
# password = "..."
sso          = false
browser_sso  = false
insecure_tls = false

[connections.DEV.aliases]
bp  = "/sap/opu/odata/sap/API_BUSINESS_PARTNER"
so  = "/sap/opu/odata/sap/API_SALES_ORDER_SRV"
po  = "/sap/opu/odata/sap/API_PURCHASEORDER_PROCESS_SRV"

[connections.QAS]
base_url    = "https://s4-qas.example.com:44301"
client      = "200"
language    = "EN"
sso         = true    # Windows SSO via SPNEGO
```

Sensitive material (basic-auth passwords, Browser SSO session cookies) is never written to this file — it lives in the OS keyring under the service name `sap-odata-explorer`.

## Troubleshooting

The CLI surfaces SAP-specific hints alongside raw HTTP status codes. These are the most common ones:

| Symptom | Likely cause | Fix |
|---|---|---|
| `service not found: /sap/opu/odata/... — Hint: The OData service path may be wrong, inactive, or not registered in /IWFND/MAINT_SERVICE.` | Service exists in the backend but isn't activated in Gateway | Add it via `/IWFND/MAINT_SERVICE` (transaction) |
| `server returned 403 Forbidden — Hint: Access to the SAP Gateway catalog was denied.` | User lacks catalog read authorisation | Check roles for `/IWFND/CATALOGSERVICE`; users often need `SAP_GATEWAY_ADMIN` or equivalent |
| `browser sign-in incomplete; SAP or the IdP returned HTML instead of OData` | Browser SSO session expired / cookies stale | Run `sap-odata signout <profile>` then sign in again from the desktop app |
| `server returned 500 ... — Hint: SAP returned a server-side error. Check /IWFND/ERROR_LOG, ST22, and backend application logs.` | Dump or application error in the backend | Use transaction `/IWFND/ERROR_LOG` for the gateway error, `ST22` for ABAP short dumps |
| `Authentication was accepted by the HTTP stack but SAP rejected the request.` | SSO transport worked, but the resolved user lacks service authorisation | Check SU53 on the backend for missing authorisation objects |
| `No services found. Warnings: ...` | Catalog returned 0 services, often because of filters that are too strict or a silent catalog error | Rerun with `--verbose` and inspect the trace; try without `--v4`/`--v2`; check the warnings the catalog itself reported |

For anything else, `--verbose` and reading the HTTP trace is almost always enough to spot the cause — it includes the redirect chain, server-reported SAP error messages, and the first ~4KB of any response body.
