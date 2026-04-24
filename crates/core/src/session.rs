//! Persist browser SSO session cookies in the OS keyring.
//!
//! Cookies are serialized to JSON, gzip-compressed, then stored as a byte
//! secret under a keyring entry keyed by profile name. On Windows, the
//! Credential Manager blob limit is ~2.5KB — compression keeps typical
//! SAP + Azure AD cookie sets well under that.

use std::io::{Read, Write};

use base64::Engine;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Serialize};

#[cfg_attr(test, allow(dead_code))]
const KEYRING_SERVICE: &str = "sap-odata-explorer:session";

// ── keyring backend ──
//
// Production uses the OS credential store via the `keyring` crate. Tests use
// an in-memory HashMap so we can exercise the full save → load → clear path
// without touching the user's real OS keyring. The mock provided by the
// `keyring` crate itself doesn't work here because it has no persistence
// across separate `Entry::new()` calls, which is how this module talks to the
// keyring.

#[cfg(not(test))]
fn kv_set(profile_name: &str, encoded: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    entry
        .set_password(encoded)
        .map_err(|e| anyhow::anyhow!("failed to store session: {e}"))?;
    Ok(())
}

#[cfg(not(test))]
fn kv_get(profile_name: &str) -> anyhow::Result<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("failed to load session: {e}")),
    }
}

#[cfg(not(test))]
fn kv_del(profile_name: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("failed to clear session: {e}")),
    }
}

#[cfg(test)]
mod test_backend {
    use std::collections::HashMap;
    use std::sync::Mutex;

    static STORE: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

    fn with_store<R>(f: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
        let mut guard = STORE.lock().unwrap();
        f(guard.get_or_insert_with(HashMap::new))
    }

    pub fn set(key: &str, value: &str) -> anyhow::Result<()> {
        with_store(|m| {
            m.insert(key.to_string(), value.to_string());
        });
        Ok(())
    }

    pub fn get(key: &str) -> anyhow::Result<Option<String>> {
        Ok(with_store(|m| m.get(key).cloned()))
    }

    pub fn del(key: &str) -> anyhow::Result<()> {
        with_store(|m| {
            m.remove(key);
        });
        Ok(())
    }
}

#[cfg(test)]
fn kv_set(profile_name: &str, encoded: &str) -> anyhow::Result<()> {
    test_backend::set(profile_name, encoded)
}

#[cfg(test)]
fn kv_get(profile_name: &str) -> anyhow::Result<Option<String>> {
    test_backend::get(profile_name)
}

#[cfg(test)]
fn kv_del(profile_name: &str) -> anyhow::Result<()> {
    test_backend::del(profile_name)
}

/// A persisted browser SSO session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    /// URL that the cookies were captured from — used as cookie jar scope.
    pub request_url: String,
    /// Fingerprint of the connection (base_url|client|language). Used to detect
    /// when a profile has been edited to point at a different system, so we
    /// don't replay stale cookies to the wrong target.
    #[serde(default)]
    pub connection_fingerprint: String,
    /// Raw `Set-Cookie`-style strings, ready to feed to a reqwest cookie jar.
    pub cookies: Vec<String>,
    /// Timestamp when persisted (unix seconds). Useful for diagnostics.
    #[serde(default)]
    pub saved_at: u64,
}

/// Host substrings we recognize as federated IdPs. Used across auth flows
/// — sign-in completion detection, sign-out cookie sweep, and expired-session
/// redirect detection — so all three stay consistent.
///
/// These are substring matches: both `login.microsoftonline.com` and
/// `login.microsoftonline.com/<tenant-id>` match `microsoftonline.com`.
const IDP_HOST_PATTERNS: &[&str] = &[
    "microsoftonline.com", // Azure AD (modern login)
    "login.microsoft.com", // Azure AD (legacy/entra)
    "login.windows.net",   // Azure AD (legacy ADFS-integrated)
    "ondemand.com",        // SAP BTP, SAP IAS on cloud
    "okta.com",
    "auth0.com",
    "accounts.sap.com", // SAP IAS
    "adfs",             // on-premise ADFS (matches e.g. adfs.corp.example)
];

/// Returns true if `host` looks like a federated IdP that could silently
/// re-authenticate a user if its cookies aren't cleared on sign-out, and
/// whose presence in a redirect chain implies sign-in is required.
pub fn is_idp_host(host: &str) -> bool {
    IDP_HOST_PATTERNS.iter().any(|p| host.contains(p))
}

/// Returns true if a `Location` header value indicates an IdP redirect.
/// Covers both host-based detection and SAML path markers like `/saml2/`.
pub fn is_idp_redirect_location(location: &str) -> bool {
    if location.contains("/saml2/") || location.contains("/oauth2/") {
        return true;
    }
    // Extract host portion from a URL-ish string for substring matching.
    is_idp_host(location)
}

/// Compute a deterministic fingerprint for a connection definition.
pub fn connection_fingerprint(base_url: &str, client: &str, language: &str) -> String {
    format!(
        "{}|{}|{}",
        base_url.trim_end_matches('/').to_lowercase(),
        client.trim(),
        language.trim().to_uppercase()
    )
}

impl PersistedSession {
    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Save cookies for a profile to the OS keyring.
pub fn save(
    profile_name: &str,
    request_url: &str,
    connection_fingerprint: &str,
    cookies: &[String],
) -> anyhow::Result<()> {
    let session = PersistedSession {
        request_url: request_url.to_string(),
        connection_fingerprint: connection_fingerprint.to_string(),
        cookies: cookies.to_vec(),
        saved_at: PersistedSession::now(),
    };

    let json = serde_json::to_vec(&session)?;
    let compressed = gzip_compress(&json)?;
    // Base64-encode so we can use set_password (many backends prefer UTF-8
    // strings). Overhead is ~33% but keeps compatibility across platforms.
    let encoded = base64::engine::general_purpose::STANDARD.encode(&compressed);

    kv_set(profile_name, &encoded)?;

    tracing::debug!(
        "Session persisted for profile '{}' ({} cookies, {} bytes compressed)",
        profile_name,
        cookies.len(),
        compressed.len()
    );
    Ok(())
}

/// Load cookies for a profile from the OS keyring. Returns `None` if nothing stored.
/// Does NOT validate the connection fingerprint — callers should prefer
/// `load_for_connection` to prevent replaying cookies to the wrong SAP system.
pub fn load(profile_name: &str) -> anyhow::Result<Option<PersistedSession>> {
    let encoded = match kv_get(profile_name)? {
        Some(s) => s,
        None => return Ok(None),
    };

    let compressed = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .map_err(|e| anyhow::anyhow!("corrupt session blob: {e}"))?;
    let json = gzip_decompress(&compressed)?;
    let session: PersistedSession =
        serde_json::from_slice(&json).map_err(|e| anyhow::anyhow!("corrupt session data: {e}"))?;

    Ok(Some(session))
}

/// Load cookies for a profile, but only if the stored connection fingerprint
/// matches the current one. Stale sessions (from a profile that's been edited
/// to point at a different system) are automatically cleared and `None` is
/// returned — forcing a fresh sign-in.
pub fn load_for_connection(
    profile_name: &str,
    expected_fingerprint: &str,
) -> anyhow::Result<Option<PersistedSession>> {
    let session = match load(profile_name)? {
        Some(s) => s,
        None => return Ok(None),
    };

    // Legacy sessions (pre-fingerprint) have empty fingerprint — treat as stale.
    if session.connection_fingerprint.is_empty()
        || session.connection_fingerprint != expected_fingerprint
    {
        tracing::debug!(
            "Discarding stale session for '{}' (fingerprint mismatch)",
            profile_name
        );
        let _ = clear(profile_name);
        return Ok(None);
    }

    Ok(Some(session))
}

/// Delete the persisted session for a profile. Safe to call even if nothing is stored.
pub fn clear(profile_name: &str) -> anyhow::Result<()> {
    kv_del(profile_name)
}

// ── gzip helpers ──

fn gzip_compress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn gzip_decompress(data: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_compress_decompress() {
        let data = b"some cookie data like MYSAPSSO2=abc; Path=/; Domain=example.com";
        let compressed = gzip_compress(data).unwrap();
        let decompressed = gzip_decompress(&compressed).unwrap();
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn fingerprint_is_normalized() {
        // Trailing slash, URL case, and language case must not matter.
        let a = connection_fingerprint("https://sap.corp:8000/", "100", "en");
        let b = connection_fingerprint("https://SAP.corp:8000", "100", "EN");
        assert_eq!(a, b);
    }

    #[test]
    fn fingerprint_differs_across_connections() {
        let base = connection_fingerprint("https://sap.corp", "100", "EN");
        assert_ne!(
            base,
            connection_fingerprint("https://other.corp", "100", "EN")
        );
        assert_ne!(
            base,
            connection_fingerprint("https://sap.corp", "200", "EN")
        );
        assert_ne!(
            base,
            connection_fingerprint("https://sap.corp", "100", "DE")
        );
    }

    #[test]
    fn is_idp_host_matches_known_idps() {
        assert!(is_idp_host("login.microsoftonline.com"));
        assert!(is_idp_host("login.microsoftonline.com/tenant-id"));
        assert!(is_idp_host("login.microsoft.com"));
        assert!(is_idp_host("login.windows.net"));
        assert!(is_idp_host("foo.okta.com"));
        assert!(is_idp_host("example.auth0.com"));
        assert!(is_idp_host("accounts.sap.com"));
        assert!(is_idp_host("iam.ondemand.com"));
        assert!(is_idp_host("adfs.corp.example"));
        assert!(!is_idp_host("sap.corp"));
        assert!(!is_idp_host("example.com"));
    }

    #[test]
    fn is_idp_redirect_location_catches_saml_and_oauth_paths() {
        assert!(is_idp_redirect_location("/sap/saml2/sso"));
        assert!(is_idp_redirect_location("/oauth2/authorize"));
        assert!(is_idp_redirect_location(
            "https://login.microsoftonline.com/common/oauth2/authorize"
        ));
        assert!(is_idp_redirect_location("https://accounts.sap.com/saml/"));
        assert!(!is_idp_redirect_location("https://sap.corp/sap/opu/odata/"));
    }

    #[test]
    fn load_for_connection_returns_session_when_fingerprint_matches() {
        let profile = "load_matches";
        let fp = connection_fingerprint("https://sap.corp", "100", "EN");
        save(profile, "https://sap.corp/", &fp, &["X=1".to_string()]).unwrap();

        let loaded = load_for_connection(profile, &fp).unwrap();
        assert!(loaded.is_some(), "matching fingerprint must return session");
        assert_eq!(loaded.unwrap().cookies, vec!["X=1".to_string()]);
        // Match must not clear the entry.
        assert!(load(profile).unwrap().is_some());
    }

    #[test]
    fn load_for_connection_clears_on_fingerprint_mismatch() {
        let profile = "load_mismatch";
        let stored_fp = connection_fingerprint("https://sap.corp", "100", "EN");
        save(
            profile,
            "https://sap.corp/",
            &stored_fp,
            &["X=1".to_string()],
        )
        .unwrap();

        let expected_fp = connection_fingerprint("https://other.corp", "100", "EN");
        let loaded = load_for_connection(profile, &expected_fp).unwrap();
        assert!(loaded.is_none(), "mismatch must yield None");

        // Critical side effect: stale entry must have been cleared from the
        // keyring, not left behind for future load() calls to resurrect.
        assert!(
            load(profile).unwrap().is_none(),
            "stale session must be cleared on mismatch"
        );
    }

    #[test]
    fn load_for_connection_clears_legacy_empty_fingerprint() {
        let profile = "load_legacy";
        // Simulate a pre-fingerprint session by writing an empty fingerprint.
        save(profile, "https://sap.corp/", "", &["X=1".to_string()]).unwrap();

        let expected_fp = connection_fingerprint("https://sap.corp", "100", "EN");
        let loaded = load_for_connection(profile, &expected_fp).unwrap();
        assert!(
            loaded.is_none(),
            "empty fingerprint must be treated as stale"
        );
        assert!(
            load(profile).unwrap().is_none(),
            "legacy session must be cleared on load"
        );
    }

    #[test]
    fn load_for_connection_returns_none_when_absent() {
        let profile = "load_absent";
        let fp = connection_fingerprint("https://sap.corp", "100", "EN");
        assert!(load_for_connection(profile, &fp).unwrap().is_none());
    }

    #[test]
    fn clear_is_idempotent() {
        let profile = "clear_idempotent";
        // Second call must also succeed (NoEntry treated as success).
        clear(profile).expect("clear on absent entry must not error");
        clear(profile).expect("clear must still succeed when nothing is stored");
    }

    #[test]
    fn save_then_load_roundtrips_cookies_and_fingerprint() {
        let profile = "save_load_rt";
        let fp = connection_fingerprint("https://sap.corp", "200", "DE");
        let cookies = vec![
            "MYSAPSSO2=abc; Path=/".to_string(),
            "sap-XSRF-1234=xyz; Path=/; Secure".to_string(),
        ];
        save(profile, "https://sap.corp/", &fp, &cookies).unwrap();
        let got = load(profile).unwrap().expect("session must be present");
        assert_eq!(got.connection_fingerprint, fp);
        assert_eq!(got.cookies, cookies);
        assert_eq!(got.request_url, "https://sap.corp/");
    }
}
