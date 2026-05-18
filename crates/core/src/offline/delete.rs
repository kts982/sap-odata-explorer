// Delete path for the offline EDMX library.
//
// Two granularities:
// - `delete_offline_service(config, config_dir, profile, service_id)`
//   removes one row from the index and one file from disk.
// - `delete_offline_profile(config, config_dir, profile_name)` removes
//   the whole bucket (every service row + every file + the bucket
//   subdirectory + the bucket entry).
//
// Both operations:
// - Acquire the cross-process `SaveLock` (same lockfile as save /
//   import) and reload config under the lock so concurrent writes
//   can't be lost-updated by the delete.
// - Resolve every disk path through `safe_join_under` + `canonicalize_under`
//   (strict-descendancy enforced for the bucket directory; the file
//   case is via `canonicalize_under` on the resolved file path).
// - Atomically rewrite the TOML index after mutation.
//
// **Strict-descendancy on the bucket directory is load-bearing.** A
// recursive `remove_dir_all` against the offline root itself would
// obliterate the library; the boundary check refuses to authorize that.

use std::path::Path;

use thiserror::Error;
use tracing::warn;

use super::paths::{PathError, canonicalize_under, safe_join_under, slugify};
use super::save::{CONFIG_FILENAME, OFFLINE_DIR_NAME, SaveError, SaveLock};
use super::storage::{StorageError, write_toml_atomically};
use crate::config::ConfigFile;

#[derive(Debug, Error)]
pub enum DeleteError {
    #[error("offline profile '{0}' not found")]
    ProfileNotFound(String),
    #[error("offline service '{service_id}' not found in profile '{profile}'")]
    ServiceNotFound { profile: String, service_id: String },
    #[error("path safety violation: {0}")]
    Path(#[from] PathError),
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
    /// The internal save-side errors (lockfile contention, TOML
    /// serialize failure) bubble up via the existing `SaveError`
    /// variants — re-using them keeps the error surface unified for
    /// the UI / CLI.
    #[error("save error: {0}")]
    Save(#[from] SaveError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct DeleteServiceOutcome {
    pub profile: String,
    pub service_id: String,
    /// Set to true if the on-disk EDMX file was removed. Already-gone
    /// is treated as success (idempotent); the field lets the caller
    /// log a warning if the indexed file was missing.
    pub file_removed: bool,
}

#[derive(Debug, Clone)]
pub struct DeleteProfileOutcome {
    pub profile: String,
    /// Number of `offline_services` rows that were removed.
    pub services_removed: usize,
    /// Number of EDMX files that were removed from disk. Can be less
    /// than `services_removed` if some files were already missing —
    /// idempotent.
    pub files_removed: usize,
    /// Set to true if the bucket subdirectory was removed.
    /// `false` if it didn't exist (idempotent).
    pub directory_removed: bool,
}

/// Delete a single offline service. Removes the indexed `edmx_file`
/// from disk (via the canonical-under-root resolver) and the matching
/// `offline_services` row from the TOML index. Idempotent on the
/// disk side: a missing file is logged + treated as success.
pub fn delete_offline_service(
    config: &mut ConfigFile,
    config_dir: &Path,
    profile: &str,
    service_id: &str,
) -> Result<DeleteServiceOutcome, DeleteError> {
    let _lock = SaveLock::acquire(config_dir)?;

    // Reload config under the lock, same discipline as save / import.
    let toml_path = config_dir.join(CONFIG_FILENAME);
    if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).map_err(|e| StorageError::Io {
            path: toml_path.clone(),
            source: e,
        })?;
        *config = toml::from_str(&content).map_err(|e| {
            DeleteError::Save(SaveError::TomlSerialize(format!("reload parse: {e}")))
        })?;
    }

    // Locate the row.
    let idx = config
        .offline_services
        .iter()
        .position(|s| s.profile == profile && s.id == service_id)
        .ok_or_else(|| DeleteError::ServiceNotFound {
            profile: profile.to_string(),
            service_id: service_id.to_string(),
        })?;

    let edmx_relative = config.offline_services[idx].edmx_file.clone();
    let offline_root = config_dir.join(OFFLINE_DIR_NAME);

    // Try to resolve + remove the file. Both `safe_join_under` and
    // `canonicalize_under` must succeed for us to authorize the
    // removal; if either fails we still drop the index row (the
    // file's gone or pointed somewhere we won't touch) and report.
    let mut file_removed = false;
    if let Ok(joined) = safe_join_under(&offline_root, &edmx_relative) {
        match canonicalize_under(&joined, &offline_root) {
            Ok(canonical) => {
                match std::fs::remove_file(&canonical) {
                    Ok(()) => file_removed = true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // File already gone — idempotent.
                        warn!(
                            profile,
                            service_id,
                            "EDMX file was already absent on disk; removing index entry only"
                        );
                    }
                    Err(e) => {
                        return Err(DeleteError::Io {
                            path: canonical.display().to_string(),
                            source: e,
                        });
                    }
                }
            }
            Err(e) => {
                // Path resolved syntactically but escaped the offline
                // root at runtime (symlink/reparse). Don't follow the
                // link — drop the index row only.
                warn!(
                    profile,
                    service_id,
                    edmx_file = %edmx_relative,
                    error = %e,
                    "indexed EDMX path resolved outside offline root; dropping index row only"
                );
            }
        }
    } else {
        // `safe_join_under` rejected the relative path — corrupt
        // index entry. Drop the row and log.
        warn!(
            profile,
            service_id,
            edmx_file = %edmx_relative,
            "indexed EDMX path failed safe_join_under; dropping index row only"
        );
    }

    // Mutate index + atomically rewrite TOML.
    config.offline_services.remove(idx);
    let serialized = toml::to_string_pretty(&*config)
        .map_err(|e| DeleteError::Save(SaveError::TomlSerialize(e.to_string())))?;
    write_toml_atomically(config_dir, CONFIG_FILENAME, &serialized)?;

    Ok(DeleteServiceOutcome {
        profile: profile.to_string(),
        service_id: service_id.to_string(),
        file_removed,
    })
}

/// Delete a whole offline profile. Removes every `offline_services`
/// row for that profile, every corresponding EDMX file, the bucket
/// subdirectory under `{config}/offline/`, and the `offline_profiles`
/// entry. Strict-descendancy is enforced on the bucket directory
/// before any recursive remove — a misconfigured bucket name that
/// resolves to the offline root itself returns an error rather than
/// authorizing a wipe of the entire library.
pub fn delete_offline_profile(
    config: &mut ConfigFile,
    config_dir: &Path,
    profile_name: &str,
) -> Result<DeleteProfileOutcome, DeleteError> {
    let _lock = SaveLock::acquire(config_dir)?;

    // Reload under lock.
    let toml_path = config_dir.join(CONFIG_FILENAME);
    if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).map_err(|e| StorageError::Io {
            path: toml_path.clone(),
            source: e,
        })?;
        *config = toml::from_str(&content).map_err(|e| {
            DeleteError::Save(SaveError::TomlSerialize(format!("reload parse: {e}")))
        })?;
    }

    if !config.offline_profiles.contains_key(profile_name) {
        return Err(DeleteError::ProfileNotFound(profile_name.to_string()));
    }

    let offline_root = config_dir.join(OFFLINE_DIR_NAME);

    // Collect file paths to remove before mutating the index — the
    // resolution might fail for individual rows but we still want to
    // try the rest.
    let to_remove: Vec<(String, String)> = config
        .offline_services
        .iter()
        .filter(|s| s.profile == profile_name)
        .map(|s| (s.id.clone(), s.edmx_file.clone()))
        .collect();
    let services_removed = to_remove.len();

    let mut files_removed = 0usize;
    for (service_id, edmx_relative) in &to_remove {
        match safe_join_under(&offline_root, edmx_relative)
            .and_then(|p| canonicalize_under(&p, &offline_root))
        {
            Ok(canonical) => match std::fs::remove_file(&canonical) {
                Ok(()) => files_removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    warn!(profile = profile_name, service_id, "EDMX file already gone");
                }
                Err(e) => {
                    return Err(DeleteError::Io {
                        path: canonical.display().to_string(),
                        source: e,
                    });
                }
            },
            Err(e) => {
                warn!(
                    profile = profile_name,
                    service_id,
                    edmx_file = %edmx_relative,
                    error = %e,
                    "indexed EDMX path resolved unsafely or escaped offline root; skipping file removal"
                );
            }
        }
    }

    // Try to remove the bucket subdirectory itself. The dir name is
    // `slugify(profile_name)`; if it doesn't exist (empty bucket
    // never wrote a file), that's fine. Strict-descendancy is the
    // key safety property here — `canonicalize_under` refuses to
    // return a path equal to the offline root.
    let mut directory_removed = false;
    let bucket_slug = slugify(profile_name);
    if let Ok(bucket_path) = safe_join_under(&offline_root, &bucket_slug)
        && bucket_path.exists()
    {
        match canonicalize_under(&bucket_path, &offline_root) {
            Ok(canonical_bucket) => match std::fs::remove_dir_all(&canonical_bucket) {
                Ok(()) => directory_removed = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // Race with another process. Idempotent.
                }
                Err(e) => {
                    return Err(DeleteError::Io {
                        path: canonical_bucket.display().to_string(),
                        source: e,
                    });
                }
            },
            Err(e) => {
                warn!(
                    profile = profile_name,
                    error = %e,
                    "bucket directory resolved outside offline root or to root itself; refusing recursive remove"
                );
            }
        }
    }

    // Mutate index.
    config.offline_profiles.remove(profile_name);
    config
        .offline_services
        .retain(|s| s.profile != profile_name);

    let serialized = toml::to_string_pretty(&*config)
        .map_err(|e| DeleteError::Save(SaveError::TomlSerialize(e.to_string())))?;
    write_toml_atomically(config_dir, CONFIG_FILENAME, &serialized)?;

    Ok(DeleteProfileOutcome {
        profile: profile_name.to_string(),
        services_removed,
        files_removed,
        directory_removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::OfflineProfile;
    use crate::offline::OfflineService;
    use crate::offline::storage::write_bytes_atomically;
    use std::fs;
    use std::path::PathBuf;

    fn unique_dir(label: &str) -> PathBuf {
        let tid = std::thread::current().id();
        let p = std::env::temp_dir().join(format!("sap_odata_del_{label}_{tid:?}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    /// Seed a config + on-disk layout with one offline profile and
    /// one service. Returns (config, edmx_file_path_on_disk).
    fn seed(cfg_dir: &Path, profile: &str, service_id: &str, edmx_rel: &str) -> ConfigFile {
        let offline_root = cfg_dir.join(OFFLINE_DIR_NAME);
        fs::create_dir_all(&offline_root).unwrap();
        write_bytes_atomically(&offline_root, edmx_rel, b"<edmx/>").unwrap();
        let mut config = ConfigFile::default();
        config.offline_profiles.insert(
            profile.to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "x".to_string(),
            },
        );
        config.offline_services.push(OfflineService {
            id: service_id.to_string(),
            profile: profile.to_string(),
            label: "L".to_string(),
            label_at_creation: "L".to_string(),
            source_service_path: None,
            edmx_file: edmx_rel.to_string(),
            fetched_at: None,
            imported_at: Some("x".to_string()),
            source_url: None,
            original_filename: None,
            sha256: "0".repeat(64),
            size_bytes: 7,
            odata_version: "V4".to_string(),
            note: String::new(),
        });
        config
    }

    // ── delete_offline_service ──

    #[test]
    fn delete_service_removes_row_and_file() {
        let dir = unique_dir("del_svc_happy");
        let mut cfg = seed(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
        );
        let edmx_full = dir.join("offline").join("imported/svc-12345678.edmx");
        assert!(edmx_full.exists());

        let outcome = delete_offline_service(&mut cfg, &dir, "Imported", "svc-12345678").unwrap();
        assert!(outcome.file_removed);
        assert_eq!(outcome.profile, "Imported");
        assert!(cfg.offline_services.is_empty());
        // Profile bucket entry still exists (only the service was deleted).
        assert!(cfg.offline_profiles.contains_key("Imported"));
        // File gone.
        assert!(!edmx_full.exists());
        cleanup(&dir);
    }

    #[test]
    fn delete_service_is_idempotent_on_missing_file() {
        let dir = unique_dir("del_svc_missing_file");
        let mut cfg = seed(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
        );
        // Pre-delete the file (simulate a manual cleanup).
        fs::remove_file(dir.join("offline").join("imported/svc-12345678.edmx")).unwrap();

        let outcome = delete_offline_service(&mut cfg, &dir, "Imported", "svc-12345678").unwrap();
        assert!(!outcome.file_removed);
        // Row still removed.
        assert!(cfg.offline_services.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn delete_service_rejects_unknown_service() {
        let dir = unique_dir("del_svc_unknown");
        let mut cfg = seed(
            &dir,
            "Imported",
            "svc-aaaaaaaa",
            "imported/svc-aaaaaaaa.edmx",
        );
        let err = delete_offline_service(&mut cfg, &dir, "Imported", "svc-bbbbbbbb").unwrap_err();
        assert!(matches!(err, DeleteError::ServiceNotFound { .. }));
        // Original row + file untouched.
        assert_eq!(cfg.offline_services.len(), 1);
        assert!(
            dir.join("offline")
                .join("imported/svc-aaaaaaaa.edmx")
                .exists()
        );
        cleanup(&dir);
    }

    // ── delete_offline_profile ──

    #[test]
    fn delete_profile_removes_bucket_services_and_dir() {
        let dir = unique_dir("del_prof_happy");
        let mut cfg = seed(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
        );
        // Add a second service so the count assertion is meaningful.
        let offline_root = dir.join(OFFLINE_DIR_NAME);
        write_bytes_atomically(&offline_root, "imported/svc-deadbeef.edmx", b"<edmx/>").unwrap();
        cfg.offline_services.push(OfflineService {
            id: "svc-deadbeef".to_string(),
            profile: "Imported".to_string(),
            label: "L2".to_string(),
            label_at_creation: "L2".to_string(),
            source_service_path: None,
            edmx_file: "imported/svc-deadbeef.edmx".to_string(),
            fetched_at: None,
            imported_at: Some("x".to_string()),
            source_url: None,
            original_filename: None,
            sha256: "0".repeat(64),
            size_bytes: 7,
            odata_version: "V4".to_string(),
            note: String::new(),
        });

        let outcome = delete_offline_profile(&mut cfg, &dir, "Imported").unwrap();
        assert_eq!(outcome.services_removed, 2);
        assert_eq!(outcome.files_removed, 2);
        assert!(outcome.directory_removed);

        // Index empty.
        assert!(cfg.offline_profiles.is_empty());
        assert!(cfg.offline_services.is_empty());
        // Bucket dir gone.
        assert!(!dir.join("offline").join("imported").exists());
        cleanup(&dir);
    }

    #[test]
    fn delete_profile_rejects_unknown_profile() {
        let dir = unique_dir("del_prof_unknown");
        let mut cfg = ConfigFile::default();
        let err = delete_offline_profile(&mut cfg, &dir, "Nonexistent").unwrap_err();
        assert!(matches!(err, DeleteError::ProfileNotFound(n) if n == "Nonexistent"));
        cleanup(&dir);
    }

    #[test]
    fn delete_profile_is_idempotent_when_dir_missing() {
        // Edge case: the bucket has rows in the index but the on-disk
        // directory was already cleaned up externally. Delete should
        // still succeed; `directory_removed = false`.
        let dir = unique_dir("del_prof_dir_gone");
        let mut cfg = seed(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
        );
        // Remove the bucket directory entirely.
        fs::remove_dir_all(dir.join("offline").join("imported")).unwrap();

        let outcome = delete_offline_profile(&mut cfg, &dir, "Imported").unwrap();
        assert_eq!(outcome.services_removed, 1);
        assert_eq!(outcome.files_removed, 0);
        assert!(!outcome.directory_removed);
        cleanup(&dir);
    }
}
