// Read path for the offline EDMX library.
//
// Given a `(profile_name, service_id)` pair previously resolved via
// `MetadataSource::resolve`, returns the EDMX XML as a `String` suitable
// for handing to the existing `metadata::parse_metadata` pipeline.
// The byte read goes through both `safe_join_under` (syntactic) and
// `canonicalize_under` (runtime symlink/reparse-point boundary check),
// so an indexed `edmx_file` that resolves outside the offline root is
// rejected even if the syntactic path looks innocuous.
//
// Strict-descendancy is intentionally enforced: a row whose `edmx_file`
// canonicalizes to the offline root itself would otherwise pass — and
// while that can't happen via the normal save / import flow (filenames
// are tool-generated under a profile-slug subdirectory), defense in
// depth is cheap and aligns with the delete-path discipline.

use std::path::Path;

use thiserror::Error;

use super::OfflineService;
use super::import::strip_utf8_bom;
use super::paths::{PathError, canonicalize_under, safe_join_under};
use super::save::{CONFIG_FILENAME, OFFLINE_DIR_NAME};
use crate::config::ConfigFile;

const _CONFIG_FILENAME_REFERENCED_FOR_DOCS: &str = CONFIG_FILENAME;

#[derive(Debug, Error)]
pub enum OfflineReadError {
    #[error("offline profile '{0}' not found in connections.toml")]
    ProfileNotFound(String),

    #[error("offline service '{service_id}' not found in profile '{profile}'")]
    ServiceNotFound { profile: String, service_id: String },

    #[error("path safety violation reading offline EDMX: {0}")]
    Path(#[from] PathError),

    #[error("I/O error reading offline EDMX at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("offline EDMX is not valid UTF-8 (file may have been corrupted on disk)")]
    NotUtf8,
}

/// Read the cached `$metadata` XML for an offline service.
///
/// Caller has already resolved the `MetadataSource::Offline` variant
/// (which carries the canonical `service_id`), so this fn only looks
/// up by exact `(profile_name, service_id)` — no fallback search by
/// `source_service_path` or `label`. The dispatch happens earlier in
/// `MetadataSource::resolve`.
///
/// Returns the bytes BOM-stripped + UTF-8-decoded so the result is
/// drop-in compatible with `parse_metadata(xml: &str)`. If the on-disk
/// file is somehow not valid UTF-8 — only possible via manual tampering
/// since the import pipeline rejects non-UTF-8 — returns a typed error
/// rather than panicking on `from_utf8`.
pub fn read_offline_metadata(
    config: &ConfigFile,
    config_dir: &Path,
    profile_name: &str,
    service_id: &str,
) -> Result<String, OfflineReadError> {
    // 1. Profile must exist as an offline profile.
    if !config.offline_profiles.contains_key(profile_name) {
        return Err(OfflineReadError::ProfileNotFound(profile_name.to_string()));
    }

    // 2. Locate the service row by exact `(profile, id)`.
    let row: &OfflineService = config
        .offline_services
        .iter()
        .find(|s| s.profile == profile_name && s.id == service_id)
        .ok_or_else(|| OfflineReadError::ServiceNotFound {
            profile: profile_name.to_string(),
            service_id: service_id.to_string(),
        })?;

    // 3. Resolve `edmx_file` through both the syntactic and runtime
    //    boundary checks. Matches the discipline of the sweep / delete
    //    paths — `safe_join_under` rejects `..` / absolute / Windows
    //    reserved names; `canonicalize_under` rejects symlink escapes.
    let offline_root = config_dir.join(OFFLINE_DIR_NAME);
    let joined = safe_join_under(&offline_root, &row.edmx_file)?;
    let canonical = canonicalize_under(&joined, &offline_root)?;

    // 4. Read + UTF-8 decode. BOM is stripped on the way out so the
    //    result is the canonical XML body — `parse_metadata` doesn't
    //    have to handle a leading BOM character.
    let bytes = std::fs::read(&canonical).map_err(|e| OfflineReadError::Io {
        path: canonical.display().to_string(),
        source: e,
    })?;
    let stripped = strip_utf8_bom(&bytes);
    let xml = std::str::from_utf8(stripped).map_err(|_| OfflineReadError::NotUtf8)?;
    Ok(xml.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::offline::OfflineProfile;
    use crate::offline::storage::write_bytes_atomically;
    use std::fs;
    use std::path::PathBuf;

    const VALID_V4_EDMX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="x">
      <EntityType Name="X"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    fn unique_dir(label: &str) -> PathBuf {
        let tid = std::thread::current().id();
        let p = std::env::temp_dir().join(format!("sap_odata_read_{label}_{tid:?}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    /// Build a `ConfigFile` with one offline profile + one service, and
    /// pre-write the EDMX file at the indexed path. Returns the
    /// (config, config_dir) pair plus the service id for assertion use.
    fn seed_offline(
        cfg_dir: &Path,
        profile: &str,
        service_id: &str,
        edmx_relative: &str,
        bytes: &[u8],
    ) -> ConfigFile {
        let offline_root = cfg_dir.join(OFFLINE_DIR_NAME);
        fs::create_dir_all(&offline_root).unwrap();
        write_bytes_atomically(&offline_root, edmx_relative, bytes).unwrap();

        let mut config = ConfigFile::default();
        config.offline_profiles.insert(
            profile.to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "2026-05-18T08:00:00Z".to_string(),
            },
        );
        config.offline_services.push(OfflineService {
            id: service_id.to_string(),
            profile: profile.to_string(),
            label: "SVC".to_string(),
            label_at_creation: "SVC".to_string(),
            source_service_path: None,
            edmx_file: edmx_relative.to_string(),
            fetched_at: None,
            imported_at: Some("2026-05-18T08:00:00Z".to_string()),
            source_url: None,
            original_filename: Some("svc.edmx".to_string()),
            sha256: "0".repeat(64),
            size_bytes: bytes.len() as u64,
            odata_version: "V4".to_string(),
            note: String::new(),
        });
        config
    }

    #[test]
    fn reads_indexed_service_bytes() {
        let dir = unique_dir("happy");
        let cfg = seed_offline(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
            VALID_V4_EDMX.as_bytes(),
        );
        let xml = read_offline_metadata(&cfg, &dir, "Imported", "svc-12345678").unwrap();
        assert!(xml.contains("Namespace=\"x\""));
        cleanup(&dir);
    }

    #[test]
    fn strips_bom_from_stored_file() {
        // Storage usually saves BOM-stripped, but a manually-placed
        // file might have a BOM. Read path must handle it.
        let dir = unique_dir("bom");
        let mut bytes = Vec::from(&[0xEF, 0xBB, 0xBF][..]);
        bytes.extend_from_slice(VALID_V4_EDMX.as_bytes());
        let cfg = seed_offline(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
            &bytes,
        );
        let xml = read_offline_metadata(&cfg, &dir, "Imported", "svc-12345678").unwrap();
        // The returned XML must not start with the BOM bytes.
        assert!(xml.starts_with("<?xml"));
        cleanup(&dir);
    }

    #[test]
    fn rejects_unknown_profile() {
        let dir = unique_dir("unknown_profile");
        let cfg = seed_offline(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
            VALID_V4_EDMX.as_bytes(),
        );
        let err = read_offline_metadata(&cfg, &dir, "Nonexistent", "svc-12345678").unwrap_err();
        assert!(matches!(err, OfflineReadError::ProfileNotFound(name) if name == "Nonexistent"));
        cleanup(&dir);
    }

    #[test]
    fn rejects_unknown_service_id() {
        let dir = unique_dir("unknown_service");
        let cfg = seed_offline(
            &dir,
            "Imported",
            "svc-12345678",
            "imported/svc-12345678.edmx",
            VALID_V4_EDMX.as_bytes(),
        );
        let err = read_offline_metadata(&cfg, &dir, "Imported", "svc-deadbeef").unwrap_err();
        match err {
            OfflineReadError::ServiceNotFound {
                profile,
                service_id,
            } => {
                assert_eq!(profile, "Imported");
                assert_eq!(service_id, "svc-deadbeef");
            }
            other => panic!("expected ServiceNotFound, got {other:?}"),
        }
        cleanup(&dir);
    }

    #[test]
    fn rejects_index_entry_with_unsafe_edmx_file() {
        // Manual corruption / hostile config: edmx_file contains
        // `..`. `safe_join_under` rejects synthetically.
        let dir = unique_dir("unsafe_path");
        let mut config = ConfigFile::default();
        config.offline_profiles.insert(
            "Imported".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "2026-05-18T08:00:00Z".to_string(),
            },
        );
        config.offline_services.push(OfflineService {
            id: "svc-evil-1234".to_string(),
            profile: "Imported".to_string(),
            label: "EVIL".to_string(),
            label_at_creation: "EVIL".to_string(),
            source_service_path: None,
            edmx_file: "../escape.edmx".to_string(),
            fetched_at: None,
            imported_at: Some("now".to_string()),
            source_url: None,
            original_filename: None,
            sha256: "0".repeat(64),
            size_bytes: 1,
            odata_version: "V4".to_string(),
            note: String::new(),
        });

        let err = read_offline_metadata(&config, &dir, "Imported", "svc-evil-1234").unwrap_err();
        assert!(matches!(err, OfflineReadError::Path(_)));
        cleanup(&dir);
    }

    #[test]
    fn rejects_missing_file_on_disk() {
        // Index entry exists, but the EDMX file doesn't. I/O error
        // surfaces via the typed variant rather than panicking.
        let dir = unique_dir("missing_disk");
        let mut config = ConfigFile::default();
        config.offline_profiles.insert(
            "Imported".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "x".to_string(),
            },
        );
        config.offline_services.push(OfflineService {
            id: "svc-12345678".to_string(),
            profile: "Imported".to_string(),
            label: "X".to_string(),
            label_at_creation: "X".to_string(),
            source_service_path: None,
            edmx_file: "imported/svc-12345678.edmx".to_string(),
            fetched_at: None,
            imported_at: Some("x".to_string()),
            source_url: None,
            original_filename: None,
            sha256: "0".repeat(64),
            size_bytes: 0,
            odata_version: "V4".to_string(),
            note: String::new(),
        });
        // offline root exists, but the file doesn't.
        fs::create_dir_all(dir.join(OFFLINE_DIR_NAME).join("imported")).unwrap();
        let err = read_offline_metadata(&config, &dir, "Imported", "svc-12345678").unwrap_err();
        assert!(
            matches!(err, OfflineReadError::Path(_) | OfflineReadError::Io { .. }),
            "expected Path or Io error, got {err:?}"
        );
        cleanup(&dir);
    }
}
