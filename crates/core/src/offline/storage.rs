// Atomic write helpers + load-time consistency sweep for the offline EDMX
// library.
//
// Two flavours of write happen in offline mode:
//
// 1. EDMX bytes to disk under `{config}/offline/<slug(profile)>/<id>.edmx`.
// 2. The TOML index in `{config}/connections.toml`.
//
// Both have to be crash-safe in the sense that a process death mid-write
// must not leave the user with: (a) corrupted/half-written EDMX content,
// (b) a corrupted/half-written `connections.toml`, or (c) a TOML index
// pointing at a file that doesn't exist (dangling reference). The write-
// ordering we commit to:
//
//   write EDMX bytes atomically → write TOML index atomically
//
// If we crash between the two, we leave behind an *orphan* EDMX file (on
// disk but not referenced by the index). Orphans are recoverable and
// invisible to the user — the load-time sweep finds them and logs them.
// The reverse ordering would produce *dangling references* (index entry
// pointing at a file that never made it to disk), which would break the
// describe/entities/lint commands at read time.
//
// Atomicity is achieved with the standard temp-file + rename pattern.
// On both Windows (NTFS) and POSIX, a successful `fs::rename` of a file
// onto another file is atomic from the perspective of other processes
// reading the target. We sync the file's contents to disk before the
// rename so power-loss recovery is well-defined.
//
// `save_config` in `config.rs` currently writes the TOML directly without
// this discipline — that's a latent risk flagged for a separate cleanup
// commit, not bundled into offline-mode work.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tracing::warn;

use super::OfflineService;
use super::paths::{PathError, canonicalize_under, safe_join_under};

/// Errors from the atomic-write and sweep helpers. Distinct from
/// `PathError` so callers can match on the specific failure surface.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("path safety violation: {0}")]
    Path(#[from] PathError),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not create directory {path}: {source}")]
    Mkdir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Write `bytes` to `<root>/<relative>` atomically, enforcing the trust
/// boundary that the final file's parent directory canonicalizes inside
/// `root`. Returns the final canonical path on success.
///
/// The helper owns the full write contract:
///
/// 1. **Syntactic safety.** `relative` is validated via `safe_join_under`
///    (no `..`, no absolute, no Windows reserved names, etc.).
/// 2. **Parent creation.** Parent directories are created with
///    `create_dir_all`.
/// 3. **Boundary check.** After `create_dir_all`, the parent is
///    canonicalized and verified to start with the canonicalized `root`.
///    This catches the case where the user (or a malicious process)
///    has placed a junction / reparse point / symlink along the parent
///    chain that resolves outside the offline root — a check
///    `safe_join_under` alone cannot make, because it operates purely
///    on the input string.
/// 4. **Atomic write** via a unique tmp sibling: every call generates a
///    `<final>.tmp.<pid>.<nanos>.<counter>` so two concurrent writes to
///    the same logical target can't race-truncate each other's tmp file.
/// 5. **Durable rename.** File contents are `sync_all`-ed before the
///    rename; the parent directory is `sync_all`-ed afterwards (best
///    effort, no-op on platforms that don't expose dir fsync) so the
///    directory-entry update survives power loss alongside the contents.
///
/// On crash:
/// - between create + write: the tmp file may be partial. Not visible
///   to anything that reads `<root>/<relative>`.
/// - after rename: target has the new bytes; parent fsync makes the
///   directory entry durable.
/// - The tmp sibling is removed on success; it may linger after a
///   crash. The load-time sweep is tolerant of stray `.tmp` files.
pub fn write_bytes_atomically(
    root: &Path,
    relative: &str,
    bytes: &[u8],
) -> Result<PathBuf, StorageError> {
    // Step 1: syntactic safety.
    let target = safe_join_under(root, relative)?;

    // Step 2: ensure parent exists. `safe_join_under` guarantees at least
    // one Normal component, so `target.parent()` is always `Some`.
    let parent = target
        .parent()
        .ok_or(StorageError::Path(PathError::Empty))?;
    fs::create_dir_all(parent).map_err(|e| StorageError::Mkdir {
        path: parent.to_path_buf(),
        source: e,
    })?;

    // Step 3: boundary check. Canonicalize both root and the parent
    // we'll write into; the parent must be inside (or equal to) root.
    // Equality is allowed here — for TOML writes the target lives
    // directly inside root, not in a subdirectory. (The stricter
    // strict-descendancy check in `canonicalize_under` is for read /
    // delete callers and would falsely reject this case.)
    let canonical_root = fs::canonicalize(root).map_err(|e| StorageError::Io {
        path: root.to_path_buf(),
        source: e,
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|e| StorageError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(StorageError::Path(PathError::EscapesRoot(format!(
            "parent {} resolved outside of root {}",
            parent.display(),
            root.display()
        ))));
    }

    // Compose the final target inside the canonicalized parent so we
    // never re-traverse a possibly-symlinked path on the way to the
    // file open.
    let file_name = target
        .file_name()
        .ok_or(StorageError::Path(PathError::Empty))?;
    let final_target = canonical_parent.join(file_name);

    // Step 4: atomic write via unique tmp sibling.
    let tmp = unique_tmp_sibling(&final_target);

    // Scope the file handle so it's closed before the rename. On Windows,
    // `rename` over an open file fails; on POSIX it's allowed but it's
    // cleanest to flush + close first regardless.
    {
        let mut f = fs::File::create(&tmp).map_err(|e| StorageError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        f.write_all(bytes).map_err(|e| StorageError::Io {
            path: tmp.clone(),
            source: e,
        })?;
        f.sync_all().map_err(|e| StorageError::Io {
            path: tmp.clone(),
            source: e,
        })?;
    }

    fs::rename(&tmp, &final_target).map_err(|e| {
        // Best-effort: try to remove the tmp file if the rename failed
        // so a retry isn't blocked by leftover state. Ignore secondary
        // errors.
        let _ = fs::remove_file(&tmp);
        StorageError::Io {
            path: final_target.clone(),
            source: e,
        }
    })?;

    // Step 5: sync the parent directory so the directory-entry update
    // is durable alongside the file contents. POSIX requires this for
    // the rename to survive a power loss; Windows doesn't expose dir
    // fsync the same way and the call may be a no-op or fail — either
    // is acceptable, the rename itself already succeeded.
    if let Ok(dir) = fs::File::open(&canonical_parent) {
        let _ = dir.sync_all();
    }

    Ok(final_target)
}

/// Serialize `toml_contents` and write it to `<root>/<relative>`
/// atomically. Thin wrapper around `write_bytes_atomically` — exists to
/// make call-site intent ("updating the TOML index") explicit. For the
/// `connections.toml` case the natural `root` is the config dir, and
/// `relative` is just `"connections.toml"` (the target's parent equals
/// root, which the boundary check explicitly allows).
pub fn write_toml_atomically(
    root: &Path,
    relative: &str,
    toml_contents: &str,
) -> Result<PathBuf, StorageError> {
    write_bytes_atomically(root, relative, toml_contents.as_bytes())
}

/// Build a per-call-unique tmp sibling so two concurrent writes to the
/// same logical target can't share a tmp file. Shape:
/// `<target>.tmp.<pid>.<nanos>.<counter>`.
///
/// The pid + nanos + atomic counter combination gives uniqueness across:
/// - distinct processes (pid)
/// - rapid same-process retries within the same nanosecond (counter)
/// - serial calls in the same process (nanos monotonically increase in
///   practice; counter is a tiebreaker)
///
/// 32 bits of nanos-fragment entropy + a process-wide counter is more
/// than enough — collision would require two writes generating the same
/// pid/nanos/counter triplet, which can't happen by construction.
fn unique_tmp_sibling(target: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id();
    let mut tmp = target.as_os_str().to_os_string();
    tmp.push(format!(".tmp.{pid}.{nanos:x}.{n}"));
    PathBuf::from(tmp)
}

/// Outcome of a load-time consistency sweep over `{config}/offline/` vs
/// the indexed `offline_services` list. Both fields are intentionally
/// non-destructive — the sweep reports, the caller decides what to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OfflineSweepReport {
    /// EDMX files present in `{config}/offline/<profile_slug>/` that no
    /// `offline_services` entry references. Typically leftover `.tmp`
    /// or stale rows from manual edits; safe to ignore at runtime.
    pub orphan_files: Vec<PathBuf>,
    /// `offline_services` entries whose `edmx_file` resolves to a
    /// nonexistent path on disk. The UI should mark these services as
    /// broken ("file missing — re-save or remove"). The index entry is
    /// kept so the user can investigate; we don't auto-purge.
    pub missing_files: Vec<MissingService>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingService {
    pub id: String,
    pub profile: String,
    pub expected_path: PathBuf,
}

/// Walk the offline root and the indexed services list, reporting which
/// files have no index entry (orphans) and which index entries have no
/// file (missing). Non-destructive: nothing is deleted or rewritten.
///
/// `offline_root` is `{config}/offline/`. If it doesn't exist yet (no
/// offline activity in this config), returns an empty report.
///
/// Sweep semantics:
/// - Orphans are reported with their full path on disk.
/// - Missing references are reported with the expected resolved path so
///   the UI can show "this is where I looked."
/// - The sweep deliberately ignores `.tmp` files (left behind by a
///   crash mid-`write_bytes_atomically`) so they don't pollute the
///   orphan list. A separate cleanup pass can sweep `.tmp` files if
///   that ever becomes a maintenance concern.
/// - Symlinks are not followed when walking; the offline dir is
///   tool-managed and shouldn't contain links, but if a user manually
///   added one we don't want to traverse outside the root.
pub fn sweep_offline_dir(
    offline_root: &Path,
    services: &[OfflineService],
) -> Result<OfflineSweepReport, StorageError> {
    if !offline_root.exists() {
        return Ok(OfflineSweepReport::default());
    }

    // Build the set of `edmx_file` paths the index claims. To compare
    // safely against the disk walk's canonicalized results we have to
    // canonicalize the expected paths too — otherwise Windows' `\\?\`
    // verbatim long-path form (from `fs::canonicalize`) won't match a
    // `safe_join_under` result that's still in its original prefix
    // shape, and every on-disk file would falsely look orphaned.
    //
    // Index entries that don't pass path safety, or that resolve to
    // nonexistent files, are reported as missing. The `expected_path`
    // in the report uses the pre-canonical form because that's what's
    // helpful to show the user ("we looked at <this path>").
    let mut expected: Vec<PathBuf> = Vec::with_capacity(services.len());
    let mut missing_files = Vec::new();
    for svc in services {
        match safe_join_under(offline_root, &svc.edmx_file) {
            Ok(resolved) => {
                // `exists()` returns true for directories too — a
                // corrupted state where the index points at a path that
                // turned into a directory (manual fs surgery, or a bug
                // elsewhere) would otherwise silently pass as "file
                // present" here. Require an actual regular file via
                // `symlink_metadata().is_file()` so the broken row
                // surfaces as `missing` rather than slipping through.
                let is_regular_file = fs::symlink_metadata(&resolved)
                    .map(|m| m.is_file())
                    .unwrap_or(false);
                if !is_regular_file {
                    missing_files.push(MissingService {
                        id: svc.id.clone(),
                        profile: svc.profile.clone(),
                        expected_path: resolved,
                    });
                    continue;
                }
                match canonicalize_under(&resolved, offline_root) {
                    Ok(canon) => expected.push(canon),
                    Err(e) => {
                        warn!(
                            service_id = %svc.id,
                            profile = %svc.profile,
                            edmx_file = %svc.edmx_file,
                            error = %e,
                            "offline service file failed canonicalization (symlink escape?); treating as missing",
                        );
                        missing_files.push(MissingService {
                            id: svc.id.clone(),
                            profile: svc.profile.clone(),
                            expected_path: resolved,
                        });
                    }
                }
            }
            Err(e) => {
                warn!(
                    service_id = %svc.id,
                    profile = %svc.profile,
                    edmx_file = %svc.edmx_file,
                    error = %e,
                    "offline service index entry has unsafe edmx_file; treating as missing",
                );
                missing_files.push(MissingService {
                    id: svc.id.clone(),
                    profile: svc.profile.clone(),
                    expected_path: offline_root.join(&svc.edmx_file),
                });
            }
        }
    }

    // Scan the offline dir for `.edmx` files. Two-level depth is enough
    // for the planned layout (`offline_root/<profile_slug>/<id>.edmx`),
    // but the walk is depth-agnostic so a future layout change doesn't
    // silently break it.
    let mut on_disk: Vec<PathBuf> = Vec::new();
    walk_edmx_files(offline_root, offline_root, &mut on_disk)?;

    // Anything on disk not in the expected set is orphan. Both sides
    // are canonicalized so byte-equality is meaningful comparison.
    let mut orphan_files = Vec::new();
    for path in on_disk {
        let in_expected = expected.iter().any(|p| p == &path);
        if !in_expected {
            orphan_files.push(path);
        }
    }
    // Deterministic order for tests / logs.
    orphan_files.sort();
    missing_files.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(OfflineSweepReport {
        orphan_files,
        missing_files,
    })
}

/// Recursively collect `.edmx` files under `current`, validating each
/// against the **original** `offline_root` (not the recursion's current
/// directory). Carrying the original root through the recursion is
/// load-bearing: if we validated each file against its immediate parent
/// dir, a Windows junction / reparse-point nested deeper than the top
/// level could escape outside the offline root and the sweep wouldn't
/// catch it. Always anchor to the original boundary.
///
/// Skips `.tmp` artifacts from crashed atomic writes and skips symlinks
/// (the offline dir is tool-managed; following links would let a
/// manually-placed link escape the sweep boundary). Returns paths in
/// filesystem-order.
fn walk_edmx_files(
    current: &Path,
    offline_root: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), StorageError> {
    let entries = match fs::read_dir(current) {
        Ok(e) => e,
        Err(e) => {
            return Err(StorageError::Io {
                path: current.to_path_buf(),
                source: e,
            });
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        // Use symlink_metadata (not metadata) so symlinks don't get
        // followed into directories outside the root.
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            walk_edmx_files(&path, offline_root, out)?;
            continue;
        }
        if meta.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.ends_with(".edmx")
            && !name.ends_with(".tmp")
        {
            // Canonicalize against the **original** offline_root (not
            // `current`) so a junction/reparse-point that resolves
            // outside the offline root anywhere along the recursion
            // chain still gets caught.
            if let Ok(canonical) = canonicalize_under(&path, offline_root) {
                out.push(canonical);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_tmp_dir(label: &str) -> PathBuf {
        // Tests run in parallel by default; embed a per-test label and
        // the OS thread id so concurrent tests don't collide on the
        // same `/tmp/sap_odata_*` path.
        let tid = std::thread::current().id();
        let p = std::env::temp_dir().join(format!("sap_odata_{label}_{tid:?}"));
        let _ = fs::remove_dir_all(&p); // best-effort cleanup from previous run
        fs::create_dir_all(&p).expect("temp dir create");
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    // ── write_bytes_atomically ──

    #[test]
    fn writes_new_file_atomically() {
        let dir = unique_tmp_dir("storage_write_new");
        let final_path = write_bytes_atomically(&dir, "out.edmx", b"hello").unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"hello");
        // No `.tmp.*` siblings left behind.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.contains(".tmp.")))
            .collect();
        assert!(leftovers.is_empty(), "stray tmp files: {leftovers:?}");
        cleanup(&dir);
    }

    #[test]
    fn overwrites_existing_file_atomically() {
        let dir = unique_tmp_dir("storage_overwrite");
        let target = dir.join("out.edmx");
        fs::write(&target, b"old").unwrap();
        write_bytes_atomically(&dir, "out.edmx", b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        cleanup(&dir);
    }

    #[test]
    fn creates_missing_parent_directory() {
        let dir = unique_tmp_dir("storage_mkparent");
        let final_path = write_bytes_atomically(&dir, "nested/dir/out.edmx", b"data").unwrap();
        assert_eq!(fs::read(&final_path).unwrap(), b"data");
        cleanup(&dir);
    }

    #[test]
    fn writes_toml_atomically_round_trip() {
        // TOML write: target's parent == root. The boundary check
        // explicitly allows this (TOML lives directly inside the config
        // dir, not in a subdirectory).
        let dir = unique_tmp_dir("storage_toml_atomic");
        let body = "[connections.DEV]\nbase_url = \"https://x\"\n";
        let final_path = write_toml_atomically(&dir, "connections.toml", body).unwrap();
        assert_eq!(fs::read_to_string(&final_path).unwrap(), body);
        cleanup(&dir);
    }

    #[test]
    fn write_rejects_path_traversal_relative() {
        // Boundary check #1: syntactic. `..` in the relative path
        // rejects at the `safe_join_under` step.
        let dir = unique_tmp_dir("storage_traversal");
        let err = write_bytes_atomically(&dir, "../escape.edmx", b"x").unwrap_err();
        assert!(
            matches!(err, StorageError::Path(PathError::ParentTraversal(_))),
            "expected ParentTraversal, got {err:?}"
        );
        cleanup(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_rejects_symlinked_parent_escaping_root() {
        // Boundary check #2: filesystem. A pre-existing symlink along
        // the parent chain that resolves outside the root must be
        // rejected by the post-`create_dir_all` canonicalization, even
        // though the relative path looks fine syntactically.
        let dir = unique_tmp_dir("storage_symlink_parent");
        let root = dir.join("offline");
        let outside = dir.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();

        // Symlink `<root>/sneaky` -> `<outside>` so a write to
        // `sneaky/file.edmx` would materialize at `<outside>/file.edmx`.
        let link = root.join("sneaky");
        std::os::unix::fs::symlink(&outside, &link).unwrap();

        let err = write_bytes_atomically(&root, "sneaky/file.edmx", b"x").unwrap_err();
        assert!(
            matches!(err, StorageError::Path(PathError::EscapesRoot(_))),
            "expected EscapesRoot from parent canonicalization, got {err:?}"
        );
        // And no file was actually written to `<outside>`.
        assert!(!outside.join("file.edmx").exists());

        cleanup(&dir);
    }

    #[test]
    fn unique_tmp_sibling_differs_across_calls() {
        // Two consecutive calls for the same target must produce
        // distinct tmp paths. If they shared a tmp, concurrent re-saves
        // of the same offline service could truncate each other's
        // contents mid-write.
        let target = Path::new("/some/dir/file.edmx");
        let a = unique_tmp_sibling(target);
        let b = unique_tmp_sibling(target);
        assert_ne!(
            a, b,
            "consecutive tmp siblings should differ: {a:?} vs {b:?}"
        );
        // And both start with the right base.
        let a_str = a.to_string_lossy();
        assert!(
            a_str.contains("file.edmx.tmp."),
            "tmp sibling lost its base prefix: {a:?}"
        );
    }

    // ── sweep_offline_dir ──

    fn dummy_service(id: &str, profile: &str, edmx_file: &str) -> OfflineService {
        OfflineService {
            id: id.into(),
            profile: profile.into(),
            label: id.into(),
            label_at_creation: id.into(),
            source_service_path: None,
            edmx_file: edmx_file.into(),
            fetched_at: None,
            imported_at: Some("2026-05-18T08:00:00Z".into()),
            source_url: None,
            original_filename: None,
            sha256: "00".repeat(32),
            size_bytes: 4,
            odata_version: "V4".into(),
            note: "".into(),
        }
    }

    #[test]
    fn sweep_empty_root_returns_empty_report() {
        let dir = unique_tmp_dir("sweep_empty");
        let offline = dir.join("offline");
        // offline/ doesn't exist — sweep should not error.
        let report = sweep_offline_dir(&offline, &[]).unwrap();
        assert!(report.orphan_files.is_empty());
        assert!(report.missing_files.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn sweep_clean_state_reports_nothing() {
        let dir = unique_tmp_dir("sweep_clean");
        let offline = dir.join("offline");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        let file_path = offline.join("dev_offline").join("svc-12345678.edmx");
        fs::write(&file_path, b"data").unwrap();

        let svc = dummy_service(
            "svc-12345678",
            "DEV (offline)",
            "dev_offline/svc-12345678.edmx",
        );
        let report = sweep_offline_dir(&offline, &[svc]).unwrap();
        assert!(report.orphan_files.is_empty(), "{:?}", report.orphan_files);
        assert!(report.missing_files.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn sweep_detects_orphan_file() {
        let dir = unique_tmp_dir("sweep_orphan");
        let offline = dir.join("offline");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        // One file on disk; no index entry.
        let orphan_path = offline.join("dev_offline").join("svc-deadbeef.edmx");
        fs::write(&orphan_path, b"orphan").unwrap();

        let report = sweep_offline_dir(&offline, &[]).unwrap();
        assert_eq!(report.orphan_files.len(), 1);
        assert!(
            report.orphan_files[0].ends_with("dev_offline/svc-deadbeef.edmx")
                || report.orphan_files[0].ends_with("dev_offline\\svc-deadbeef.edmx")
        );
        assert!(report.missing_files.is_empty());
        cleanup(&dir);
    }

    #[test]
    fn sweep_detects_missing_file() {
        let dir = unique_tmp_dir("sweep_missing");
        let offline = dir.join("offline");
        fs::create_dir_all(&offline).unwrap();
        // Index entry but no file on disk.
        let svc = dummy_service(
            "svc-ab12cd34",
            "DEV (offline)",
            "dev_offline/svc-ab12cd34.edmx",
        );

        let report = sweep_offline_dir(&offline, &[svc]).unwrap();
        assert!(report.orphan_files.is_empty());
        assert_eq!(report.missing_files.len(), 1);
        assert_eq!(report.missing_files[0].id, "svc-ab12cd34");
        assert_eq!(report.missing_files[0].profile, "DEV (offline)");
        cleanup(&dir);
    }

    #[test]
    fn sweep_ignores_tmp_artifacts() {
        // A `.tmp` left behind by a crashed atomic write must NOT show up
        // as an orphan, otherwise every crash produces a new false report.
        let dir = unique_tmp_dir("sweep_tmp");
        let offline = dir.join("offline");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        let real = offline.join("dev_offline").join("svc-aaaaaaaa.edmx");
        let tmp = offline.join("dev_offline").join("svc-aaaaaaaa.edmx.tmp");
        fs::write(&real, b"real").unwrap();
        fs::write(&tmp, b"crashed-partial").unwrap();

        let svc = dummy_service(
            "svc-aaaaaaaa",
            "DEV (offline)",
            "dev_offline/svc-aaaaaaaa.edmx",
        );
        let report = sweep_offline_dir(&offline, &[svc]).unwrap();
        assert!(
            report.orphan_files.is_empty(),
            "tmp file should not be reported as orphan: {:?}",
            report.orphan_files
        );
        cleanup(&dir);
    }

    #[test]
    fn sweep_treats_directory_as_missing_not_present() {
        // Corrupted state: an indexed `edmx_file` resolves to a path
        // that exists *as a directory*. `Path::exists()` returns true,
        // but the index entry is broken — the service can't be loaded.
        // Sweep must report this as missing rather than letting the
        // broken row slip through.
        let dir = unique_tmp_dir("sweep_dir_as_edmx");
        let offline = dir.join("offline");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        // Create a *directory* where the index expects a file.
        let dir_path = offline.join("dev_offline").join("svc-deadbeef.edmx");
        fs::create_dir_all(&dir_path).unwrap();

        let svc = dummy_service(
            "svc-deadbeef",
            "DEV (offline)",
            "dev_offline/svc-deadbeef.edmx",
        );
        let report = sweep_offline_dir(&offline, &[svc]).unwrap();
        assert_eq!(report.missing_files.len(), 1, "{report:?}");
        assert_eq!(report.missing_files[0].id, "svc-deadbeef");
        // Also: the directory itself should NOT be reported as an
        // orphan file — the walk filters to regular `.edmx` files only.
        assert!(report.orphan_files.is_empty(), "{:?}", report.orphan_files);
        cleanup(&dir);
    }

    #[test]
    fn sweep_treats_unsafe_index_entry_as_missing() {
        // If somehow a `..`-laden edmx_file lands in the index (manual
        // edit, corruption), we can't read the file via the normal
        // resolver — so report it as missing rather than letting the
        // unsafe path leak into the on-disk comparison.
        let dir = unique_tmp_dir("sweep_unsafe");
        let offline = dir.join("offline");
        fs::create_dir_all(&offline).unwrap();
        let svc = dummy_service("svc-evil-1234", "DEV (offline)", "../escape.edmx");
        let report = sweep_offline_dir(&offline, &[svc]).unwrap();
        assert_eq!(report.missing_files.len(), 1);
        assert_eq!(report.missing_files[0].id, "svc-evil-1234");
        cleanup(&dir);
    }

    #[test]
    fn sweep_reports_mixed_orphan_and_missing() {
        let dir = unique_tmp_dir("sweep_mixed");
        let offline = dir.join("offline");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        // One file on disk that the index does *not* reference (orphan).
        let orphan = offline.join("dev_offline").join("svc-orphan01.edmx");
        fs::write(&orphan, b"x").unwrap();
        // One index entry whose file is *not* on disk (missing).
        let svc_missing = dummy_service(
            "svc-missing1",
            "DEV (offline)",
            "dev_offline/svc-missing1.edmx",
        );

        let report = sweep_offline_dir(&offline, &[svc_missing]).unwrap();
        assert_eq!(report.orphan_files.len(), 1);
        assert_eq!(report.missing_files.len(), 1);
        cleanup(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn sweep_skips_symlinks_in_offline_dir() {
        // A symlink that targets a file outside the offline root must
        // not be followed into the walk — that would let manually-placed
        // links escape the sweep boundary and surface files outside the
        // tool's purview as orphans.
        let dir = unique_tmp_dir("sweep_symlink");
        let offline = dir.join("offline");
        let outside = dir.join("outside");
        fs::create_dir_all(offline.join("dev_offline")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let outside_file = outside.join("secret.edmx");
        fs::write(&outside_file, b"secret").unwrap();

        let link = offline.join("dev_offline").join("link.edmx");
        let _ = std::os::unix::fs::symlink(&outside_file, &link);

        let report = sweep_offline_dir(&offline, &[]).unwrap();
        // The symlink is not followed and is not added to orphans.
        assert!(
            report.orphan_files.is_empty(),
            "symlink should be skipped: {:?}",
            report.orphan_files
        );
        cleanup(&dir);
        cleanup(&outside);
    }
}
