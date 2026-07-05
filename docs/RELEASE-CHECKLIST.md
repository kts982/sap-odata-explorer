# Release checklist

Use this before publishing a GitHub release.

## Release posture

- From v0.1.0 on, releases ship as normal (non-prerelease) GitHub releases with the unsigned status documented in the release notes and README. Code signing remains a forward item, not a release gate. Use the pre-release flag only for genuinely unstable interface previews.
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
- Submit the four binaries to VirusTotal before publishing. Unsigned Windows binaries normally pick up 1-3 single-vendor ML/reputation hits; the rule to ship on is **Microsoft Defender clean across all artifacts**. Include per-asset SHA256 + VT URLs in a `## Verification` section of the release notes so users can re-check independently — especially helpful for the locked-down-environment audience downloading the portable exe.
- If VirusTotal's *Microsoft* engine flags a binary, verify with **live Defender** before blocking the release: `MpCmdRun.exe -Scan -ScanType 3 -File <exe>` (exit 0 = clean). VT runs Defender in a more aggressive configuration than the shipping product; its `!ml` hits are config-driven and do not clear on rescan. Live-clean means proceed.

## Distribution channels (after the GitHub release is live)

Run in this order — every channel below references the published release assets.

1. **crates.io** — `cargo publish -p sap-odata-core`, then `cargo publish -p sap-odata-cli` (retry after ~1 min if the CLI publish can't resolve the core version yet). Requires a `cargo login` session with a token scoped to `publish-new`/`publish-update`. Published versions are immutable (yank-only) — treat the publish as part of the release, not a draft.
2. **Scoop** — in [`kts982/scoop-bucket`](https://github.com/kts982/scoop-bucket): bump `version`, `url`, and `hash` in `bucket/sap-odata.json` (CLI) and `bucket/sap-odata-explorer.json` (portable GUI). Hashes come from the release's `SHA256SUMS.txt`; the manifests' `checkver`/`autoupdate` blocks template the URLs so this is mostly mechanical.
3. **winget** — manifests live under `packaging/winget/manifests/k/kts982/SAPODataExplorer/<ver>/` (schema 1.12.0, three YAML files). `winget validate` the directory, then `wingetcreate submit --prtitle "Add version: kts982.SAPODataExplorer version <ver>"` (first submission: "New package: …"). The wingetcreate GitHub token is cached machine-wide. wingetbot validation usually runs within minutes; reviewer approval takes hours. A `/AzurePipelines run` PR comment re-triggers flaky single-vendor AV hits.
