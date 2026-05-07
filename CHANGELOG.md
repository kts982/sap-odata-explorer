# Changelog

All notable changes to this project are documented here. Format loosely follows [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

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
