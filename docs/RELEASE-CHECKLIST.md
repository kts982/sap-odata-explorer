# Release checklist

Use this before publishing a GitHub release.

## Release posture

- Mark early releases as pre-release / alpha unless code signing, CI packaging, and broader auth validation are complete.
- Keep the README's responsible-use note and SAP non-affiliation note visible.
- Do not position the project as an autonomous API agent or bulk extraction tool.

## Repository hygiene

- `git status --short` should not show local artifacts, customer metadata, credentials, or generated packages.
- Keep release packages under `dist/` only as local build output; do not commit them.
- Keep scratch SAP metadata, traces, and test captures under `tmp/` only; do not commit them.
- Verify `.gitignore` still excludes `dist/`, `tmp/`, `target/`, `node_modules/`, local env files, TLS keys/certs, and `connections.toml`.

## Verification

Run from the repository root unless noted:

```bash
cargo fmt --check
cargo test --workspace
git diff --check
node scripts/lint-innerhtml.mjs
node scripts/test-safe-html.mjs
```

Run from `tauri-app/`:

```bash
npm run css
```

`cargo clippy --workspace --all-targets -- -D warnings` runs in CI as a hard gate; release builds inherit the same constraint. `scripts/test-safe-html.mjs` is local-only for now — it pins the parser-vs-renderer escaping contract; CI wiring is a follow-up.

## Desktop security

- Confirm `tauri-app/src-tauri/capabilities/default.json` scopes IPC to the `main` window only.
- Confirm browser SSO popup windows are not included in a capability.
- Confirm CSP remains locked to local assets and IPC.
- Confirm keyring failures do not silently fall back to plaintext storage.

## Artifacts

Each release **must** attach exactly these five assets so the Installation table in the README stays accurate:

| File on release | Source path in build tree | Notes |
|---|---|---|
| `SAP-OData-Explorer_<ver>_x64_en-US.msi` | `target/release/bundle/msi/` | GUI installer (MSI) — "easiest path" in the README |
| `SAP-OData-Explorer_<ver>_x64-setup.exe` | `target/release/bundle/nsis/` | GUI installer (NSIS) |
| `SAP-OData-Explorer_<ver>_portable.exe` | `target/release/sap-odata-explorer-app.exe` — **rename before upload** | GUI portable (no install); for Citrix / no-admin / customer-laptop scenarios |
| `sap-odata.exe` | `target/release/sap-odata.exe` | CLI portable |
| `SHA256SUMS.txt` | Generated locally over the four binaries above | Checksums |

- Build artifacts from a clean tree:
  ```
  cargo build --release -p sap-odata-cli
  cd tauri-app && cargo tauri build && cd ..
  node scripts/stage-release-assets.mjs --tag v<release-tag>
  ```
- `scripts/stage-release-assets.mjs` handles the portable rename and SHA256SUMS regeneration in one shot — it copies the four expected binaries (MSI, NSIS, portable GUI, CLI) into `dist/v<tag>/` with the canonical filenames the README's Installation table references, then writes `SHA256SUMS.txt` over them. The portable rename is load-bearing: Tauri's default output is `sap-odata-explorer-app.exe` (from `tauri-app/src-tauri/Cargo.toml`'s `[package].name`), but the release page expects `SAP-OData-Explorer_<ver>_portable.exe` so the asset table stays accurate.
- Confirm the resulting `dist/v<tag>/` has exactly the five files in the table above before uploading.
- Check packaged README/INSTALL docs for the same security and responsible-use language as the source README.
- Label unsigned Windows artifacts clearly in release notes.
- Do not attach archives that contain `tmp/`, local configs, traces, credentials, or customer metadata.
