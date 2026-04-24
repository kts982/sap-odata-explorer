# Security Policy

## Supported versions

This project is in alpha. Security fixes are applied to `main` and to the most recent release only.

| Version  | Supported |
| -------- | --------- |
| 0.1.x    | ✅        |
| < 0.1    | ❌        |

## Reporting a vulnerability

If you believe you have found a security issue in `sap-odata-explorer`, please **do not open a public GitHub issue**. Public disclosure before a fix is available puts users at risk.

Use one of these paths instead:

- **Open a private [GitHub Security Advisory](../../security/advisories/new)** for this repository. This is the preferred channel.
- **Contact the maintainer directly** if you already have a private contact channel.

I will try to acknowledge valid reports quickly and coordinate a fix before public disclosure when practical. This project runs `cargo audit` in CI against the committed `Cargo.lock`; if you find an advisory that CI missed, I'd like to hear about it.
