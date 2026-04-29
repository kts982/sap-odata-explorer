//! Lint findings and severity/category enums — the data shapes the
//! evaluator emits and that callers (CLI, desktop) consume. No logic
//! here beyond the small `finding` / `pass` / `actionable` builders
//! that the evaluator uses to keep its body readable.

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
    Profile,
    Identity,
    ListReport,
    Filtering,
    Fields,
    Capabilities,
    /// Dangling-reference checks — annotations whose Path/target
    /// points at a property name that doesn't exist on the entity.
    /// Usually the result of a rename in one CDS layer that wasn't
    /// propagated to the annotations.
    Integrity,
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

pub(super) fn pass(category: LintCategory, code: &'static str, message: String) -> LintFinding {
    finding(LintSeverity::Pass, category, code, message)
}

/// Build a Warn/Miss finding with its ABAP-CDS "fix hint" attached.
/// `suggested` is the CDS annotation token (e.g. `@UI.headerInfo`);
/// `why` is a one-line "what Fiori does with it" explanation.
pub(super) fn actionable(
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
