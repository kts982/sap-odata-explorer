// Import validation pipeline for the offline EDMX library.
//
// Path B ("Open EDMX file") accepts files from a wide range of sources —
// SAP API Hub downloads, `/IWFND/GW_CLIENT` "Save Response" output, browser
// save-as on `<base>/$metadata`, `curl > out.xml`, etc. — and the user can
// land any byte sequence in the file picker. The validation pipeline is
// the gate: it has to accept legitimate metadata in V2 or V4, reject
// hostile or wrong-shape inputs cleanly, and produce friendly enough error
// messages that the user knows what to fix.
//
// Order matters here. Cheap rejects come first so a 9 MB HTML page doesn't
// get fully UTF-8 decoded + XML-parsed before we tell the user it's the
// wrong file:
//
// 1. Empty input — reject early.
// 2. File size ≤ 10 MB. Real `$metadata` is rarely >2 MB; oversize is
//    almost always the wrong file or an attack.
// 3. Magic-byte sniff for gzip (`\x1f\x8b…`). Out of scope for v0.2.
// 4. Magic-byte sniff for an HTTP status line. Some `wget -S` / HAR
//    exports prepend "HTTP/1.1 200 OK\r\n…". Reject with a hint to strip.
// 5. Strip UTF-8 BOM (byte-level).
// 6. XXE defense: byte-level case-insensitive scan of the first 4 KB for
//    `<!DOCTYPE` / `<!ENTITY`. Operating on bytes (not the UTF-8-decoded
//    string) means the prefix slice can't panic on a multibyte character
//    straddling the scan boundary, and we catch DOCTYPE-laced inputs
//    that also fail UTF-8 decode. roxmltree doesn't expand external
//    entities by default, so this is defense in depth — rejecting at the
//    gate also sends the right signal.
// 7. UTF-8 decode. Reject non-UTF-8 with a clear error rather than
//    letting roxmltree do it.
// 8. String-level wrong-root classification (HTML page, Atom service
//    document, OData error envelope) so the user gets a specific hint
//    instead of a generic "root is not Edmx" message.
// 9. XML parse.
// 10. Document root must be local-name `Edmx` in a known EDMX namespace.
// 11. At least one `Schema` element with a non-empty `Namespace` — using
//     `find_map` so an EDMX with a leading helper Schema lacking
//     `Namespace` plus a later real Schema still validates.
//
// Common wrong-input shapes map onto distinct error variants so the UI
// can render different hints (login redirect, service document, OData
// error envelope, etc.). The mapping is in `ImportError`'s `Display`
// impls.

use thiserror::Error;

use crate::metadata::ODataVersion;

/// Hard cap on import size. Real `$metadata` is rarely above 2 MB; anything
/// past 10 MB is almost certainly the wrong file or hostile input.
pub const MAX_IMPORT_SIZE_BYTES: u64 = 10 * 1024 * 1024;

/// How many leading bytes to scan for `<!DOCTYPE` / `<!ENTITY`. 4 KB is
/// well past any legitimate XML declaration + comment prolog and short
/// enough to be free regardless of total file size.
const XXE_SCAN_PREFIX_BYTES: usize = 4096;

/// UTF-8 byte-order mark. Skipped before XML parsing because roxmltree's
/// behavior with a leading BOM is parser-version-dependent and we'd rather
/// not depend on it.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Gzip magic bytes. Some clients gzip the `$metadata` response on the
/// wire; if the user saved the compressed payload, we reject with a clear
/// "decompress first" hint rather than producing a confusing parse error.
const GZIP_MAGIC: &[u8] = &[0x1F, 0x8B];

/// Two known EDMX wrapper namespaces. V4 uses the OASIS URI; V2 uses the
/// Microsoft Edmx URI. Anything else at the document root means the file
/// isn't EDMX even if it happens to use the element name `Edmx`.
const EDMX_NAMESPACE_V4: &str = "http://docs.oasis-open.org/odata/ns/edmx";
const EDMX_NAMESPACE_V2: &str = "http://schemas.microsoft.com/ado/2007/06/edmx";

/// Outcome of a successful validation. Carries just enough metadata for
/// the caller (the path-B import command) to populate the
/// `OfflineService` index entry. The bytes themselves are *not* returned
/// — the caller already has them, and any BOM should be stripped at write
/// time via `strip_utf8_bom`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEdmx {
    /// First `Schema Namespace` value found in the parsed document. Used
    /// downstream to suggest a service label via
    /// `derive_label_from_schema_namespace`.
    pub schema_namespace: String,
    /// `V2` or `V4`, detected from the `<edmx:Edmx>` wrapper. Stored on
    /// the `OfflineService` row so the UI can filter without re-parsing.
    pub odata_version: ODataVersion,
    /// `true` if the input started with a UTF-8 BOM. Callers should strip
    /// before writing to disk so the on-disk file is canonical UTF-8.
    pub had_bom: bool,
}

/// What went wrong during validation. Each variant maps to a specific
/// user-facing hint in `Display` so the UI can render actionable error
/// text without having to interpret the variant itself.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ImportError {
    #[error("file is empty — nothing to import")]
    Empty,

    #[error(
        "file is too large ({size} bytes); imports are capped at {limit} bytes. Real $metadata is rarely above 2 MB — this is probably the wrong file."
    )]
    TooLarge { size: u64, limit: u64 },

    #[error(
        "file is gzipped. Decompress it first (e.g. `gunzip file.xml.gz`), then import the resulting `.xml`."
    )]
    Gzipped,

    #[error(
        "file starts with an HTTP status/headers prefix. Strip everything above the `<?xml` declaration and try again."
    )]
    HttpHeadersPrefix,

    #[error(
        "file is not valid UTF-8. Re-save with UTF-8 encoding (no UTF-16, no ISO-8859-x) and try again."
    )]
    NotUtf8,

    #[error(
        "file contains DOCTYPE or ENTITY declarations, which are not allowed in OData metadata. Re-export from the source and try again."
    )]
    DoctypeForbidden,

    #[error("XML parse failed: {0}")]
    XmlParse(String),

    #[error(
        "this is an HTML page, not metadata. The SAP system likely returned a login redirect — you may need to authenticate first and re-fetch."
    )]
    LooksLikeHtmlPage,

    #[error(
        "this looks like an Atom service document. You want the `$metadata` URL: `<base>/<service>/$metadata`, not the service root."
    )]
    LooksLikeServiceDocument,

    #[error(
        "this is an OData error response, not metadata. Check the service URL and authentication, then re-fetch."
    )]
    LooksLikeODataError,

    #[error(
        "document root is `<{root_name}>`, expected `<edmx:Edmx>` in a known EDMX namespace. Make sure you're importing a `$metadata` response."
    )]
    NotEdmxRoot { root_name: String },

    #[error(
        "EDMX wrapper found but no Schema element with a non-empty Namespace — this metadata is empty or truncated."
    )]
    NoSchema,
}

/// Strip a leading UTF-8 BOM from `bytes`, if present. Returns the
/// original slice unchanged otherwise. Use this at storage time so the
/// on-disk file is canonical UTF-8 regardless of how the import source
/// encoded itself.
pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(UTF8_BOM) {
        &bytes[3..]
    } else {
        bytes
    }
}

/// Run the full validation pipeline. Returns metadata about the file
/// (schema namespace, OData version, BOM presence) on success; a typed
/// error suitable for direct UI rendering on failure.
pub fn validate_edmx(bytes: &[u8]) -> Result<ValidatedEdmx, ImportError> {
    let size = bytes.len() as u64;
    if size == 0 {
        return Err(ImportError::Empty);
    }
    if size > MAX_IMPORT_SIZE_BYTES {
        return Err(ImportError::TooLarge {
            size,
            limit: MAX_IMPORT_SIZE_BYTES,
        });
    }

    if bytes.starts_with(GZIP_MAGIC) {
        return Err(ImportError::Gzipped);
    }

    if looks_like_http_headers_prefix(bytes) {
        return Err(ImportError::HttpHeadersPrefix);
    }

    let had_bom = bytes.starts_with(UTF8_BOM);
    let xml_bytes = strip_utf8_bom(bytes);

    // XXE defense: scan the leading bytes for `<!DOCTYPE` or `<!ENTITY`
    // *before* UTF-8 decode. Operating on raw bytes has two benefits:
    // (1) cannot panic on a multibyte UTF-8 character straddling the scan
    // boundary, because we never slice a `&str` on an arbitrary index; and
    // (2) catches DOCTYPE-laced inputs that happen to also fail UTF-8
    // decode, so the user gets the more specific "DOCTYPE forbidden"
    // message. The needles are ASCII, so byte-level case-insensitive
    // matching is correct.
    if contains_xxe_marker(xml_bytes) {
        return Err(ImportError::DoctypeForbidden);
    }

    let xml_str = std::str::from_utf8(xml_bytes).map_err(|_| ImportError::NotUtf8)?;

    // Map the wrong-shape XML cases to specific hints before letting
    // roxmltree report a generic "unexpected element" diagnostic. The
    // sniff is on the raw string (post-BOM-strip) so we don't pay for a
    // full parse on inputs we already know we'll reject.
    if let Some(specific) = classify_wrong_root(xml_str) {
        return Err(specific);
    }

    let doc =
        roxmltree::Document::parse(xml_str).map_err(|e| ImportError::XmlParse(e.to_string()))?;

    // Document root must be local-name `Edmx` in a known EDMX namespace.
    // We don't accept `Edmx` in any other namespace, because then it
    // probably means someone hand-rolled XML that copies the EDMX shape
    // without being EDMX.
    let root = doc.root_element();
    let root_local_name = root.tag_name().name();
    let root_namespace = root.tag_name().namespace().unwrap_or("");
    if root_local_name != "Edmx"
        || (root_namespace != EDMX_NAMESPACE_V4 && root_namespace != EDMX_NAMESPACE_V2)
    {
        return Err(ImportError::NotEdmxRoot {
            root_name: root_local_name.to_string(),
        });
    }

    // OData version detection: V4 wrappers carry `Version="4.x"` on the
    // root and the OASIS namespace; V2 wrappers carry the Microsoft
    // Edmx namespace. The existing core detector lives in
    // `metadata/mod.rs::detect_version` but is private; reproduce the
    // logic here against the same signals so import-time and parse-time
    // can't disagree.
    let version = if root_namespace == EDMX_NAMESPACE_V4
        || root
            .attribute("Version")
            .is_some_and(|v| v.starts_with('4'))
    {
        ODataVersion::V4
    } else {
        ODataVersion::V2
    };

    // Walk descendants for the *first Schema element that has a non-empty
    // `Namespace`*. Plan says "at least one Schema with non-empty
    // Namespace" — so an initial helper Schema lacking Namespace followed
    // by a real Schema with one should still validate. `find_map` over
    // all Schema nodes captures that. Mirrors how `parse_metadata` finds
    // schemas — searching
    // by descendants because the V4 / V2 nesting under `edmx:DataServices`
    // differs slightly.
    let schema_namespace = doc
        .descendants()
        .filter(|n| n.has_tag_name("Schema"))
        .find_map(|n| {
            n.attribute("Namespace")
                .filter(|ns| !ns.is_empty())
                .map(String::from)
        })
        .ok_or(ImportError::NoSchema)?;

    Ok(ValidatedEdmx {
        schema_namespace,
        odata_version: version,
        had_bom,
    })
}

/// Byte-level case-insensitive scan for the XXE markers `<!DOCTYPE` and
/// `<!ENTITY` in the first `XXE_SCAN_PREFIX_BYTES` of input. Operates on
/// bytes so the scan can never panic on a multibyte UTF-8 character
/// straddling the prefix boundary, and so we can run it before UTF-8
/// decode. Both needles are pure ASCII, so byte-uppercase comparison is
/// correct: non-ASCII bytes won't accidentally match the uppercase
/// ASCII keyword.
fn contains_xxe_marker(bytes: &[u8]) -> bool {
    let scan_len = bytes.len().min(XXE_SCAN_PREFIX_BYTES);
    let prefix = &bytes[..scan_len];
    contains_ascii_ci(prefix, b"<!DOCTYPE") || contains_ascii_ci(prefix, b"<!ENTITY")
}

/// Case-insensitive substring search on bytes. `needle` must be uppercase
/// ASCII; each byte from `haystack` is uppercased before comparison.
fn contains_ascii_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    let last_start = haystack.len() - needle.len();
    (0..=last_start).any(|i| {
        haystack[i..i + needle.len()]
            .iter()
            .zip(needle)
            .all(|(h, n)| h.to_ascii_uppercase() == *n)
    })
}

/// Quick check for an HTTP status/headers prefix. Real-world cases come
/// from `wget -S` or HAR exports where the response is saved with the
/// status line and headers stuck on top of the body. We sniff the first
/// few bytes for either an HTTP version token or a known status-line
/// shape so the user gets a clean "strip the headers" message rather
/// than a misleading "XML parse failed" further down the pipeline.
fn looks_like_http_headers_prefix(bytes: &[u8]) -> bool {
    // Take the first 32 bytes and check ASCII-only prefix patterns.
    let head = &bytes[..bytes.len().min(32)];
    let head_str = match std::str::from_utf8(head) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let trimmed = head_str.trim_start();
    trimmed.starts_with("HTTP/1.0 ")
        || trimmed.starts_with("HTTP/1.1 ")
        || trimmed.starts_with("HTTP/2 ")
}

/// String-level sniff for common wrong-root XML shapes so we can return
/// a specific hint instead of a generic "root is not Edmx" message.
/// Looks at the first non-prolog element name. Conservative — matches a
/// few well-known SAP/OData shapes; anything else falls through to the
/// XML parse + generic root check.
fn classify_wrong_root(xml: &str) -> Option<ImportError> {
    let trimmed = xml.trim_start();
    // Skip optional XML declaration.
    let after_decl = if trimmed.starts_with("<?xml") {
        match trimmed.find("?>") {
            Some(end) => trimmed[end + 2..].trim_start(),
            None => return None,
        }
    } else {
        trimmed
    };

    // HTML pages: a sniff on the first few characters catches `<!DOCTYPE
    // html>`, `<html>`, `<HTML>` — including whitespace and BOM-stripped
    // variants. Most SSO redirect bodies match one of these.
    let head_lower = after_decl
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    if head_lower.starts_with("<!doctype html") || head_lower.starts_with("<html") {
        return Some(ImportError::LooksLikeHtmlPage);
    }

    // Atom service document root: `<service xmlns="http://www.w3.org/2007/app">`.
    // Match on `<service ` with the `app`-namespace URI nearby.
    if head_lower.starts_with("<service ") && after_decl.contains("http://www.w3.org/2007/app") {
        return Some(ImportError::LooksLikeServiceDocument);
    }

    // OData error envelopes have variants:
    //   V2:  <error xmlns="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
    //   V4:  <error xmlns="http://docs.oasis-open.org/odata/ns/metadata">
    //   prefixed: <m:error …> / <e:error …>
    if head_lower.starts_with("<error ")
        || head_lower.starts_with("<error>")
        || head_lower.starts_with("<m:error")
        || head_lower.starts_with("<e:error")
    {
        return Some(ImportError::LooksLikeODataError);
    }

    None
}

/// Derive a default service label from the first `Schema Namespace` in an
/// imported EDMX. Used by the path-B import UX to pre-fill the label
/// field so the user usually doesn't have to type anything.
///
/// Rules (from the plan):
/// - V4 SAP gateway services with the namespace shape
///   `com.sap.gateway.srvd.<svc>.v<NNNN>` → `<SVC>_<N>` where `<N>` is
///   the version with leading zeros stripped. This matches the canonical
///   SAP tooling presentation — namespace `…ui_physstockprod.v0001`
///   renders to `UI_PHYSSTOCKPROD_1`, the form consultants see in SEGW /
///   RAP service definitions and `/IWFND/MAINT_SERVICE`. Without the
///   `_<N>` suffix two different versions of the same service would
///   collide on import.
/// - V4 SAP gateway services without a recognizable `.v<digits>` suffix
///   → uppercase `<SVC>` only.
/// - V2 SAP services with a `<vendor>.<SVC>` or bare `<SVC>` namespace
///   → take the trailing dot-segment as-is.
/// - Anything that doesn't match these shapes → empty string. The
///   caller is expected to fall back to the filename stem.
pub fn derive_label_from_schema_namespace(namespace: &str) -> String {
    let ns = namespace.trim();
    if ns.is_empty() {
        return String::new();
    }

    // V4 SAP service-definition namespace. SAP uses several variants
    // under the `com.sap.gateway.<family>.<svc>.v<NNNN>` shape:
    //   - `srvd`       — RAP service definitions
    //   - `srvd_a2x`   — RAP A2X bindings (application-to-application)
    //   - `iwbep` etc. — older OData V2 IWBEP services
    // The trailing `_<suffix>` after `srvd` (or after any other family
    // name) varies by service kind but the surrounding shape is stable.
    // Strip the `com.sap.gateway.<family>.` prefix where `<family>` is
    // the first dotted segment, then apply the same `<svc>.v<NNNN>`
    // logic. This catches both `srvd` and `srvd_a2x` (and future
    // variants) under a single rule.
    if let Some(after_gateway) = ns.strip_prefix("com.sap.gateway.")
        && let Some(family_end) = after_gateway.find('.')
    {
        let stripped = &after_gateway[family_end + 1..];
        // Look for the `.v<digits>` version suffix. If present and the
        // digit sequence is non-empty, emit `<SVC>_<N>` where N has
        // leading zeros stripped. Otherwise emit `<SVC>` only.
        if let Some(idx) = stripped.rfind(".v") {
            let after_v = &stripped[idx + 2..];
            if !after_v.is_empty() && after_v.chars().all(|c| c.is_ascii_digit()) {
                let core = &stripped[..idx];
                // Trim leading zeros; preserve at least one digit.
                let trimmed = after_v.trim_start_matches('0');
                let version = if trimmed.is_empty() { "0" } else { trimmed };
                return format!("{}_{}", core.to_ascii_uppercase(), version);
            }
        }
        return stripped.to_ascii_uppercase();
    }

    // Generic fallback: take the trailing dot-segment — but only for
    // namespaces that look like dotted ASCII identifiers (the V2 SAP
    // service convention). URL-shaped namespaces
    // (`http://example.com/schema`), namespaces with whitespace or
    // unusual punctuation, and non-ASCII namespaces all produce
    // misleading labels under a naive `rsplit('.')`, so we return empty
    // and let the caller fall back to the filename stem instead.
    let dotted_identifier_shape = ns
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    if !dotted_identifier_shape {
        return String::new();
    }
    let trailing = ns.rsplit('.').next().unwrap_or(ns);
    if trailing.is_empty() {
        return String::new();
    }
    trailing.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Minimal in-fixture EDMX bodies ──
    //
    // We avoid disk fixtures so the test data is visible at the point of
    // assertion. The shapes mirror what real SAP servers emit; the
    // `metadata/mod.rs` test corpus is the larger reference for parser
    // coverage.

    const VALID_V4_EDMX: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="com.sap.gateway.srvd.ui_physstockprod.v0001">
      <EntityType Name="Product"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;

    const VALID_V2_EDMX: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://schemas.microsoft.com/ado/2007/06/edmx" Version="1.0">
  <edmx:DataServices xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata" m:DataServiceVersion="2.0">
    <Schema xmlns="http://schemas.microsoft.com/ado/2008/09/edm" Namespace="ZTEST_SRV">
      <EntityType Name="Order"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"##;

    // ── Happy-path validation ──

    #[test]
    fn validates_v4_edmx() {
        let v = validate_edmx(VALID_V4_EDMX.as_bytes()).unwrap();
        assert_eq!(v.odata_version, ODataVersion::V4);
        assert_eq!(
            v.schema_namespace,
            "com.sap.gateway.srvd.ui_physstockprod.v0001"
        );
        assert!(!v.had_bom);
    }

    #[test]
    fn validates_v2_edmx() {
        let v = validate_edmx(VALID_V2_EDMX.as_bytes()).unwrap();
        assert_eq!(v.odata_version, ODataVersion::V2);
        assert_eq!(v.schema_namespace, "ZTEST_SRV");
        assert!(!v.had_bom);
    }

    #[test]
    fn validates_v4_edmx_with_utf8_bom() {
        let mut bytes = Vec::with_capacity(VALID_V4_EDMX.len() + 3);
        bytes.extend_from_slice(UTF8_BOM);
        bytes.extend_from_slice(VALID_V4_EDMX.as_bytes());
        let v = validate_edmx(&bytes).unwrap();
        assert_eq!(v.odata_version, ODataVersion::V4);
        assert!(v.had_bom, "BOM presence should be reported");
    }

    // ── Cheap rejects (size / magic-bytes / encoding) ──

    #[test]
    fn rejects_empty_input() {
        assert_eq!(validate_edmx(b""), Err(ImportError::Empty));
    }

    #[test]
    fn rejects_oversized_input() {
        // Synthesize: don't actually allocate 11 MB if we can avoid it.
        // A vec the size of the cap + 1 is fine for a one-shot test.
        let big = vec![b'<'; (MAX_IMPORT_SIZE_BYTES + 1) as usize];
        match validate_edmx(&big).unwrap_err() {
            ImportError::TooLarge { size, limit } => {
                assert_eq!(size, MAX_IMPORT_SIZE_BYTES + 1);
                assert_eq!(limit, MAX_IMPORT_SIZE_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn rejects_gzipped_input() {
        // First two bytes are the gzip magic; anything after doesn't
        // matter — we reject before reading the rest.
        let gzipped = [0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00];
        assert_eq!(validate_edmx(&gzipped), Err(ImportError::Gzipped));
    }

    #[test]
    fn rejects_http_headers_prefix() {
        let with_headers =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\n\r\n<?xml version=\"1.0\"?>";
        assert_eq!(
            validate_edmx(with_headers),
            Err(ImportError::HttpHeadersPrefix)
        );
    }

    #[test]
    fn rejects_non_utf8_input() {
        // Latin-1 encoded bytes for "<?xml version=...><é"
        let latin1 = vec![0x3C, 0x3F, 0x78, 0x6D, 0x6C, 0xE9, 0x3E];
        assert_eq!(validate_edmx(&latin1), Err(ImportError::NotUtf8));
    }

    // ── XXE defense ──

    #[test]
    fn rejects_doctype_declaration() {
        let with_doctype = br#"<?xml version="1.0"?>
<!DOCTYPE edmx:Edmx [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0"/>"#;
        assert_eq!(
            validate_edmx(with_doctype),
            Err(ImportError::DoctypeForbidden)
        );
    }

    #[test]
    fn rejects_entity_declaration_without_doctype() {
        let with_entity = br#"<?xml version="1.0"?>
<!ENTITY xxe "boom">
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0"/>"#;
        assert_eq!(
            validate_edmx(with_entity),
            Err(ImportError::DoctypeForbidden)
        );
    }

    #[test]
    fn xxe_scan_does_not_panic_on_multibyte_boundary() {
        // Build an input where a multi-byte UTF-8 character (3-byte CJK)
        // straddles the XXE scan prefix boundary. The previous
        // implementation sliced `xml_str[..4096]` and would panic if 4096
        // landed inside the character. The current byte-level scan must
        // handle this without panicking.
        //
        // Strategy: pad with a benign comment to push the CJK char across
        // the 4096-byte boundary. The CJK char `日` is `\xE6\x97\xA5`
        // (3 bytes).
        let mut input = Vec::with_capacity(XXE_SCAN_PREFIX_BYTES + 64);
        input.extend_from_slice(b"<?xml version=\"1.0\"?><!-- ");
        // Pad with ASCII so the CJK char lands at byte position 4095 or
        // 4096 — straddling the scan boundary either way.
        let pad_target = XXE_SCAN_PREFIX_BYTES - 2;
        while input.len() < pad_target {
            input.push(b'x');
        }
        // Inject the CJK char so its second byte sits at exactly 4096.
        input.extend_from_slice("日本".as_bytes());
        input.extend_from_slice(b" -->\n<edmx:Edmx xmlns:edmx=\"http://docs.oasis-open.org/odata/ns/edmx\" Version=\"4.0\"><edmx:DataServices><Schema xmlns=\"http://docs.oasis-open.org/odata/ns/edm\" Namespace=\"X\"/></edmx:DataServices></edmx:Edmx>");

        // Must not panic. The validation may succeed (no DOCTYPE in the
        // padded prolog) or fail with a non-panic error — both outcomes
        // are acceptable, the point is no boundary panic.
        let _ = validate_edmx(&input);
    }

    #[test]
    fn xxe_scan_works_on_invalid_utf8_with_doctype() {
        // DOCTYPE present in a file whose later bytes happen to be
        // invalid UTF-8. With byte-level XXE scan, we now reject for
        // DoctypeForbidden (the more specific reason) rather than for
        // NotUtf8.
        let mut input = Vec::new();
        input.extend_from_slice(b"<!DOCTYPE evil><edmx:Edmx>");
        input.extend_from_slice(&[0xFF, 0xFE, 0xFD]); // invalid UTF-8 tail
        assert_eq!(validate_edmx(&input), Err(ImportError::DoctypeForbidden));
    }

    #[test]
    fn rejects_lowercase_doctype() {
        // Case-insensitive sniff: SAP servers should never emit a
        // lowercase doctype, but if some niche tooling does, we still
        // reject.
        let with_doctype = br#"<?xml version="1.0"?>
<!doctype edmx:Edmx>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0"/>"#;
        assert_eq!(
            validate_edmx(with_doctype),
            Err(ImportError::DoctypeForbidden)
        );
    }

    // ── Wrong-root classification ──

    #[test]
    fn classifies_html_redirect_as_html_page() {
        let html = br#"<!DOCTYPE html>
<html><head><title>SAP Login</title></head><body>...</body></html>"#;
        // Catch: DOCTYPE scan happens first. To exercise the classifier,
        // pass an HTML page *without* a DOCTYPE declaration.
        let html_no_doctype = b"<html><head><title>SAP Login</title></head><body></body></html>";
        assert_eq!(
            validate_edmx(html_no_doctype),
            Err(ImportError::LooksLikeHtmlPage)
        );
        // The DOCTYPE-laced HTML page rejects via XXE defense first —
        // also the correct outcome (the file is hostile-shaped either way).
        assert_eq!(validate_edmx(html), Err(ImportError::DoctypeForbidden));
    }

    #[test]
    fn classifies_atom_service_document() {
        let service_doc = br#"<?xml version="1.0" encoding="utf-8"?>
<service xmlns="http://www.w3.org/2007/app" xmlns:atom="http://www.w3.org/2005/Atom" xml:base="https://sap.example.com/sap/opu/odata/sap/UI_SVC/">
  <workspace><atom:title>Default</atom:title><collection href="EntitySet"/></workspace>
</service>"#;
        assert_eq!(
            validate_edmx(service_doc),
            Err(ImportError::LooksLikeServiceDocument)
        );
    }

    #[test]
    fn classifies_odata_error_envelope_v2() {
        let err = br#"<?xml version="1.0" encoding="utf-8"?>
<error xmlns="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
  <code>FORBIDDEN</code>
  <message xml:lang="en">User not authorized</message>
</error>"#;
        assert_eq!(validate_edmx(err), Err(ImportError::LooksLikeODataError));
    }

    #[test]
    fn classifies_odata_error_envelope_v4_prefixed() {
        let err = br#"<?xml version="1.0" encoding="utf-8"?>
<m:error xmlns:m="http://docs.oasis-open.org/odata/ns/metadata">
  <m:code>403</m:code>
  <m:message>Forbidden</m:message>
</m:error>"#;
        assert_eq!(validate_edmx(err), Err(ImportError::LooksLikeODataError));
    }

    // ── Structural-shape rejects ──

    #[test]
    fn rejects_xml_with_unknown_root_namespace() {
        // `Edmx` element but wrong namespace — someone hand-rolled XML
        // copying the EDMX shape.
        let weird = br#"<?xml version="1.0"?>
<Edmx xmlns="http://example.com/not-edmx" Version="4.0"/>"#;
        match validate_edmx(weird).unwrap_err() {
            ImportError::NotEdmxRoot { root_name } => assert_eq!(root_name, "Edmx"),
            other => panic!("expected NotEdmxRoot, got {other:?}"),
        }
    }

    #[test]
    fn rejects_xml_with_unknown_root_local_name() {
        let weird = br#"<?xml version="1.0"?>
<something xmlns="http://docs.oasis-open.org/odata/ns/edmx"/>"#;
        match validate_edmx(weird).unwrap_err() {
            ImportError::NotEdmxRoot { root_name } => assert_eq!(root_name, "something"),
            other => panic!("expected NotEdmxRoot, got {other:?}"),
        }
    }

    #[test]
    fn rejects_edmx_without_schema() {
        let no_schema = br#"<?xml version="1.0"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices/>
</edmx:Edmx>"#;
        assert_eq!(validate_edmx(no_schema), Err(ImportError::NoSchema));
    }

    #[test]
    fn accepts_edmx_with_helper_schema_lacking_namespace_before_real_one() {
        // SAP services sometimes emit multiple Schema elements — common
        // case is a helper Schema (no namespace) followed by the real
        // service Schema. Previous code only checked the *first* Schema
        // and would reject this; `find_map` over all schemas accepts.
        let multi_schema = br#"<?xml version="1.0"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm"/>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace="com.sap.gateway.srvd.real_svc.v0001">
      <EntityType Name="X"/>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let v = validate_edmx(multi_schema).unwrap();
        assert_eq!(v.schema_namespace, "com.sap.gateway.srvd.real_svc.v0001");
    }

    #[test]
    fn rejects_edmx_when_no_schema_has_non_empty_namespace() {
        // All Schema elements have empty / missing Namespace — nothing
        // for `find_map` to return. Reject.
        let empty_ns = br#"<?xml version="1.0"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" Version="4.0">
  <edmx:DataServices>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm" Namespace=""/>
    <Schema xmlns="http://docs.oasis-open.org/odata/ns/edm"/>
  </edmx:DataServices>
</edmx:Edmx>"#;
        assert_eq!(validate_edmx(empty_ns), Err(ImportError::NoSchema));
    }

    // ── Real-world fixtures from .dev/ (ignored by default — only run
    //     locally with `cargo test -- --ignored` when the gitignored
    //     fixtures are present on disk). These prove the pipeline
    //     handles actual SAP-emitted bytes end-to-end, not just our
    //     hand-rolled minimal samples. ──

    #[ignore = "requires gitignored .dev/ fixtures; run with `cargo test -- --ignored`"]
    #[test]
    fn validates_real_api_hub_edmx() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.dev/OP_WAREHOUSEORDER_0001.edmx");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let v = validate_edmx(&bytes).expect("API Hub EDMX should validate");
        assert_eq!(v.odata_version, ODataVersion::V4);
        assert!(!v.schema_namespace.is_empty());
        // The API Hub file uses the `srvd_a2x` namespace family
        // (RAP A2X binding), not `srvd`. The label-derivation rule
        // must recognize the family-segment shape and produce
        // `API_WAREHOUSE_ORDER_TASK_2_1` — the canonical service
        // name a consultant would see in SEGW / API Hub. Earlier
        // versions of this rule only matched `srvd` and would have
        // returned the version-suffix `v0001` here.
        assert_eq!(
            derive_label_from_schema_namespace(&v.schema_namespace),
            "API_WAREHOUSE_ORDER_TASK_2_1"
        );
    }

    #[ignore = "requires gitignored .dev/ fixtures; run with `cargo test -- --ignored`"]
    #[test]
    fn validates_real_gw_client_xml() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.dev/sample_gw_client_metadata.xml");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let v = validate_edmx(&bytes).expect("GW_CLIENT XML should validate");
        assert_eq!(v.odata_version, ODataVersion::V4);
        assert_eq!(
            v.schema_namespace,
            "com.sap.gateway.srvd.ui_physstockprod.v0001"
        );
        // Label auto-derivation should suggest the canonical SAP name —
        // including the `_1` version suffix that consultants see in SEGW
        // / `/IWFND/MAINT_SERVICE` for this service.
        assert_eq!(
            derive_label_from_schema_namespace(&v.schema_namespace),
            "UI_PHYSSTOCKPROD_1"
        );
    }

    // ── Helpers ──

    #[test]
    fn strip_utf8_bom_strips_when_present() {
        let mut input = Vec::new();
        input.extend_from_slice(UTF8_BOM);
        input.extend_from_slice(b"hello");
        assert_eq!(strip_utf8_bom(&input), b"hello");
    }

    #[test]
    fn strip_utf8_bom_passthrough_when_absent() {
        let input = b"hello";
        assert_eq!(strip_utf8_bom(input), b"hello");
    }

    // ── Label auto-derivation ──

    #[test]
    fn derives_label_from_v4_sap_namespace() {
        // The version suffix becomes `_N` with leading zeros stripped —
        // this is the canonical SAP tooling presentation that consultants
        // see in SEGW / `/IWFND/MAINT_SERVICE` (`UI_PHYSSTOCKPROD_1`, not
        // `UI_PHYSSTOCKPROD`).
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.ui_physstockprod.v0001"),
            "UI_PHYSSTOCKPROD_1"
        );
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.zorder_management.v0002"),
            "ZORDER_MANAGEMENT_2"
        );
    }

    #[test]
    fn derives_label_from_v4_srvd_a2x_namespace() {
        // RAP A2X binding namespace shape — observed on real API Hub
        // V4 services (e.g. `OP_WAREHOUSEORDER_0001.edmx`). The
        // namespace family segment is `srvd_a2x` instead of `srvd`;
        // the rule must still extract the service core + version.
        assert_eq!(
            derive_label_from_schema_namespace(
                "com.sap.gateway.srvd_a2x.api_warehouse_order_task_2.v0001"
            ),
            "API_WAREHOUSE_ORDER_TASK_2_1"
        );
        // Real SAP API Hub example: ZCUSTOMERS_O4 service.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd_a2x.zcustomers_o4.v0001"),
            "ZCUSTOMERS_O4_1"
        );
    }

    #[test]
    fn derives_label_from_other_gateway_families() {
        // Future-proofing: the rule strips the
        // `com.sap.gateway.<family>.` prefix uniformly so unknown
        // family segments (e.g. a hypothetical `srvd_v2`) still produce
        // a sensible label rather than falling through to the
        // trailing-segment fallback.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.someotherfamily.my_service.v0003"),
            "MY_SERVICE_3"
        );
    }

    #[test]
    fn derives_label_v4_strips_leading_zeros_from_version() {
        // Two-digit version: still emit `_10`.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.ui_physstockprod.v0010"),
            "UI_PHYSSTOCKPROD_10"
        );
        // Three-digit version: `_100`.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.ui_physstockprod.v0100"),
            "UI_PHYSSTOCKPROD_100"
        );
    }

    #[test]
    fn derives_label_v4_handles_zero_version() {
        // All-zeros: preserve a single `0` rather than collapsing to empty.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.ui_physstockprod.v0000"),
            "UI_PHYSSTOCKPROD_0"
        );
    }

    #[test]
    fn derives_label_from_v4_namespace_without_version_suffix() {
        // Some V4 namespaces don't carry the version suffix; emit
        // `<SVC>` only.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.ui_physstockprod"),
            "UI_PHYSSTOCKPROD"
        );
    }

    #[test]
    fn derives_label_from_v2_plain_namespace() {
        assert_eq!(derive_label_from_schema_namespace("ZTEST_SRV"), "ZTEST_SRV");
    }

    #[test]
    fn derives_label_from_v2_vendor_prefixed_namespace() {
        assert_eq!(
            derive_label_from_schema_namespace("vendor.ZTEST_SRV"),
            "ZTEST_SRV"
        );
        assert_eq!(
            derive_label_from_schema_namespace("acme.subscope.MY_SERVICE"),
            "MY_SERVICE"
        );
    }

    #[test]
    fn derive_label_handles_empty_namespace() {
        assert_eq!(derive_label_from_schema_namespace(""), "");
        assert_eq!(derive_label_from_schema_namespace("   "), "");
    }

    #[test]
    fn derive_label_returns_empty_for_url_shaped_namespace() {
        // Naïve `rsplit('.').next()` would return `com/custom/schema`
        // here — useless as a service label. Return empty so the caller
        // falls back to the filename stem.
        assert_eq!(
            derive_label_from_schema_namespace("http://example.com/custom/schema"),
            ""
        );
        assert_eq!(
            derive_label_from_schema_namespace("https://api.example.com/v1/Service"),
            ""
        );
    }

    #[test]
    fn derive_label_returns_empty_for_non_ascii_namespace() {
        // Non-ASCII namespaces are exotic; rather than slugify-mangle them,
        // return empty and let the user pick a filename.
        assert_eq!(derive_label_from_schema_namespace("日本.語"), "");
        assert_eq!(derive_label_from_schema_namespace("café.service"), "");
    }

    #[test]
    fn derive_label_returns_empty_for_punctuation_in_namespace() {
        // Anything outside `[A-Za-z0-9_.]` disqualifies the V2 fallback.
        assert_eq!(derive_label_from_schema_namespace("weird:funky:thing"), "");
        assert_eq!(derive_label_from_schema_namespace("a/b/c"), "");
        assert_eq!(derive_label_from_schema_namespace("has spaces.SVC"), "");
        assert_eq!(derive_label_from_schema_namespace("svc-with-dash"), "");
    }

    #[test]
    fn derive_label_accepts_canonical_dotted_identifier_fallback() {
        // The classic V2 SAP pattern still works under the tightened
        // fallback — alphanumerics, underscore, and dot only.
        assert_eq!(
            derive_label_from_schema_namespace("some.random.namespace"),
            "namespace"
        );
        assert_eq!(
            derive_label_from_schema_namespace("acme.subscope.MY_SERVICE"),
            "MY_SERVICE"
        );
    }

    #[test]
    fn derive_label_handles_non_digit_v_suffix() {
        // `.vfoo` is not a version suffix — leave the namespace as-is for
        // the upper-case conversion to handle.
        assert_eq!(
            derive_label_from_schema_namespace("com.sap.gateway.srvd.something.vfoo"),
            "SOMETHING.VFOO"
        );
    }
}
