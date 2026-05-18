// URL sanitization for offline-mode attribution.
//
// Path-A captures record the full `$metadata` URL the bytes were fetched
// from. That URL is shown to the user inside the tool — they want to know
// "where did this come from." But it also lands in `connections.toml`,
// which means:
//
// 1. It will be in any config backup the user makes.
// 2. It will be in any offline-pack the user later exports to share with
//    another consultant.
//
// `userinfo` (the `user:pass@host` portion of an HTTP URL) is the one piece
// that's never legitimate to persist — even for local-only use. If a user
// pasted a URL with embedded credentials into the connection form, we
// shouldn't quietly bake those credentials into the offline library.
//
// `strip_userinfo` is deliberately tolerant: if `Url::parse` accepts the
// input we use its setter API; otherwise we fall back to a byte-level scan
// for `scheme://userinfo@` and strip what we recognize. The contract is
// "credentials don't get persisted via this field" — the security floor.
// Inputs we can't recognize at all (no `scheme://` token, no `@` in the
// authority) pass through unchanged because they're indistinguishable from
// legitimate free-text values.

use url::Url;

/// Remove any `user:pass@` portion of a URL.
///
/// - If the URL parses and has userinfo, returns the serialized form with
///   username and password cleared.
/// - If the URL parses but has no userinfo, returns the serialized form
///   (which normalizes some forms but is otherwise equivalent).
/// - If the URL fails to parse, falls back to a conservative byte-level
///   strip of `scheme://userinfo@`, so credentials in invalid-but-
///   recognizable URLs (`https://u:p@bad host/path`) don't survive.
///
/// Some URL schemes (`mailto:`, `data:`, etc.) cannot host userinfo at all;
/// the `set_username` / `set_password` calls on those schemes return
/// `Err(())`, which we treat as "nothing to strip" and fall through to the
/// serialized form.
pub fn strip_userinfo(raw: &str) -> String {
    if let Ok(mut parsed) = Url::parse(raw) {
        // set_username / set_password return Err(()) for cannot-be-a-base URLs;
        // ignore — there's no userinfo to strip in that case.
        let _ = parsed.set_username("");
        let _ = parsed.set_password(None);
        return parsed.into();
    }
    fallback_strip_userinfo(raw)
}

/// Best-effort credential strip for inputs that don't pass `Url::parse`.
///
/// Looks for the `scheme://` token; if found, slices off the authority
/// section (everything up to the first `/`, `?`, or `#`), and within that
/// authority finds the **last** `@`. Everything before that last `@` is
/// userinfo and gets excised.
///
/// **Why the *last* `@`, not the first:** RFC 3986 forbids unencoded `@`
/// inside userinfo, so a well-formed URL has at most one. But this
/// fallback only fires when `Url::parse` rejected the input — i.e., the
/// well-formedness assumption is exactly what's broken. A malformed input
/// like `https://u:p@ss@bad host/path` contains two `@`s; the
/// userinfo/host boundary is at the **last** one (WHATWG URL parsing
/// convention). Using the first `@` would leave `ss@bad host/path` in the
/// output, which still leaks `ss` and looks like another userinfo+host
/// pair.
///
/// This is intentionally narrow: it must not corrupt strings that contain
/// `@` for legitimate reasons (`user@example.com` plain text, an `@` in a
/// query parameter, etc.). The contract is "credentials don't get
/// persisted via this field" — the security floor — not "make every URL
/// pretty."
fn fallback_strip_userinfo(raw: &str) -> String {
    let Some(scheme_end) = raw.find("://") else {
        return raw.to_string();
    };
    let authority_start = scheme_end + 3;
    let after_scheme = &raw[authority_start..];

    // Slice the authority section: everything up to the first `/`, `?`,
    // or `#` (or the whole remainder if no terminator present).
    let authority_terminator = after_scheme.find(['/', '?', '#']);
    let authority = match authority_terminator {
        Some(t) => &after_scheme[..t],
        None => after_scheme,
    };

    // Find the *last* `@` inside the authority. If none, no userinfo to
    // strip — the `@` (if any) is in the path/query/fragment.
    let Some(last_at_in_authority) = authority.rfind('@') else {
        return raw.to_string();
    };

    let prefix = &raw[..authority_start];
    // last_at_in_authority is an index into `authority`, which starts at
    // `after_scheme[0]`, so it's also a valid index into `after_scheme`.
    let suffix = &after_scheme[last_at_in_authority + 1..];
    format!("{prefix}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_userinfo_with_password() {
        let out = strip_userinfo("https://alice:secret@sap.example.com/path?x=1");
        assert!(!out.contains("alice"), "username leaked through: {out}");
        assert!(!out.contains("secret"), "password leaked through: {out}");
        assert!(out.starts_with("https://sap.example.com/"));
        assert!(out.contains("?x=1"));
    }

    #[test]
    fn strips_userinfo_username_only() {
        let out = strip_userinfo("https://alice@sap.example.com/path");
        assert!(!out.contains("alice"));
        assert_eq!(out, "https://sap.example.com/path");
    }

    #[test]
    fn passthrough_no_userinfo() {
        let url = "https://sap.example.com/sap/opu/odata/sap/UI_SVC/$metadata?sap-client=100";
        // Round-trip through Url::parse normalizes some forms but should
        // leave a userinfo-free SAP URL byte-equivalent.
        let out = strip_userinfo(url);
        assert_eq!(out, url);
    }

    #[test]
    fn passthrough_unparseable() {
        // Not a valid URL — `strip_userinfo` is best-effort, must not panic
        // and must not corrupt the input.
        let weird = "not a url at all";
        assert_eq!(strip_userinfo(weird), weird);
    }

    // ── Fallback path: Url::parse failures with credentials present ──
    //
    // The security contract is "credentials don't get persisted via this
    // field." A URL like `https://u:p@bad host/path` fails strict parsing
    // (space in authority) but still leaks creds if returned verbatim.

    #[test]
    fn fallback_strips_userinfo_from_invalid_url_with_space() {
        let out = strip_userinfo("https://alice:secret@bad host/path");
        assert!(!out.contains("alice"), "username leaked: {out}");
        assert!(!out.contains("secret"), "password leaked: {out}");
        assert_eq!(out, "https://bad host/path");
    }

    #[test]
    fn fallback_strips_userinfo_with_unusual_scheme() {
        // Custom scheme that the url crate may reject — still must strip.
        let out = strip_userinfo("ftp+weird://u:p@host with space/");
        assert!(!out.contains("u:p"));
        assert_eq!(out, "ftp+weird://host with space/");
    }

    #[test]
    fn fallback_preserves_plain_email_address() {
        // No `://` → never touched. Otherwise a free-text field containing
        // an email would get mangled.
        assert_eq!(strip_userinfo("user@example.com"), "user@example.com");
        assert_eq!(
            strip_userinfo("contact me: user@example.com"),
            "contact me: user@example.com"
        );
    }

    #[test]
    fn fallback_preserves_at_sign_in_path_or_query() {
        // Inputs where `Url::parse` may fail but the `@` belongs to the
        // path/query, not the authority. The fallback must not strip
        // anything in that case.
        let path_at = "weird-scheme://example.com/path/@v1.2.3";
        assert_eq!(strip_userinfo(path_at), path_at);

        let query_at = "weird-scheme://example.com/p?author=user@x";
        assert_eq!(strip_userinfo(query_at), query_at);

        let frag_at = "weird-scheme://example.com/p#user@frag";
        assert_eq!(strip_userinfo(frag_at), frag_at);
    }

    #[test]
    fn fallback_handles_userinfo_with_no_path() {
        // `scheme://u:p@host` with no trailing slash. The `@` precedes any
        // path delimiter so it's authority-userinfo. Strip.
        let out = strip_userinfo("weird-scheme://u:p@host");
        assert_eq!(out, "weird-scheme://host");
    }

    #[test]
    fn fallback_uses_last_at_in_authority_with_multiple_ats() {
        // Multi-`@` malformed input. RFC 3986 forbids unencoded `@` in
        // userinfo, so this input must have failed Url::parse. The
        // userinfo/host boundary is the *last* `@` in the authority.
        // Using the first `@` would leave `ss@bad host/path` — still
        // leaks `ss` as credential material.
        let out = strip_userinfo("https://u:p@ss@bad host/path");
        assert!(!out.contains("u:p"), "username:password leaked: {out}");
        assert!(!out.contains("p@ss"), "first segment leaked: {out}");
        assert!(!out.contains("ss@"), "middle segment leaked: {out}");
        assert_eq!(out, "https://bad host/path");
    }

    #[test]
    fn fallback_last_at_does_not_touch_path_at() {
        // Two `@`s but the second is in the path. Only the first one is
        // in the authority section. The "last @ in authority" rule must
        // not reach past the authority terminator.
        let out = strip_userinfo("weird-scheme://u:p@host with space/path/@v1");
        assert!(!out.contains("u:p"));
        // The path-level `@v1` must survive.
        assert!(out.contains("/path/@v1"), "path `@v1` got mangled: {out}");
        assert_eq!(out, "weird-scheme://host with space/path/@v1");
    }

    #[test]
    fn handles_port_and_path() {
        let out = strip_userinfo("https://u:p@host:8443/path");
        assert_eq!(out, "https://host:8443/path");
    }

    #[test]
    fn handles_percent_encoded_userinfo() {
        let out = strip_userinfo("https://alice%40corp:s%2Fecret@sap.example.com/");
        assert!(!out.contains("alice"));
        assert!(!out.contains("ecret"));
        assert_eq!(out, "https://sap.example.com/");
    }
}
