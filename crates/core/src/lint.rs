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
}

/// Evaluate an entity type for Fiori readiness. Returns a list of
/// findings in a stable order — callers can render them as-is.
pub fn evaluate_entity_type(et: &EntityType) -> Vec<LintFinding> {
    let mut out = Vec::new();

    // Identity ─────────────────────────────────────────────────────────
    if et.header_info.is_some() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::Identity,
            code: "header_info",
            message: "UI.HeaderInfo declared.".to_string(),
        });
    } else {
        out.push(LintFinding {
            severity: LintSeverity::Miss,
            category: LintCategory::Identity,
            code: "header_info",
            message: "No UI.HeaderInfo — Fiori will fall back to the technical type name for titles.".to_string(),
        });
    }
    if !et.semantic_keys.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::Identity,
            code: "semantic_key",
            message: format!(
                "Common.SemanticKey: {}",
                et.semantic_keys.join(", "),
            ),
        });
    } else if et.keys.iter().any(|k| is_uuid_like_key(k)) {
        // UUID technical keys without a business SemanticKey are a red
        // flag — users see a UUID instead of a human-meaningful ID.
        out.push(LintFinding {
            severity: LintSeverity::Warn,
            category: LintCategory::Identity,
            code: "semantic_key",
            message: "Technical key looks UUID-ish but no Common.SemanticKey declared — Fiori will show the UUID.".to_string(),
        });
    }

    // List-report ──────────────────────────────────────────────────────
    if !et.line_item.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::ListReport,
            code: "line_item",
            message: format!("UI.LineItem: {} column(s) declared.", et.line_item.len()),
        });
    } else {
        out.push(LintFinding {
            severity: LintSeverity::Miss,
            category: LintCategory::ListReport,
            code: "line_item",
            message: "No UI.LineItem — the list report will have no default columns.".to_string(),
        });
    }
    if !et.request_at_least.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::ListReport,
            code: "request_at_least",
            message: format!(
                "UI.PresentationVariant.RequestAtLeast: +{} support field(s).",
                et.request_at_least.len(),
            ),
        });
    }
    if !et.sort_order.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::ListReport,
            code: "sort_order",
            message: format!(
                "UI.PresentationVariant.SortOrder: {} clause(s).",
                et.sort_order.len(),
            ),
        });
    }

    // Filtering ────────────────────────────────────────────────────────
    if !et.selection_fields.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::Filtering,
            code: "selection_fields",
            message: format!(
                "UI.SelectionFields: {} field(s).",
                et.selection_fields.len(),
            ),
        });
    } else {
        out.push(LintFinding {
            severity: LintSeverity::Miss,
            category: LintCategory::Filtering,
            code: "selection_fields",
            message: "No UI.SelectionFields — the filter bar will be empty by default.".to_string(),
        });
    }
    if !et.selection_variants.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Pass,
            category: LintCategory::Filtering,
            code: "selection_variants",
            message: format!(
                "UI.SelectionVariant: {} declared.",
                et.selection_variants.len(),
            ),
        });
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
        out.push(LintFinding {
            severity: LintSeverity::Warn,
            category: LintCategory::Fields,
            code: "unit_missing",
            message: format!(
                "Decimal/amount-looking properties without Measures.Unit or ISOCurrency: {}",
                decimal_without_unit.join(", "),
            ),
        });
    }
    let code_without_text: Vec<&str> = et
        .properties
        .iter()
        .filter(|p| looks_code(&p.name) && p.text_path.is_none())
        .map(|p| p.name.as_str())
        .collect();
    if !code_without_text.is_empty() {
        out.push(LintFinding {
            severity: LintSeverity::Warn,
            category: LintCategory::Fields,
            code: "text_missing",
            message: format!(
                "Code-looking properties without Common.Text: {}",
                code_without_text.join(", "),
            ),
        });
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
        out.push(LintFinding {
            severity: LintSeverity::Warn,
            category: LintCategory::Capabilities,
            code: "capabilities_silent",
            message: "No Capabilities.* annotations — clients can't pre-flight validate; they'll discover limits at runtime.".to_string(),
        });
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
