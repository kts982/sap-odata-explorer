//! Lint rules grouped by category. Each `check_*` function pushes its
//! findings onto a shared `Vec<LintFinding>` in the order the parent
//! evaluator calls them; the in-function ordering is part of the
//! contract too — `tests::consistency_rules_emit_in_canonical_order`
//! and `tests::integrity_rules_emit_in_canonical_order` in
//! `lint/mod.rs` lock the stream order the CLI and desktop render.
//!
//! Behaviour-preserving move from `lint/mod.rs`: no rule changes, no
//! visibility widening past `pub(super)`, no public-API impact.

use crate::metadata::EntityType;

use super::profile::LintProfile;
use super::types::{LintCategory, LintFinding, LintSeverity, actionable, pass};

// ── Identity ────────────────────────────────────────────────────────
// Value-help entities don't need HeaderInfo — they're internal lookup
// targets. Everything else does.
pub(super) fn check_identity(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;

    if et.header_info.is_some() {
        out.push(pass(
            LintCategory::Identity,
            "header_info",
            "UI.HeaderInfo declared.".to_string(),
        ));
    } else if !is_value_help {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::Identity,
            "header_info",
            "No UI.HeaderInfo — Fiori will fall back to the technical type name for titles."
                .to_string(),
            "@UI.headerInfo",
            "Drives object-page titles and the list report's singular/plural labels.",
        ));
    }
    if !et.semantic_keys.is_empty() {
        out.push(pass(
            LintCategory::Identity,
            "semantic_key",
            format!("Common.SemanticKey: {}", et.semantic_keys.join(", ")),
        ));
    } else if et.keys.iter().any(|k| is_uuid_like_key(k)) {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Identity,
            "semantic_key",
            "Technical key looks UUID-ish but no Common.SemanticKey declared — Fiori will show the UUID.".to_string(),
            "@ObjectModel.semanticKey",
            "Lets Fiori show the *business* key (e.g. Product, OrderID) instead of the technical UUID.",
        ));
    }
}

// ── List-report ─────────────────────────────────────────────────────
// LineItem only makes sense for list-report / analytical shapes.
// Value-help entities never need it; object-page roots defer it
// to their item-child entity.
pub(super) fn check_list_report(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;
    let is_object_page = profile == LintProfile::ObjectPage;
    let is_analytical = profile == LintProfile::Analytical;

    if !et.line_item.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "line_item",
            format!("UI.LineItem: {} column(s) declared.", et.line_item.len()),
        ));
    } else if !is_value_help && !is_object_page && !is_analytical {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::ListReport,
            "line_item",
            "No UI.LineItem — the list report will have no default columns.".to_string(),
            "@UI.lineItem",
            "Declares which columns the list report shows by default and in what order.",
        ));
    }
    if !et.request_at_least.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "request_at_least",
            format!(
                "UI.PresentationVariant.RequestAtLeast: +{} support field(s).",
                et.request_at_least.len(),
            ),
        ));
    }
    if !et.sort_order.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "sort_order",
            format!(
                "UI.PresentationVariant.SortOrder: {} clause(s).",
                et.sort_order.len(),
            ),
        ));
    }
}

// ── Filtering ───────────────────────────────────────────────────────
// SelectionFields drive the filter bar — relevant for list-report
// and transactional-list services. Object-page and value-help
// don't surface a filter bar to the end user.
pub(super) fn check_filtering(et: &EntityType, profile: LintProfile, out: &mut Vec<LintFinding>) {
    let is_value_help = profile == LintProfile::ValueHelp;
    let is_object_page = profile == LintProfile::ObjectPage;

    if !et.selection_fields.is_empty() {
        out.push(pass(
            LintCategory::Filtering,
            "selection_fields",
            format!(
                "UI.SelectionFields: {} field(s).",
                et.selection_fields.len()
            ),
        ));
    } else if !is_value_help && !is_object_page {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::Filtering,
            "selection_fields",
            "No UI.SelectionFields — the filter bar will be empty by default.".to_string(),
            "@UI.selectionField",
            "Populates the Fiori filter bar — the properties the user sees as ready-to-filter fields.",
        ));
    }
    if !et.selection_variants.is_empty() {
        out.push(pass(
            LintCategory::Filtering,
            "selection_variants",
            format!(
                "UI.SelectionVariant: {} declared.",
                et.selection_variants.len(),
            ),
        ));
    }
}

// ── Field-level consistency ─────────────────────────────────────────
// Decimal-shaped properties without a unit / currency companion;
// code-shaped properties without a paired Common.Text.
pub(super) fn check_fields(et: &EntityType, out: &mut Vec<LintFinding>) {
    let decimal_without_unit: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| {
            looks_monetary_or_quantity(&p.name, &p.edm_type)
                && p.unit_path.is_none()
                && p.iso_currency_path.is_none()
        })
        .map(|p| p.name.as_str())
        .collect();
    if !decimal_without_unit.is_empty() {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Fields,
            "unit_missing",
            format!(
                "Decimal/amount-looking properties without Measures.Unit or ISOCurrency: {}",
                decimal_without_unit.join(", "),
            ),
            "@Semantics.amount.currencyCode / @Semantics.quantity.unitOfMeasure",
            "Lets Fiori format the value with the right currency or unit — otherwise it's a raw decimal.",
        ));
    }
    let code_without_text: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| looks_code(&p.name) && p.text_path.is_none())
        .map(|p| p.name.as_str())
        .collect();
    if !code_without_text.is_empty() {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Fields,
            "text_missing",
            format!(
                "Code-looking properties without Common.Text: {}",
                code_without_text.join(", "),
            ),
            "@ObjectModel.text.element",
            "Pairs the code column with a human-readable description column (Fiori renders them together per UI.TextArrangement).",
        ));
    }
}

// ── Consistency ─────────────────────────────────────────────────────
// Contradictions inside the declared annotations. Warn-level because
// SAP services CAN run with these; the server returns 4xx the first
// time the user tries the bad combo — better to catch them up front.
//
// Stream order is contract: the seven rules below ship in the order
// `tests::consistency_rules_emit_in_canonical_order` asserts.
pub(super) fn check_consistency(et: &EntityType, out: &mut Vec<LintFinding>) {
    // 1. SelectionField points at a non-filterable property.
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|name| {
                et.properties
                    .iter()
                    .find(|p| &p.name == *name)
                    .is_some_and(|p| p.filterable == Some(false))
            })
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_non_filterable",
                format!(
                    "UI.SelectionFields references non-filterable column(s): {}",
                    bad.join(", "),
                ),
                "@Consumption.filter.hidden + @UI.selectionField consistency",
                "Fiori will show the chip in the filter bar but the server will reject `$filter` on it — pick one.",
            ));
        }
    }
    // 2. SortOrder references a non-sortable property.
    {
        let bad: Vec<&str> = et
            .sort_order
            .iter()
            .filter_map(|s| {
                et.properties
                    .iter()
                    .find(|p| p.name == s.property)
                    .filter(|p| p.sortable == Some(false))
                    .map(|_| s.property.as_str())
            })
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::ListReport,
                "sort_non_sortable",
                format!(
                    "UI.PresentationVariant.SortOrder references non-sortable column(s): {}",
                    bad.join(", "),
                ),
                "@Consumption.filter.sortable + SortOrder consistency",
                "Server will 400 on `$orderby` — the declared default sort will never actually run.",
            ));
        }
    }
    // 3. UI.Hidden property appears in SelectionFields.
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|name| {
                et.properties
                    .iter()
                    .find(|p| &p.name == *name)
                    .is_some_and(|p| p.hidden)
            })
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_hidden",
                format!(
                    "UI.SelectionFields includes UI.Hidden column(s): {}",
                    bad.join(", "),
                ),
                "@UI.hidden + @UI.selectionField consistency",
                "Fiori would hide the column but also offer it as a filter — the two signals contradict.",
            ));
        }
    }
    // 4. ValueList without Out/InOut parameter — no way to write
    //    back to the local property on pick.
    {
        let bad: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| {
                p.value_list_variants.iter().any(|vl| {
                    !vl.parameters.iter().any(|param| {
                        matches!(
                            param.kind,
                            crate::metadata::ValueListParameterKind::InOut
                                | crate::metadata::ValueListParameterKind::Out
                        ) && param.local_property.is_some()
                    })
                })
            })
            .map(|p| p.name.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Fields,
                "value_list_no_out",
                format!(
                    "Common.ValueList without an InOut/Out parameter bound to the local property: {}",
                    bad.join(", "),
                ),
                "@Consumption.valueHelpDefinition parameter mapping",
                "The picker can show a list but has nowhere to write the picked value back — users can browse but not select.",
            ));
        }
    }
    // 5. TextArrangement without a Common.Text to arrange.
    {
        let bad: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| p.text_arrangement.is_some() && p.text_path.is_none())
            .map(|p| p.name.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Fields,
                "text_arrangement_lonely",
                format!(
                    "UI.TextArrangement declared without Common.Text on: {}",
                    bad.join(", "),
                ),
                "@ObjectModel.text.element + @UI.textArrangement pair",
                "There's nothing to arrange — the arrangement hint has no companion description column.",
            ));
        }
    }
    // 6. SelectionVariant references hidden or non-filterable property.
    //    The variant would surface a default filter that the column
    //    can't actually filter on — Fiori shows the chip, the server
    //    rejects the `$filter`. Walks both Parameters and SelectOptions
    //    since either form ends up in the generated `$filter`.
    {
        let mut bad: Vec<String> = Vec::new();
        for sv in &et.selection_variants {
            let names = sv
                .parameters
                .iter()
                .map(|p| p.property_name.as_str())
                .chain(sv.select_options.iter().map(|s| s.property_name.as_str()));
            for name in names {
                if let Some(p) = et.properties.iter().find(|p| p.name == name)
                    && (p.hidden || p.filterable == Some(false))
                    && !bad.iter().any(|b| b == name)
                {
                    bad.push(name.to_string());
                }
            }
        }
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Filtering,
                "selection_variant_dead_filter",
                format!(
                    "UI.SelectionVariant references hidden or non-filterable column(s): {}",
                    bad.join(", "),
                ),
                "@UI.selectionVariant + filter/hidden consistency",
                "The variant injects a `$filter` clause for a column the server won't filter on — the variant won't apply at runtime.",
            ));
        }
    }
    // 7. SemanticObject without SemanticKey. Fiori cross-app navigation
    //    forms the target URL from the SemanticObject + business-key
    //    parameters; without a SemanticKey it has no business key to
    //    pass, so the launchpad link will be malformed or generic.
    {
        let semantic_object_props: Vec<&str> = et
            .properties
            .iter()
            .filter(|p| p.semantic_object.is_some())
            .map(|p| p.name.as_str())
            .collect();
        if !semantic_object_props.is_empty() && et.semantic_keys.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Identity,
                "semantic_object_no_key",
                format!(
                    "Common.SemanticObject declared on {} but no Common.SemanticKey on the entity.",
                    semantic_object_props.join(", "),
                ),
                "@Consumption.semanticObject + @ObjectModel.semanticKey",
                "Cross-app nav target won't carry a business key — the launchpad link will be malformed or open the generic landing page.",
            ));
        }
    }
}

// ── Integrity (dangling-reference) checks ───────────────────────────
// These flag annotations whose Path/target references a property
// that doesn't exist on the entity. The usual cause is a column
// renamed in one CDS layer without the annotation being updated;
// the service still serves metadata but the target column won't
// resolve at runtime. Each finding names the source annotation and
// the bad target so the fix is obvious.
//
// Stream order is contract: the nine rules below ship in the order
// `tests::integrity_rules_emit_in_canonical_order` asserts.
pub(super) fn check_integrity(et: &EntityType, out: &mut Vec<LintFinding>) {
    let prop_names: std::collections::HashSet<&str> =
        et.properties.iter().map(|p| p.name.as_str()).collect();
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.text_path
                    .as_ref()
                    .filter(|target| !prop_names.contains(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "text_target_missing",
                format!(
                    "Common.Text points at a column that doesn't exist on this entity: {}",
                    dangling.join(", "),
                ),
                "@ObjectModel.text.element",
                "The referenced description column isn't reachable — Fiori will render the raw code with no text.",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.unit_path
                    .as_ref()
                    .filter(|target| !prop_names.contains(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "unit_target_missing",
                format!(
                    "Measures.Unit / sap:unit points at a column that doesn't exist: {}",
                    dangling.join(", "),
                ),
                "@Semantics.quantity.unitOfMeasure",
                "The quantity column has no resolvable unit companion — Fiori will show the raw number.",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| {
                p.iso_currency_path
                    .as_ref()
                    .filter(|target| !prop_names.contains(target.as_str()))
                    .map(|target| format!("{} → {}", p.name, target))
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "currency_target_missing",
                format!(
                    "Measures.ISOCurrency points at a column that doesn't exist: {}",
                    dangling.join(", "),
                ),
                "@Semantics.amount.currencyCode",
                "The amount column has no resolvable currency companion — Fiori can't format the value.",
            ));
        }
    }
    {
        let dangling: Vec<String> = et
            .properties
            .iter()
            .filter_map(|p| match &p.criticality {
                Some(crate::metadata::Criticality::Path(path))
                    if !prop_names.contains(path.as_str()) =>
                {
                    Some(format!("{} → {}", p.name, path))
                }
                _ => None,
            })
            .collect();
        if !dangling.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "criticality_target_missing",
                format!(
                    "UI.Criticality Path references a column that doesn't exist: {}",
                    dangling.join(", "),
                ),
                "@UI.criticality (Path form)",
                "Cell coloring depends on a column that won't resolve — cells will never get colored.",
            ));
        }
    }
    if let Some(hi) = &et.header_info
        && let Some(title) = &hi.title_path
        && !prop_names.contains(title.as_str())
    {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Integrity,
            "header_info_title_missing",
            format!(
                "UI.HeaderInfo.Title references a column that doesn't exist: {}",
                title,
            ),
            "@UI.headerInfo.title.value",
            "Object-page title won't resolve — Fiori will fall back to the technical type name.",
        ));
    }
    {
        let bad: Vec<&str> = et
            .semantic_keys
            .iter()
            .filter(|n| !prop_names.contains(n.as_str()))
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "semantic_key_target_missing",
                format!(
                    "Common.SemanticKey references column(s) that don't exist: {}",
                    bad.join(", "),
                ),
                "@ObjectModel.semanticKey",
                "The business-key hint points at columns Fiori can't resolve.",
            ));
        }
    }
    {
        let bad: Vec<&str> = et
            .selection_fields
            .iter()
            .filter(|n| !prop_names.contains(n.as_str()))
            .map(String::as_str)
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "selection_field_target_missing",
                format!(
                    "UI.SelectionFields references column(s) that don't exist: {}",
                    bad.join(", "),
                ),
                "@UI.selectionField",
                "Filter-bar chip will be dead — server will 400 when the user types a value.",
            ));
        }
    }
    {
        let bad: Vec<String> = et
            .line_item
            .iter()
            .filter(|f| !prop_names.contains(f.value_path.as_str()))
            .map(|f| f.value_path.clone())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "line_item_target_missing",
                format!(
                    "UI.LineItem DataField references column(s) that don't exist: {}",
                    bad.join(", "),
                ),
                "@UI.lineItem",
                "Declared list-report columns point at properties that aren't on the entity — they'll silently drop at runtime.",
            ));
        }
    }
    {
        let bad: Vec<&str> = et
            .sort_order
            .iter()
            .filter(|s| !prop_names.contains(s.property.as_str()))
            .map(|s| s.property.as_str())
            .collect();
        if !bad.is_empty() {
            out.push(actionable(
                LintSeverity::Warn,
                LintCategory::Integrity,
                "sort_order_target_missing",
                format!(
                    "UI.PresentationVariant.SortOrder references column(s) that don't exist: {}",
                    bad.join(", "),
                ),
                "@UI.presentationVariant.sortOrder",
                "Declared default sort won't run — Fiori silently drops the clause.",
            ));
        }
    }
}

// ── Capabilities ────────────────────────────────────────────────────
// We don't reject services that omit these entirely (many transactional
// services do), but flag it as a hint.
pub(super) fn check_capabilities(et: &EntityType, out: &mut Vec<LintFinding>) {
    let any_capability = et.searchable.is_some()
        || et.countable.is_some()
        || et.top_supported.is_some()
        || et.skip_supported.is_some()
        || et.expandable.is_some()
        || et.properties.iter().any(|p| {
            p.filterable.is_some()
                || p.sortable.is_some()
                || p.creatable.is_some()
                || p.updatable.is_some()
                || p.required_in_filter.is_some()
        });
    if !any_capability {
        out.push(actionable(
            LintSeverity::Warn,
            LintCategory::Capabilities,
            "capabilities_silent",
            "No Capabilities.* annotations — clients can't pre-flight validate; they'll discover limits at runtime.".to_string(),
            "@Search.searchable / @Search.defaultSearchElement / @Consumption.filter.*",
            "Tells clients which operations the service actually supports so the UI can disable unsupported gestures up front.",
        ));
    }
}

// ── Heuristic predicates ────────────────────────────────────────────
// Live alongside their sole consumers (check_identity uses
// is_uuid_like_key; check_fields uses the other two). Inline so a
// future contributor reading rules.rs sees the whole story without
// jumping files.

/// Heuristic: does the property name look UUID-ish? We only use this
/// to hint a missing SemanticKey, so false positives are cheap.
fn is_uuid_like_key(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with("uuid") || lower.ends_with("_id") && lower.contains("uuid")
}

/// Heuristic: does the property look like money / quantity, so it
/// *should* have a unit companion? We key off the edm_type (Decimal)
/// plus common name suffixes to cut false positives.
fn looks_monetary_or_quantity(name: &str, edm_type: &str) -> bool {
    if !edm_type.contains("Decimal") {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    lower.contains("amount")
        || lower.contains("price")
        || lower.contains("value")
        || lower.contains("quantity")
        || lower.contains("weight")
        || lower.contains("volume")
}

/// Heuristic: does the property look like a business code (`ID`,
/// `Code`) that a human-readable text column would pair with? Tuned
/// to avoid noisy hits on UUIDs, technical-numbering fields (`*Number`
/// often means an internal sequence, not a human-facing code), and
/// classifier columns that commonly stand alone.
fn looks_code(name: &str) -> bool {
    if name.len() < 3 {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    // Skip things that are ALREADY text columns by naming convention,
    // or UUID / internal-identifier fields that shouldn't pair with a
    // description at all.
    if lower.ends_with("name")
        || lower.ends_with("description")
        || lower.ends_with("text")
        || lower.ends_with("label")
        || lower.ends_with("uuid")
        || lower.ends_with("guid")
    {
        return false;
    }
    lower.ends_with("id") || lower.ends_with("code")
}
