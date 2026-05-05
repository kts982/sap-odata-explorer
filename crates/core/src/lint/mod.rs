//! Fiori-readiness linter — evaluates an `EntityType` against a
//! checklist of annotations a Fiori list-report / object-page service
//! would normally declare. Callable from the desktop's describe panel
//! (SAP View) and from the CLI's `lint` subcommand. Purely derives
//! from already-parsed metadata; no I/O.

use crate::metadata::EntityType;

mod profile;
mod rules;
mod types;
pub use profile::{LintProfile, detect_profile};
pub use types::{LintCategory, LintFinding, LintSeverity};

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
///
/// The body is the canonical rule order: every `check_*` call below
/// pushes its findings onto `out` in the order users see them in the
/// CLI table and desktop panel. Re-ordering or inserting a call
/// changes user-visible output and will trip the canonical-order
/// tests in this module.
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

    rules::check_identity(et, profile, &mut out);
    rules::check_list_report(et, profile, &mut out);
    rules::check_filtering(et, profile, &mut out);
    rules::check_fields(et, &mut out);
    rules::check_consistency(et, &mut out);
    rules::check_integrity(et, &mut out);
    rules::check_capabilities(et, &mut out);

    out
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
