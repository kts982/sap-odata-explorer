//! Fiori-readiness linter — evaluates an `EntityType` against a
//! checklist of annotations a Fiori list-report / object-page service
//! would normally declare. Callable from the desktop's describe panel
//! (SAP View) and from the CLI's `lint` subcommand. Purely derives
//! from already-parsed metadata; no I/O.

use crate::metadata::EntityType;

mod profile;
mod types;
pub use profile::{LintProfile, detect_profile};
pub use types::{LintCategory, LintFinding, LintSeverity};
use types::{actionable, pass};

/// Evaluate an entity type for Fiori readiness. Returns a list of
/// findings in a stable order — callers can render them as-is. Each
/// actionable (Warn/Miss) finding carries an ABAP-CDS "fix hint" so
/// the linter teaches instead of just grading. First finding is a
/// Profile marker; subsequent checks are profile-aware — a
/// value-help entity doesn't get dinged for missing `UI.LineItem`.
pub fn evaluate_entity_type(et: &EntityType) -> Vec<LintFinding> {
    let profile = detect_profile(et);
    evaluate_entity_type_with_profile(et, profile)
}

/// Variant that accepts an explicit profile — useful for CLI
/// overrides and tests. The auto-detected profile is not always
/// right (heuristics are thin when key annotations are missing).
pub fn evaluate_entity_type_with_profile(
    et: &EntityType,
    profile: LintProfile,
) -> Vec<LintFinding> {
    let mut out = Vec::new();

    // Profile banner — always first so the UI can show it up top.
    out.push(LintFinding {
        severity: LintSeverity::Pass,
        category: LintCategory::Profile,
        code: "profile",
        message: format!("Evaluated as {}.", profile.label()),
        suggested_cds: None,
        why_in_fiori: None,
    });

    let is_value_help = profile == LintProfile::ValueHelp;
    let is_object_page = profile == LintProfile::ObjectPage;
    let is_analytical = profile == LintProfile::Analytical;

    // Identity ─────────────────────────────────────────────────────────
    // Value-help entities don't need HeaderInfo — they're internal
    // lookup targets. Everything else does.
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

    // List-report ──────────────────────────────────────────────────────
    // LineItem only makes sense for list-report / analytical shapes.
    // Value-help entities never need it; object-page roots defer it
    // to their item-child entity.
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

    // Filtering ────────────────────────────────────────────────────────
    // SelectionFields drive the filter bar — relevant for list-report
    // and transactional-list services. Object-page and value-help
    // don't surface a filter bar to the end user.
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

    // Field-level consistency ──────────────────────────────────────────
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

    // Consistency — contradictions inside the declared annotations.
    // These are warns because SAP services CAN run with them; the
    // server will return 4xx the first time the user tries the bad
    // combo. Better to catch them up front.
    //
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

    // Integrity (dangling-reference) checks ─────────────────────────────
    // These flag annotations whose Path/target references a property
    // that doesn't exist on the entity. The usual cause is a column
    // renamed in one CDS layer without the annotation being updated;
    // the service still serves metadata but the target column won't
    // resolve at runtime. Each finding names the source annotation
    // and the bad target so the fix is obvious.
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

    // Capabilities ─────────────────────────────────────────────────────
    // We don't reject services that omit these entirely (many transactional
    // services do), but flag it as a hint.
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

    out
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::parse_metadata;

    #[test]
    fn reports_missing_header_line_item_selection_fields() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="PlainType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Plain" EntityType="n.PlainType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("PlainType").unwrap();
        let findings = evaluate_entity_type(et);
        let codes: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Miss)
            .map(|f| f.code)
            .collect();
        assert!(codes.contains(&"header_info"));
        assert!(codes.contains(&"line_item"));
        assert!(codes.contains(&"selection_fields"));
    }

    #[test]
    fn flags_decimal_without_unit() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="NetAmount" Type="Edm.Decimal"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        assert!(
            findings
                .iter()
                .any(|f| f.code == "unit_missing" && f.message.contains("NetAmount"))
        );
    }

    #[test]
    fn actionable_findings_carry_cds_fix_hints() {
        // Same "Plain" service with nothing declared; every miss/warn
        // finding should carry suggested_cds + why_in_fiori. Passes
        // stay bare (no hint needed).
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="PlainType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="NetAmount" Type="Edm.Decimal"/>
        <Property Name="CustomerID" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Plain" EntityType="n.PlainType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("PlainType").unwrap();
        let findings = evaluate_entity_type(et);
        for f in &findings {
            if f.severity == LintSeverity::Pass {
                // Passes can skip hints — they're not actionable.
                continue;
            }
            assert!(
                f.suggested_cds.is_some(),
                "actionable finding {:?} should carry suggested_cds",
                f.code
            );
            assert!(
                f.why_in_fiori.is_some(),
                "actionable finding {:?} should carry why_in_fiori",
                f.code
            );
        }
        // Spot-check specific mappings.
        let by_code: std::collections::HashMap<_, _> =
            findings.iter().map(|f| (f.code, f)).collect();
        assert_eq!(by_code["header_info"].suggested_cds, Some("@UI.headerInfo"));
        assert_eq!(by_code["line_item"].suggested_cds, Some("@UI.lineItem"));
        assert_eq!(
            by_code["selection_fields"].suggested_cds,
            Some("@UI.selectionField")
        );
        assert_eq!(
            by_code["text_missing"].suggested_cds,
            Some("@ObjectModel.text.element")
        );
    }

    #[test]
    fn value_help_profile_skips_list_report_misses() {
        // A *_VH entity (auto-detected as ValueHelp) should NOT get
        // dinged for missing HeaderInfo / LineItem / SelectionFields.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="WarehouseVHType">
        <Key><PropertyRef Name="Warehouse"/></Key>
        <Property Name="Warehouse" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="WarehouseVH" EntityType="n.WarehouseVHType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("WarehouseVHType").unwrap();
        let findings = evaluate_entity_type(et);
        // Profile banner is first.
        assert_eq!(findings[0].code, "profile");
        assert!(findings[0].message.contains("value_help"));
        // No miss findings for list-report-shaped checks.
        let miss_codes: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Miss)
            .map(|f| f.code)
            .collect();
        assert!(!miss_codes.contains(&"header_info"));
        assert!(!miss_codes.contains(&"line_item"));
        assert!(!miss_codes.contains(&"selection_fields"));
    }

    #[test]
    fn consistency_rule_selection_on_non_filterable() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="InternalNote" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
            <PropertyPath>InternalNote</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.Container/Orders">
        <Annotation Term="SAP__capabilities.FilterRestrictions">
          <Record>
            <PropertyValue Property="NonFilterableProperties">
              <Collection>
                <PropertyPath>InternalNote</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let contradiction = findings
            .iter()
            .find(|f| f.code == "selection_non_filterable")
            .expect("consistency rule should fire");
        assert_eq!(contradiction.severity, LintSeverity::Warn);
        assert!(contradiction.message.contains("InternalNote"));
    }

    #[test]
    fn consistency_rule_text_arrangement_without_common_text() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Product" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.TextArrangement" EnumMember="UI.TextArrangementType/TextFirst"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let lonely = findings
            .iter()
            .find(|f| f.code == "text_arrangement_lonely");
        assert!(
            lonely.is_some(),
            "should flag lonely TextArrangement on Product"
        );
        assert!(lonely.unwrap().message.contains("Product"));
    }

    #[test]
    fn integrity_dangling_text_and_line_item_targets() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Product" Type="Edm.String"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Product">
        <Annotation Term="SAP__common.Text" Path="ProductDescription"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.LineItem">
          <Collection>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="Product"/></Record>
            <Record Type="UI.DataField"><PropertyValue Property="Value" Path="RenamedColumn"/></Record>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
            <PropertyPath>OldWarehouse</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let by_code: std::collections::HashMap<_, _> =
            findings.iter().map(|f| (f.code, f)).collect();
        // Common.Text on Product points at missing "ProductDescription".
        assert!(
            by_code
                .get("text_target_missing")
                .map(|f| f.message.contains("ProductDescription"))
                .unwrap_or(false),
            "dangling text target should be flagged",
        );
        // LineItem references "RenamedColumn" which doesn't exist.
        assert!(
            by_code
                .get("line_item_target_missing")
                .map(|f| f.message.contains("RenamedColumn"))
                .unwrap_or(false),
            "dangling LineItem target should be flagged",
        );
        // SelectionFields references "OldWarehouse" which doesn't exist.
        assert!(
            by_code
                .get("selection_field_target_missing")
                .map(|f| f.message.contains("OldWarehouse"))
                .unwrap_or(false),
            "dangling SelectionField target should be flagged",
        );
        // All integrity findings should be warns.
        for f in by_code.values() {
            if f.category == LintCategory::Integrity {
                assert_eq!(f.severity, LintSeverity::Warn);
            }
        }
    }

    #[test]
    fn explicit_list_report_profile_emits_full_miss_set() {
        // Locks down what `LintProfile::ListReport` means when nothing is
        // declared: every list-report-shaped Miss should fire. Uses the
        // explicit-profile API so this test stays stable even if the
        // auto-detection heuristics change.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type_with_profile(et, LintProfile::ListReport);
        assert_eq!(findings[0].code, "profile");
        assert!(findings[0].message.contains("list_report"));
        let miss_codes: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Miss)
            .map(|f| f.code)
            .collect();
        assert!(miss_codes.contains(&"header_info"));
        assert!(miss_codes.contains(&"line_item"));
        assert!(miss_codes.contains(&"selection_fields"));
    }

    #[test]
    fn explicit_object_page_profile_skips_line_item_and_selection_fields() {
        // Object-page roots defer LineItem to their item child and don't
        // surface a filter bar, so neither miss should fire — but
        // header_info still matters for the page title.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type_with_profile(et, LintProfile::ObjectPage);
        assert_eq!(findings[0].code, "profile");
        assert!(findings[0].message.contains("object_page"));
        let miss_codes: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Miss)
            .map(|f| f.code)
            .collect();
        assert!(
            miss_codes.contains(&"header_info"),
            "object-page roots still need HeaderInfo for the page title"
        );
        assert!(
            !miss_codes.contains(&"line_item"),
            "object-page roots defer LineItem to the item child; should not miss"
        );
        assert!(
            !miss_codes.contains(&"selection_fields"),
            "object-page roots don't surface a filter bar; should not miss"
        );
    }

    #[test]
    fn consistency_rule_selection_hidden_fires_on_hidden_in_selection() {
        // UI.Hidden + SelectionFields contradict — the column would be
        // hidden in the table but still offered as a filter chip.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="InternalNote" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/InternalNote">
        <Annotation Term="SAP__UI.Hidden" Bool="true"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
            <PropertyPath>InternalNote</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let hit = findings
            .iter()
            .find(|f| f.code == "selection_hidden")
            .expect(
                "selection_hidden should fire when a UI.Hidden property appears in SelectionFields",
            );
        assert_eq!(hit.severity, LintSeverity::Warn);
        assert!(hit.message.contains("InternalNote"));
        // The visible Warehouse field should not appear in the message.
        assert!(!hit.message.contains("Warehouse"));
    }

    #[test]
    fn consistency_rule_value_list_no_out_fires_on_in_only_picker() {
        // A ValueList with only In parameters can show a list but has no
        // Out/InOut binding to write the picked value back to the local
        // property — the picker can browse but not select.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="SAP__common.ValueList">
          <Record>
            <PropertyValue Property="CollectionPath" String="WarehouseVH"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="SAP__common.ValueListParameterIn">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Warehouse"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let hit = findings
            .iter()
            .find(|f| f.code == "value_list_no_out")
            .expect("value_list_no_out should fire on a picker with no Out/InOut parameter");
        assert_eq!(hit.severity, LintSeverity::Warn);
        assert!(hit.message.contains("Warehouse"));
    }

    #[test]
    fn consistency_rules_emit_in_canonical_order() {
        // Locks the relative order of consistency findings so a future
        // edit can't silently re-shuffle them. The CLI and desktop both
        // render findings in stream order; a re-order would change the
        // user-visible report.
        //
        // Triggers (in expected order): selection_non_filterable,
        // selection_hidden, value_list_no_out, text_arrangement_lonely.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Warehouse" Type="Edm.String"/>
        <Property Name="InternalNote" Type="Edm.String"/>
        <Property Name="Product" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/InternalNote">
        <Annotation Term="SAP__UI.Hidden" Bool="true"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType/Warehouse">
        <Annotation Term="SAP__common.ValueList">
          <Record>
            <PropertyValue Property="CollectionPath" String="WarehouseVH"/>
            <PropertyValue Property="Parameters">
              <Collection>
                <Record Type="SAP__common.ValueListParameterIn">
                  <PropertyValue Property="LocalDataProperty" PropertyPath="Warehouse"/>
                  <PropertyValue Property="ValueListProperty" String="Warehouse"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>Warehouse</PropertyPath>
            <PropertyPath>InternalNote</PropertyPath>
          </Collection>
        </Annotation>
        <!-- Type-level TextArrangement applies to all properties without
             their own override (per-property TextArrangement only parses
             when nested inside Common.Text). None of these properties has
             a Common.Text, so text_arrangement_lonely fires. -->
        <Annotation Term="SAP__UI.TextArrangement" EnumMember="UI.TextArrangementType/TextFirst"/>
      </Annotations>
      <Annotations Target="SAP__self.Container/Orders">
        <Annotation Term="SAP__capabilities.FilterRestrictions">
          <Record>
            <PropertyValue Property="NonFilterableProperties">
              <Collection>
                <PropertyPath>InternalNote</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let consistency_codes: Vec<&str> = findings
            .iter()
            .filter(|f| {
                matches!(
                    f.code,
                    "selection_non_filterable"
                        | "selection_hidden"
                        | "value_list_no_out"
                        | "text_arrangement_lonely"
                )
            })
            .map(|f| f.code)
            .collect();
        assert_eq!(
            consistency_codes,
            vec![
                "selection_non_filterable",
                "selection_hidden",
                "value_list_no_out",
                "text_arrangement_lonely",
            ],
            "consistency rules must stream in the canonical order rendered by CLI/desktop",
        );
    }

    #[test]
    fn consistency_rule_selection_variant_references_non_filterable() {
        // SelectionVariant select_options point at a non-filterable
        // property — the variant would inject a `$filter` clause the
        // server rejects.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="InternalNote" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.SelectionVariant" Qualifier="WithNote">
          <Record>
            <PropertyValue Property="Text" String="Notes only"/>
            <PropertyValue Property="SelectOptions">
              <Collection>
                <Record>
                  <PropertyValue Property="PropertyName" PropertyPath="InternalNote"/>
                  <PropertyValue Property="Ranges">
                    <Collection>
                      <Record Type="UI.SelectionRangeType">
                        <PropertyValue Property="Sign" EnumMember="UI.SelectionRangeSignType/I"/>
                        <PropertyValue Property="Option" EnumMember="UI.SelectionRangeOptionType/EQ"/>
                        <PropertyValue Property="Low" String="x"/>
                      </Record>
                    </Collection>
                  </PropertyValue>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
      <Annotations Target="SAP__self.Container/Orders">
        <Annotation Term="SAP__capabilities.FilterRestrictions">
          <Record>
            <PropertyValue Property="NonFilterableProperties">
              <Collection>
                <PropertyPath>InternalNote</PropertyPath>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let hit = findings
            .iter()
            .find(|f| f.code == "selection_variant_dead_filter")
            .expect("selection_variant_dead_filter should fire when a variant references a non-filterable property");
        assert_eq!(hit.severity, LintSeverity::Warn);
        assert!(hit.message.contains("InternalNote"));
    }

    #[test]
    fn consistency_rule_semantic_object_without_semantic_key() {
        // SemanticObject declared but no SemanticKey — Fiori cross-app
        // nav can't form the target URL with a business key.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="WarehouseId" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/WarehouseId">
        <Annotation Term="SAP__common.SemanticObject" String="WarehouseManagement"/>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let hit = findings
            .iter()
            .find(|f| f.code == "semantic_object_no_key")
            .expect("semantic_object_no_key should fire when SemanticObject is set without a SemanticKey");
        assert_eq!(hit.severity, LintSeverity::Warn);
        assert!(hit.message.contains("WarehouseId"));
    }

    #[test]
    fn consistency_rule_semantic_object_with_semantic_key_passes() {
        // Same SemanticObject, but now with a Common.SemanticKey
        // declared — the cross-app nav has a business key to pass, so
        // the rule should NOT fire.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="WarehouseId" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/WarehouseId">
        <Annotation Term="SAP__common.SemanticObject" String="WarehouseManagement"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__common.SemanticKey">
          <Collection>
            <PropertyPath>WarehouseId</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        assert!(
            !findings.iter().any(|f| f.code == "semantic_object_no_key"),
            "rule should not fire when SemanticKey is declared",
        );
    }

    #[test]
    fn integrity_rules_emit_in_canonical_order() {
        // Locks integrity-rule order: text_target_missing, then
        // semantic_key_target_missing, then selection_field_target_missing,
        // then line_item_target_missing, then sort_order_target_missing.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
        <Property Name="Product" Type="Edm.String"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType/Product">
        <Annotation Term="SAP__common.Text" Path="MissingText"/>
      </Annotations>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__common.SemanticKey">
          <Collection>
            <PropertyPath>MissingKey</PropertyPath>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>MissingFilter</PropertyPath>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.LineItem">
          <Collection>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="MissingColumn"/>
            </Record>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.PresentationVariant">
          <Record>
            <PropertyValue Property="SortOrder">
              <Collection>
                <Record Type="Common.SortOrderType">
                  <PropertyValue Property="Property" PropertyPath="MissingSort"/>
                </Record>
              </Collection>
            </PropertyValue>
          </Record>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let integrity_codes: Vec<&str> = findings
            .iter()
            .filter(|f| f.category == LintCategory::Integrity)
            .map(|f| f.code)
            .collect();
        assert_eq!(
            integrity_codes,
            vec![
                "text_target_missing",
                "semantic_key_target_missing",
                "selection_field_target_missing",
                "line_item_target_missing",
                "sort_order_target_missing",
            ],
            "integrity rules must stream in the canonical order rendered by CLI/desktop",
        );
    }

    #[test]
    fn passes_when_header_and_line_item_declared() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<edmx:Edmx xmlns:edmx="http://docs.oasis-open.org/odata/ns/edmx" xmlns="http://docs.oasis-open.org/odata/ns/edm" Version="4.0">
  <edmx:DataServices>
    <Schema Namespace="n" Alias="SAP__self">
      <EntityType Name="OrderType">
        <Key><PropertyRef Name="ID"/></Key>
        <Property Name="ID" Type="Edm.String" Nullable="false"/>
      </EntityType>
      <EntityContainer Name="Container"><EntitySet Name="Orders" EntityType="n.OrderType"/></EntityContainer>
      <Annotations Target="SAP__self.OrderType">
        <Annotation Term="SAP__UI.HeaderInfo">
          <Record>
            <PropertyValue Property="TypeName" String="Order"/>
          </Record>
        </Annotation>
        <Annotation Term="SAP__UI.LineItem">
          <Collection>
            <Record Type="UI.DataField">
              <PropertyValue Property="Value" Path="ID"/>
            </Record>
          </Collection>
        </Annotation>
        <Annotation Term="SAP__UI.SelectionFields">
          <Collection>
            <PropertyPath>ID</PropertyPath>
          </Collection>
        </Annotation>
      </Annotations>
    </Schema>
  </edmx:DataServices>
</edmx:Edmx>"#;
        let meta = parse_metadata(xml).unwrap();
        let et = meta.find_entity_type("OrderType").unwrap();
        let findings = evaluate_entity_type(et);
        let misses: Vec<_> = findings
            .iter()
            .filter(|f| f.severity == LintSeverity::Miss)
            .collect();
        assert!(misses.is_empty(), "unexpected misses: {misses:?}");
    }
}
