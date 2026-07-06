---
name: sap-odata-cli
description: Query and inspect SAP OData services with the sap-odata CLI — service discovery, entity metadata and SAP/UI5 annotations, test queries, Fiori-readiness lint, service health checks, and offline EDMX browsing. Use when the user asks about SAP OData or Gateway services, entity sets, $metadata, SAP annotations, Fiori readiness, or wants live data samples from an SAP system.
---

# sap-odata CLI for agents

`sap-odata` is a **read-only** SAP OData explorer. It never creates, updates,
or deletes business data on the SAP side — every SAP-facing operation is a GET.
The only things it mutates are its own local files (profiles in
`connections.toml`, the offline EDMX library).

## Ground rules

- **Add `--json` to any command whose output you parse.** Default output is a
  human table. JSON goes to stdout; progress banners and `--verbose` HTTP
  traces go to stderr — parse stdout only.
- **Never run `sap-odata setup`** — it is an interactive wizard and will hang a
  non-interactive shell. Profiles are set up by the user (wizard or desktop
  app). If no profile exists, ask the user to create one; for scripted/CI use,
  `profile add` with flags or `SAP_BASE_URL`/`SAP_CLIENT`/`SAP_USER`/`SAP_PASSWORD`
  env vars work non-interactively.
- **Never put passwords in command lines you compose.** Credentials live in
  the OS keyring (or env vars the user set). If auth is missing, say so —
  don't invent `--password` values.
- **Prefer small probes.** Use `--top 5` (or less) for samples; use
  `--count --top 0` when only a row count is needed.
- Exit codes are meaningful: non-zero = failure. `verify` exits 1 if any
  entity-set probe failed; `lint --fail-on <warn|miss>` exits non-zero at that
  severity — both are CI-gate friendly.

## Orientation — what system, what services

```bash
sap-odata profile list                      # configured systems + auth state
sap-odata -p DEV services -f <text> --json  # find services (substring match)
sap-odata -p DEV -s <SERVICE> entities      # entity sets in a service
```

`-s` accepts a technical name (`API_BUSINESS_PARTNER`, resolved via the
Gateway catalog), a full path (`/sap/opu/odata/sap/...` — required when the
catalog is unpublished, common for V4), or a per-profile alias.

## Inspect an entity

```bash
sap-odata -p DEV -s API_BUSINESS_PARTNER describe A_BusinessPartner --json
```

`describe --json` is the highest-density command: properties, keys, nav
props, labels, plus parsed SAP/UI5 annotations (UI.LineItem columns,
UI.SelectionFields, value helps, field control, text arrangement, search/
count/expand capabilities). Use it before composing queries — it tells you
which properties are filterable/sortable and what the Fiori defaults are.

For raw or bulk annotation questions:

```bash
sap-odata -p DEV -s <SVC> annotations --namespace UI --json   # by vocabulary
sap-odata -p DEV -s <SVC> annotations --filter <text> --json  # by substring
sap-odata -p DEV -s <SVC> metadata > svc.edmx                 # raw $metadata (stdout is clean XML)
```

## Query data

```bash
# Preview the URL without sending anything (always safe):
sap-odata -p DEV -s <SVC> build <EntitySet> --filter "..." --top 5

# Execute:
sap-odata -p DEV -s <SVC> run <EntitySet> \
  --select Col1,Col2 --filter "Col1 eq 'X'" --orderby "Col2 desc" \
  --top 5 --json
```

Useful flags: `--key "'0001'"` (single entity; composite:
`--key "SalesOrder='1',Item='10'"`), `--expand to_Item`,
`--in Prop V1 V2 V3` (expands to OR'd equality, quoting handled),
`--count` (adds `$inlinecount`/`$count`).

JSON shape passes the server response through: V2 rows are under
`.d.results[]`, V4 rows under `.value[]`. Check `entities` output or the
service list's version column to know which.

## Health checks and lint

```bash
sap-odata -p DEV -s <SVC> verify --json     # probe every entity set; exit 1 on any FAIL
sap-odata -p DEV -s <SVC> lint --fail-on miss --json   # Fiori-readiness gate
```

`lint --json` findings carry `severity`, stable `code`, `suggested_cds`
(e.g. `@UI.headerInfo`) and `why_in_fiori` — quote those directly when
advising on CDS annotation fixes.

## Offline EDMX library (no network needed)

The library caches `$metadata` documents locally. The CLI **manages** it;
browsing cached services with `describe`/`lint` is desktop-app-only today.

```bash
sap-odata offline list                              # buckets
sap-odata offline list --profile "DEV (offline)"    # services + ids in a bucket
sap-odata offline import file.edmx --label MYSVC    # ingest a hand-carried EDMX
sap-odata -p DEV -s <SVC> offline save              # capture from a live system
```

To answer metadata questions offline, read the cached file directly — it is
plain EDMX XML at `{config}/offline/<bucket-slug>/<service_id>.edmx`, where
`{config}` is the directory `sap-odata profile where` prints. Parse entity
types, properties, and annotations straight from the XML.

`offline import` accepts API-Hub `.edmx`, `/IWFND/GW_CLIENT` "Save
Response" XML, `curl <base>/$metadata` dumps. `offline delete` prompts for
confirmation — a script must pass `-y`; treat it as destructive and only run
it when the user asked.

## When things fail

Error text includes SAP-specific hints — surface them to the user verbatim,
they name the fixing transaction. Common cases:

| Signal | Meaning | Your move |
|---|---|---|
| `not registered in /IWFND/MAINT_SERVICE` | service not activated in Gateway | tell the user; it needs an admin |
| 403 on catalog | user lacks catalog authorization | tell the user (roles / SU53) |
| `browser sign-in incomplete` / HTML instead of OData | Browser SSO session expired | user must run `sap-odata signout <profile>` and re-sign-in from the desktop app — you cannot do this |
| `keyring locked` / `keyring corrupt` in `profile list` | OS credential store issue | user unlocks/repairs the credential store; do NOT re-add the profile |
| V4 service not found via catalog search | V4 catalog node often unpublished | use the full `/sap/opu/odata4/...` path with `-s` |

For anything unclear, re-run the failing command with `--verbose` and read
the HTTP trace on stderr (auth headers are redacted; safe to inspect).
