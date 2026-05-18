// Path safety primitives for the offline EDMX library.
//
// Offline mode persists user-controlled labels into on-disk filenames and
// directory names under `{config}/offline/`. A naïve `root.join(user_input)`
// is a path-traversal sink — and the reviewer specifically flagged this as a
// HIGH-severity boundary. These helpers exist so that every read/delete in
// the offline-mode code path goes through one of two well-tested entry points.
//
// Split into two layers because canonicalization requires the path to exist
// on disk:
//
// - `safe_join_under` is pure validation: it checks the relative-path
//   components for `..`, absoluteness, Windows reserved names, illegal
//   characters, and then joins. Safe to call before writing.
// - `canonicalize_under` does the runtime check that resolves symlinks /
//   reparse points and verifies the resolved path is still inside the root.
//   Use for read/delete where the target already exists.

use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathError {
    #[error("path is empty")]
    Empty,
    #[error("path is absolute, expected relative: {0}")]
    Absolute(String),
    #[error("path contains parent traversal ('..'): {0}")]
    ParentTraversal(String),
    #[error("path component is a Windows reserved name: {0}")]
    ReservedName(String),
    #[error("path contains illegal characters: {0}")]
    IllegalChars(String),
    #[error("resolved path escapes root: {0}")]
    EscapesRoot(String),
    #[error("path I/O error: {0}")]
    Io(String),
}

// Windows reserved device names. Lower-case comparison; matches with or
// without extension (CON, CON.txt, con, Con.EDMX all reserved). Case-
// insensitive comparison required because Windows filesystems are case-
// insensitive but case-preserving — and the reservation applies regardless
// of how the name was originally cased.
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

// Characters forbidden in Windows filenames. The NUL byte and the
// ASCII-control range are forbidden on every platform because they
// cause silent behavior differences across tools.
const ILLEGAL_FILENAME_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*', '\\', '\0'];

/// True if `component` (a single filename or directory name, with or without
/// extension) collides with a Windows reserved device name.
fn is_windows_reserved(component: &str) -> bool {
    // Strip trailing dot-extension before comparison: "con.edmx" reserves
    // the same way "con" does on Windows.
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    WINDOWS_RESERVED.contains(&stem.as_str())
}

/// Pure-validation join. Returns `root.join(relative)` only if every
/// component of `relative` is safe.
///
/// Rejects:
/// - empty input
/// - absolute paths (`/foo`, `C:\foo`)
/// - any `..` component
/// - any component that is a Windows reserved device name
/// - any component containing `<`, `>`, `:`, `"`, `|`, `?`, `*`, backslash,
///   NUL, or any ASCII control character
///
/// Does **not** canonicalize. Symlink/reparse-point escapes are not caught
/// here — call `canonicalize_under` on the returned path when the target
/// already exists.
pub fn safe_join_under(root: &Path, relative: &str) -> Result<PathBuf, PathError> {
    if relative.is_empty() {
        return Err(PathError::Empty);
    }

    // Reject anything that smells absolute before we even tokenize. This
    // catches Unix `/foo`, Windows `C:\foo`, and the UNC-ish `\\server\share`.
    let candidate = Path::new(relative);
    if candidate.is_absolute() {
        return Err(PathError::Absolute(relative.to_string()));
    }
    // Path::is_absolute() doesn't fire on bare-drive `C:foo` (drive-relative)
    // or on `\foo` (root-relative without drive). Reject both explicitly so
    // that the join can't introduce an unexpected root.
    if relative.starts_with('\\') || relative.starts_with('/') {
        return Err(PathError::Absolute(relative.to_string()));
    }
    if let Some((first, _)) = relative.split_once(':')
        && first.len() == 1
        && first
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return Err(PathError::Absolute(relative.to_string()));
    }

    let mut saw_normal = false;
    for component in candidate.components() {
        match component {
            Component::Normal(os) => {
                let s = os
                    .to_str()
                    .ok_or_else(|| PathError::IllegalChars(relative.to_string()))?;
                validate_component(s, relative)?;
                saw_normal = true;
            }
            Component::CurDir => {
                // `./foo` segments are harmless; drop them. But a path of
                // only `.` / `./` / `./.` resolves to the root itself —
                // dangerous for recursive-delete callers. We require at
                // least one `Normal` component below.
                continue;
            }
            Component::ParentDir => {
                return Err(PathError::ParentTraversal(relative.to_string()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(PathError::Absolute(relative.to_string()));
            }
        }
    }

    if !saw_normal {
        // No usable path components — `.` or `./` only. Rejecting prevents
        // a delete-by-relative-path call from accidentally targeting the
        // root directory itself.
        return Err(PathError::Empty);
    }

    Ok(root.join(candidate))
}

fn validate_component(component: &str, full_path: &str) -> Result<(), PathError> {
    if component.is_empty() || component == "." {
        return Ok(());
    }
    if is_windows_reserved(component) {
        return Err(PathError::ReservedName(component.to_string()));
    }
    if component
        .chars()
        .any(|c| ILLEGAL_FILENAME_CHARS.contains(&c) || c.is_ascii_control())
    {
        return Err(PathError::IllegalChars(full_path.to_string()));
    }
    // Windows trims trailing dots and spaces from filenames at the kernel
    // level — an attacker who appends "x. " to a name can collide with the
    // un-suffixed "x". Reject up front.
    if component.ends_with(' ') || component.ends_with('.') {
        return Err(PathError::IllegalChars(full_path.to_string()));
    }
    Ok(())
}

/// Canonicalize `path` and verify the result is **strictly inside** the
/// canonicalized `root` — i.e. a proper descendant, not the root itself.
/// Use after `safe_join_under` (which is purely syntactic) when the target
/// exists, to defend against symlink / reparse-point escapes.
///
/// Strict-descendancy matters because recursive delete callers (e.g.,
/// `delete_offline_profile`) gate their `remove_dir_all` on this check. If
/// `path` resolved to `root` itself, a successful return would authorize
/// deleting the entire offline library — definitely not what the caller
/// intended.
///
/// Both `path` and `root` must exist; this calls `fs::canonicalize` on each.
pub fn canonicalize_under(path: &Path, root: &Path) -> Result<PathBuf, PathError> {
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| PathError::Io(format!("canonicalize root {}: {e}", root.display())))?;
    let canonical_path = std::fs::canonicalize(path)
        .map_err(|e| PathError::Io(format!("canonicalize path {}: {e}", path.display())))?;
    if canonical_path == canonical_root {
        return Err(PathError::EscapesRoot(format!(
            "{} resolves to the root itself — strict-descendancy required",
            path.display()
        )));
    }
    if !canonical_path.starts_with(&canonical_root) {
        return Err(PathError::EscapesRoot(format!(
            "{} resolved outside of {}",
            path.display(),
            root.display()
        )));
    }
    Ok(canonical_path)
}

/// Generate a filesystem-safe slug from an arbitrary user-supplied name.
///
/// - Lower-cased.
/// - ASCII alphanumeric + `-` + `_` preserved; everything else collapses to `_`.
/// - Runs of `_` collapse to a single `_`.
/// - Leading and trailing `_` and `-` trimmed.
/// - Capped at 64 characters (leaves room for `-<8-hex>` suffix and
///   `.edmx` extension within the standard 255-char filename limit).
/// - Empty result becomes `"unnamed"`.
/// - If the result is a Windows reserved name, prefix with `_` so it can't
///   collide regardless of casing.
///
/// **Important:** slugify is intentionally aggressive and lossy. Two distinct
/// input labels (`Customer X / DEV` and `Customer-X-DEV`) can produce the
/// same slug. Callers needing collision-free filenames must append a
/// uniqueness suffix — the offline-mode `service_id` does this via the
/// `<slug>-<8-hex>` shape (the 8-hex is opaque and frozen at creation;
/// see `build_service_id`), and that final id is what becomes the on-disk
/// filename stem.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len().min(64));
    let mut last_was_underscore = false;
    for ch in name.chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch == '-' || ch == '_' {
            ch
        } else {
            '_'
        };
        if normalized == '_' {
            if last_was_underscore {
                continue;
            }
            last_was_underscore = true;
        } else {
            last_was_underscore = false;
        }
        out.push(normalized);
        if out.len() >= 64 {
            break;
        }
    }
    let trimmed = out.trim_matches(|c| c == '_' || c == '-');
    let result = if trimmed.is_empty() {
        "unnamed".to_string()
    } else {
        trimmed.to_string()
    };
    if is_windows_reserved(&result) {
        format!("_{result}")
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        PathBuf::from("/tmp/offline-root")
    }

    // ── safe_join_under ──

    #[test]
    fn safe_join_accepts_simple_relative() {
        let got = safe_join_under(&root(), "DEV-offline/UI_SVC.edmx").unwrap();
        assert!(
            got.ends_with("DEV-offline/UI_SVC.edmx") || got.ends_with("DEV-offline\\UI_SVC.edmx")
        );
    }

    #[test]
    fn safe_join_rejects_empty() {
        assert!(matches!(
            safe_join_under(&root(), ""),
            Err(PathError::Empty)
        ));
    }

    #[test]
    fn safe_join_rejects_parent_traversal() {
        assert!(matches!(
            safe_join_under(&root(), "..").unwrap_err(),
            PathError::ParentTraversal(_)
        ));
        assert!(matches!(
            safe_join_under(&root(), "DEV/../etc").unwrap_err(),
            PathError::ParentTraversal(_)
        ));
        assert!(matches!(
            safe_join_under(&root(), "DEV/sub/..").unwrap_err(),
            PathError::ParentTraversal(_)
        ));
    }

    #[test]
    fn safe_join_rejects_absolute_paths() {
        for abs in &[
            "/etc/passwd",
            "C:\\Windows\\System32",
            "\\\\server\\share",
            "\\foo",
        ] {
            assert!(
                matches!(
                    safe_join_under(&root(), abs).unwrap_err(),
                    PathError::Absolute(_)
                ),
                "expected absolute rejection for {abs}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_drive_relative() {
        // `C:foo` on Windows means "foo under the current dir on drive C"
        // — drive-relative, not the same as `C:\foo`. Both are unsafe to
        // accept from user input.
        assert!(matches!(
            safe_join_under(&root(), "C:foo").unwrap_err(),
            PathError::Absolute(_)
        ));
    }

    #[test]
    fn safe_join_rejects_windows_reserved_names() {
        for reserved in &["CON", "con", "PRN.edmx", "aux", "NUL", "COM1", "lpt9"] {
            assert!(
                matches!(
                    safe_join_under(&root(), reserved).unwrap_err(),
                    PathError::ReservedName(_)
                ),
                "expected reserved-name rejection for {reserved}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_illegal_characters() {
        for bad in &["a<b", "a>b", "a\"b", "a|b", "a?b", "a*b", "a\0b"] {
            assert!(
                matches!(
                    safe_join_under(&root(), bad).unwrap_err(),
                    PathError::IllegalChars(_)
                ),
                "expected illegal-chars rejection for {bad:?}"
            );
        }
    }

    #[test]
    fn safe_join_rejects_trailing_dot_or_space() {
        // Windows kernel trims these silently → "foo." aliases to "foo".
        assert!(matches!(
            safe_join_under(&root(), "foo.").unwrap_err(),
            PathError::IllegalChars(_)
        ));
        assert!(matches!(
            safe_join_under(&root(), "foo ").unwrap_err(),
            PathError::IllegalChars(_)
        ));
    }

    #[test]
    fn safe_join_accepts_curdir_segments() {
        // `./foo` should resolve like `foo` — these are harmless and emitted
        // by some Path APIs.
        let got = safe_join_under(&root(), "./DEV-offline/x.edmx").unwrap();
        assert!(got.ends_with("DEV-offline/x.edmx") || got.ends_with("DEV-offline\\x.edmx"));
    }

    #[test]
    fn safe_join_rejects_curdir_only() {
        // A relative path of only `.` / `./` / `./.` would otherwise resolve
        // to the root itself — fine for joining, lethal for a caller that
        // then runs remove_dir_all. The reviewer flagged this as a HIGH-
        // severity gap in the first cut. Requires at least one Normal
        // component.
        for only_cur in &[".", "./", "./.", "././."] {
            assert!(
                matches!(
                    safe_join_under(&root(), only_cur).unwrap_err(),
                    PathError::Empty
                ),
                "expected Empty rejection for {only_cur:?}"
            );
        }
    }

    // ── slugify ──

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("UI_PHYSSTOCKPROD_1"), "ui_physstockprod_1");
        assert_eq!(slugify("My Service"), "my_service");
        assert_eq!(slugify("DEV (offline)"), "dev_offline");
    }

    #[test]
    fn slugify_collapses_runs() {
        assert_eq!(slugify("A    B"), "a_b");
        assert_eq!(slugify("A___B"), "a_b");
        assert_eq!(slugify("A_-_B"), "a_-_b"); // mixed: only `_` runs collapse
    }

    #[test]
    fn slugify_trims_edges() {
        assert_eq!(slugify("___foo___"), "foo");
        assert_eq!(slugify("---foo---"), "foo");
        assert_eq!(slugify("_-_foo_-_"), "foo");
    }

    #[test]
    fn slugify_empty_fallback() {
        assert_eq!(slugify(""), "unnamed");
        assert_eq!(slugify("___"), "unnamed");
        assert_eq!(slugify("!!!"), "unnamed");
    }

    #[test]
    fn slugify_caps_length() {
        let long = "a".repeat(200);
        let got = slugify(&long);
        assert!(got.len() <= 64, "got len {}: {got:?}", got.len());
    }

    #[test]
    fn slugify_unicode_strips() {
        // Non-ASCII collapses to `_`. Lossy by design; the hash suffix on
        // `service_id` is what carries uniqueness. Trailing `_` is trimmed
        // by the edge-trim step, so "café" → "caf__" → "caf".
        assert_eq!(slugify("café"), "caf");
        assert_eq!(slugify("日本語"), "unnamed");
        // Embedded non-ASCII becomes `_` and survives (not on an edge).
        assert_eq!(slugify("caf_é_e"), "caf_e");
    }

    #[test]
    fn slugify_avoids_windows_reserved() {
        // After normalization, a slug that lands on a reserved name must be
        // shifted away from it.
        assert_eq!(slugify("CON"), "_con");
        assert_eq!(slugify("nul"), "_nul");
        assert_eq!(slugify("COM1"), "_com1");
    }

    #[test]
    fn slugify_preserves_safe_chars() {
        assert_eq!(slugify("abc-123_def"), "abc-123_def");
    }

    // ── canonicalize_under (smoke; full I/O coverage in integration tests) ──

    #[test]
    fn canonicalize_under_accepts_path_inside_root() {
        let tmp = std::env::temp_dir().join("sap_odata_canonicalize_under_test");
        let _ = std::fs::create_dir_all(&tmp);
        let inside = tmp.join("inside.txt");
        std::fs::write(&inside, b"x").unwrap();

        let resolved = canonicalize_under(&inside, &tmp).expect("inside root must resolve");
        assert!(resolved.starts_with(std::fs::canonicalize(&tmp).unwrap()));

        std::fs::remove_file(&inside).ok();
        std::fs::remove_dir(&tmp).ok();
    }

    #[test]
    fn canonicalize_under_rejects_path_outside_root() {
        let tmp = std::env::temp_dir().join("sap_odata_canonicalize_outside_test");
        let other = std::env::temp_dir().join("sap_odata_canonicalize_outside_other");
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&other);
        let outside_file = other.join("outside.txt");
        std::fs::write(&outside_file, b"x").unwrap();

        let result = canonicalize_under(&outside_file, &tmp);
        assert!(matches!(result, Err(PathError::EscapesRoot(_))));

        std::fs::remove_file(&outside_file).ok();
        std::fs::remove_dir(&tmp).ok();
        std::fs::remove_dir(&other).ok();
    }

    #[test]
    fn canonicalize_under_rejects_root_itself() {
        // The most dangerous case: the resolved path *is* the root. A
        // recursive-delete caller acting on this would obliterate the
        // entire offline library. starts_with(root) returns true for
        // path == root, so the additional equality check is what holds
        // this line.
        let tmp = std::env::temp_dir().join("sap_odata_canonicalize_root_self_test");
        let _ = std::fs::create_dir_all(&tmp);
        let result = canonicalize_under(&tmp, &tmp);
        assert!(
            matches!(result, Err(PathError::EscapesRoot(msg)) if msg.contains("root itself")),
            "expected strict-descendancy rejection"
        );
        std::fs::remove_dir(&tmp).ok();
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_under_rejects_symlink_escape() {
        // Create a symlink inside the offline root that points outside the
        // root. `safe_join_under` can't catch this — the link is a Normal
        // component with a benign-looking name. `canonicalize_under` must
        // resolve the link and reject the escape. Unix-only because
        // creating symlinks on Windows requires admin or developer mode;
        // the canonicalization semantics are equivalent on Windows.
        let tmp = std::env::temp_dir().join("sap_odata_symlink_escape_test");
        let outside = std::env::temp_dir().join("sap_odata_symlink_escape_outside");
        let _ = std::fs::create_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&outside);
        let outside_file = outside.join("secret.txt");
        std::fs::write(&outside_file, b"x").unwrap();

        let link_inside_root = tmp.join("link");
        let _ = std::os::unix::fs::symlink(&outside_file, &link_inside_root);

        let result = canonicalize_under(&link_inside_root, &tmp);
        assert!(
            matches!(result, Err(PathError::EscapesRoot(_))),
            "expected symlink escape rejection, got {result:?}"
        );

        std::fs::remove_file(&link_inside_root).ok();
        std::fs::remove_file(&outside_file).ok();
        std::fs::remove_dir(&tmp).ok();
        std::fs::remove_dir(&outside).ok();
    }
}
