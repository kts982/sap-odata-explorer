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
```

Run from `tauri-app/`:

```bash
npm run css
```

`cargo clippy --workspace --all-targets -- -D warnings` runs in CI as a hard gate; release builds inherit the same constraint.

## Desktop security

- Confirm `tauri-app/src-tauri/capabilities/default.json` scopes IPC to the `main` window only.
- Confirm browser SSO popup windows are not included in a capability.
- Confirm CSP remains locked to local assets and IPC.
- Confirm keyring failures do not silently fall back to plaintext storage.

## Artifacts

- Build artifacts from a clean tree.
- Check packaged README/INSTALL docs for the same security and responsible-use language as the source README.
- Label unsigned Windows artifacts clearly in release notes.
- Publish checksums for attached binaries/archives.
- Do not attach archives that contain `tmp/`, local configs, traces, credentials, or customer metadata.
