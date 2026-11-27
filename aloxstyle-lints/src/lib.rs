//! A source-level lint checker enforcing the `AloxStyle` prose rules.
//!
//! This crate is the engine for the `aloxstyle-lints` CLI. It models a
//! [`Finding`] and exposes a [`Linter`] that runs a rule set over a source
//! string and returns every finding the rules produce.
//!
//! The rule set is empty in the initial crate so that the toolchain and the
//! finding loop resolve before any per-check logic lands.

use std::path::{Path, PathBuf};

/// Severity of a [`Finding`]. Every `AloxStyle` rule is a build-failing gate, so
/// findings are errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
}

/// Identifier of the rule that produced a [`Finding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleId {
    NoNonSafetyComment,
    UnsafeNeedsSafetyMarker,
    Underflow,
}

impl RuleId {
    /// Returns the dotted rule name used in CLI output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RuleId::NoNonSafetyComment => "no_non_safety_comment",
            RuleId::UnsafeNeedsSafetyMarker => "unsafe_needs_safety_marker",
            RuleId::Underflow => "underflow",
        }
    }
}

/// A single lint finding: where it was produced and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub severity: Severity,
    pub rule_id: RuleId,
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl std::fmt::Display for Finding {
    /// Writes the finding as `path:line:col: message`.
    ///
    /// # Errors
    ///
    /// Returns [`std::fmt::Error`] if the underlying writer fails.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}:{}: {}",
            self.path.display(),
            self.line,
            self.column,
            self.message
        )
    }
}

/// A single source-level lint rule.
pub trait Rule {
    /// Inspects `source` and pushes a [`Finding`] into `findings` for every
    /// violation found. Findings are attributed to `path`.
    fn check(&self, source: &str, path: &Path, findings: &mut Vec<Finding>);
}

/// Runs a rule set over a single source file.
pub struct Linter {
    path: PathBuf,
}

impl Linter {
    /// Creates a linter bound to `path`, the location findings are attributed to.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Runs every registered rule over `source`, returning all findings.
    ///
    /// The initial rule set is empty, so this returns no findings until
    /// per-check rules are added.
    #[must_use]
    pub fn check(&self, source: &str) -> Vec<Finding> {
        let mut findings = Vec::new();
        for rule in rules() {
            rule.check(source, &self.path, &mut findings);
        }
        findings
    }
}

fn rules() -> &'static [&'static dyn Rule] {
    &[]
}

#[cfg(test)]
mod tests {
    use super::{Finding, Linter, RuleId};
    use std::path::PathBuf;

    #[test]
    fn empty_rule_set_returns_no_findings() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        assert!(linter.check("pub fn fx() {}").is_empty(), "no rules yet");
    }

    #[test]
    fn rule_id_as_str_returns_dotted_name() {
        assert_eq!(RuleId::Underflow.as_str(), "underflow", "rule id name");
        assert_eq!(
            RuleId::NoNonSafetyComment.as_str(),
            "no_non_safety_comment",
            "rule id name"
        );
        assert_eq!(
            RuleId::UnsafeNeedsSafetyMarker.as_str(),
            "unsafe_needs_safety_marker",
            "rule id name"
        );
    }

    #[test]
    fn finding_displays_path_line_col_message() {
        let finding = Finding {
            severity: super::Severity::Error,
            rule_id: RuleId::Underflow,
            path: PathBuf::from("src/lib.rs"),
            line: 3,
            column: 7,
            message: "underflow".to_string(),
        };
        assert_eq!(
            finding.to_string(),
            "src/lib.rs:3:7: underflow",
            "finding display"
        );
    }
}
