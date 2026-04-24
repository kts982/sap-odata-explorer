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

const KEYRING_SERVICE: &str = "sap-odata-explorer:session";

/// A persisted browser SSO session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    /// URL that the cookies were captured from — used as cookie jar scope.
    pub request_url: String,
    /// Raw `Set-Cookie`-style strings, ready to feed to a reqwest cookie jar.
    pub cookies: Vec<String>,
    /// Timestamp when persisted (unix seconds). Useful for diagnostics.
    #[serde(default)]
    pub saved_at: u64,
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
    cookies: &[String],
) -> anyhow::Result<()> {
    let session = PersistedSession {
        request_url: request_url.to_string(),
        cookies: cookies.to_vec(),
        saved_at: PersistedSession::now(),
    };

    let json = serde_json::to_vec(&session)?;
    let compressed = gzip_compress(&json)?;
    // Base64-encode so we can use set_password (many backends prefer UTF-8
    // strings). Overhead is ~33% but keeps compatibility across platforms.
    let encoded = base64::engine::general_purpose::STANDARD.encode(&compressed);

    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    entry
        .set_password(&encoded)
        .map_err(|e| anyhow::anyhow!("failed to store session: {e}"))?;

    tracing::debug!(
        "Session persisted for profile '{}' ({} cookies, {} bytes compressed)",
        profile_name,
        cookies.len(),
        compressed.len()
    );
    Ok(())
}

/// Load cookies for a profile from the OS keyring. Returns `None` if nothing stored.
pub fn load(profile_name: &str) -> anyhow::Result<Option<PersistedSession>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    let encoded = match entry.get_password() {
        Ok(s) => s,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(e) => return Err(anyhow::anyhow!("failed to load session: {e}")),
    };

    let compressed = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .map_err(|e| anyhow::anyhow!("corrupt session blob: {e}"))?;
    let json = gzip_decompress(&compressed)?;
    let session: PersistedSession = serde_json::from_slice(&json)
        .map_err(|e| anyhow::anyhow!("corrupt session data: {e}"))?;

    Ok(Some(session))
}

/// Delete the persisted session for a profile. Safe to call even if nothing is stored.
pub fn clear(profile_name: &str) -> anyhow::Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, profile_name)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()), // already absent, treat as success
        Err(e) => Err(anyhow::anyhow!("failed to clear session: {e}")),
    }
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
}
