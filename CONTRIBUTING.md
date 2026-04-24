# Contributing

Thanks for your interest! This is an early-stage project — issues, discussions, and PRs are welcome.

## Development setup

**Prerequisites:**
- Rust 1.85+ ([rustup.rs](https://rustup.rs))
- Node.js 18+ (for Tauri app frontend build)
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
cargo test
```

## Project structure

```
sap-odata-explorer/
├── crates/
│   ├── core/       # Shared library (protocol, auth, HTTP, metadata, query, catalog, config)
│   └── cli/        # CLI binary (clap)
└── tauri-app/
    ├── src/        # Frontend (vanilla HTML + Tailwind CSS + JS)
    └── src-tauri/  # Tauri backend (Rust commands wrapping core)
```

All business logic lives in `crates/core`. The CLI and Tauri app are thin shells — add new capabilities in core first.

## Making changes

1. Fork and create a feature branch
2. Run `cargo test` and make sure all tests pass
3. For frontend changes: rebuild CSS with `cd tauri-app && npm run css`
4. For Tauri changes: test the app with `cargo tauri build`
5. Keep commits focused — one concern per commit
6. Open a PR with a clear description

### Code style

- **Rust**: run `cargo fmt` before committing
- **JS**: no framework, no TypeScript. Keep it simple.
- **HTML/CSS**: Tailwind utility classes, no inline event handlers (CSP forbids them)

### No inline event handlers

The Tauri app runs under a strict CSP that blocks `onclick="..."` in HTML. Use `addEventListener` in `app.js` or `data-action` attributes with the document-level event delegation handler.

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
