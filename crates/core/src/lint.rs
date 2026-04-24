//! Fiori-readiness linter — evaluates an `EntityType` against a
//! checklist of annotations a Fiori list-report / object-page service
//! would normally declare. Callable from the desktop's describe panel
//! (SAP View) and from the CLI's `lint` subcommand. Purely derives
//! from already-parsed metadata; no I/O.

use crate::metadata::EntityType;
use serde::Serialize;

/// Severity of a lint finding.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LintSeverity {
    /// The check passed — the expected annotation is present.
    Pass,
    /// The check *could* be improved, but the service is usable as-is.
    Warn,
    /// Expected annotation is missing. Fiori / consuming apps will
    /// fall back to defaults or render awkwardly.
    Miss,
}

/// Grouping tag — what part of the Fiori stack the finding affects.
/// Keeps the panel readable when there are many findings.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LintCategory {
    Identity,
    ListReport,
    Filtering,
    Fields,
    Capabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct LintFinding {
    pub severity: LintSeverity,
    pub category: LintCategory,
    pub code: &'static str,
    pub message: String,
    /// Suggested ABAP CDS annotation(s) the developer would add to
    /// *fix* this finding at the source. Short, pasteable tokens
    /// like `@ObjectModel.text.element` or
    /// `@Consumption.valueHelpDefinition`. `None` for findings that
    /// are informational rather than actionable at source level
    /// (e.g. a `Pass`).
    pub suggested_cds: Option<&'static str>,
    /// One-line explanation of what Fiori does with the annotation
    /// once it's declared — the "why does this matter?" copy that
    /// pairs with the suggestion. `None` when the finding is a
    /// simple pass or the message already says it.
    pub why_in_fiori: Option<&'static str>,
}

/// Small helper so findings stay readable — most findings don't need
/// to fill every optional field, and spreading `..` across a huge
/// struct literal fights the formatter.
fn finding(
    severity: LintSeverity,
    category: LintCategory,
    code: &'static str,
    message: String,
) -> LintFinding {
    LintFinding {
        severity,
        category,
        code,
        message,
        suggested_cds: None,
        why_in_fiori: None,
    }
}

fn pass(category: LintCategory, code: &'static str, message: String) -> LintFinding {
    finding(LintSeverity::Pass, category, code, message)
}

/// Build a Warn/Miss finding with its ABAP-CDS "fix hint" attached.
/// `suggested` is the CDS annotation token (e.g. `@UI.headerInfo`);
/// `why` is a one-line "what Fiori does with it" explanation.
fn actionable(
    severity: LintSeverity,
    category: LintCategory,
    code: &'static str,
    message: String,
    suggested: &'static str,
    why: &'static str,
) -> LintFinding {
    LintFinding {
        severity,
        category,
        code,
        message,
        suggested_cds: Some(suggested),
        why_in_fiori: Some(why),
    }
}

/// Evaluate an entity type for Fiori readiness. Returns a list of
/// findings in a stable order — callers can render them as-is. Each
/// actionable (Warn/Miss) finding carries an ABAP-CDS "fix hint" so
/// the linter teaches instead of just grading.
pub fn evaluate_entity_type(et: &EntityType) -> Vec<LintFinding> {
    let mut out = Vec::new();

    // Identity ─────────────────────────────────────────────────────────
    if et.header_info.is_some() {
        out.push(pass(
            LintCategory::Identity,
            "header_info",
            "UI.HeaderInfo declared.".to_string(),
        ));
    } else {
        out.push(actionable(
            LintSeverity::Miss,
            LintCategory::Identity,
            "header_info",
            "No UI.HeaderInfo — Fiori will fall back to the technical type name for titles.".to_string(),
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
    if !et.line_item.is_empty() {
        out.push(pass(
            LintCategory::ListReport,
            "line_item",
            format!("UI.LineItem: {} column(s) declared.", et.line_item.len()),
        ));
    } else {
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
    if !et.selection_fields.is_empty() {
        out.push(pass(
            LintCategory::Filtering,
            "selection_fields",
            format!("UI.SelectionFields: {} field(s).", et.selection_fields.len()),
        ));
    } else {
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
        assert!(findings
            .iter()
            .any(|f| f.code == "unit_missing" && f.message.contains("NetAmount")));
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
        assert_eq!(
            by_code["header_info"].suggested_cds,
            Some("@UI.headerInfo")
        );
        assert_eq!(
            by_code["line_item"].suggested_cds,
            Some("@UI.lineItem")
        );
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
