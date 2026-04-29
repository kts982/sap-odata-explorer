//! Entity-shape detection — different SAP service kinds have
//! different annotation expectations. A value-help entity doesn't
//! need `UI.LineItem`; judging it like a list report would fire a
//! false "miss". The evaluator consults the detected profile to
//! suppress irrelevant checks and adjust messages.

use crate::metadata::EntityType;
use serde::Serialize;

/// Detected shape of an entity — different service kinds have
/// different annotation expectations. A value-help entity doesn't
/// need `UI.LineItem`; judging it like a list report would fire a
/// false "miss". The evaluator uses this to suppress irrelevant
/// checks and adjust messages.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LintProfile {
    /// Fiori list-report / worklist entity. The default when
    /// nothing else fits — needs HeaderInfo, LineItem, SelectionFields.
    ListReport,
    /// Object-page root entity. HeaderInfo matters; LineItem is the
    /// child entity's job.
    ObjectPage,
    /// F4 value-help entity. Declares `Common.ValueListMapping`
    /// targeting the parent service. LineItem / SelectionFields /
    /// HeaderInfo are not expected.
    ValueHelp,
    /// Analytical / KPI service. Often emits
    /// `UI.PresentationVariant` with `Visualizations` pointing at
    /// charts instead of line items.
    Analytical,
    /// Generic / transactional / write-oriented entity that doesn't
    /// fit the other shapes. Capabilities annotations matter more
    /// than presentation ones.
    Transactional,
}

impl LintProfile {
    pub fn label(self) -> &'static str {
        match self {
            LintProfile::ListReport => "list_report",
            LintProfile::ObjectPage => "object_page",
            LintProfile::ValueHelp => "value_help",
            LintProfile::Analytical => "analytical",
            LintProfile::Transactional => "transactional",
        }
    }
}

/// Auto-detect the entity's profile from its name + declared
/// annotations. We key off name conventions first (`*VH` is almost
/// always a value help on SAP/S4) because they're the most reliable
/// signal, then fall through to annotation-based hints.
pub fn detect_profile(et: &EntityType) -> LintProfile {
    let name_lower = et.name.to_ascii_lowercase();
    // Value-help naming conventions dominate S/4 services.
    if name_lower.ends_with("vh")
        || name_lower.ends_with("vhtype")
        || name_lower.ends_with("valuehelp")
        || name_lower.ends_with("valuehelptype")
    {
        return LintProfile::ValueHelp;
    }
    // Object-page-ish signals: Facets-family annotations aren't
    // parsed yet, so we infer from "HeaderInfo without LineItem" —
    // an ABAP object-page root typically has HeaderInfo and defers
    // LineItem to the item child entity.
    if et.header_info.is_some() && et.line_item.is_empty() && !et.selection_fields.is_empty() {
        // If it has SelectionFields but no LineItem, Fiori treats it
        // as a list-with-no-default-columns — still list_report-ish.
        return LintProfile::ListReport;
    }
    if et.header_info.is_some() && et.line_item.is_empty() {
        return LintProfile::ObjectPage;
    }
    // Analytical hint: RequestAtLeast without LineItem is a thin
    // signal; SAP analytical CDS usually declares Chart visualisations
    // we don't parse. Keep this as a heuristic for now.
    if !et.request_at_least.is_empty() && et.line_item.is_empty() {
        return LintProfile::Analytical;
    }
    // LineItem + SelectionFields → classic list report.
    if !et.line_item.is_empty() && !et.selection_fields.is_empty() {
        return LintProfile::ListReport;
    }
    // Anything with a LineItem alone → still list_report.
    if !et.line_item.is_empty() {
        return LintProfile::ListReport;
    }
    // Default fallback: transactional. Capabilities checks dominate.
    LintProfile::Transactional
}
