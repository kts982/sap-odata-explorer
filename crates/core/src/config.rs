use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, warn};

use crate::auth::{AuthConfig, SapConnection};

/// Why a keyring read failed. Distinguishes "the user can fix this by
/// unlocking the OS credential store" from "the credential blob is broken"
/// from "something else went wrong" — the catch-all `Backend` is honest
/// about cases the underlying crate can't classify cleanly.
///
/// `NoEntry` is *not* an error in this enum: missing entries surface as
/// `Ok(None)` from `try_get_password_from_keyring` so the call site can
/// keep the existing "no password found" guidance for fresh profiles.
#[derive(Debug, Error)]
pub enum KeyringReadError {
    /// The OS credential store is reachable but refused access — typically
    /// a locked Windows Credential Manager / macOS Keychain, or a Linux
    /// session without a running secret service. User-actionable: unlock
    /// or sign in to the credential store.
    #[error("keyring is locked or access was denied: {0}")]
    Locked(String),
    /// The stored credential blob exists but is corrupt (not valid UTF-8).
    /// Re-saving the password will overwrite it.
    #[error("stored credential is corrupt: {0}")]
    Corrupt(String),
    /// Catch-all for platform-runtime errors and credential-attribute
    /// validation failures (TooLong / Invalid / Ambiguous). The underlying
    /// message is preserved for diagnostics.
    #[error("keyring backend failure: {0}")]
    Backend(String),
}

fn classify_keyring_error(err: keyring::Error) -> KeyringReadError {
    match err {
        keyring::Error::NoStorageAccess(inner) => KeyringReadError::Locked(inner.to_string()),
        keyring::Error::BadEncoding(_) => {
            KeyringReadError::Corrupt("stored bytes are not valid UTF-8".to_string())
        }
        // PlatformFailure, TooLong, Invalid, Ambiguous, and any future
        // variants fold into Backend with the crate's own Display string
        // so operators still see the underlying reason in trace output.
        other => KeyringReadError::Backend(other.to_string()),
    }
}

const CONFIG_FILENAME: &str = "connections.toml";
const KEYRING_SERVICE: &str = "sap-odata-explorer";

/// A named connection profile stored in connections.toml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub base_url: String,
    #[serde(default = "default_client")]
    pub client: String,
    #[serde(default = "default_language")]
    pub language: String,
    /// Username (empty for SSO profiles).
    #[serde(default)]
    pub username: String,
    /// Password stored in plaintext (not recommended).
    /// If absent, the tool will try the OS keyring. Not used for SSO.
    pub password: Option<String>,
    /// Use Windows SSO (SPNEGO/Kerberos) instead of basic auth.
    #[serde(default)]
    pub sso: bool,
    /// Use browser-based SSO (Azure AD / SAP IAS / SAML).
    #[serde(default)]
    pub browser_sso: bool,
    /// Disable TLS certificate verification (for self-signed certs). Default: false.
    #[serde(default)]
    pub insecure_tls: bool,
    /// Opt-in to Kerberos delegation when `sso = true`. The SAP server can then
    /// authenticate as the user to further backends (constrained delegation).
    /// Only enable for landscapes that actually need multi-hop auth.
    #[serde(default)]
    pub sso_delegate: bool,
    /// Service path aliases: short name → full OData service path.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

fn default_client() -> String {
    "100".to_string()
}

fn default_language() -> String {
    "EN".to_string()
}

/// Top-level config file structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigFile {
    #[serde(default)]
    pub connections: BTreeMap<String, ConnectionProfile>,
}

/// Resolved config directory location.
#[derive(Debug)]
pub struct ConfigDir {
    pub path: PathBuf,
    pub is_portable: bool,
}

/// Determine the config directory. Portable-first:
/// 1. Look for connections.toml next to the executable
/// 2. Fall back to OS-standard config dir (~/.config/sap-odata-explorer or AppData)
pub fn find_config_dir() -> Option<ConfigDir> {
    // Try portable: next to the executable
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let portable_config = exe_dir.join(CONFIG_FILENAME);
        if portable_config.exists() {
            debug!("Using portable config at {}", exe_dir.display());
            return Some(ConfigDir {
                path: exe_dir.to_path_buf(),
                is_portable: true,
            });
        }
    }

    // Fall back to OS config directory
    if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "sap-odata-explorer") {
        let config_dir = proj_dirs.config_dir().to_path_buf();
        debug!("Using config dir at {}", config_dir.display());
        return Some(ConfigDir {
            path: config_dir,
            is_portable: false,
        });
    }

    None
}

/// Get the config directory, creating it if it doesn't exist.
/// If portable config exists next to exe, use that. Otherwise use OS config dir.
pub fn get_or_create_config_dir() -> anyhow::Result<ConfigDir> {
    // Check portable first
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let portable_config = exe_dir.join(CONFIG_FILENAME);
        if portable_config.exists() {
            return Ok(ConfigDir {
                path: exe_dir.to_path_buf(),
                is_portable: true,
            });
        }
    }

    // Use OS config dir
    let proj_dirs = directories::ProjectDirs::from("", "", "sap-odata-explorer")
        .ok_or_else(|| anyhow::anyhow!("could not determine config directory"))?;

    let config_dir = proj_dirs.config_dir();
    std::fs::create_dir_all(config_dir)?;

    Ok(ConfigDir {
        path: config_dir.to_path_buf(),
        is_portable: false,
    })
}

/// Load the config file from the resolved config directory.
pub fn load_config() -> anyhow::Result<(ConfigFile, ConfigDir)> {
    let config_dir = match find_config_dir() {
        Some(dir) => dir,
        None => {
            return Ok((
                ConfigFile::default(),
                ConfigDir {
                    path: PathBuf::new(),
                    is_portable: false,
                },
            ));
        }
    };

    let config_path = config_dir.path.join(CONFIG_FILENAME);
    if !config_path.exists() {
        return Ok((ConfigFile::default(), config_dir));
    }

    let content = std::fs::read_to_string(&config_path)?;
    let config: ConfigFile = toml::from_str(&content)?;
    debug!(
        "Loaded {} connection(s) from {}",
        config.connections.len(),
        config_path.display()
    );

    Ok((config, config_dir))
}

/// Save the config file to the given directory.
pub fn save_config(config: &ConfigFile, config_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(config_dir)?;
    let config_path = config_dir.join(CONFIG_FILENAME);
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&config_path, &content)?;
    Ok(config_path)
}

/// Given the previous and new connection parameters for a profile, clear any
/// persisted Browser SSO session if the connection fingerprint has changed.
/// This prevents replaying cookies to a different SAP target after a profile
/// is edited in place. No-op when fingerprints match or no old profile exists.
///
/// Returns `Ok(true)` if a clear was attempted and succeeded, `Ok(false)` if
/// no clear was needed, and `Err(_)` if the clear was attempted but failed —
/// in which case the caller should warn the user that a stale session may
/// survive on the next sign-in.
pub fn clear_session_if_connection_changed(
    profile_name: &str,
    old_profile: Option<&ConnectionProfile>,
    new_base_url: &str,
    new_client: &str,
    new_language: &str,
) -> anyhow::Result<bool> {
    let new_fp = crate::session::connection_fingerprint(new_base_url, new_client, new_language);
    let should_clear = match old_profile {
        None => false, // brand-new profile — nothing to clear
        Some(old) => {
            let old_fp =
                crate::session::connection_fingerprint(&old.base_url, &old.client, &old.language);
            old_fp != new_fp
        }
    };
    if should_clear {
        crate::session::clear(profile_name)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Read a password from the OS keyring, distinguishing missing entries from
/// real failures.
///
/// - `Ok(Some(pw))` — entry found.
/// - `Ok(None)` — no entry stored. Normal for a fresh profile or a basic-auth
///   profile whose password lives in `connections.toml` instead.
/// - `Err(KeyringReadError)` — the credential store rejected the read.
///   Callers should surface a category-specific message rather than treating
///   this as "no password" (which would prompt the user to re-add the
///   profile when the real fix is to unlock the OS credential store).
pub fn try_get_password_from_keyring(
    profile_name: &str,
    username: &str,
) -> Result<Option<String>, KeyringReadError> {
    let target = format!("{KEYRING_SERVICE}:{profile_name}:{username}");
    let entry = keyring::Entry::new(&target, username).map_err(classify_keyring_error)?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(classify_keyring_error(e)),
    }
}

/// Compatibility wrapper that collapses any failure to `None` and emits a
/// `tracing::warn!` so trace logs still distinguish "no entry" (silent) from
/// real backend errors. New callers should prefer `try_get_password_from_keyring`
/// so they can show users the difference.
pub fn get_password_from_keyring(profile_name: &str, username: &str) -> Option<String> {
    match try_get_password_from_keyring(profile_name, username) {
        Ok(value) => value,
        Err(e) => {
            warn!(
                profile = profile_name,
                error = %e,
                "keyring read failed (treating as no password)",
            );
            None
        }
    }
}

/// Store a password in the OS keyring.
pub fn set_password_in_keyring(
    profile_name: &str,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let target = format!("{KEYRING_SERVICE}:{profile_name}:{username}");
    let entry = keyring::Entry::new(&target, username)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    entry
        .set_password(password)
        .map_err(|e| anyhow::anyhow!("failed to store password in keyring: {e}"))?;
    Ok(())
}

/// Delete a password from the OS keyring. Idempotent: a missing entry is
/// treated as success, since callers don't always know whether the password
/// was stored in keyring vs. plaintext config.
pub fn delete_password_from_keyring(profile_name: &str, username: &str) -> anyhow::Result<()> {
    let target = format!("{KEYRING_SERVICE}:{profile_name}:{username}");
    let entry = keyring::Entry::new(&target, username)
        .map_err(|e| anyhow::anyhow!("keyring error: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("failed to delete from keyring: {e}")),
    }
}

/// Resolve a connection profile into a SapConnection.
/// For SSO profiles, no password is needed.
/// For basic auth: plaintext in config → OS keyring → error.
pub fn resolve_connection(
    profile_name: &str,
    profile: &ConnectionProfile,
) -> anyhow::Result<SapConnection> {
    if profile.browser_sso {
        return Ok(SapConnection {
            base_url: profile.base_url.clone(),
            client: profile.client.clone(),
            language: profile.language.clone(),
            auth: AuthConfig::Browser,
            insecure_tls: profile.insecure_tls,
            sso_delegate: false,
        });
    }

    if profile.sso {
        return Ok(SapConnection {
            base_url: profile.base_url.clone(),
            client: profile.client.clone(),
            language: profile.language.clone(),
            auth: AuthConfig::Sso,
            insecure_tls: profile.insecure_tls,
            sso_delegate: profile.sso_delegate,
        });
    }

    let password = if let Some(ref pw) = profile.password {
        pw.clone()
    } else {
        match try_get_password_from_keyring(profile_name, &profile.username) {
            Ok(Some(pw)) => pw,
            Ok(None) => anyhow::bail!(
                "no password found for profile '{}' (user: {}). \
                 Use 'sap-odata profile add' to set one, or set SAP_PASSWORD env var.",
                profile_name,
                profile.username
            ),
            // Distinct messaging for read failures: re-adding the profile
            // wouldn't help if the OS credential store itself is unreachable.
            Err(KeyringReadError::Locked(msg)) => anyhow::bail!(
                "cannot read keyring for profile '{}': {}. \
                 Unlock your OS credential store (e.g. sign in to the keychain / Credential Manager) and retry.",
                profile_name,
                msg
            ),
            Err(KeyringReadError::Corrupt(msg)) => anyhow::bail!(
                "stored password for profile '{}' is corrupt: {}. \
                 Re-save the password with 'sap-odata profile add' to overwrite it.",
                profile_name,
                msg
            ),
            Err(KeyringReadError::Backend(msg)) => anyhow::bail!(
                "keyring backend error for profile '{}': {}. \
                 Check the OS credential store status; this is not a missing-password issue.",
                profile_name,
                msg
            ),
        }
    };

    Ok(SapConnection {
        base_url: profile.base_url.clone(),
        client: profile.client.clone(),
        language: profile.language.clone(),
        auth: AuthConfig::Basic {
            username: profile.username.clone(),
            password,
        },
        insecure_tls: profile.insecure_tls,
        sso_delegate: false,
    })
}

/// Initialize a portable config next to the executable.
pub fn init_portable_config(config: &ConfigFile) -> anyhow::Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine exe directory"))?;
    save_config(config, exe_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_no_storage_access_maps_to_locked() {
        let inner: Box<dyn std::error::Error + Send + Sync> = "credential store is locked".into();
        let classified = classify_keyring_error(keyring::Error::NoStorageAccess(inner));
        match classified {
            KeyringReadError::Locked(msg) => assert!(msg.contains("locked")),
            other => panic!("expected Locked, got {other:?}"),
        }
    }

    #[test]
    fn classify_bad_encoding_maps_to_corrupt() {
        let classified = classify_keyring_error(keyring::Error::BadEncoding(vec![0xff, 0xfe]));
        assert!(matches!(classified, KeyringReadError::Corrupt(_)));
    }

    #[test]
    fn classify_platform_failure_maps_to_backend() {
        let inner: Box<dyn std::error::Error + Send + Sync> = "dbus connect failed".into();
        let classified = classify_keyring_error(keyring::Error::PlatformFailure(inner));
        match classified {
            KeyringReadError::Backend(msg) => assert!(msg.contains("dbus")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    #[test]
    fn classify_invalid_attribute_maps_to_backend() {
        // Invalid is a credential-attribute validation error — fold into
        // Backend so users still get the underlying reason without us adding
        // a one-off enum variant that callers would need to handle.
        let classified = classify_keyring_error(keyring::Error::Invalid(
            "username".to_string(),
            "empty".to_string(),
        ));
        assert!(matches!(classified, KeyringReadError::Backend(_)));
    }

    #[test]
    fn classify_too_long_maps_to_backend() {
        let classified =
            classify_keyring_error(keyring::Error::TooLong("password".to_string(), 256));
        assert!(matches!(classified, KeyringReadError::Backend(_)));
    }

    #[test]
    fn keyring_read_error_display_is_actionable() {
        // The Display strings are what end up in user-facing CLI/Tauri text,
        // so they need to point at the OS credential store rather than at
        // the profile, which is the classification's whole point.
        let locked = KeyringReadError::Locked("session timeout".to_string());
        assert!(locked.to_string().contains("locked"));

        let corrupt = KeyringReadError::Corrupt("UTF-8 decode failed".to_string());
        assert!(corrupt.to_string().contains("corrupt"));

        let backend = KeyringReadError::Backend("dbus dropped".to_string());
        assert!(backend.to_string().contains("backend"));
    }
}
