# Changelog

All notable changes to this project are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.1.0-alpha.4] — 2026-05-19

**Headline:** offline EDMX library — capture `$metadata` from a connected SAP system or import an EDMX file from disk, then browse it in the same desktop UI without any network connection. Designed as a route-around for the unsigned-exe install friction at customer sites: someone with `curl` / `/IWFND/GW_CLIENT` / browser access can pull a `$metadata` document and hand it to a consultant running the explorer on their own laptop.

Also: CLI ergonomics polish, a dedicated service-health subcommand, a centralised error-hint layer used by both surfaces, profile edit-in-place, and a V4-catalog-miss workaround hint.

### New — Offline mode

- **"Save offline" button (desktop)** captures the bytes of a connected service's `$metadata` to the offline library under a `<NAME> (offline)` bucket attributed to the source system. Same atomic-write discipline as the rest of the config: temp file + rename + parent-dir sync, cross-process lockfile, content-hash-verified byte-identical skip on re-save.
- **"Import EDMX" modal (desktop)** accepts files from any reasonable source — SAP API Hub `.edmx`, `/IWFND/GW_CLIENT` "Save Response" `.xml`, browser save-as on `<base>/$metadata`, `curl > out.xml`. Validated through an 11-step pipeline before persistence: size cap, gzip / HTTP-headers sniff, BOM strip, byte-level XXE scan (DOCTYPE / ENTITY), UTF-8 decode, wrong-root classification (HTML login page → friendly hint, Atom service doc, OData error envelope), XML parse, EDMX root + namespace check, Schema presence.
- **`OFFLINE` badge in the profile picker** with source-system attribution ("DEV (offline) — from DEV" / "Imported — imports"). Run / query / value-help buttons disabled for offline profiles (UI gating mirrors backend `assert_network_allowed`).
- **CLI parity:** `sap-odata offline save`, `offline import <FILE>`, `offline list [--profile NAME]`, `offline delete --profile NAME [--service-id ID]`. All four work without any configured profile — useful for receiving a hand-carried EDMX file on a fresh consultant laptop.
- **Label auto-derivation** from `Schema Namespace`. V4 SAP services (`com.sap.gateway.srvd.*` and `srvd_a2x` family) render as `<SVC>_<N>` with leading zeros stripped (e.g. `com.sap.gateway.srvd_a2x.api_warehouse_order_task_2.v0001` → `API_WAREHOUSE_ORDER_TASK_2_1` — the canonical name a consultant sees in SEGW / `/IWFND/MAINT_SERVICE`). V2 services use the trailing dot-segment; URL-shaped or non-ASCII namespaces fall through to the filename stem.
- **Global name uniqueness** across `connections` + `offline_profiles` enforced at every add path (desktop add-profile modal, CLI `profile add`, CLI `setup` wizard, path-A save, path-B import) plus at dispatch time (`MetadataSource::resolve` fails closed on collision) so the no-network guarantee is closed at the boundary, not just the entry points.
- **TOML schema extensions:** `[offline_profiles.*]` (bucket metadata) and `[[offline_services]]` (one row per cached service with `id`, `label`, `source_service_path`, `edmx_file`, `sha256`, `size_bytes`, `odata_version`, attribution timestamps, optional note). Backward-compatible with pre-offline `connections.toml` (every field defaults).
- **Storage layout** — `{config}/offline/<slug(profile)>/<service_id>.edmx` where `service_id` is `<slug(label)>-<8-hex>`. Hash-suffixed filenames are tool-generated; no raw user input flows into a filename component except via the slugifier. Path-traversal safety via `safe_join_under` (syntactic) + `canonicalize_under` (runtime symlink/reparse-point boundary check with strict-descendancy).
- **`userinfo` (`user:pass@host`) stripped from `source_url` at save time** so consultant credentials never land in an offline-pack hand-carried to a peer.

### New — Other

- **Profile edit (desktop)**: pencil button next to the profile dropdown opens the existing add-profile modal in edit mode. URL, password, auth mode, language, and the Kerberos-delegation toggle are editable; profile name and basic-auth username are intentionally locked (both feed the OS keyring entry key, so changing them requires delete + re-add). Blank password keeps the existing keyring entry.
- **V4 catalog-miss UX hint (desktop)**: when the V4 ServiceGroups catalog returns 403/404 (common on customer systems where the `/IWFND/CONFIG` V4 SICF node isn't active), the sidebar shows an amber footer pointing to the paste-full-path workaround (`/sap/opu/odata4/...`). Pasted paths already bypass the catalog via the existing `isServicePath` shortcut; the hint is purely educational.
- **`verify` subcommand (CLI)**: `sap-odata verify <service>` issues a small `$top` GET against each entity set in a service and prints an OK/FAIL table with row counts. Exit code reflects whether any probe failed — designed as a CI- and agent-friendly health check after a backend change. Flags: `--quick` (list only), `--top N`, `--json`. SAP V4 framework sets (`SAP__*`, `__*` prefixes) are skipped.
- **`--in PROP V1 V2 ...` filter shortcut (CLI)** on `run` / `build`: expands to `(PROP eq 'V1' or PROP eq 'V2' or ...)`. Combines with `--filter` via AND-wrapped parens, preserving operator precedence. Apostrophes inside values are doubled per the OData spec (`O'Brien` → `'O''Brien'`) before URL-encoding. Saves PowerShell-escaping pain when filtering against a small enumerated set.
- **Centralised `response_hint`** covering common SAP failure shapes — used by both CLI and desktop surfaces so error text stays consistent. New cases: 404 on a V4 path names the three actionable causes (wrong service path / wrong entity set / unpublished V4 SICF node); 404 on V2 appends "use `entities -s <svc>` to list valid sets"; 401/403 on Browser SSO points to `signout`; 401/403 on basic mentions SU53 for missing authorisations; 4xx responses with a parseable SAP error code in the OData envelope suggest an SE91 lookup (placeholder `/0` codes filtered out).
- **Service-resolution banner now goes to stderr** instead of stdout, so `sap-odata -s SVC metadata > out.xml` captures clean XML without the "Resolving..." preamble. Mirrored in `CLI-REFERENCE.md`.

### Fixes

- `verify --verbose` no longer drops the HTTP trace on FAIL — `cmd_verify` was using `std::process::exit(1)` which bypassed the post-command trace emitter; switched to `anyhow::bail!` so the trace fires before the error propagates (precisely when it's most useful).
- `build --json` was documented to emit a JSON string but always printed plain text. Now honors `--json`.
- `extract_sap_error_code` walks past placeholder `/0` entries at both the top level and nested `errordetails`. SAP sometimes emits a leading `/0` warm-up entry followed by the real code (e.g. `SY/530`); the previous `.first()` returned the placeholder and produced unhelpful suggestions.
- Browser-SSO HTML-fallback (the common expired-session shape on SAP/IdP gateways) didn't route through `response_hint`, so the `signout <profile>` pointer never fired for that path. Hardcoded messages now mention `signout` whether the failure surfaces as 401/403 or HTML interception.

### Docs

- **README Installation table** enumerates the five release assets (MSI / NSIS / portable GUI / CLI / SHA256SUMS) with one-line "when to use" guidance per file, including the locked-down-environment framing for the portable GUI exe.
- **`docs/RELEASE-CHECKLIST.md`** documents the five-asset contract and references `scripts/stage-release-assets.mjs` for the portable rename + checksum regeneration.
- **README "Config location" section** under "Build from source" makes the `connections.toml` paths discoverable independently of the desktop quick-start, since the CLI and desktop app share the same file.
- **README Quick-start** mentions the `✎` (edit) / `−` (remove) buttons and the V4 paste-full-path workaround.

### Dev tooling

- `tauri-app/src-tauri/tauri.conf.json`: `beforeDevCommand` changed from `npm run css` to `npm run dev` so `cargo tauri dev` boots the static server on `:1420` itself instead of hanging on "Waiting for your frontend dev server...".
- `scripts/stage-release-assets.mjs`: copies build artifacts into `dist/v<tag>/` under the canonical filenames the README references, handles the `sap-odata-explorer-app.exe` → `SAP-OData-Explorer_<ver>_portable.exe` rename, and writes `SHA256SUMS.txt` over the final names in one shot.

### Tests

- `client::tests` covering each new `response_hint` branch (V4 404, V2 404 hint append, basic 401/403 SU53 mention, Browser SSO signout pointer, SAP-error-code SE91 suggestion).
- Regression test for `extract_sap_error_code` walking past placeholder `/0` entries.

### Internals

- `get_services` Tauri command now returns `{ services, warnings }` instead of a bare service list, so per-version catalog warnings (e.g. "V4 catalog: 403 Forbidden") surface to the renderer instead of being dropped.
- `ProfileInfo` gains `language` + `sso_delegate` so the edit modal can prefill without silently regressing those fields on save.

## [0.1.0-alpha.2] — 2026-05-07

Second public alpha. Adds the Fiori-readiness footer popup + the SAP V4 system-field filter, wraps the post-alpha.1 security/correctness hardening (CSP `style-src` tightening, XSS attribute-context coverage, structured keyring errors, entity-key encoding), and ships local fonts that actually decode. Interfaces may still change.

### New

- **Fiori readiness moved to a footer popup.** Used to render at the bottom of the describe panel whenever SAP View was on. Now a severity-tinted pill (`Fiori N miss` / `Fiori N warn` / `Fiori X pass`) next to the annotations badge, opening a modal with the same category-grouped findings and ABAP CDS hints. Tone is driven by the worst severity present.
- **SAP V4 system fields hidden** from the entity-set picker, the describe property tables, and the results grid. The reserved `SAP__` / `__` double-underscore prefix family (`SAP__Messages`, `__OperationControl`, `SAP__Currencies`, ...) is V4 framework noise, not part of the data model. Same filter mirrored in the CLI's `entities` / `describe` / `run` table output. `--json` is intentionally untouched on every command so scripted consumers see raw service truth; `lint` and `annotations` are also unchanged (inspection commands by design).
- **Lint output polish**: `--lint-profile <list-report|object-page|value-help|analytical|transactional>` override (renamed from `--profile` to avoid colliding with the global connection-profile flag), `--fail-on <warn|miss>` for CI use, top-level `detected_profile` in JSON output, two new consistency rules.
- **Structured keyring read errors** surfaced to CLI/Tauri user text. Distinguishes "no password stored", "keyring backend unavailable / locked", and "credential corrupted" so the user knows whether to unlock the OS credential store vs. re-add the profile.
- **Copy-row column moved to the front** of the results grid (admin-table convention).

### Fixes

- **Entity key-path encoding**: `ODataQuery::build()` percent-encodes path-unsafe characters in the key segment (spaces, slashes, OData-quoted apostrophes). Composite keys keep their delimiters intact. CLI key syntax is unchanged.
- **CLI `profile list` Browser SSO row**: was shadowed by the basic-auth credential branch in some cases; now branches on Browser SSO first so the password column reads `browser SSO` instead of `none`.
- **Local fonts**: `tauri-app/src/fonts/*.woff2` had been Google "Error 404 (Not Found)" HTML pages saved with a `.woff2` extension since the fonts dir first landed (1.6 KB each, magic bytes `<!DO`). Browser fetched them silently, OTS rejected them, fell back to system fonts. Replaced with the canonical variable-weight woff2 binaries from `@fontsource-variable` (DM Sans + JetBrains Mono). Strict CSP posture preserved — fonts stay self-hosted.
- **Add-profile dialog a11y**: seven `<label>` tags lacked `for=` association with their inputs. Added explicit associations.

### Security

- **CSP `style-src` no longer carries `'unsafe-inline'`.** The 432-line inline app stylesheet and `@font-face` block were externalised from `index.html` into `input.css` (compiled by Tailwind v4 into the existing `style.css`); three inline `style="..."` attributes were converted to utility classes. The Tauri WebView CSP is now `default-src 'self'; script-src 'self'; style-src 'self'; font-src 'self'; img-src 'self' data:; connect-src ipc: http://ipc.localhost`. CSSOM property setters (`el.style.foo = ...`) are not gated by `style-src` and continue to work; only inline `<style>`, `style="..."`, and string-parsed `cssText` are blocked, none of which exist in the codebase.
- **`safeHtml` now covers HTML-attribute context**, not only element-text. `escapeHtml` was extended to handle attribute-quote characters, and the renderers were audited so any value interpolated into a `class="…"` / `data-…="…"` / `title="…"` slot is escaped. CI gate (`scripts/lint-innerhtml.mjs`) now also rejects unescaped attribute interpolations.
- **Malicious-metadata escaping fixtures** pin the parser/renderer boundary contract: the metadata parser keeps untrusted strings (entity names, labels, annotation values) as raw data, and the renderer is responsible for escaping via `safeHtml` / `escapeHtml`. Fixtures cover HTML-shaped payloads in label, annotation, entity-name, and property-name slots.
- **Redaction**: `Authorization`, `Cookie`, `X-CSRF-Token`, and SAP password values are scrubbed from the HTTP trace inspector, including body-preview echo defense (a server reflecting a header value back in the response body no longer leaks the secret). 13 redaction tests across two layers.
- **Existing CSP work preserved**: `script-src 'self'` (no `'unsafe-inline'`); `withGlobalTauri: false`; the Tauri API is loaded as a vendored ES module from `tauri-app/src/vendor/tauri-core.js`. IPC capability scoped to the `main` window only.
- **Central XSS sanitiser**: `safeHtml` tagged-template helper auto-escapes every `${...}` interpolation; `raw()` opt-out is keyed by a closure-private `Symbol` so JSON-shaped payloads can't forge a marker.
- **Fail-closed keyring writes** with explicit user confirmation before any plaintext fallback (signalled via `KEYRING_FAILED:` prefix from the backend).

### Tests

- Focused lint regressions for previously under-specified rules (list-report / object-page profile behavior, `selection_hidden`, `value_list_no_out`, `selection_non_filterable`) and stable finding ordering.
- Integration-test corpus (now 158 tests across the workspace) covering V4 CSDL annotations, V2 EDMX `sap:*` attributes, CSRF roundtrip, Browser SSO redirect detection, error-envelope edge cases, malformed XML, composite keys, and a synthetic 1500-entity perf-regression test (`#[ignore]`'d by default).

### CI / Build

- **Clippy is a hard gate**: `cargo clippy --workspace --all-targets -- -D warnings`.
- **Workspace tests**: `cargo test --workspace` (covers core + CLI + the Tauri-app crate).
- **Tauri build gated**: CI compiles the desktop app on `ubuntu-latest` and `windows-latest` (`npx tauri build --ci --no-bundle`).
- **innerHTML lint**: required gate; auto-discovers every `tauri-app/src/**/*.js` so new modules don't escape coverage. Now also covers attribute-context interpolations.

### Internal

- Lint evaluator body split out of `mod.rs` into `lint/rules.rs` and grouped by category (identity, list-report, filtering, fields, consistency, integrity, capabilities). Output ordering is preserved by the focused regression tests above.
- 432-line inline app stylesheet + two `@font-face` blocks moved out of `index.html` into `input.css` (compiled to `style.css` via Tailwind v4).
- Frontend Fiori-readiness renderer split into a footer-pill renderer + a modal-content renderer, replacing the older inline panel layout.
- (Carry-forward) `crates/core/src/metadata.rs` split into `metadata/{mod,model,annotations}.rs`; `crates/core/src/lint.rs` split into `lint/{mod,types,profile,rules}.rs`; `tauri-app/src/app.js` split into 21 focused ES modules.

### Known limitations

- Binaries remain **unsigned** — Windows SmartScreen will show an "unrecognized app" warning on first launch. Code-signing path is documented but cert-gated.
- Manual Browser SSO validation on Azure AD + SAP IAS / Okta / ADFS is still pending real-landscape access; the unit-tested behaviour has not yet been field-tested across the full IdP matrix.
- Windows-only release artifacts; Linux/macOS builds remain source-only for this alpha.

## [0.1.0-alpha.1] — 2026-04-24

First public alpha. Interfaces may still change — feedback welcome via GitHub issues.

### What's in this release

- **SAP OData V2 and V4 client** with SAP Gateway catalog discovery, metadata browsing, and an interactive query builder (`$select` / `$expand` / `$filter` / `$orderby` / `$top` / `$skip`).
- **Three authentication modes:** Basic (passwords stored in the OS keyring; fails closed on keyring errors), Windows SSO via SPNEGO/Kerberos, and Browser SSO for SAML / Azure AD / SAP IAS flows with session persistence.
- **SAP View overlay** — reads SAP/UI5 annotations and renders the explorer the way a Fiori app would: Fiori-ordered columns with TextArrangement, `Common.Text` description folding, `UI.SelectionFields` filter chips, `UI.PresentationVariant` / `UI.SelectionVariant` one-click buttons, `Common.ValueList` F4 pickers (including the S/4HANA `ValueListReferences` pattern), `Capabilities.*` restriction validation.
- **Fiori-readiness linter** — `sap-odata lint` and the desktop describe panel. Profile-aware (list-report / object-page / value-help / analytical / transactional), catches dangling `Path` references, emits ABAP CDS fix hints on actionable findings.
- **CLI:** interactive setup wizard, connection profiles, aliases, services / describe / run / build (dry-run) / metadata / lint / annotations.
- **Desktop app (Tauri 2):** multi-tab workspace, favorites & history, per-tab HTTP inspector with copy-as-curl. CSP locked to local assets and IPC; IPC capability scoped to the `main` window only.

### Known limitations

- Binaries are **unsigned** — Windows SmartScreen will show an "unrecognized app" warning.
- Auth validated on some Azure AD configurations; SAP IAS, Okta, and ADFS still to validate.
- CI builds on Windows and Linux; macOS builds are source-only for now.
- 77 clippy style warnings remain (style, not correctness) — tracked for cleanup.

### Security

- `cargo audit` green (0 vulnerabilities) and runs in CI on every push.
- Disclosure process: see [SECURITY.md](SECURITY.md).
