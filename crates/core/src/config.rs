use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::auth::{AuthConfig, SapConnection};

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
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let portable_config = exe_dir.join(CONFIG_FILENAME);
            if portable_config.exists() {
                debug!("Using portable config at {}", exe_dir.display());
                return Some(ConfigDir {
                    path: exe_dir.to_path_buf(),
                    is_portable: true,
                });
            }
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
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let portable_config = exe_dir.join(CONFIG_FILENAME);
            if portable_config.exists() {
                return Ok(ConfigDir {
                    path: exe_dir.to_path_buf(),
                    is_portable: true,
                });
            }
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

/// Try to get a password from the OS keyring.
pub fn get_password_from_keyring(profile_name: &str, username: &str) -> Option<String> {
    let target = format!("{KEYRING_SERVICE}:{profile_name}:{username}");
    match keyring::Entry::new(&target, username) {
        Ok(entry) => match entry.get_password() {
            Ok(pw) => Some(pw),
            Err(_) => None,
        },
        Err(_) => None,
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
        });
    }

    if profile.sso {
        return Ok(SapConnection {
            base_url: profile.base_url.clone(),
            client: profile.client.clone(),
            language: profile.language.clone(),
            auth: AuthConfig::Sso,
            insecure_tls: profile.insecure_tls,
        });
    }

    let password = if let Some(ref pw) = profile.password {
        pw.clone()
    } else if let Some(pw) = get_password_from_keyring(profile_name, &profile.username) {
        pw
    } else {
        anyhow::bail!(
            "no password found for profile '{}' (user: {}). \
             Use 'sap-odata profile add' to set one, or set SAP_PASSWORD env var.",
            profile_name,
            profile.username
        );
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
