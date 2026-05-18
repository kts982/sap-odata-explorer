// Offline EDMX library — types, dispatch enum, and security primitives.
//
// Two ingestion paths feed the library:
// - Path A: "Save for offline" on a connected profile captures the bytes of
//   `$metadata` to local storage, attributed back to the source profile.
// - Path B: "Open EDMX file" imports a file the user pulled outside the tool
//   (API Hub, /IWFND/GW_CLIENT "Save Response", browser save-as on
//   `<base>/$metadata`, `curl > out.xml`, etc.).
//
// Within the library, services are grouped under `OfflineProfile` buckets
// that share an origin: a real connected system (`source_profile = "DEV"`)
// or the catch-all `Imported` bucket for path-B uploads.
//
// `MetadataSource` is the dispatch enum: every Tauri command that reads
// metadata or executes a query takes one of these and either fetches live
// or reads from the offline cache. Network-touching commands MUST call
// `assert_network_allowed` first — UI-side button disabling is defense in
// depth, not the primary guard. See `tauri-app/src-tauri/src/main.rs`'s
// `resolve_value_list_reference` for the canonical case the reviewer
// flagged: a command that would otherwise silently build a live SapClient
// even for an offline-source request.
//
// Path safety primitives live in the `paths` submodule and are exercised
// by every read/delete path. URL sanitization lives in `url_sanitize` and
// runs at save time on `source_url`.

pub mod delete;
pub mod import;
pub mod import_file;
pub mod paths;
pub mod read;
pub mod save;
pub mod storage;
pub mod url_sanitize;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use delete::{
    DeleteError, DeleteProfileOutcome, DeleteServiceOutcome, delete_offline_profile,
    delete_offline_service,
};
pub use import::{
    ImportError, MAX_IMPORT_SIZE_BYTES, ValidatedEdmx, derive_label_from_schema_namespace,
    strip_utf8_bom, validate_edmx,
};
pub use import_file::{ImportOptions, import_edmx_file, import_edmx_from_bytes};
pub use paths::{PathError, canonicalize_under, safe_join_under, slugify};
pub use read::{OfflineReadError, read_offline_metadata};
pub use save::{
    SaveError, SaveKind, SaveOptions, SaveOutcome, auto_offline_profile_name, current_iso8601,
    save_service_offline, save_service_offline_from_bytes,
};
// `check_connected_profile_name_available` lives directly in `mod.rs`;
// no re-export needed since `crate::offline::check_...` already works.
pub use storage::{
    MissingService, OfflineSweepReport, StorageError, sweep_offline_dir, write_bytes_atomically,
    write_toml_atomically,
};
pub use url_sanitize::strip_userinfo;

/// A bucket grouping offline services that share an origin. The `name` is
/// the BTreeMap key in `ConfigFile::offline_profiles`; uniqueness is enforced
/// globally across `connections` + `offline_profiles` at add-time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineProfile {
    /// Name of the connected `ConnectionProfile` this bucket was captured
    /// from, when known. Empty string for the catch-all `Imported` bucket
    /// (path-B uploads with no source system identity).
    #[serde(default)]
    pub source_profile: String,
    /// UTC ISO-8601 timestamp the bucket was created.
    pub created_at: String,
}

/// A single captured service inside an offline profile. Primary key is
/// `id` — a **stable logical identifier** generated at creation time,
/// shaped `<slug(label_at_creation)>-<8-hex-uniqueness-suffix>`. The id
/// **never changes** after the first save, even if:
///
/// - the content bytes change (re-save / fresh fetch)
/// - the display `label` is edited
/// - the `note` is edited
///
/// This stability matters because open tabs, query history, favorites, and
/// trace contexts all reference services by id. If id were content-
/// addressed (re-derived from `sha256(bytes)` on every save), every re-save
/// would invalidate those references and produce dead links in the UI.
///
/// The 8-hex suffix is generated once at creation; typical sources are the
/// first 8 hex of `sha256(profile + ":" + label + ":" + created_at)`,
/// `sha256(content)` at creation, or random bytes — the exact source
/// doesn't matter as long as it's persistent in TOML from then on.
///
/// The same `id` is also the on-disk filename stem (`<id>.edmx`), which
/// gets us collision safety and path-traversal safety for free: filename
/// construction is entirely tool-controlled, with no raw user input
/// flowing into a filename component except via `slugify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfflineService {
    /// Stable logical identifier shaped `<slug(label_at_creation)>-<8-hex>`.
    /// Primary key, on-disk filename stem, and unique-id reference for
    /// open tabs / history / favorites. **Never changes after first save.**
    /// See the type-level doc for the full stability contract.
    pub id: String,
    /// Offline-profile bucket name this service belongs to.
    pub profile: String,
    /// Display name. Editable in the UI. Defaults to the auto-derived
    /// suggestion from `Schema Namespace` (path A) or the user-supplied
    /// label (path B); subsequent label edits update this field but
    /// **do not** change `id`, `label_at_creation`, or `edmx_file`.
    pub label: String,
    /// The label as recorded at first-save time. Immutable — used as
    /// the logical-identity anchor for path-B re-import matching
    /// (`(profile, label_at_creation)` decides "is this a re-save of the
    /// same thing?"). For path A entries `source_service_path` is the
    /// identity anchor instead; this field is still populated as a
    /// historical snapshot for debugging/audit.
    pub label_at_creation: String,
    /// `Some` for path-A captures: the SAP service path the bytes came
    /// from (`/sap/opu/odata/sap/UI_SVC` or
    /// `/sap/opu/odata4/sap/.../v0001`). `None` for path-B imports —
    /// EDMX files don't always carry their original service URL.
    /// **Immutable** once set; this is the path-A logical-identity anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_service_path: Option<String>,
    /// Path to the EDMX bytes on disk, relative to `{config}/offline/`.
    /// Shape: `<slug(profile)>/<id>.edmx`. Resolved through
    /// `safe_join_under` on every read/delete.
    pub edmx_file: String,
    /// UTC ISO-8601 timestamp, `Some` for path-A captures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<String>,
    /// UTC ISO-8601 timestamp, `Some` for path-B imports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    /// `Some` for path-A captures. `userinfo` (`user:pass@`) is stripped at
    /// save time — see `strip_userinfo`. Capped at 2 KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Original filename for path-B imports; preserved for display only
    /// (the on-disk filename is the tool-generated `<id>.edmx`). Capped
    /// at 256 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_filename: Option<String>,
    /// Full hex of `sha256(content)`, **updated on every re-save** to
    /// reflect current bytes. Used for byte-identical re-save detection
    /// ("you're saving the same bytes you already have, skipping"). The
    /// `id` field is independent and stable — see `build_service_id` for
    /// how the id's 8-hex suffix is generated at creation time and frozen.
    pub sha256: String,
    /// Size of the EDMX bytes in bytes. Defense-in-depth alongside the
    /// 10 MB import cap.
    pub size_bytes: u64,
    /// `"V2"` or `"V4"`. Cached at save time so the UI doesn't re-parse
    /// just to filter by version.
    pub odata_version: String,
    /// User-supplied free-form note. Optional; capped at 2 KB.
    #[serde(default)]
    pub note: String,
}

/// Where the metadata for a given operation comes from. Resolved at the
/// entry of every Tauri command that touches metadata or queries; passed
/// down through the core code paths so the network/offline split is a
/// single explicit branch rather than scattered checks.
#[derive(Debug, Clone)]
pub enum MetadataSource {
    /// Live connection — the existing path. Network access permitted.
    Connected { profile_name: String },
    /// Offline cache. Network access forbidden; reads dispatch to the
    /// EDMX bytes on disk via `safe_join_under` + `canonicalize_under`.
    Offline {
        profile_name: String,
        service_id: String,
    },
}

#[derive(Debug, Error)]
pub enum NetworkNotAllowed {
    #[error(
        "operation requires network access, but the active profile '{profile}' is an offline cache"
    )]
    OfflineSource { profile: String },
}

/// What can go wrong when resolving a `(profile_name, service_identifier)`
/// pair into a `MetadataSource`. Distinct from `NetworkNotAllowed` because
/// resolution failures happen *before* any network-allowed assertion.
#[derive(Debug, Error)]
pub enum MetadataSourceError {
    #[error("profile '{0}' not found in either connections or offline_profiles")]
    ProfileNotFound(String),
    #[error(
        "offline profile '{profile}' has no service matching '{identifier}' (tried `id` and `source_service_path`)"
    )]
    OfflineServiceNotFound { profile: String, identifier: String },
    /// Corrupt state: a profile name exists in *both* `connections` and
    /// `offline_profiles`. Dispatch can't pick a side safely (the
    /// previous "connections first" precedence would silently route an
    /// offline-intended call to the live HTTP branch, defeating the
    /// no-network guarantee). Fail hard so the user fixes the
    /// underlying `connections.toml`. The add-time uniqueness check
    /// (step 7) prevents new collisions; this variant exists for
    /// configs that pre-date that check or were edited by hand.
    #[error(
        "profile name '{0}' is present in BOTH connections and offline_profiles — dispatch is ambiguous. Rename or remove one of them in connections.toml."
    )]
    NameCollision(String),
}

impl MetadataSource {
    /// Verify this source allows network access. Always-Err for offline
    /// sources. Network-touching commands (`fetch_metadata`, `query`,
    /// `resolve_value_list_reference`, $batch, future writes) MUST call
    /// this before doing anything else — UI-side button disabling is
    /// defense in depth, not the primary guard.
    pub fn assert_network_allowed(&self) -> Result<(), NetworkNotAllowed> {
        match self {
            MetadataSource::Connected { .. } => Ok(()),
            MetadataSource::Offline { profile_name, .. } => Err(NetworkNotAllowed::OfflineSource {
                profile: profile_name.clone(),
            }),
        }
    }

    /// Profile name common to both variants — for logging, history keying,
    /// trace context. Profile names are globally unique across the two
    /// kinds (enforced at add-time), so this is unambiguous.
    pub fn profile_name(&self) -> &str {
        match self {
            MetadataSource::Connected { profile_name } => profile_name,
            MetadataSource::Offline { profile_name, .. } => profile_name,
        }
    }

    /// Dispatch a `(profile_name, service_identifier)` pair to the right
    /// variant by inspecting the in-memory `ConfigFile`:
    ///
    /// - If `profile_name` is a connected profile → `Connected` (the
    ///   identifier is treated as a live service path by the caller).
    /// - If `profile_name` is an offline profile → `Offline`. The
    ///   `service_identifier` is matched against each row's `id` and,
    ///   as a fallback for path-A entries, `source_service_path` — so
    ///   the UI can stay agnostic about which path produced the row
    ///   and pass either form. The resolved `MetadataSource::Offline`
    ///   always carries the canonical stable `id`.
    ///
    /// Global name uniqueness across `connections` and
    /// `offline_profiles` (enforced at add-time, both directions)
    /// means this dispatch is unambiguous by construction.
    pub fn resolve(
        profile_name: &str,
        service_identifier: &str,
        config: &crate::config::ConfigFile,
    ) -> Result<Self, MetadataSourceError> {
        let in_connections = config.connections.contains_key(profile_name);
        let in_offline = config.offline_profiles.contains_key(profile_name);

        // Fail-closed for name collisions. The previous shape returned
        // `Connected` as soon as it saw a match in `connections`,
        // without checking `offline_profiles` — meaning a corrupt
        // config with the same name in both maps would silently route
        // offline-intended dispatch to the live-network branch and
        // bypass the no-network guarantee. The add-time check (step 7)
        // prevents new collisions from being introduced; this guard
        // protects pre-existing configs and any hand-edited state.
        if in_connections && in_offline {
            return Err(MetadataSourceError::NameCollision(profile_name.to_string()));
        }

        if in_connections {
            return Ok(MetadataSource::Connected {
                profile_name: profile_name.to_string(),
            });
        }
        if in_offline {
            let row = config
                .offline_services
                .iter()
                .find(|s| {
                    s.profile == profile_name
                        && (s.id == service_identifier
                            || s.source_service_path.as_deref() == Some(service_identifier))
                })
                .ok_or_else(|| MetadataSourceError::OfflineServiceNotFound {
                    profile: profile_name.to_string(),
                    identifier: service_identifier.to_string(),
                })?;
            return Ok(MetadataSource::Offline {
                profile_name: profile_name.to_string(),
                service_id: row.id.clone(),
            });
        }
        Err(MetadataSourceError::ProfileNotFound(
            profile_name.to_string(),
        ))
    }
}

/// Check whether `name` can be used for a NEW connected profile.
/// Returns Ok if the name is either unused or already a connected
/// profile (upsert), errors with a friendly message if the name is
/// taken by an offline profile.
///
/// Used by every connected-add path (Tauri `add_profile`, CLI
/// `cmd_profile_add`, CLI `cmd_setup_wizard`) so the global-uniqueness
/// rule has one source of truth. The reverse direction (refusing to
/// create an offline profile that collides with `connections`) is
/// checked inline by `save_service_offline_from_bytes` and
/// `import_edmx_file`.
pub fn check_connected_profile_name_available(
    name: &str,
    config: &crate::config::ConfigFile,
) -> Result<(), String> {
    if config.connections.contains_key(name) {
        // Upsert path — existing connected profile is allowed to be
        // re-saved with new settings.
        return Ok(());
    }
    if config.offline_profiles.contains_key(name) {
        return Err(format!(
            "Cannot add connected profile '{name}' — an offline profile with the same name already exists. Pick a different name, or remove the offline profile first."
        ));
    }
    Ok(())
}

/// Build a stable `service_id` from a display label and an opaque 8-hex
/// uniqueness suffix.
///
/// Shape: `<slug(label)>-<suffix_hex[0..8]>`. The slug component is bounded
/// by `slugify`'s 64-char cap; the suffix adds 9 chars (`-` + 8 hex). Total
/// length stays well under filesystem limits.
///
/// **The suffix is opaque.** It is generated once at creation time and
/// persisted in TOML; it never changes after that, even if the content
/// `sha256` updates on re-save. Treating it as "content hash" would make
/// `service_id` content-addressed and break stability — see the
/// `OfflineService` docs.
///
/// Typical sources for the suffix at creation:
/// - First 8 hex of `sha256(profile + ":" + label + ":" + created_at)` —
///   deterministic, debuggable, fine for the volumes a personal offline
///   library will ever see.
/// - First 8 hex of `sha256(content)` at first save — also fine; the
///   important property is that the value is then *frozen* into TOML,
///   not re-derived.
/// - 4 random bytes hex-encoded — also fine.
///
/// 32 bits of entropy is enough to avoid collisions across a personal
/// library. The storage layer treats `(profile, source_service_path)`
/// (path A) or `(profile, label_at_creation)` (path B) as the *logical*
/// identity for overwrite-on-resave; the hex suffix only exists for
/// filename uniqueness and id stability.
pub fn build_service_id(label: &str, suffix_hex: &str) -> String {
    let slug = slugify(label);
    let suffix = suffix_hex
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    format!("{slug}-{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_source_network_allowed_for_connected() {
        let src = MetadataSource::Connected {
            profile_name: "DEV".into(),
        };
        assert!(src.assert_network_allowed().is_ok());
    }

    #[test]
    fn metadata_source_network_forbidden_for_offline() {
        let src = MetadataSource::Offline {
            profile_name: "DEV (offline)".into(),
            service_id: "ui_svc-ab12cd34".into(),
        };
        let err = src.assert_network_allowed().unwrap_err();
        // Make sure the user-facing message names the profile so the error
        // is actionable when it bubbles up to the UI.
        assert!(err.to_string().contains("DEV (offline)"));
    }

    #[test]
    fn metadata_source_profile_name_uniform() {
        let connected = MetadataSource::Connected {
            profile_name: "DEV".into(),
        };
        let offline = MetadataSource::Offline {
            profile_name: "DEV (offline)".into(),
            service_id: "x-12345678".into(),
        };
        assert_eq!(connected.profile_name(), "DEV");
        assert_eq!(offline.profile_name(), "DEV (offline)");
    }

    // ── MetadataSource::resolve dispatch ──

    fn build_test_config() -> crate::config::ConfigFile {
        use std::collections::BTreeMap;
        let mut cfg = crate::config::ConfigFile::default();
        cfg.connections.insert(
            "DEV".to_string(),
            crate::config::ConnectionProfile {
                base_url: "https://x".into(),
                client: "100".into(),
                language: "EN".into(),
                username: String::new(),
                password: None,
                sso: false,
                browser_sso: true,
                insecure_tls: false,
                sso_delegate: false,
                aliases: BTreeMap::new(),
            },
        );
        cfg.offline_profiles.insert(
            "DEV (offline)".to_string(),
            OfflineProfile {
                source_profile: "DEV".to_string(),
                created_at: "2026-05-18T08:00:00Z".to_string(),
            },
        );
        cfg.offline_services.push(OfflineService {
            id: "ui_physstockprod_1-ab12cd34".to_string(),
            profile: "DEV (offline)".to_string(),
            label: "UI_PHYSSTOCKPROD_1".to_string(),
            label_at_creation: "UI_PHYSSTOCKPROD_1".to_string(),
            source_service_path: Some("/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1".to_string()),
            edmx_file: "dev_offline/ui_physstockprod_1-ab12cd34.edmx".to_string(),
            fetched_at: Some("2026-05-18T08:00:00Z".to_string()),
            imported_at: None,
            source_url: None,
            original_filename: None,
            sha256: "0".repeat(64),
            size_bytes: 1,
            odata_version: "V4".to_string(),
            note: String::new(),
        });
        cfg.offline_services.push(OfflineService {
            id: "imported_svc-7f3a91be".to_string(),
            profile: "DEV (offline)".to_string(),
            label: "IMPORTED_SVC".to_string(),
            label_at_creation: "IMPORTED_SVC".to_string(),
            source_service_path: None, // path B
            edmx_file: "dev_offline/imported_svc-7f3a91be.edmx".to_string(),
            fetched_at: None,
            imported_at: Some("2026-05-18T08:00:00Z".to_string()),
            source_url: None,
            original_filename: Some("imported.edmx".to_string()),
            sha256: "0".repeat(64),
            size_bytes: 1,
            odata_version: "V4".to_string(),
            note: String::new(),
        });
        cfg
    }

    #[test]
    fn resolve_picks_connected_for_connected_profile() {
        let cfg = build_test_config();
        let src = MetadataSource::resolve("DEV", "/anything/here", &cfg).unwrap();
        match src {
            MetadataSource::Connected { profile_name } => assert_eq!(profile_name, "DEV"),
            other => panic!("expected Connected, got {other:?}"),
        }
    }

    #[test]
    fn resolve_picks_offline_by_service_id() {
        let cfg = build_test_config();
        let src =
            MetadataSource::resolve("DEV (offline)", "ui_physstockprod_1-ab12cd34", &cfg).unwrap();
        match src {
            MetadataSource::Offline {
                profile_name,
                service_id,
            } => {
                assert_eq!(profile_name, "DEV (offline)");
                assert_eq!(service_id, "ui_physstockprod_1-ab12cd34");
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[test]
    fn resolve_picks_offline_by_source_service_path_fallback() {
        // Path-A entries can also be addressed by their original
        // service path — lets the UI stay agnostic about whether the
        // string it received is an `id` or a live service path.
        let cfg = build_test_config();
        let src = MetadataSource::resolve(
            "DEV (offline)",
            "/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1",
            &cfg,
        )
        .unwrap();
        match src {
            MetadataSource::Offline {
                profile_name,
                service_id,
            } => {
                assert_eq!(profile_name, "DEV (offline)");
                // The resolved source always carries the canonical id,
                // not whatever fallback identifier the caller passed.
                assert_eq!(service_id, "ui_physstockprod_1-ab12cd34");
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[test]
    fn resolve_picks_path_b_offline_by_id() {
        let cfg = build_test_config();
        let src = MetadataSource::resolve("DEV (offline)", "imported_svc-7f3a91be", &cfg).unwrap();
        match src {
            MetadataSource::Offline { service_id, .. } => {
                assert_eq!(service_id, "imported_svc-7f3a91be")
            }
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[test]
    fn resolve_rejects_unknown_profile() {
        let cfg = build_test_config();
        let err = MetadataSource::resolve("NOPE", "anything", &cfg).unwrap_err();
        assert!(matches!(err, MetadataSourceError::ProfileNotFound(n) if n == "NOPE"));
    }

    #[test]
    fn resolve_fails_closed_on_name_collision() {
        // Pre-step-7 (or hand-edited) `connections.toml` could contain
        // the same name in both maps. The previous "check connections
        // first, return early" precedence would silently route offline-
        // intended dispatch to the live branch. After the fix, this
        // returns a hard `NameCollision` error so dispatch can never be
        // ambiguous.
        use std::collections::BTreeMap;
        let mut cfg = crate::config::ConfigFile::default();
        cfg.connections.insert(
            "Imported".to_string(),
            crate::config::ConnectionProfile {
                base_url: "https://x".into(),
                client: "100".into(),
                language: "EN".into(),
                username: String::new(),
                password: None,
                sso: false,
                browser_sso: true,
                insecure_tls: false,
                sso_delegate: false,
                aliases: BTreeMap::new(),
            },
        );
        cfg.offline_profiles.insert(
            "Imported".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "x".to_string(),
            },
        );

        let err = MetadataSource::resolve("Imported", "anything", &cfg).unwrap_err();
        match err {
            MetadataSourceError::NameCollision(name) => assert_eq!(name, "Imported"),
            other => panic!("expected NameCollision, got {other:?}"),
        }
    }

    #[test]
    fn check_connected_name_available_allows_unused_name() {
        let cfg = build_test_config();
        assert!(check_connected_profile_name_available("BrandNew", &cfg).is_ok());
    }

    #[test]
    fn check_connected_name_available_allows_upsert() {
        // The connected-add path is also the upsert path. An existing
        // connected profile name must NOT be rejected — the user is
        // editing it, not adding a duplicate.
        let cfg = build_test_config();
        assert!(check_connected_profile_name_available("DEV", &cfg).is_ok());
    }

    #[test]
    fn check_connected_name_available_rejects_offline_collision() {
        // The whole point of the helper: refuse to add a connected
        // profile whose name is already taken by an offline bucket.
        let cfg = build_test_config();
        let err = check_connected_profile_name_available("DEV (offline)", &cfg).unwrap_err();
        assert!(err.contains("DEV (offline)"));
        assert!(err.contains("offline profile"));
    }

    #[test]
    fn resolve_name_collision_short_circuits_assert_network_allowed() {
        // Defense-in-depth check on the full chain: a colliding name
        // never reaches `assert_network_allowed`, so an offline-intended
        // call can never bypass the no-network guard via the collision.
        use std::collections::BTreeMap;
        let mut cfg = crate::config::ConfigFile::default();
        cfg.connections.insert(
            "Shared".to_string(),
            crate::config::ConnectionProfile {
                base_url: "https://x".into(),
                client: "100".into(),
                language: "EN".into(),
                username: String::new(),
                password: None,
                sso: false,
                browser_sso: true,
                insecure_tls: false,
                sso_delegate: false,
                aliases: BTreeMap::new(),
            },
        );
        cfg.offline_profiles.insert(
            "Shared".to_string(),
            OfflineProfile {
                source_profile: String::new(),
                created_at: "x".to_string(),
            },
        );
        // The whole call chain: resolve → then assert_network_allowed
        // on the result. With the bug, resolve would have returned
        // Connected and assert would have passed. With the fix, resolve
        // returns Err before assert ever runs.
        let result = MetadataSource::resolve("Shared", "/svc", &cfg);
        assert!(
            matches!(result, Err(MetadataSourceError::NameCollision(_))),
            "collision must fail-close at resolve, not slip through to assert_network_allowed"
        );
    }

    #[test]
    fn resolve_rejects_unknown_service_in_offline_profile() {
        let cfg = build_test_config();
        let err = MetadataSource::resolve("DEV (offline)", "does-not-exist", &cfg).unwrap_err();
        match err {
            MetadataSourceError::OfflineServiceNotFound {
                profile,
                identifier,
            } => {
                assert_eq!(profile, "DEV (offline)");
                assert_eq!(identifier, "does-not-exist");
            }
            other => panic!("expected OfflineServiceNotFound, got {other:?}"),
        }
    }

    #[test]
    fn build_service_id_basic_shape() {
        let id = build_service_id(
            "UI_PHYSSTOCKPROD_1",
            "ab12cd34ef56789a000000000000000000000000000000000000000000000000",
        );
        assert_eq!(id, "ui_physstockprod_1-ab12cd34");
    }

    #[test]
    fn build_service_id_normalizes_case() {
        let id = build_service_id(
            "My Service",
            "AB12CD34EF56789A0000000000000000000000000000000000000000000000",
        );
        // sha256 prefix lower-cased, label slugified.
        assert_eq!(id, "my_service-ab12cd34");
    }

    #[test]
    fn build_service_id_handles_short_hash_defensively() {
        // Unexpected — sha256 is always 64 hex chars — but the helper
        // shouldn't panic if a caller passes a stub. Take what's there.
        let id = build_service_id("svc", "abc");
        assert_eq!(id, "svc-abc");
    }

    // Type round-trip: ensures the `OfflineService` shape we're committing
    // to actually survives a TOML write+read cycle. The full ConfigFile
    // round-trip lives alongside `save_config` in config.rs, but locking
    // the shape here keeps the test failure focused if a field changes
    // incompatibly.
    #[test]
    fn offline_service_toml_round_trip_full() {
        let svc = OfflineService {
            id: "ui_physstockprod_1-ab12cd34".into(),
            profile: "DEV (offline)".into(),
            label: "UI_PHYSSTOCKPROD_1".into(),
            label_at_creation: "UI_PHYSSTOCKPROD_1".into(),
            source_service_path: Some("/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1".into()),
            edmx_file: "dev_offline/ui_physstockprod_1-ab12cd34.edmx".into(),
            fetched_at: Some("2026-05-18T08:00:00Z".into()),
            imported_at: None,
            source_url: Some(
                "https://sap.example.com/sap/opu/odata/sap/UI_PHYSSTOCKPROD_1/$metadata?sap-client=100".into(),
            ),
            original_filename: None,
            sha256: "ab12cd34".repeat(8),
            size_bytes: 123_456,
            odata_version: "V4".into(),
            note: "".into(),
        };
        let s = toml::to_string(&svc).unwrap();
        let back: OfflineService = toml::from_str(&s).unwrap();
        assert_eq!(back.id, svc.id);
        assert_eq!(back.label, svc.label);
        assert_eq!(back.label_at_creation, svc.label_at_creation);
        assert_eq!(back.source_service_path, svc.source_service_path);
        assert_eq!(back.imported_at, None);
        assert_eq!(back.source_url, svc.source_url);
        assert_eq!(back.size_bytes, svc.size_bytes);
    }

    #[test]
    fn offline_service_label_at_creation_decouples_from_editable_label() {
        // Simulate the post-edit state: user changed `label` after import
        // but `label_at_creation` is frozen. This is the field the
        // path-B re-import logic uses to decide "is this the same service?"
        let svc = OfflineService {
            id: "ui_physstockprod_1-ab12cd34".into(),
            profile: "DEV (offline)".into(),
            label: "ZUI_PHYSSTOCKPROD_CUSTOM".into(), // user-edited
            label_at_creation: "UI_PHYSSTOCKPROD_1".into(), // immutable, set at import time
            source_service_path: None,                // path B
            edmx_file: "dev_offline/ui_physstockprod_1-ab12cd34.edmx".into(),
            fetched_at: None,
            imported_at: Some("2026-05-18T08:00:00Z".into()),
            source_url: None,
            original_filename: Some("gw_client_data_02CC7A66.xml".into()),
            sha256: "ab12cd34".repeat(8),
            size_bytes: 123_456,
            odata_version: "V4".into(),
            note: "".into(),
        };
        let s = toml::to_string(&svc).unwrap();
        let back: OfflineService = toml::from_str(&s).unwrap();
        // Both fields persist independently.
        assert_eq!(back.label, "ZUI_PHYSSTOCKPROD_CUSTOM");
        assert_eq!(back.label_at_creation, "UI_PHYSSTOCKPROD_1");
        // The on-disk filename and id stem still reflect the creation-time
        // slug — that's the whole point of the stability contract.
        assert!(back.id.starts_with("ui_physstockprod_1-"));
        assert!(back.edmx_file.contains("ui_physstockprod_1-"));
    }

    #[test]
    fn offline_service_toml_round_trip_path_b_shape() {
        // Path B: no source_service_path, no fetched_at/source_url; has
        // imported_at + original_filename. Verify Option fields skip
        // cleanly so the on-disk TOML stays clean.
        let svc = OfflineService {
            id: "op_warehouseorder_0001-7f3a91be".into(),
            profile: "Imported".into(),
            label: "OP_WAREHOUSEORDER_0001".into(),
            label_at_creation: "OP_WAREHOUSEORDER_0001".into(),
            source_service_path: None,
            edmx_file: "imported/op_warehouseorder_0001-7f3a91be.edmx".into(),
            fetched_at: None,
            imported_at: Some("2026-05-18T08:00:00Z".into()),
            source_url: None,
            original_filename: Some("gw_client_data_02CC7A66.xml".into()),
            sha256: "7f3a91be".repeat(8),
            size_bytes: 92_671,
            odata_version: "V4".into(),
            note: "From customer X DEV via GW_CLIENT".into(),
        };
        let s = toml::to_string(&svc).unwrap();
        // Skip-serialize-if for None fields means the TOML output should
        // not mention `source_service_path` / `fetched_at` / `source_url`.
        assert!(!s.contains("source_service_path"));
        assert!(!s.contains("fetched_at"));
        assert!(!s.contains("source_url"));
        // imported_at and original_filename should be present.
        assert!(s.contains("imported_at"));
        assert!(s.contains("original_filename"));

        let back: OfflineService = toml::from_str(&s).unwrap();
        assert_eq!(back.source_service_path, None);
        assert_eq!(back.fetched_at, None);
        assert_eq!(
            back.original_filename.as_deref(),
            Some("gw_client_data_02CC7A66.xml")
        );
    }

    #[test]
    fn offline_profile_toml_round_trip() {
        let p = OfflineProfile {
            source_profile: "DEV".into(),
            created_at: "2026-05-18T08:00:00Z".into(),
        };
        let s = toml::to_string(&p).unwrap();
        let back: OfflineProfile = toml::from_str(&s).unwrap();
        assert_eq!(back.source_profile, "DEV");
        assert_eq!(back.created_at, "2026-05-18T08:00:00Z");
    }

    #[test]
    fn offline_profile_default_source_profile_for_imported_bucket() {
        // The `Imported` bucket persists with empty source_profile; the
        // `#[serde(default)]` lets older configs that omit the field
        // forward-compatibly default to empty.
        let s = r#"created_at = "2026-05-18T08:00:00Z""#;
        let p: OfflineProfile = toml::from_str(s).unwrap();
        assert_eq!(p.source_profile, "");
    }
}
