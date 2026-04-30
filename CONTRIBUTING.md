# Contributing

Thanks for your interest! This is an early-stage project — issues, discussions, and PRs are welcome.

## Development setup

**Prerequisites:**
- Rust 1.85+ ([rustup.rs](https://rustup.rs))
- Node.js 20+ (for Tauri app frontend build; Tailwind CSS v4 requires it)
- Windows, Linux, or macOS

**Clone and build:**

```bash
git clone https://github.com/kts982/sap-odata-explorer.git
cd sap-odata-explorer

# CLI
cargo build --release
# Binary at target/release/sap-odata.exe (or sap-odata on Linux/macOS)

# Desktop app
cd tauri-app
npm install
cargo tauri build
# Binary at ../target/release/sap-odata-explorer-app.exe
```

**Run tests:**

```bash
cargo fmt --check
cargo test --workspace

# Frontend CSS build
cd tauri-app
npm run css
```

## Project structure

```
sap-odata-explorer/
├── crates/
│   ├── core/       # Shared library (protocol, auth, HTTP, metadata, query, catalog, config)
│   └── cli/        # CLI binary (clap)
└── tauri-app/
    ├── src/        # Frontend — vanilla HTML + Tailwind CSS v4 + ES modules
    └── src-tauri/  # Tauri backend (Rust commands wrapping core)
```

All business logic lives in `crates/core`. The CLI and Tauri app are thin shells — add new capabilities in core first.

The frontend is split into focused ES modules — `app.js` is wiring only (DOM ready, button bindings, click delegate). Feature logic lives in dedicated modules: `state.js`, `tabs.js`, `auth.js`, `services.js`, `query.js`, `executor.js`, `results.js`, `trace.js`, `describe.js`, `annotations.js`, `history.js`, `valueList.js`, `fiori.js`, `favorites.js`, `addProfile.js`, plus utility modules (`html.js`, `format.js`, `status.js`, `clipboard.js`, `api.js`, `resultCache.js`). All imports flow downward — no module imports back up to `app.js`.

## Making changes

1. Fork and create a feature branch
2. Run `cargo fmt --check`, `cargo test --workspace`, and `git diff --check`
3. For frontend changes: rebuild CSS with `cd tauri-app && npm run css`
4. For Tauri changes: test the app with `cargo tauri build`
5. Keep commits focused — one concern per commit
6. Open a PR with a clear description

### Code style

- **Rust**: run `cargo fmt` before committing
- **JS**: ES modules, no framework, no TypeScript, no bundler (vendored Tauri API loaded directly). Keep it simple. Add new feature logic in a dedicated module — `app.js` is the entry/wiring file and stays small.
- **HTML/CSS**: Tailwind utility classes, no inline event handlers (CSP forbids them)
- **Clippy**: `cargo clippy --workspace --all-targets -- -D warnings` is the desired long-term gate, but the current codebase still has existing style-only lints. Do not introduce new clippy warnings in touched code.

### Release hygiene

- Do not commit local release builds, scratch metadata, traces, customer captures, or credentials.
- Keep release packages under `dist/` and scratch SAP metadata under `tmp/`; both are ignored.
- Do not commit `connections.toml`, `.env` files, TLS private keys, or customer certificates.
- Keep public docs clear that this project is independent from SAP and intended for authorized, human-driven exploration of documented or customer-owned OData services.
- See [docs/RELEASE-CHECKLIST.md](docs/RELEASE-CHECKLIST.md) before publishing a GitHub release.

### No inline event handlers

The Tauri app's CSP sets `script-src 'self'` (no `'unsafe-inline'`), which blocks inline scripts and inline event-handler attributes such as `onclick="..."` from executing. Use `addEventListener` in the appropriate module, or `data-action` attributes routed through the document-level event-delegation handler.

### Always escape `innerHTML` interpolations

SAP `$metadata` is untrusted input — entity names, annotation values, and error messages can contain HTML special characters. Any HTML string assigned to `.innerHTML` must escape every interpolation. Use the `safeHtml` tagged-template helper from `tauri-app/src/html.js`:

```js
el.innerHTML = safeHtml`<div title="${title}">${name}</div>`;       // both auto-escaped
el.innerHTML = safeHtml`<table>${raw(prebuiltSafeRows)}</table>`;   // opt-out via raw()
el.innerHTML = '<div class="foo">static</div>';                     // plain string OK
```

Bare template literals (`el.innerHTML = \`...${x}...\``) are forbidden — `scripts/lint-innerhtml.mjs` runs in CI and fails the build on any matching pattern across `tauri-app/src/**/*.js` and `index.html`.

If a renderer builds HTML in a variable before assignment, build each dynamic fragment with `safeHtml` and join only those safe fragments. Do not build `let html = \`...\`` or append unescaped interpolated template literals before assigning `el.innerHTML = html`.

## What's welcome

- Bug fixes
- Better SAP error handling (edge cases, auth flows, exotic metadata)
- Integration tests with mock SAP responses
- Documentation improvements
- Small UX polish in the Tauri app
- Cross-platform testing (Linux/macOS builds)

## What needs discussion first

Open an issue before starting work on:

- New authentication methods (beyond basic, SPNEGO, browser SSO)
- Write operations (POST/PATCH/DELETE)
- MCP server integration
- Major UI redesigns
- Breaking changes to the `sap-odata-core` public API

## Reporting issues

When reporting a bug, please include:

- Your OS and version
- Rust/Node versions
- The exact command or workflow
- The error message or unexpected behavior
- If possible: a redacted `$metadata` fragment and the query being run

## Security

If you find a security issue, please **do not open a public issue**. Email the maintainer instead.

## License

By contributing, you agree that your contributions will be licensed under the MIT License (see [LICENSE](LICENSE)).
