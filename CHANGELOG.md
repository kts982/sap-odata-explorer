# Changelog

All notable changes to this project are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

Post-alpha hardening and internal refactors. No new user-facing features yet — focus is on locking the security posture and the internal architecture before cutting `0.1.0`.

### Security

- **Redaction**: `Authorization`, `Cookie`, `X-CSRF-Token`, and SAP password values are scrubbed from the HTTP trace inspector, including body-preview echo defense (a server reflecting a header value back in the response body no longer leaks the secret). 13 redaction tests across two layers.
- **CSP tightened**: `script-src 'self'` (no `'unsafe-inline'`); `withGlobalTauri: false`; the Tauri API is loaded as a vendored ES module from `tauri-app/src/vendor/tauri-core.js`. IPC capability scoped to the `main` window only.
- **Central XSS sanitiser**: `safeHtml` tagged-template helper auto-escapes every `${...}` interpolation; `raw()` opt-out is keyed by a closure-private `Symbol` so JSON-shaped payloads can't forge a marker. Every renderer in the desktop app builds DOM strings through `safeHtml` internally; CI runs `scripts/lint-innerhtml.mjs` to fail on bare-template-literal `innerHTML` assignments across `tauri-app/src/**/*.js` + `index.html`.
- **Fail-closed keyring writes** with explicit user confirmation before plaintext fallback (signalled via `KEYRING_FAILED:` prefix from the backend).

### Tests

- New integration-test corpus (131 tests total) covering V4 CSDL annotations, V2 EDMX `sap:*` attributes, CSRF roundtrip, browser-SSO redirect detection, error-envelope edge cases, malformed XML, composite keys, and a synthetic 1500-entity perf-regression test (#[ignore]'d by default).

### Internal

- `crates/core/src/metadata.rs` (3926 LOC) split into `metadata/{mod,model,annotations}.rs`.
- `crates/core/src/lint.rs` (1143 LOC) split into `lint/{mod,types,profile}.rs`.
- `tauri-app/src/app.js` (3835 LOC) split into 21 focused ES modules. Entry file is now wiring-only (~467 LOC). Zero upward imports — every cross-module reference flows downward or is sibling-pair.

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
