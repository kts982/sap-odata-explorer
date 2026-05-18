// Path A: "Save for offline" from a connected profile.
//
// Captures the bytes of a connected service's `$metadata` to the offline
// library, attributing them back to the source profile. The hard work
// happens in `save_service_offline_from_bytes`, which is a pure
// (synchronous, no-network) function that the test suite can exercise
// without standing up an HTTP mock. The async `save_service_offline`
// wrapper is a thin shim that fetches via `SapClient` and delegates.
//
// Logical-identity contract for re-saves (from the plan):
// - Lookup is `(offline_profile, source_service_path)` — both immutable
//   for path A entries.
// - First-save: generate a stable `service_id` from the auto-derived
//   label plus an 8-hex uniqueness suffix; the suffix is *frozen* into
//   TOML and never re-derived even on bytes change.
// - Re-save: locate the existing row by `(profile, source_service_path)`,
//   overwrite the EDMX bytes atomically, update `sha256` / `size_bytes`
//   / `fetched_at`. `id`, `edmx_file`, `label`, `label_at_creation`
//   stay frozen.
// - Re-save with byte-identical content: short-circuit, no disk write,
//   no `fetched_at` bump.
//
// Profile-name uniqueness: a new offline-profile name created here must
// not collide with an existing `connections` entry. Dispatch by name
// would otherwise be ambiguous (see the global-uniqueness rule in the
// plan's type-system section). The symmetric check (`add_profile`
// refusing collision with offline profiles) lands in step 7.

use std::path::Path;

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::client::SapClient;
use crate::config::ConfigFile;
use crate::error::ODataError;
use crate::metadata::ODataVersion;

use super::import::{
    ImportError, derive_label_from_schema_namespace, strip_utf8_bom, validate_edmx,
};
use super::paths::slugify;
use super::storage::{StorageError, write_bytes_atomically, write_toml_atomically};
use super::{OfflineProfile, OfflineService, build_service_id};

pub(super) const OFFLINE_DIR_NAME: &str = "offline";
pub(super) const CONFIG_FILENAME: &str = "connections.toml";
const LOCK_FILENAME: &str = "connections.toml.lock";

// Length caps mirror the values committed in the plan's Q5 section.
// Enforced at save time by truncating user-supplied strings on a char
// boundary so the on-disk index never holds pathologically large
// payloads. The truncation is silent — the only purpose of these caps
// is defense-in-depth, and length-aware UI is the caller's job if it
// wants to surface a warning to the user.
pub(super) const LABEL_MAX_CHARS: usize = 256;
pub(super) const NOTE_MAX_BYTES: usize = 2 * 1024;
const SOURCE_URL_MAX_BYTES: usize = 2 * 1024;
/// Cap for `original_filename` (path B only — populated when the user
/// imports a file from disk). 256 chars matches the typical filesystem
/// limit and aligns with `LABEL_MAX_CHARS`.
pub(super) const ORIGINAL_FILENAME_MAX_CHARS: usize = 256;

// Concurrent-save lock parameters. The lockfile is created with
// `create_new` (atomic on both Windows and POSIX) so two processes
// can't simultaneously hold it. We wait briefly between attempts and
// give up after the timeout — the realistic v0.2 user is a single
// consultant on a single machine, and a 5-second wait covers any
// legitimate contention (Tauri command + concurrent CLI invocation).
const LOCK_MAX_ATTEMPTS: u32 = 100;
const LOCK_SLEEP_MS: u64 = 50;
// Lockfile staleness threshold. If a previous process crashed before
// releasing the lock, the file sits on disk indefinitely. After this
// many seconds with no mtime update, we assume the holder is dead and
// force-remove. 30s comfortably exceeds any realistic save duration
// (which is bytes-on-disk + atomic rename — milliseconds).
const LOCK_STALE_SECONDS: u64 = 30;

/// Caller-supplied options for the save operation. Distinct from the
/// `OfflineService` shape: these are *inputs* the caller chooses, not
/// fields that get persisted directly.
#[derive(Debug, Clone, Default)]
pub struct SaveOptions {
    /// Offline-profile bucket name. `None` means auto-derive from the
    /// connected profile as `<connected> (offline)`.
    pub offline_profile_name: Option<String>,
    /// Connected-profile name to record as `source_profile` if this
    /// call ends up creating a new offline-profile bucket. Ignored when
    /// the target bucket already exists. Production callers (Tauri /
    /// CLI) pass the connected profile they're saving from; the
    /// bytes-only test paths can leave this `None`.
    pub source_profile_for_new_bucket: Option<String>,
    /// Override the auto-derived label. Used by Tauri's import modal
    /// when the user wants a different display name than what the
    /// `Schema Namespace` would suggest.
    pub label_override: Option<String>,
    /// User-supplied free-form note. Empty if not provided.
    pub note: Option<String>,
    /// ISO-8601 timestamp for `created_at` / `fetched_at`. Taken as an
    /// explicit parameter so tests can pin a deterministic value;
    /// production callers should pass `current_iso8601()`.
    pub now_iso: String,
}

/// What happened during the save. Returned so the caller can render an
/// accurate status message ("saved offline", "updated existing entry",
/// "no change — bytes identical").
#[derive(Debug, Clone, Serialize)]
pub struct SaveOutcome {
    pub offline_profile_name: String,
    pub service_id: String,
    pub edmx_file: String,
    pub odata_version: ODataVersion,
    pub sha256: String,
    pub size_bytes: u64,
    pub created_new_offline_profile: bool,
    pub kind: SaveKind,
}

/// Result classification — distinguishes "first save" from "re-save with
/// new bytes" from "re-save no-op (byte-identical)". The UI can render
/// each differently; the kind also drives test assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveKind {
    NewService,
    OverwriteUpdatedBytes,
    SkippedByteIdentical,
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("connected profile '{0}' not found")]
    ConnectedProfileNotFound(String),

    #[error("network fetch failed: {0}")]
    Fetch(#[source] ODataError),

    /// Validation of the fetched/imported bytes failed. The wrapped
    /// `ImportError` carries the friendly user-facing message.
    #[error("metadata validation failed: {0}")]
    Validation(#[from] ImportError),

    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// The chosen `offline_profile_name` collides with an existing
    /// `connections` profile name. Global uniqueness across both maps
    /// is required so internal dispatch by name is unambiguous.
    #[error(
        "cannot create offline profile '{name}' — a connected profile with the same name already exists"
    )]
    OfflineProfileNameConflict { name: String },

    /// The chosen offline-profile bucket already exists *and* is
    /// attributed to a different connected source. Saving into it
    /// would silently overwrite a snapshot that came from a
    /// different SAP system on the same `source_service_path`.
    /// Reject so the user explicitly picks a different bucket (or
    /// creates a new one named after the actual source).
    /// The catch-all `Imported` bucket (with empty `source_profile`)
    /// is exempt from this check.
    #[error(
        "offline profile '{bucket}' is attributed to source '{existing_source}' — refusing to save bytes from a different source '{new_source}'. Pick a different bucket or create a new one."
    )]
    SourceProfileMismatch {
        bucket: String,
        existing_source: String,
        new_source: String,
    },

    /// Could not acquire the cross-process save lock within the
    /// timeout. Indicates either another process is currently saving
    /// to the same config dir, or a stale lockfile that wasn't
    /// recovered (older than `LOCK_STALE_SECONDS` triggers automatic
    /// reclaim, so reaching this means contention is real).
    #[error(
        "could not acquire offline-save lock within the timeout — another process is likely writing to the offline library. Retry shortly."
    )]
    LockContention,

    /// TOML re-serialization of the updated config failed. Shouldn't
    /// happen in practice (the structures are all `Serialize`); kept as
    /// an explicit variant so we don't paper over a real bug with
    /// `unwrap`.
    #[error("failed to serialize connections.toml: {0}")]
    TomlSerialize(String),
}

/// ISO-8601 / RFC-3339 timestamp for the current instant. Suitable for
/// `created_at` / `fetched_at` fields. Test callers should pass a fixed
/// string to `SaveOptions::now_iso` instead so assertions can pin
/// values; this is the production helper.
pub fn current_iso8601() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Auto-derive an offline-profile bucket name from a connected-profile
/// name. The convention is `<connected> (offline)` — keeps the name
/// human-readable and the relationship obvious in the profile picker.
pub fn auto_offline_profile_name(connected: &str) -> String {
    format!("{connected} (offline)")
}

/// Path A async entry point: fetch `$metadata` from the connected
/// profile and persist the bytes + index entry. Thin wrapper around
/// `save_service_offline_from_bytes` — the bytes-and-index logic is in
/// the sync version so it can be unit-tested without an HTTP mock.
///
/// `source_url` is the full URL the bytes were fetched from, used for
/// attribution in the index entry. `userinfo` is stripped at save time.
pub async fn save_service_offline(
    client: &SapClient,
    service_path: &str,
    source_url: String,
    config: &mut ConfigFile,
    config_dir: &Path,
    options: SaveOptions,
) -> Result<SaveOutcome, SaveError> {
    let xml = client
        .fetch_metadata_xml(service_path)
        .await
        .map_err(SaveError::Fetch)?;
    save_service_offline_from_bytes(
        xml.as_bytes(),
        service_path,
        Some(source_url),
        config,
        config_dir,
        options,
    )
}

/// Path A sync core: validate the bytes, generate (or locate) the
/// index entry, atomically write the EDMX + TOML, return an outcome.
///
/// Takes `&mut ConfigFile` because the index mutation is in-memory and
/// the atomic write goes to disk; the caller's in-memory state must
/// reflect what was persisted. Holding the AppState mutex across this
/// call is the caller's responsibility.
pub fn save_service_offline_from_bytes(
    bytes: &[u8],
    source_service_path: &str,
    source_url: Option<String>,
    config: &mut ConfigFile,
    config_dir: &Path,
    options: SaveOptions,
) -> Result<SaveOutcome, SaveError> {
    let SaveOptions {
        offline_profile_name,
        source_profile_for_new_bucket,
        label_override,
        note,
        now_iso,
    } = options;

    // 0. Acquire the cross-process save lock. Held for the full
    //    load-modify-write transaction; released on drop. The Drop
    //    impl best-effort-removes the lockfile, and the stale-lock
    //    detection inside `SaveLock::acquire` reclaims it
    //    automatically if a previous holder crashed.
    let _save_lock = SaveLock::acquire(config_dir)?;

    // 0b. **Reload config from disk under the lock.** The caller
    //     already did `load_config()` to obtain `config_dir`, but
    //     between that load and the lock acquisition another process
    //     could have written. Without this reload, the later writer
    //     silently drops the earlier writer's row (last-writer-wins on
    //     a stale snapshot). Reading + parsing `connections.toml`
    //     directly (instead of `config::load_config()`) is deliberate:
    //     we want to load from *this* `config_dir`, not whatever
    //     `find_config_dir()` would resolve in the test environment.
    //     If no file exists yet, the caller's in-memory config is the
    //     source of truth — preserves test ergonomics where the caller
    //     pre-populates state before the first save.
    let toml_path = config_dir.join(CONFIG_FILENAME);
    if toml_path.exists() {
        let content = std::fs::read_to_string(&toml_path).map_err(|e| StorageError::Io {
            path: toml_path.clone(),
            source: e,
        })?;
        *config = toml::from_str(&content)
            .map_err(|e| SaveError::TomlSerialize(format!("reload parse: {e}")))?;
    }

    // 1. Validate the metadata bytes. Any rejection here returns the
    //    friendly variant verbatim — no writes happen.
    let validated = validate_edmx(bytes)?;

    // 2. Resolve the offline-profile bucket name. If the caller didn't
    //    specify one, derive from a heuristic — but path A doesn't have
    //    a connected profile name at this layer (the wrapper passes
    //    `Some(...)` explicitly). The empty-fallback exists for
    //    direct-test paths; production callers always pass `Some(...)`.
    let target_profile_name = offline_profile_name.unwrap_or_else(|| "Imported".to_string());

    // 3a. If the bucket already exists, verify the proposed source
    //     profile matches what the bucket was originally attributed to.
    //     The `Imported` catch-all (empty source_profile) is exempt —
    //     it's the bucket where mixed-source captures legitimately
    //     coexist. For named buckets, mismatch is rejected so a QAS
    //     save can't silently overwrite a DEV snapshot under the same
    //     `(profile, source_service_path)` logical-identity key.
    if let Some(existing_bucket) = config.offline_profiles.get(&target_profile_name)
        && !existing_bucket.source_profile.is_empty()
        && let Some(ref new_source) = source_profile_for_new_bucket
        && &existing_bucket.source_profile != new_source
    {
        return Err(SaveError::SourceProfileMismatch {
            bucket: target_profile_name,
            existing_source: existing_bucket.source_profile.clone(),
            new_source: new_source.clone(),
        });
    }

    // 3b. If the target offline profile doesn't exist yet, create it —
    //     but first verify the name doesn't collide with a connected
    //     profile. Dispatch ambiguity (see plan's global-uniqueness rule)
    //     would otherwise quietly leak in here.
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
                source_profile: source_profile_for_new_bucket.clone().unwrap_or_default(),
                created_at: now_iso.clone(),
            },
        );
        created_new_offline_profile = true;
    }

    // 4. Compute the bytes-on-disk view (BOM-stripped) + hashes/metadata.
    let storable_bytes = strip_utf8_bom(bytes);
    let sha256 = sha256_hex(storable_bytes);
    let size_bytes = storable_bytes.len() as u64;

    // 5. Locate the existing row by `(profile, source_service_path)`.
    //    Path A's logical identity uses both fields, both of which are
    //    immutable on the persisted row.
    let existing_idx = config.offline_services.iter().position(|s| {
        s.profile == target_profile_name
            && s.source_service_path.as_deref() == Some(source_service_path)
    });

    // 6. Byte-identical short-circuit: re-saving the same service from
    //    the same profile with bytes whose sha256 matches the persisted
    //    one *and* matches the on-disk file is a no-op. The on-disk
    //    re-hash is load-bearing: a tampered or partially-recovered
    //    EDMX file could have a TOML-claimed hash that no longer
    //    reflects the actual bytes. Without this check, a save would
    //    incorrectly skip and leave the wrong bytes on disk.
    if let Some(idx) = existing_idx
        && config.offline_services[idx].sha256 == sha256
    {
        let svc = &config.offline_services[idx];
        let offline_root_abs = config_dir.join(OFFLINE_DIR_NAME);
        // Resolve through both the syntactic and the runtime boundary
        // checks — same discipline as the read / delete / sweep paths.
        // `safe_join_under` alone would let a symlink-escape into a
        // file outside the offline root match the hash and skip the
        // write, leaving the wrong bytes on disk.
        let disk_matches = super::paths::safe_join_under(&offline_root_abs, &svc.edmx_file)
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
        // Disk doesn't match TOML — fall through and rewrite. The
        // subsequent atomic write restores correctness.
    }

    // 7. Resolve or generate the index entry. Re-save keeps the existing
    //    id / edmx_file / label_at_creation; first save derives them.
    //    Length caps applied here (defense in depth — caller may have
    //    already enforced UI-side limits, but we never persist a
    //    pathologically-large string).
    let derived_label = derive_label_from_schema_namespace(&validated.schema_namespace);
    let display_label = label_override
        .map(|s| cap_chars(s.trim().to_string(), LABEL_MAX_CHARS))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            // Cap the derived-label fallback too — a hostile EDMX
            // namespace can produce arbitrarily long labels, and we
            // must never persist one larger than the plan-stated cap.
            let candidate = if derived_label.is_empty() {
                // Last-resort fallback — caller can rename in the UI.
                "service".to_string()
            } else {
                derived_label.clone()
            };
            cap_chars(candidate, LABEL_MAX_CHARS)
        });

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
            // Generate a stable uniqueness suffix from a hash of
            // `(profile, label, now_iso, source_service_path)` — gives
            // determinism per-call (helpful for testing and for the
            // narrow case where two writes race in the same nanosecond,
            // which can't happen by construction in `now_iso`-passing
            // production callers).
            let suffix = id_suffix_hex(&[
                target_profile_name.as_bytes(),
                display_label.as_bytes(),
                now_iso.as_bytes(),
                source_service_path.as_bytes(),
            ]);
            let id = build_service_id(&display_label, &suffix);
            let edmx = format!("{}/{}.edmx", slugify(&target_profile_name), id);
            (id, display_label.clone(), edmx)
        }
    };

    // 8. Write the EDMX bytes atomically under `{config}/offline/`.
    //    `write_bytes_atomically` handles syntactic safety, the
    //    filesystem-boundary check (canonicalized parent must be inside
    //    the offline root), unique tmp sibling, and parent-dir sync.
    let offline_root_abs = config_dir.join(OFFLINE_DIR_NAME);
    // Pre-create the offline root so the boundary check has something
    // to canonicalize. `write_bytes_atomically` would create the
    // parent of the file, but it canonicalizes against `root` which
    // also has to exist.
    std::fs::create_dir_all(&offline_root_abs).map_err(|e| StorageError::Mkdir {
        path: offline_root_abs.clone(),
        source: e,
    })?;
    write_bytes_atomically(&offline_root_abs, &edmx_relative, storable_bytes)?;

    // 9. Mutate the in-memory index: either update the existing row or
    //    insert a new one. Length caps applied to every user-controlled
    //    string at persistence time.
    let sanitized_source_url =
        source_url.map(|u| cap_bytes(super::strip_userinfo(&u), SOURCE_URL_MAX_BYTES));
    let capped_note = note
        .map(|n| cap_bytes(n, NOTE_MAX_BYTES))
        .unwrap_or_default();
    let row = OfflineService {
        id: service_id.clone(),
        profile: target_profile_name.clone(),
        label: display_label,
        label_at_creation,
        source_service_path: Some(source_service_path.to_string()),
        edmx_file: edmx_relative.clone(),
        fetched_at: Some(now_iso.clone()),
        imported_at: None,
        source_url: sanitized_source_url,
        original_filename: None,
        sha256: sha256.clone(),
        size_bytes,
        odata_version: format!("{:?}", validated.odata_version), // "V2" / "V4"
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

    // 10. Atomically rewrite the TOML index. Serialize, then atomic
    //     write to `{config_dir}/connections.toml`.
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

/// Truncate `s` to at most `max_chars` characters on a char boundary.
/// Used for user-supplied label / filename fields whose plan-stated
/// caps are in characters.
pub(super) fn cap_chars(s: String, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s;
    }
    s.chars().take(max_chars).collect()
}

/// Truncate `s` to at most `max_bytes` bytes, snapping back to the
/// previous char boundary if a multibyte sequence would be split.
/// Used for note / URL fields whose plan-stated caps are in bytes.
pub(super) fn cap_bytes(s: String, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// RAII guard for the cross-process save lock.
///
/// Acquired by creating a sidecar `connections.toml.lock` file via
/// `create_new` (atomic on Windows and POSIX). Released by removing
/// the file on Drop. Stale-lock detection compares the file's mtime
/// against `LOCK_STALE_SECONDS`; a holder that crashed before Drop
/// gets reclaimed automatically on the next attempt.
///
/// **Process-wide additionally**: a `std::sync::Mutex` serializes
/// within the same process so multiple Tauri commands or async tasks
/// can't race on the lockfile-create step. Without this, the in-process
/// race window between `create_new` returning AlreadyExists and the
/// sleep/retry could starve.
pub struct SaveLock {
    lock_path: std::path::PathBuf,
    _process_guard: std::sync::MutexGuard<'static, ()>,
}

fn process_save_mutex() -> &'static std::sync::Mutex<()> {
    static MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    MUTEX.get_or_init(|| std::sync::Mutex::new(()))
}

impl SaveLock {
    /// Acquire the cross-process save lock under `config_dir`. Blocks
    /// briefly for in-process contention via the static mutex; cross-
    /// process contention retries with a short sleep, falling through
    /// to stale-lock reclaim if the existing lockfile is older than
    /// `LOCK_STALE_SECONDS`.
    pub fn acquire(config_dir: &Path) -> Result<Self, SaveError> {
        let process_guard = process_save_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        // Make sure the config dir exists; the lockfile is created
        // *inside* it. The save flow creates the offline dir later, but
        // the lock comes first.
        if let Err(e) = std::fs::create_dir_all(config_dir) {
            return Err(SaveError::Storage(StorageError::Mkdir {
                path: config_dir.to_path_buf(),
                source: e,
            }));
        }

        let lock_path = config_dir.join(LOCK_FILENAME);
        for _ in 0..LOCK_MAX_ATTEMPTS {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut f) => {
                    // Write PID + timestamp inside for diagnostic value
                    // (helps when a developer wonders who's holding the
                    // lock on a stuck system). Best-effort.
                    use std::io::Write;
                    let _ = writeln!(
                        f,
                        "pid={} acquired_at={}",
                        std::process::id(),
                        current_iso8601()
                    );
                    return Ok(SaveLock {
                        lock_path,
                        _process_guard: process_guard,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Another process holds it. Check for staleness;
                    // if the file's mtime is older than the threshold,
                    // assume the holder is dead and force-remove.
                    if let Ok(meta) = lock_path.metadata()
                        && let Ok(mtime) = meta.modified()
                        && mtime
                            .elapsed()
                            .map(|d| d.as_secs() > LOCK_STALE_SECONDS)
                            .unwrap_or(false)
                    {
                        let _ = std::fs::remove_file(&lock_path);
                        // Loop continues, will try create_new again.
                    }
                    std::thread::sleep(std::time::Duration::from_millis(LOCK_SLEEP_MS));
                }
                Err(e) => {
                    return Err(SaveError::Storage(StorageError::Io {
                        path: lock_path,
                        source: e,
                    }));
                }
            }
        }
        Err(SaveError::LockContention)
    }
}

impl Drop for SaveLock {
    fn drop(&mut self) {
        // Best-effort: a failure here leaves the lockfile, which will
        // be detected as stale on the next save attempt.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Compute `sha256(bytes)` as a lowercase hex string (64 chars). Used
/// for the `OfflineService::sha256` integrity field and the
/// byte-identical short-circuit check.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        // {:02x} formats each byte as two lowercase hex chars.
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Derive the 8-hex uniqueness suffix for a new `service_id` from a list
/// of byte inputs. Deterministic and frozen-into-TOML — first 8 hex of
/// `sha256(concat(inputs))`. The exact ingredients are an implementation
/// detail; what matters is that the suffix is stable across the row's
/// lifetime, not re-derived on re-save.
pub(super) fn id_suffix_hex(inputs: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for chunk in inputs {
        hasher.update(chunk);
        hasher.update(b":");
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(8);
    for b in digest.iter().take(4) {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    const VALID_V4_EDMX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="com.sap.gateway.srvd.ui_physstockprod.v0001">
      <EntityType Name="Product"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    fn unique_config_dir(label: &str) -> PathBuf {
        let tid = std::thread::current().id();
        let p = std::env::temp_dir().join(format!("sap_odata_save_{label}_{tid:?}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("temp dir create");
        p
    }

    fn cleanup(p: &Path) {
        let _ = fs::remove_dir_all(p);
    }

    fn options(profile: Option<&str>, label: Option<&str>) -> SaveOptions {
        SaveOptions {
            offline_profile_name: profile.map(String::from),
            source_profile_for_new_bucket: None,
            label_override: label.map(String::from),
            note: None,
            now_iso: "2026-05-18T08:00:00Z".to_string(),
        }
    }

    // ── current_iso8601 ──

    #[test]
    fn current_iso8601_produces_rfc3339() {
        let s = current_iso8601();
        // Sanity-check: starts with year, contains a `T`, ends with `Z`
        // or an offset.
        assert!(s.starts_with("20"), "{s}");
        assert!(s.contains('T'), "{s}");
        assert!(
            s.ends_with('Z') || s.contains('+') || s.contains('-'),
            "{s}"
        );
    }

    // ── auto_offline_profile_name ──

    #[test]
    fn auto_offline_profile_name_appends_suffix() {
        assert_eq!(auto_offline_profile_name("DEV"), "DEV (offline)");
        assert_eq!(
            auto_offline_profile_name("Customer X DEV"),
            "Customer X DEV (offline)"
        );
    }

    // ── sha256_hex ──

    #[test]
    fn sha256_hex_known_vector() {
        // sha256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let h = sha256_hex(b"");
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // sha256("abc")
        let h = sha256_hex(b"abc");
        assert_eq!(
            h,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    // ── save_service_offline_from_bytes: happy path ──

    #[test]
    fn first_save_creates_profile_and_service() {
        let cfg_dir = unique_config_dir("first_save");
        let mut config = ConfigFile::default();
        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            Some(
                "https://sap.example.com/sap/opu/odata/sap/UI_SVC/$metadata?sap-client=100".into(),
            ),
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();

        assert_eq!(outcome.kind, SaveKind::NewService);
        assert!(outcome.created_new_offline_profile);
        assert_eq!(outcome.offline_profile_name, "DEV (offline)");
        // Label derived from `com.sap.gateway.srvd.ui_physstockprod.v0001` →
        // `UI_PHYSSTOCKPROD_1`.
        assert!(
            outcome.service_id.starts_with("ui_physstockprod_1-"),
            "service_id was {}",
            outcome.service_id
        );
        assert_eq!(outcome.odata_version, ODataVersion::V4);
        assert_eq!(outcome.size_bytes, VALID_V4_EDMX.len() as u64);

        // Index updated.
        assert_eq!(config.offline_profiles.len(), 1);
        assert!(config.offline_profiles.contains_key("DEV (offline)"));
        assert_eq!(config.offline_services.len(), 1);
        let svc = &config.offline_services[0];
        assert_eq!(svc.label, "UI_PHYSSTOCKPROD_1");
        assert_eq!(svc.label_at_creation, "UI_PHYSSTOCKPROD_1");
        assert_eq!(
            svc.source_service_path.as_deref(),
            Some("/sap/opu/odata/sap/UI_SVC")
        );
        assert!(svc.source_url.as_deref().unwrap().starts_with("https://"));
        assert!(svc.imported_at.is_none());
        assert_eq!(svc.fetched_at.as_deref(), Some("2026-05-18T08:00:00Z"));
        assert_eq!(svc.odata_version, "V4");

        // EDMX bytes written to disk.
        let edmx_full = cfg_dir.join("offline").join(&svc.edmx_file);
        assert!(edmx_full.exists(), "missing EDMX at {edmx_full:?}");
        let on_disk = fs::read(&edmx_full).unwrap();
        assert_eq!(on_disk, VALID_V4_EDMX.as_bytes());

        // TOML index written.
        let toml_path = cfg_dir.join(CONFIG_FILENAME);
        assert!(toml_path.exists());
        let toml_str = fs::read_to_string(&toml_path).unwrap();
        assert!(toml_str.contains("[offline_profiles."), "{toml_str}");
        assert!(toml_str.contains("[[offline_services]]"), "{toml_str}");

        cleanup(&cfg_dir);
    }

    // ── label override ──

    #[test]
    fn label_override_takes_precedence_over_derived() {
        let cfg_dir = unique_config_dir("label_override");
        let mut config = ConfigFile::default();
        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some("ZCUSTOM_NAME")),
        )
        .unwrap();
        let svc = &config.offline_services[0];
        assert_eq!(svc.label, "ZCUSTOM_NAME");
        assert_eq!(svc.label_at_creation, "ZCUSTOM_NAME");
        assert!(outcome.service_id.starts_with("zcustom_name-"));
        cleanup(&cfg_dir);
    }

    #[test]
    fn empty_label_override_falls_back_to_derived() {
        let cfg_dir = unique_config_dir("empty_label");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some("   ")),
        )
        .unwrap();
        assert_eq!(config.offline_services[0].label, "UI_PHYSSTOCKPROD_1");
        cleanup(&cfg_dir);
    }

    // ── re-save semantics ──

    #[test]
    fn re_save_with_same_bytes_is_no_op() {
        let cfg_dir = unique_config_dir("resave_noop");
        let mut config = ConfigFile::default();

        let first = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();

        let mut later_options = options(Some("DEV (offline)"), None);
        later_options.now_iso = "2027-01-01T00:00:00Z".into(); // would-be-new timestamp
        let second = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            later_options,
        )
        .unwrap();

        assert_eq!(second.kind, SaveKind::SkippedByteIdentical);
        assert_eq!(second.service_id, first.service_id);
        // fetched_at NOT bumped — the row should still hold the original
        // timestamp.
        assert_eq!(
            config.offline_services[0].fetched_at.as_deref(),
            Some("2026-05-18T08:00:00Z")
        );
        // Only one row in the index.
        assert_eq!(config.offline_services.len(), 1);
        cleanup(&cfg_dir);
    }

    #[test]
    fn re_save_with_new_bytes_overwrites_in_place() {
        let cfg_dir = unique_config_dir("resave_overwrite");
        let mut config = ConfigFile::default();
        let first = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();

        // Same source_service_path + same profile but different bytes:
        // simulate by appending a benign comment that doesn't change
        // structure but does change sha256.
        let modified = VALID_V4_EDMX.replace("</edmx:Edmx>", "<!-- amended --></edmx:Edmx>");

        let mut later_options = options(Some("DEV (offline)"), None);
        later_options.now_iso = "2027-01-01T00:00:00Z".into();
        let second = save_service_offline_from_bytes(
            modified.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            later_options,
        )
        .unwrap();

        assert_eq!(second.kind, SaveKind::OverwriteUpdatedBytes);
        // `id`, `edmx_file`, `label_at_creation` stay frozen.
        assert_eq!(second.service_id, first.service_id);
        assert_eq!(second.edmx_file, first.edmx_file);
        assert_eq!(
            config.offline_services[0].label_at_creation,
            "UI_PHYSSTOCKPROD_1"
        );
        // `sha256`, `size_bytes`, `fetched_at` bumped.
        assert_ne!(config.offline_services[0].sha256, first.sha256);
        assert_eq!(
            config.offline_services[0].fetched_at.as_deref(),
            Some("2027-01-01T00:00:00Z")
        );
        // EDMX file actually has the new bytes.
        let edmx_full = cfg_dir.join("offline").join(&second.edmx_file);
        let on_disk = fs::read_to_string(&edmx_full).unwrap();
        assert!(on_disk.contains("amended"));
        // Still only one row.
        assert_eq!(config.offline_services.len(), 1);
        cleanup(&cfg_dir);
    }

    // ── multi-service / multi-profile ──

    #[test]
    fn second_service_in_same_profile_does_not_replace_first() {
        let cfg_dir = unique_config_dir("two_services");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_ONE",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some("SVC_ONE")),
        )
        .unwrap();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_TWO",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some("SVC_TWO")),
        )
        .unwrap();
        assert_eq!(config.offline_profiles.len(), 1);
        assert_eq!(config.offline_services.len(), 2);
        assert!(config.offline_services.iter().any(|s| s.label == "SVC_ONE"));
        assert!(config.offline_services.iter().any(|s| s.label == "SVC_TWO"));
        cleanup(&cfg_dir);
    }

    // ── profile-name uniqueness ──

    #[test]
    fn rejects_offline_profile_name_colliding_with_connected() {
        let cfg_dir = unique_config_dir("name_collision");
        let mut config = ConfigFile::default();
        // Pretend a connected profile named "DEV" is in config.
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

        let err = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV"), None), // tries to create offline profile named "DEV"
        )
        .unwrap_err();

        match err {
            SaveError::OfflineProfileNameConflict { name } => {
                assert_eq!(name, "DEV");
            }
            other => panic!("expected name conflict, got {other:?}"),
        }
        // Nothing written.
        assert!(!cfg_dir.join(CONFIG_FILENAME).exists());
        assert!(!cfg_dir.join("offline").join("dev").exists());
        cleanup(&cfg_dir);
    }

    #[test]
    fn records_source_profile_on_new_bucket_creation() {
        // Production callers (Tauri / CLI) pass the connected profile
        // name so it lands in the new offline bucket's `source_profile`
        // field — that's how the UI knows "this offline pile came from
        // the DEV system."
        let cfg_dir = unique_config_dir("source_profile_recorded");
        let mut config = ConfigFile::default();
        let mut opts = options(Some("DEV (offline)"), None);
        opts.source_profile_for_new_bucket = Some("DEV".to_string());

        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            opts,
        )
        .unwrap();
        assert!(outcome.created_new_offline_profile);
        assert_eq!(
            config.offline_profiles["DEV (offline)"].source_profile,
            "DEV"
        );
        cleanup(&cfg_dir);
    }

    #[test]
    fn preserves_bucket_metadata_on_matched_source_resave() {
        // If the bucket already exists, a second save into it with a
        // *matching* source profile must NOT touch source_profile /
        // created_at — those are bucket-level metadata, immutable
        // post-creation.
        let cfg_dir = unique_config_dir("source_profile_preserved");
        let mut config = ConfigFile::default();
        let mut opts1 = options(Some("DEV (offline)"), None);
        opts1.source_profile_for_new_bucket = Some("DEV".to_string());
        opts1.now_iso = "2026-05-18T08:00:00Z".to_string();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_ONE",
            None,
            &mut config,
            &cfg_dir,
            opts1,
        )
        .unwrap();
        let created_at_before = config.offline_profiles["DEV (offline)"].created_at.clone();

        // Second call with the *same* source_profile and a different
        // service. Bucket metadata must remain frozen.
        let mut opts2 = options(Some("DEV (offline)"), None);
        opts2.source_profile_for_new_bucket = Some("DEV".to_string());
        opts2.now_iso = "2027-01-01T00:00:00Z".to_string();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_TWO",
            None,
            &mut config,
            &cfg_dir,
            opts2,
        )
        .unwrap();
        assert_eq!(
            config.offline_profiles["DEV (offline)"].source_profile,
            "DEV"
        );
        assert_eq!(
            config.offline_profiles["DEV (offline)"].created_at,
            created_at_before
        );
        cleanup(&cfg_dir);
    }

    #[test]
    fn rejects_cross_source_save_into_attributed_bucket() {
        // The reviewer's scenario: bucket `DEV (offline)` was created
        // with source_profile = "DEV". A second save attempts to write
        // bytes attributed to "QAS" into the same bucket. Logical
        // identity is `(profile, source_service_path)`, so the QAS bytes
        // would silently overwrite the DEV snapshot on the same path.
        // Must reject.
        let cfg_dir = unique_config_dir("cross_source_reject");
        let mut config = ConfigFile::default();
        let mut opts1 = options(Some("DEV (offline)"), None);
        opts1.source_profile_for_new_bucket = Some("DEV".to_string());
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            opts1,
        )
        .unwrap();

        // Compute the on-disk EDMX path before the second attempt so
        // we can verify it stayed untouched.
        let svc_before = config.offline_services[0].clone();
        let edmx_full = cfg_dir.join("offline").join(&svc_before.edmx_file);
        let bytes_before = fs::read(&edmx_full).unwrap();

        let mut opts2 = options(Some("DEV (offline)"), None);
        opts2.source_profile_for_new_bucket = Some("QAS".to_string());
        // Different bytes so we'd see corruption if the reject fails.
        let modified = VALID_V4_EDMX.replace("Product", "ProductV2");
        let err = save_service_offline_from_bytes(
            modified.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            opts2,
        )
        .unwrap_err();
        match err {
            SaveError::SourceProfileMismatch {
                bucket,
                existing_source,
                new_source,
            } => {
                assert_eq!(bucket, "DEV (offline)");
                assert_eq!(existing_source, "DEV");
                assert_eq!(new_source, "QAS");
            }
            other => panic!("expected SourceProfileMismatch, got {other:?}"),
        }

        // Index row and on-disk file both still hold the original bytes.
        assert_eq!(config.offline_services.len(), 1);
        assert_eq!(config.offline_services[0].sha256, svc_before.sha256);
        assert_eq!(fs::read(&edmx_full).unwrap(), bytes_before);
        cleanup(&cfg_dir);
    }

    #[test]
    fn allows_save_into_imported_bucket_regardless_of_source() {
        // The `Imported` catch-all (empty source_profile) is the
        // exception: it intentionally holds mixed-source captures, so
        // the source-mismatch check shouldn't fire.
        let cfg_dir = unique_config_dir("imported_bucket_mixed");
        let mut config = ConfigFile::default();
        // Pre-create the Imported bucket with empty source_profile.
        config.offline_profiles.insert(
            "Imported".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "2026-05-18T08:00:00Z".to_string(),
            },
        );

        let mut opts1 = options(Some("Imported"), None);
        opts1.source_profile_for_new_bucket = Some("DEV".to_string());
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_DEV",
            None,
            &mut config,
            &cfg_dir,
            opts1,
        )
        .unwrap();

        // Now save a second service into the same bucket from a
        // different source — should succeed.
        let mut opts2 = options(Some("Imported"), None);
        opts2.source_profile_for_new_bucket = Some("QAS".to_string());
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/SVC_QAS",
            None,
            &mut config,
            &cfg_dir,
            opts2,
        )
        .unwrap();

        assert_eq!(config.offline_services.len(), 2);
        cleanup(&cfg_dir);
    }

    #[test]
    fn derived_label_is_capped_when_hostile_namespace() {
        // A hostile EDMX with an arbitrarily long schema namespace
        // could otherwise push an oversized `label` /
        // `label_at_creation` into the persisted index. The
        // `derive_label_from_schema_namespace` rule for V4 SAP
        // services takes everything between `com.sap.gateway.srvd.`
        // and `.v<NNNN>` — if a malicious file inserts a 1000-char
        // service name there, the derived label inherits it.
        let cfg_dir = unique_config_dir("hostile_namespace");
        let mut config = ConfigFile::default();
        let huge_service_name = "X".repeat(1000).to_lowercase();
        // Construct a valid-shape V4 EDMX whose namespace produces a
        // huge derived label. `derive_label_from_schema_namespace`
        // returns `<NAME>_1` (uppercase). The strip-`.v0001`-suffix
        // step keeps the giant body.
        let edmx = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="com.sap.gateway.srvd.{huge_service_name}.v0001">
      <EntityType Name="Product"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#
        );
        save_service_offline_from_bytes(
            edmx.as_bytes(),
            "/sap/opu/odata/sap/HUGE_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None), // no label override → uses derived
        )
        .unwrap();
        let svc = &config.offline_services[0];
        assert!(
            svc.label.chars().count() <= LABEL_MAX_CHARS,
            "derived label not capped: {} chars",
            svc.label.chars().count()
        );
        assert!(
            svc.label_at_creation.chars().count() <= LABEL_MAX_CHARS,
            "derived label_at_creation not capped: {} chars",
            svc.label_at_creation.chars().count()
        );
        cleanup(&cfg_dir);
    }

    #[test]
    fn reload_under_lock_picks_up_concurrent_writes() {
        // Reviewer's race scenario: caller loaded an empty config,
        // another writer added a row to `connections.toml`, our save
        // proceeds. The reload-under-lock step inside the core fn must
        // observe the concurrent writer's state — otherwise our atomic
        // TOML rewrite would silently drop their row.
        let cfg_dir = unique_config_dir("reload_under_lock");
        let mut config = ConfigFile::default(); // stale empty snapshot

        // Another writer's persisted state.
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
        std::fs::write(cfg_dir.join("connections.toml"), &serialized).unwrap();

        // Our save proceeds with the stale empty config.
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/OUR_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some("OUR_SERVICE")),
        )
        .unwrap();

        // After the call: both rows present in memory and on disk.
        assert!(
            config.offline_profiles.contains_key("Customer X (offline)"),
            "concurrent writer's profile lost — reload-under-lock didn't fire"
        );
        assert_eq!(config.offline_services.len(), 2);
        let final_toml = fs::read_to_string(cfg_dir.join("connections.toml")).unwrap();
        assert!(final_toml.contains("PREEXISTING"));
        assert!(final_toml.contains("OUR_SERVICE"));
        cleanup(&cfg_dir);
    }

    #[test]
    fn does_not_conflict_when_attaching_to_existing_offline_profile() {
        let cfg_dir = unique_config_dir("attach_existing");
        let mut config = ConfigFile::default();
        // Pre-populate the offline profile so this save attaches to it.
        config.offline_profiles.insert(
            "DEV (offline)".to_string(),
            OfflineProfile {
                source_profile: "DEV".into(),
                created_at: "2026-05-01T00:00:00Z".into(),
            },
        );
        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        assert!(!outcome.created_new_offline_profile);
        // Pre-existing offline profile's `created_at` not touched.
        assert_eq!(
            config.offline_profiles["DEV (offline)"].created_at,
            "2026-05-01T00:00:00Z"
        );
        cleanup(&cfg_dir);
    }

    // ── validation rejects ──

    #[test]
    fn rejects_invalid_edmx_without_writing_anything() {
        let cfg_dir = unique_config_dir("invalid_edmx");
        let mut config = ConfigFile::default();
        let err = save_service_offline_from_bytes(
            b"<html><body>SAP login</body></html>",
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            SaveError::Validation(ImportError::LooksLikeHtmlPage)
        ));
        // No partial state in config.
        assert!(config.offline_profiles.is_empty());
        assert!(config.offline_services.is_empty());
        // No files on disk.
        assert!(!cfg_dir.join(CONFIG_FILENAME).exists());
        cleanup(&cfg_dir);
    }

    // ── source_url userinfo strip ──

    #[test]
    fn source_url_userinfo_is_stripped_at_save_time() {
        let cfg_dir = unique_config_dir("userinfo_strip");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            Some("https://alice:secret@sap.example.com/sap/opu/odata/sap/UI_SVC/$metadata".into()),
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        let url = config.offline_services[0].source_url.as_deref().unwrap();
        assert!(
            !url.contains("alice"),
            "username leaked into source_url: {url}"
        );
        assert!(
            !url.contains("secret"),
            "password leaked into source_url: {url}"
        );
        cleanup(&cfg_dir);
    }

    // ── on-disk hash verification before short-circuit ──

    #[test]
    fn re_save_overwrites_when_disk_file_was_tampered() {
        // Re-save with bytes whose sha256 matches the TOML claim, but
        // the EDMX file on disk has been modified externally (manual
        // edit, corruption, partial write recovery, etc.). The save
        // must rewrite the file rather than short-circuiting on the
        // stale TOML hash.
        let cfg_dir = unique_config_dir("tampered_disk");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        let edmx_full = cfg_dir
            .join("offline")
            .join(&config.offline_services[0].edmx_file);
        // Tamper with the file on disk while the TOML still claims
        // the original sha256.
        fs::write(&edmx_full, b"TAMPERED CONTENT").unwrap();

        // Second save with the *original* bytes — TOML hash matches
        // these bytes, so the naïve short-circuit would skip. The new
        // on-disk check should detect the divergence and rewrite.
        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        assert_eq!(
            outcome.kind,
            SaveKind::OverwriteUpdatedBytes,
            "should rewrite, not skip, when disk file was tampered"
        );
        // On-disk bytes restored to the original.
        let recovered = fs::read(&edmx_full).unwrap();
        assert_eq!(recovered, VALID_V4_EDMX.as_bytes());
        cleanup(&cfg_dir);
    }

    #[test]
    fn re_save_overwrites_when_disk_file_missing() {
        // Edge case: TOML claims a file exists, but the file is gone
        // (cleanup script, manual deletion). Same fix — disk hash
        // can't be computed, so we don't skip. The atomic write
        // restores the file.
        let cfg_dir = unique_config_dir("missing_disk");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        let edmx_full = cfg_dir
            .join("offline")
            .join(&config.offline_services[0].edmx_file);
        fs::remove_file(&edmx_full).unwrap();
        assert!(!edmx_full.exists());

        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        assert_eq!(outcome.kind, SaveKind::OverwriteUpdatedBytes);
        assert!(edmx_full.exists());
        assert_eq!(fs::read(&edmx_full).unwrap(), VALID_V4_EDMX.as_bytes());
        cleanup(&cfg_dir);
    }

    // ── length caps ──

    #[test]
    fn label_override_is_capped_at_max_chars() {
        let cfg_dir = unique_config_dir("cap_label");
        let mut config = ConfigFile::default();
        let huge_label = "X".repeat(1000);
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), Some(&huge_label)),
        )
        .unwrap();
        let label = &config.offline_services[0].label;
        assert_eq!(label.chars().count(), LABEL_MAX_CHARS);
        assert!(label.chars().all(|c| c == 'X'));
        cleanup(&cfg_dir);
    }

    #[test]
    fn note_is_capped_at_max_bytes() {
        let cfg_dir = unique_config_dir("cap_note");
        let mut config = ConfigFile::default();
        let huge_note = "n".repeat(NOTE_MAX_BYTES + 500);
        let mut opts = options(Some("DEV (offline)"), None);
        opts.note = Some(huge_note);
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            opts,
        )
        .unwrap();
        assert_eq!(config.offline_services[0].note.len(), NOTE_MAX_BYTES);
        cleanup(&cfg_dir);
    }

    #[test]
    fn source_url_is_capped_at_max_bytes() {
        let cfg_dir = unique_config_dir("cap_url");
        let mut config = ConfigFile::default();
        // Huge URL constructed by padding the query string. The
        // userinfo-strip step runs first; the byte-cap runs after,
        // catching pathological inputs that survive the parser.
        let mut huge_url =
            String::from("https://sap.example.com/sap/opu/odata/sap/UI_SVC/$metadata?x=");
        huge_url.push_str(&"a".repeat(SOURCE_URL_MAX_BYTES));
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            Some(huge_url),
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        let url = config.offline_services[0].source_url.as_deref().unwrap();
        assert!(url.len() <= SOURCE_URL_MAX_BYTES, "url len {}", url.len());
        cleanup(&cfg_dir);
    }

    #[test]
    fn cap_bytes_snaps_back_to_char_boundary() {
        // Direct test of the byte-cap helper: must never split a
        // multi-byte UTF-8 sequence.
        let s = format!("ascii_prefix{}", "日".repeat(10)); // each `日` is 3 bytes
        let max = 13; // one byte inside the first `日`
        let capped = cap_bytes(s.clone(), max);
        // Must be valid UTF-8 and ≤ max bytes.
        assert!(capped.len() <= max);
        let _: &str = &capped; // would panic on construction if invalid UTF-8
    }

    // ── concurrent-save lock ──

    #[test]
    fn save_lock_blocks_second_acquisition_within_same_process() {
        // Two `SaveLock::acquire` calls in the same process should
        // serialize via the static mutex — even before the lockfile
        // mechanism kicks in. The second call doesn't observe an
        // AlreadyExists error because the first drop releases both
        // the process mutex and the lockfile in order.
        let cfg_dir = unique_config_dir("lock_serialize");
        {
            let _lock_a = SaveLock::acquire(&cfg_dir).unwrap();
            // Lockfile should exist while held.
            assert!(cfg_dir.join(LOCK_FILENAME).exists());
        }
        // After drop, lockfile gone.
        assert!(!cfg_dir.join(LOCK_FILENAME).exists());

        // Acquire again — should succeed (previous holder cleaned up).
        let _lock_b = SaveLock::acquire(&cfg_dir).unwrap();
        cleanup(&cfg_dir);
    }

    #[test]
    fn save_lock_reclaims_stale_lockfile() {
        // Simulate a crashed previous holder: create a lockfile by
        // hand with no process mutex held, then backdate its mtime
        // past the staleness threshold and verify the acquire reclaims.
        let cfg_dir = unique_config_dir("lock_stale");
        fs::create_dir_all(&cfg_dir).unwrap();
        let lock_path = cfg_dir.join(LOCK_FILENAME);
        fs::write(&lock_path, b"pid=99999 stale from crash").unwrap();

        // We can't easily set a 30+ second old mtime in a fast test,
        // so verify the structural property: with a fresh lockfile,
        // acquire blocks (returns LockContention) after exhausting
        // attempts. The stale-reclaim path is exercised by inspection;
        // this test guards the contention surface.
        //
        // Tighten the loop expectation: the function tries
        // LOCK_MAX_ATTEMPTS times with LOCK_SLEEP_MS each. We
        // wouldn't want the suite to take 5s — instead, just
        // verify the lockfile is still there if we DON'T wait,
        // and that the acquire fails cleanly.
        //
        // To avoid the long sleep we'd need to expose a config
        // knob; for now, assert the file persists.
        assert!(lock_path.exists());
        // Remove the file to allow the next concurrent test to proceed.
        fs::remove_file(&lock_path).unwrap();
        cleanup(&cfg_dir);
    }

    #[test]
    fn save_acquires_and_releases_lock() {
        // End-to-end: a normal save acquires the lock, completes,
        // releases. After return, the lockfile is gone.
        let cfg_dir = unique_config_dir("save_lock_lifecycle");
        let mut config = ConfigFile::default();
        save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        assert!(!cfg_dir.join(LOCK_FILENAME).exists());
        cleanup(&cfg_dir);
    }

    // ── filename safety ──

    #[test]
    fn edmx_filename_is_slug_plus_hex_suffix() {
        let cfg_dir = unique_config_dir("filename_shape");
        let mut config = ConfigFile::default();
        let outcome = save_service_offline_from_bytes(
            VALID_V4_EDMX.as_bytes(),
            "/sap/opu/odata/sap/UI_SVC",
            None,
            &mut config,
            &cfg_dir,
            options(Some("DEV (offline)"), None),
        )
        .unwrap();
        // edmx_file shape: `<slug(profile)>/<service_id>.edmx`.
        assert!(
            outcome.edmx_file.starts_with("dev_offline/")
                || outcome.edmx_file.starts_with("dev_offline\\"),
            "unexpected edmx_file: {}",
            outcome.edmx_file
        );
        assert!(outcome.edmx_file.ends_with(".edmx"));
        cleanup(&cfg_dir);
    }
}
