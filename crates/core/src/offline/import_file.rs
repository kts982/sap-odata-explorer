// Path B: "Open EDMX file" — import a local file into the offline library.
//
// Unlike path A, path B has no connected `SapClient` involvement:
// the user picks a file on disk (downloaded from API Hub,
// hand-pulled via `/IWFND/GW_CLIENT` "Save Response", produced by
// `curl > out.xml`, etc.) and we just read + validate + index it.
// All the heavy lifting — validation pipeline, atomic writes,
// cross-process lock, length caps — is shared with path A in
// `save.rs`; this module only contains the path-B-specific
// orchestration.
//
// Logical-identity contract for path B re-imports:
// - Lookup is `(offline_profile, label_at_creation)`.
//   `label_at_creation` is the label as recorded at the first
//   import of this logical service — frozen even if the user later
//   renames `label`.
// - First import of a `(profile, label)` pair → fresh entry.
// - Re-import with the same `(profile, label)` and **same content
//   sha256 + on-disk hash match** → `SaveKind::SkippedByteIdentical`.
// - Re-import with same `(profile, label)` and different content →
//   `SaveKind::OverwriteUpdatedBytes`. The `service_id`, `edmx_file`,
//   and `label_at_creation` stay frozen; `sha256`, `size_bytes`,
//   `imported_at` are updated; the EDMX bytes are atomically rewritten.
//
// The source-profile-mismatch check on existing buckets is naturally
// skipped for path B because the caller passes
// `source_profile_for_new_bucket = None` — the check fires only when
// both sides are non-empty. The user is responsible for choosing the
// right bucket; for the common case the default `Imported` bucket
// (empty source_profile) accepts anything.

use std::path::{Path, PathBuf};

use super::import::{
    ImportError, MAX_IMPORT_SIZE_BYTES, derive_label_from_schema_namespace, strip_utf8_bom,
    validate_edmx,
};
use super::paths::{safe_join_under, slugify};
use super::save::{
    CONFIG_FILENAME, LABEL_MAX_CHARS, NOTE_MAX_BYTES, OFFLINE_DIR_NAME,
    ORIGINAL_FILENAME_MAX_CHARS, SaveError, SaveKind, SaveLock, SaveOutcome, cap_bytes, cap_chars,
    id_suffix_hex, sha256_hex,
};
use super::storage::{StorageError, write_bytes_atomically, write_toml_atomically};
use super::{OfflineProfile, OfflineService, build_service_id};
use crate::config::ConfigFile;

const DEFAULT_IMPORT_BUCKET: &str = "Imported";

/// Caller-supplied options for the import operation. Distinct from
/// `SaveOptions` (path A) because the user-controlled inputs differ:
/// no `source_url`, no `source_profile_for_new_bucket`, no
/// `source_service_path`. The file path is the primary input.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Path to the EDMX file on disk. The file must exist, be a
    /// regular file, and be no larger than
    /// `import::MAX_IMPORT_SIZE_BYTES`. The check happens before the
    /// bytes are read so a 5 GB blob doesn't get loaded into memory
    /// just to be rejected.
    pub file_path: PathBuf,
    /// Offline-profile bucket name. `None` falls back to the
    /// catch-all `"Imported"` bucket. Auto-created if not present
    /// (with empty `source_profile` — the mixed-source convention).
    pub target_offline_profile: Option<String>,
    /// Override the auto-derived label. The label-derivation rule
    /// runs on the file's `Schema Namespace` and produces e.g.
    /// `UI_PHYSSTOCKPROD_1` for `com.sap.gateway.srvd.ui_physstockprod.v0001`.
    /// If the namespace doesn't match a known shape, derivation
    /// returns empty and the filename stem is used as a fallback.
    pub label_override: Option<String>,
    /// User-supplied free-form note. Capped at `NOTE_MAX_BYTES` at
    /// persistence time.
    pub note: Option<String>,
    /// ISO-8601 timestamp for `created_at` (new bucket) /
    /// `imported_at`. Production callers should pass
    /// `current_iso8601()`; tests pin a fixed value.
    pub now_iso: String,
}

/// Path B with bytes already in memory. Used by the Tauri frontend's
/// `<input type="file">` flow, which has the file contents but not
/// a filesystem path the Rust side can read. The `original_filename`
/// (basename of the user's pick, if known) feeds the label-derivation
/// fallback and the `original_filename` index field.
///
/// `import_edmx_file` (the disk-based entry point) is a thin wrapper
/// around this: it reads bytes via a bounded `take(MAX+1).read_to_end`,
/// then delegates. All the post-bytes work — validation, locking,
/// reload-under-lock, bucket creation, identity lookup, atomic
/// writes — lives here.
#[allow(clippy::too_many_arguments)]
pub fn import_edmx_from_bytes(
    config: &mut ConfigFile,
    config_dir: &Path,
    bytes: &[u8],
    original_filename: Option<String>,
    target_offline_profile: Option<String>,
    label_override: Option<String>,
    note: Option<String>,
    now_iso: String,
) -> Result<SaveOutcome, SaveError> {
    // 0. Acquire the cross-process save lock — same primitive as
    //    path A. Held for the full transaction.
    let _save_lock = SaveLock::acquire(config_dir)?;

    // 0b. Reload config from disk under the lock.
    let toml_path = config_dir.join(CONFIG_FILENAME);
    if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).map_err(|e| StorageError::Io {
            path: toml_path.clone(),
            source: e,
        })?;
        *config = toml::from_str(&content)
            .map_err(|e| SaveError::TomlSerialize(format!("reload parse: {e}")))?;
    }

    // 1. Size cap is the only invariant we enforce on already-loaded
    //    bytes (the file-path version's `take(MAX+1)` already
    //    enforced this, but a direct bytes-caller might not have).
    if (bytes.len() as u64) > MAX_IMPORT_SIZE_BYTES {
        return Err(SaveError::Validation(ImportError::TooLarge {
            size: bytes.len() as u64,
            limit: MAX_IMPORT_SIZE_BYTES,
        }));
    }

    // 2. Validate via the full pipeline. Any rejection here returns
    //    the friendly variant verbatim — no writes happen.
    let validated = validate_edmx(bytes)?;

    // 4. Resolve the target bucket. Default to `Imported`.
    let target_profile_name =
        target_offline_profile.unwrap_or_else(|| DEFAULT_IMPORT_BUCKET.to_string());

    // 5. Create the bucket if it doesn't exist. Name-uniqueness check
    //    against `connections` — same rule as path A. Source-profile
    //    mismatch isn't checked here because path B has no source
    //    profile; the new bucket gets an empty `source_profile` which
    //    is the mixed-source convention (matches `Imported`).
    let mut created_new_offline_profile = false;
    if !config.offline_profiles.contains_key(&target_profile_name) {
        if config.connections.contains_key(&target_profile_name) {
            return Err(SaveError::OfflineProfileNameConflict {
                name: target_profile_name,
            });
        }
        config.offline_profiles.insert(
            target_profile_name.clone(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: now_iso.clone(),
            },
        );
        created_new_offline_profile = true;
    }

    // 6. Compute the bytes-on-disk view (BOM-stripped) + hash + size.
    let storable_bytes = strip_utf8_bom(bytes);
    let sha256 = sha256_hex(storable_bytes);
    let size_bytes = storable_bytes.len() as u64;

    // 7. Derive the display label. Auto-derivation from
    //    `Schema Namespace` first; if that returns empty (unknown
    //    namespace shape), fall back to the filename stem; if even
    //    that is empty, last-resort fallback to "service".
    let auto_label = derive_label_from_schema_namespace(&validated.schema_namespace);
    // Filename stem from the user-supplied original_filename (if any),
    // used as a fallback for label derivation when the schema namespace
    // is unknown. The file-path entry point computes this from
    // `file_path.file_stem()` and passes it through; the direct-bytes
    // entry point can leave it `None`.
    let filename_stem = original_filename
        .as_deref()
        .and_then(|name| {
            std::path::Path::new(name)
                .file_stem()
                .and_then(|s| s.to_str())
                .map(String::from)
        })
        .unwrap_or_default();
    let display_label = label_override
        .map(|s| cap_chars(s.trim().to_string(), LABEL_MAX_CHARS))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            // All three fallback sources (auto-derived schema label,
            // filename stem, "service") are capped so a hostile EDMX
            // namespace or an absurdly long file name can't push an
            // oversized string into the persisted index.
            let candidate = if !auto_label.is_empty() {
                auto_label.clone()
            } else if !filename_stem.is_empty() {
                filename_stem.clone()
            } else {
                "service".to_string()
            };
            cap_chars(candidate, LABEL_MAX_CHARS)
        });

    // 8. Locate the existing row by `(profile, label_at_creation)`,
    //    **restricted to path-B rows only** (`source_service_path` is
    //    None). Without this restriction, importing a file with the
    //    same label as an existing path-A capture would overwrite the
    //    path-A row, silently clearing its `source_service_path` /
    //    `fetched_at` / `source_url` attribution. Path A and path B
    //    occupy disjoint identity spaces by `source_service_path`
    //    presence; path B's lookup must respect that.
    let existing_idx = config.offline_services.iter().position(|s| {
        s.profile == target_profile_name
            && s.source_service_path.is_none()
            && s.label_at_creation == display_label
    });

    // 9. Byte-identical short-circuit (with on-disk hash verification,
    //    matching the path-A hardening). Skips only if the new bytes
    //    sha256 matches the TOML claim AND the on-disk EDMX file
    //    actually contains those bytes.
    if let Some(idx) = existing_idx
        && config.offline_services[idx].sha256 == sha256
    {
        let svc = &config.offline_services[idx];
        let offline_root_abs = config_dir.join(OFFLINE_DIR_NAME);
        // Resolve through both the syntactic (`safe_join_under`) and
        // runtime (`canonicalize_under`) boundary checks. Without the
        // canonicalize step, a symlink at `svc.edmx_file` pointing
        // outside the offline root could match the hash and skip the
        // write, leaving the wrong bytes on disk.
        let disk_matches = safe_join_under(&offline_root_abs, &svc.edmx_file)
            .ok()
            .and_then(|p| super::paths::canonicalize_under(&p, &offline_root_abs).ok())
            .and_then(|canon| std::fs::read(&canon).ok())
            .map(|disk_bytes| sha256_hex(&disk_bytes) == sha256)
            .unwrap_or(false);
        if disk_matches {
            return Ok(SaveOutcome {
                offline_profile_name: target_profile_name,
                service_id: svc.id.clone(),
                edmx_file: svc.edmx_file.clone(),
                odata_version: validated.odata_version,
                sha256: svc.sha256.clone(),
                size_bytes: svc.size_bytes,
                created_new_offline_profile,
                kind: SaveKind::SkippedByteIdentical,
            });
        }
    }

    // 10. Resolve or generate the index entry.
    let (service_id, label_at_creation, edmx_relative) = match existing_idx {
        Some(idx) => {
            let svc = &config.offline_services[idx];
            (
                svc.id.clone(),
                svc.label_at_creation.clone(),
                svc.edmx_file.clone(),
            )
        }
        None => {
            // Suffix derivation includes the original filename so
            // re-imports of the same logical (profile, label) pair
            // from different source files get distinct ids if a row
            // didn't already exist — useful for debugging but never
            // observable to the user because the `(profile, label)`
            // identity check above runs first.
            let suffix = id_suffix_hex(&[
                target_profile_name.as_bytes(),
                display_label.as_bytes(),
                now_iso.as_bytes(),
                filename_stem.as_bytes(),
            ]);
            let id = build_service_id(&display_label, &suffix);
            let edmx = format!("{}/{}.edmx", slugify(&target_profile_name), id);
            (id, display_label.clone(), edmx)
        }
    };

    // 11. Atomic EDMX write under `{config}/offline/`.
    let offline_root_abs = config_dir.join(OFFLINE_DIR_NAME);
    std::fs::create_dir_all(&offline_root_abs).map_err(|e| StorageError::Mkdir {
        path: offline_root_abs.clone(),
        source: e,
    })?;
    write_bytes_atomically(&offline_root_abs, &edmx_relative, storable_bytes)?;

    // 12. Mutate the in-memory index. Path-B-specific field
    //     population: imported_at + original_filename are Some, the
    //     path-A fields stay None.
    let capped_original_filename = original_filename.map(|name| {
        // Capture basename only (caller may have passed a full path);
        // cap at the plan-stated max so a pathological filename can't
        // bloat the index.
        let basename = std::path::Path::new(&name)
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or(name);
        cap_chars(basename, ORIGINAL_FILENAME_MAX_CHARS)
    });
    let capped_note = note
        .map(|n| cap_bytes(n, NOTE_MAX_BYTES))
        .unwrap_or_default();

    let row = OfflineService {
        id: service_id.clone(),
        profile: target_profile_name.clone(),
        label: display_label,
        label_at_creation,
        source_service_path: None,
        edmx_file: edmx_relative.clone(),
        fetched_at: None,
        imported_at: Some(now_iso.clone()),
        source_url: None,
        original_filename: capped_original_filename,
        sha256: sha256.clone(),
        size_bytes,
        odata_version: format!("{:?}", validated.odata_version),
        note: capped_note,
    };
    let kind = match existing_idx {
        Some(idx) => {
            config.offline_services[idx] = row;
            SaveKind::OverwriteUpdatedBytes
        }
        None => {
            config.offline_services.push(row);
            SaveKind::NewService
        }
    };

    // 13. Atomic TOML index rewrite.
    let serialized =
        toml::to_string_pretty(&*config).map_err(|e| SaveError::TomlSerialize(e.to_string()))?;
    write_toml_atomically(config_dir, CONFIG_FILENAME, &serialized)?;

    Ok(SaveOutcome {
        offline_profile_name: target_profile_name,
        service_id,
        edmx_file: edmx_relative,
        odata_version: validated.odata_version,
        sha256,
        size_bytes,
        created_new_offline_profile,
        kind,
    })
}

/// Path B disk entry point: reads `options.file_path`, runs the
/// bounded read (TOCTOU-safe via `take(MAX+1)`), then delegates to
/// `import_edmx_from_bytes`. CLI / Tauri callers that have a real
/// filesystem path use this. The webview file-picker flow (which
/// only gives JS a `File` object, not a path) uses
/// `import_edmx_from_bytes` directly with bytes from
/// `file.arrayBuffer()`.
pub fn import_edmx_file(
    config: &mut ConfigFile,
    config_dir: &Path,
    options: ImportOptions,
) -> Result<SaveOutcome, SaveError> {
    let ImportOptions {
        file_path,
        target_offline_profile,
        label_override,
        note,
        now_iso,
    } = options;

    // 1a. Type check via path metadata so the user gets a friendly
    //     "not a regular file" error when they pick a directory. The
    //     TOCTOU concern the reviewer flagged is about *size*, not
    //     type — if the path is swapped to a directory between this
    //     stat and the open below, `File::open` will fail and we
    //     return a clean I/O error.
    let pre_meta = std::fs::metadata(&file_path).map_err(|e| StorageError::Io {
        path: file_path.clone(),
        source: e,
    })?;
    if !pre_meta.is_file() {
        return Err(SaveError::Validation(ImportError::NotARegularFile));
    }

    // 1b. Open the file once, stat the handle, bounded-read.
    use std::io::Read;
    let file = std::fs::File::open(&file_path).map_err(|e| StorageError::Io {
        path: file_path.clone(),
        source: e,
    })?;
    let handle_size = file.metadata().map(|m| m.len()).unwrap_or(0);
    if handle_size > MAX_IMPORT_SIZE_BYTES {
        return Err(SaveError::Validation(ImportError::TooLarge {
            size: handle_size,
            limit: MAX_IMPORT_SIZE_BYTES,
        }));
    }
    let limit = MAX_IMPORT_SIZE_BYTES.saturating_add(1);
    let mut bytes = Vec::with_capacity(handle_size.min(MAX_IMPORT_SIZE_BYTES) as usize + 1);
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|e| StorageError::Io {
            path: file_path.clone(),
            source: e,
        })?;
    if (bytes.len() as u64) > MAX_IMPORT_SIZE_BYTES {
        return Err(SaveError::Validation(ImportError::TooLarge {
            size: bytes.len() as u64,
            limit: MAX_IMPORT_SIZE_BYTES,
        }));
    }

    // Hand off to the bytes entry point. `original_filename` is the
    // basename of the user-picked path so the filename-stem fallback
    // and the persisted `original_filename` field work correctly.
    let original_filename = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from);
    import_edmx_from_bytes(
        config,
        config_dir,
        &bytes,
        original_filename,
        target_offline_profile,
        label_override,
        note,
        now_iso,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::ODataVersion;
    use std::collections::BTreeMap;
    use std::fs;

    const VALID_V4_EDMX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="com.sap.gateway.srvd.ui_physstockprod.v0001">
      <EntityType Name="Product"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    fn unique_dir(label: &str) -> PathBuf {
        let tid = std::thread::current().id();
        let p = std::env::temp_dir().join(format!("sap_odata_imp_{label}_{tid:?}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("temp dir create");
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    fn write_input_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, contents).unwrap();
        p
    }

    fn opts(file: PathBuf, profile: Option<&str>, label: Option<&str>) -> ImportOptions {
        ImportOptions {
            file_path: file,
            target_offline_profile: profile.map(String::from),
            label_override: label.map(String::from),
            note: None,
            now_iso: "2026-05-18T08:00:00Z".to_string(),
        }
    }

    // ── happy path ──

    #[test]
    fn imports_into_default_imported_bucket() {
        let dir = unique_dir("default_bucket");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "OP_WAREHOUSE.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();

        let outcome =
            import_edmx_file(&mut config, &cfg_dir, opts(input.clone(), None, None)).unwrap();

        assert_eq!(outcome.kind, SaveKind::NewService);
        assert!(outcome.created_new_offline_profile);
        assert_eq!(outcome.offline_profile_name, "Imported");
        // Auto-derived label from schema namespace.
        assert_eq!(outcome.odata_version, ODataVersion::V4);

        let svc = &config.offline_services[0];
        assert_eq!(svc.profile, "Imported");
        assert_eq!(svc.label, "UI_PHYSSTOCKPROD_1");
        assert_eq!(svc.label_at_creation, "UI_PHYSSTOCKPROD_1");
        assert!(svc.source_service_path.is_none());
        assert!(svc.fetched_at.is_none());
        assert!(svc.source_url.is_none());
        assert_eq!(svc.imported_at.as_deref(), Some("2026-05-18T08:00:00Z"));
        assert_eq!(svc.original_filename.as_deref(), Some("OP_WAREHOUSE.edmx"));

        // Bucket was created with empty source_profile (the
        // mixed-source `Imported` convention).
        assert_eq!(config.offline_profiles["Imported"].source_profile, "");

        // EDMX bytes on disk.
        let edmx_full = cfg_dir.join("offline").join(&svc.edmx_file);
        assert!(edmx_full.exists());
        assert_eq!(fs::read(&edmx_full).unwrap(), VALID_V4_EDMX.as_bytes());

        cleanup(&dir);
    }

    #[test]
    fn imports_into_named_bucket() {
        let dir = unique_dir("named_bucket");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "svc.xml", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();

        let outcome = import_edmx_file(
            &mut config,
            &cfg_dir,
            opts(input, Some("Customer X (offline)"), None),
        )
        .unwrap();
        assert_eq!(outcome.offline_profile_name, "Customer X (offline)");
        assert!(config.offline_profiles.contains_key("Customer X (offline)"));
        cleanup(&dir);
    }

    // ── label override + filename fallback ──

    #[test]
    fn label_override_takes_precedence_over_derived() {
        let dir = unique_dir("label_override");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "any.xml", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        let outcome =
            import_edmx_file(&mut config, &cfg_dir, opts(input, None, Some("ZCUSTOM"))).unwrap();
        assert_eq!(config.offline_services[0].label, "ZCUSTOM");
        assert_eq!(config.offline_services[0].label_at_creation, "ZCUSTOM");
        assert!(outcome.service_id.starts_with("zcustom-"));
        cleanup(&dir);
    }

    #[test]
    fn falls_back_to_filename_stem_when_namespace_unknown() {
        let dir = unique_dir("filename_fallback");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        // Namespace shape that derive_label can't recognize as SAP.
        let weird_edmx = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="weird:funky:thing">
      <EntityType Name="X"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let input = write_input_file(&dir, "MyService.edmx", weird_edmx.as_bytes());
        let mut config = ConfigFile::default();
        import_edmx_file(&mut config, &cfg_dir, opts(input, None, None)).unwrap();
        assert_eq!(config.offline_services[0].label, "MyService");
        cleanup(&dir);
    }

    // ── re-import behaviors ──

    #[test]
    fn re_import_with_same_bytes_and_label_is_no_op() {
        let dir = unique_dir("reimport_noop");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        let first =
            import_edmx_file(&mut config, &cfg_dir, opts(input.clone(), None, None)).unwrap();

        let mut later = opts(input, None, None);
        later.now_iso = "2027-01-01T00:00:00Z".into();
        let second = import_edmx_file(&mut config, &cfg_dir, later).unwrap();
        assert_eq!(second.kind, SaveKind::SkippedByteIdentical);
        assert_eq!(second.service_id, first.service_id);
        // imported_at NOT bumped.
        assert_eq!(
            config.offline_services[0].imported_at.as_deref(),
            Some("2026-05-18T08:00:00Z")
        );
        assert_eq!(config.offline_services.len(), 1);
        cleanup(&dir);
    }

    #[test]
    fn re_import_with_same_label_different_bytes_overwrites() {
        let dir = unique_dir("reimport_overwrite");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input_a = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        let first = import_edmx_file(&mut config, &cfg_dir, opts(input_a, None, None)).unwrap();

        // Different file, different content, but caller passes the
        // same label → same `(profile, label_at_creation)` identity
        // → overwrite.
        let modified = VALID_V4_EDMX.replace("Product", "ProductV2");
        let input_b = write_input_file(&dir, "b.edmx", modified.as_bytes());
        let mut later = opts(input_b, None, Some("UI_PHYSSTOCKPROD_1"));
        later.now_iso = "2027-01-01T00:00:00Z".into();
        let second = import_edmx_file(&mut config, &cfg_dir, later).unwrap();
        assert_eq!(second.kind, SaveKind::OverwriteUpdatedBytes);
        assert_eq!(second.service_id, first.service_id);
        assert_eq!(second.edmx_file, first.edmx_file);
        // EDMX on disk matches the new bytes.
        let edmx_full = cfg_dir.join("offline").join(&second.edmx_file);
        let on_disk = fs::read_to_string(&edmx_full).unwrap();
        assert!(on_disk.contains("ProductV2"));
        assert_eq!(config.offline_services.len(), 1);
        cleanup(&dir);
    }

    #[test]
    fn different_labels_produce_separate_rows() {
        let dir = unique_dir("two_labels");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        import_edmx_file(
            &mut config,
            &cfg_dir,
            opts(input.clone(), None, Some("LABEL_A")),
        )
        .unwrap();
        import_edmx_file(&mut config, &cfg_dir, opts(input, None, Some("LABEL_B"))).unwrap();
        assert_eq!(config.offline_services.len(), 2);
        cleanup(&dir);
    }

    #[test]
    fn re_import_overwrites_when_disk_file_was_tampered() {
        // Same hardening as path A: don't trust the TOML hash alone.
        let dir = unique_dir("reimport_tampered");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        import_edmx_file(&mut config, &cfg_dir, opts(input.clone(), None, None)).unwrap();
        let edmx_full = cfg_dir
            .join("offline")
            .join(&config.offline_services[0].edmx_file);
        fs::write(&edmx_full, b"TAMPERED").unwrap();

        let outcome = import_edmx_file(&mut config, &cfg_dir, opts(input, None, None)).unwrap();
        assert_eq!(outcome.kind, SaveKind::OverwriteUpdatedBytes);
        assert_eq!(fs::read(&edmx_full).unwrap(), VALID_V4_EDMX.as_bytes());
        cleanup(&dir);
    }

    // ── pre-read rejects ──

    #[test]
    fn rejects_nonexistent_file_without_touching_config() {
        let dir = unique_dir("nonexistent");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let mut config = ConfigFile::default();
        let err = import_edmx_file(
            &mut config,
            &cfg_dir,
            opts(dir.join("does_not_exist.edmx"), None, None),
        )
        .unwrap_err();
        assert!(matches!(err, SaveError::Storage(StorageError::Io { .. })));
        assert!(config.offline_profiles.is_empty());
        assert!(config.offline_services.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn rejects_directory_passed_as_file() {
        let dir = unique_dir("dir_as_file");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let dir_path = dir.join("looks_like_a_file");
        fs::create_dir(&dir_path).unwrap();
        let mut config = ConfigFile::default();
        let err = import_edmx_file(&mut config, &cfg_dir, opts(dir_path, None, None)).unwrap_err();
        // Distinct variant so the user-facing message says "not a
        // regular file" rather than the awkward
        // "document root is `<(not a regular file)>`" of the old shape.
        assert!(matches!(
            err,
            SaveError::Validation(ImportError::NotARegularFile)
        ));
        cleanup(&dir);
    }

    #[test]
    fn rejects_oversized_file_without_loading_bytes() {
        // Construct a file at the size-cap boundary + 1 byte. The
        // metadata-based pre-check should fire before the full read
        // so we don't load the whole thing into memory. We use
        // `set_len` to grow a sparse file cheaply — on Windows + most
        // POSIX filesystems this is O(1) and doesn't actually allocate
        // disk blocks for the unwritten range.
        let dir = unique_dir("oversize");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let big_path = dir.join("big.edmx");
        let f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&big_path)
            .unwrap();
        f.set_len(MAX_IMPORT_SIZE_BYTES + 1).unwrap();
        drop(f);

        let mut config = ConfigFile::default();
        let err = import_edmx_file(&mut config, &cfg_dir, opts(big_path, None, None)).unwrap_err();
        match err {
            SaveError::Validation(ImportError::TooLarge { size, limit }) => {
                assert_eq!(size, MAX_IMPORT_SIZE_BYTES + 1);
                assert_eq!(limit, MAX_IMPORT_SIZE_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
        cleanup(&dir);
    }

    // ── validation rejects ──

    #[test]
    fn rejects_html_login_page_without_writing_anything() {
        let dir = unique_dir("html_page");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "login.edmx", b"<html><body>SAP Login</body></html>");
        let mut config = ConfigFile::default();
        let err = import_edmx_file(&mut config, &cfg_dir, opts(input, None, None)).unwrap_err();
        assert!(matches!(
            err,
            SaveError::Validation(ImportError::LooksLikeHtmlPage)
        ));
        assert!(config.offline_profiles.is_empty());
        assert!(config.offline_services.is_empty());
        assert!(!cfg_dir.join(CONFIG_FILENAME).exists());
        cleanup(&dir);
    }

    // ── name-collision check ──

    #[test]
    fn rejects_import_target_colliding_with_connected_profile() {
        let dir = unique_dir("name_collision");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        config.connections.insert(
            "DEV".to_string(),
            crate::config::ConnectionProfile {
                base_url: "https://x".into(),
                client: "100".into(),
                language: "EN".into(),
                username: "".into(),
                password: None,
                sso: false,
                browser_sso: true,
                insecure_tls: false,
                sso_delegate: false,
                aliases: BTreeMap::new(),
            },
        );
        let err =
            import_edmx_file(&mut config, &cfg_dir, opts(input, Some("DEV"), None)).unwrap_err();
        match err {
            SaveError::OfflineProfileNameConflict { name } => assert_eq!(name, "DEV"),
            other => panic!("expected name conflict, got {other:?}"),
        }
        cleanup(&dir);
    }

    // ── caps + filename capture ──

    #[test]
    fn original_filename_captures_basename_not_full_path() {
        // The captured field should be the file's basename + ext,
        // *not* the full path the user picked. Otherwise sharing an
        // offline pack would leak the consultant's local directory
        // layout. The cap-to-`ORIGINAL_FILENAME_MAX_CHARS` behavior
        // is covered by `cap_chars` tests in `save.rs`; we can't
        // construct a >255-char filename on Windows to exercise the
        // cap end-to-end here, so this test only verifies the
        // basename-capture invariant.
        let dir = unique_dir("filename_basename");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let input = write_input_file(&nested, "MyService.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        import_edmx_file(&mut config, &cfg_dir, opts(input.clone(), None, None)).unwrap();
        let captured = config.offline_services[0]
            .original_filename
            .as_deref()
            .unwrap();
        assert_eq!(captured, "MyService.edmx");
        // Crucially: no parent-directory component leaked through.
        assert!(!captured.contains("nested"));
        assert!(!captured.contains(std::path::MAIN_SEPARATOR));
        cleanup(&dir);
    }

    #[test]
    fn import_does_not_overwrite_path_a_row_with_same_label() {
        // Setup: pre-populate a path-A row with label "UI_PHYSSTOCKPROD_1"
        // in the `DEV (offline)` bucket. A path-B import with the same
        // label into the same bucket must NOT overwrite the path-A row
        // — that would silently clear `source_service_path`,
        // `fetched_at`, and `source_url`, demoting an attributed
        // capture into an anonymous import.
        let dir = unique_dir("path_a_vs_b_isolation");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let mut config = ConfigFile::default();
        // Seed a path-A row manually (we don't call save_service_offline
        // here because it needs a SapClient; the index-shape suffices).
        config.offline_profiles.insert(
            "DEV (offline)".to_string(),
            OfflineProfile {
                source_profile: "DEV".to_string(),
                created_at: "2026-05-18T08:00:00Z".to_string(),
            },
        );
        config.offline_services.push(OfflineService {
            id: "ui_physstockprod_1-pathaaaa".into(),
            profile: "DEV (offline)".into(),
            label: "UI_PHYSSTOCKPROD_1".into(),
            label_at_creation: "UI_PHYSSTOCKPROD_1".into(),
            source_service_path: Some("/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1".into()),
            edmx_file: "dev_offline/ui_physstockprod_1-pathaaaa.edmx".into(),
            fetched_at: Some("2026-05-18T08:00:00Z".into()),
            imported_at: None,
            source_url: Some(
                "https://sap.example.com/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1/$metadata".into(),
            ),
            original_filename: None,
            sha256: "deadbeef".repeat(8),
            size_bytes: 999,
            odata_version: "V4".into(),
            note: "".into(),
        });
        // Pre-write the path-A EDMX so the disk state is consistent.
        let path_a_edmx = cfg_dir
            .join("offline")
            .join("dev_offline")
            .join("ui_physstockprod_1-pathaaaa.edmx");
        fs::create_dir_all(path_a_edmx.parent().unwrap()).unwrap();
        fs::write(&path_a_edmx, b"path-A bytes").unwrap();

        // Now import a file with the *same label* into the same bucket.
        // Auto-derivation would produce "UI_PHYSSTOCKPROD_1" from the
        // schema namespace, exactly matching the path-A row's label.
        let input = write_input_file(&dir, "import.edmx", VALID_V4_EDMX.as_bytes());
        let outcome = import_edmx_file(
            &mut config,
            &cfg_dir,
            opts(input, Some("DEV (offline)"), None),
        )
        .unwrap();

        // Must be a NEW row, not an overwrite.
        assert_eq!(outcome.kind, SaveKind::NewService);
        assert_eq!(config.offline_services.len(), 2);
        // Path-A row is untouched: attribution still present.
        let path_a = config
            .offline_services
            .iter()
            .find(|s| s.source_service_path.is_some())
            .expect("path-A row should still exist");
        assert_eq!(path_a.id, "ui_physstockprod_1-pathaaaa");
        assert!(path_a.fetched_at.is_some());
        assert!(path_a.source_url.is_some());
        assert_eq!(fs::read(&path_a_edmx).unwrap(), b"path-A bytes");
        // Path-B row is separate.
        let path_b = config
            .offline_services
            .iter()
            .find(|s| s.source_service_path.is_none())
            .expect("path-B row should have been created");
        assert!(path_b.imported_at.is_some());
        assert_ne!(path_b.id, path_a.id);
        cleanup(&dir);
    }

    #[test]
    fn reload_under_lock_picks_up_concurrent_writes() {
        // Simulate the race the reviewer flagged: the caller loaded an
        // empty config, then another writer added an offline profile +
        // service to `connections.toml`, then our save proceeds. The
        // reload-under-lock step must pick up that prior write so the
        // final TOML contains both rows, not just ours.
        let dir = unique_dir("reload_under_lock");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();

        // Caller's stale snapshot: empty.
        let mut config = ConfigFile::default();

        // Another writer's view: a single offline profile + service.
        let mut concurrent = ConfigFile::default();
        concurrent.offline_profiles.insert(
            "Customer X (offline)".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "2026-05-18T07:00:00Z".to_string(),
            },
        );
        concurrent.offline_services.push(OfflineService {
            id: "preexisting-cccccccc".into(),
            profile: "Customer X (offline)".into(),
            label: "PREEXISTING".into(),
            label_at_creation: "PREEXISTING".into(),
            source_service_path: None,
            edmx_file: "customer_x_offline/preexisting-cccccccc.edmx".into(),
            fetched_at: None,
            imported_at: Some("2026-05-18T07:00:00Z".into()),
            source_url: None,
            original_filename: Some("preexisting.edmx".into()),
            sha256: "0".repeat(64),
            size_bytes: 1,
            odata_version: "V4".into(),
            note: "".into(),
        });
        let serialized = toml::to_string_pretty(&concurrent).unwrap();
        fs::write(cfg_dir.join("connections.toml"), &serialized).unwrap();

        // Now our save proceeds with the stale empty-snapshot. The
        // reload-under-lock step should pull in the concurrent writer's
        // state before we mutate.
        let input = write_input_file(&dir, "ours.edmx", VALID_V4_EDMX.as_bytes());
        import_edmx_file(
            &mut config,
            &cfg_dir,
            opts(input, Some("Imported"), Some("OUR_SERVICE")),
        )
        .unwrap();

        // After the call: both rows present in memory AND on disk.
        assert!(
            config.offline_profiles.contains_key("Customer X (offline)"),
            "concurrent writer's profile lost — reload-under-lock didn't fire"
        );
        assert!(config.offline_profiles.contains_key("Imported"));
        assert_eq!(config.offline_services.len(), 2);
        // On-disk TOML reflects the merged state.
        let final_toml = fs::read_to_string(cfg_dir.join("connections.toml")).unwrap();
        assert!(final_toml.contains("Customer X (offline)"));
        assert!(final_toml.contains("PREEXISTING"));
        assert!(final_toml.contains("OUR_SERVICE"));
        cleanup(&dir);
    }

    #[test]
    fn note_is_capped_at_max_bytes() {
        let dir = unique_dir("note_cap");
        let cfg_dir = dir.join("cfg");
        fs::create_dir_all(&cfg_dir).unwrap();
        let input = write_input_file(&dir, "a.edmx", VALID_V4_EDMX.as_bytes());
        let mut config = ConfigFile::default();
        let mut o = opts(input, None, None);
        o.note = Some("z".repeat(NOTE_MAX_BYTES + 500));
        import_edmx_file(&mut config, &cfg_dir, o).unwrap();
        assert_eq!(config.offline_services[0].note.len(), NOTE_MAX_BYTES);
        cleanup(&dir);
    }
}
