//! A source-level lint checker enforcing the `AloxStyle` prose rules.
//!
//! This crate is the engine for the `aloxstyle-lints` CLI. It models a
//! [`Finding`] and exposes a [`Linter`] that runs a rule set over a source
//! string and returns every finding the rules produce.
//!
//! The rule set is empty in the initial crate so that the toolchain and the
//! finding loop resolve before any per-check logic lands.

use std::path::{Path, PathBuf};

use rustc_lexer::TokenKind;

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
    &[&NO_NON_SAFETY_COMMENT]
}

const NO_NON_SAFETY_COMMENT: NoNonSafetyComment = NoNonSafetyComment;

/// The rule that flags plain line and block comments.
///
/// The allowed exceptions are doc comments (`///`, `//!`, `/**`, `/*!`) and a
/// `// SAFETY:` marker with its contiguous line-span continuation. A `// SAFETY:`
/// line opens a run that continues through every immediately-following,
/// contiguous `//` line (exactly one line break between consecutive comment
/// lines, with no blank line, block comment, or code between). Only the first
/// line carries the `SAFETY:` prefix; the continuation lines carry the extended
/// rationale. Per `AloxStyle` § Explain through structure and contracts this is
/// a contract marker, not commentary. Comments inside string or char literals
/// are not tokenized as comments by `rustc_lexer`, so they are never flagged.
pub struct NoNonSafetyComment;

impl Rule for NoNonSafetyComment {
    fn check(&self, source: &str, path: &Path, findings: &mut Vec<Finding>) {
        let mut offset = 0;
        let mut safety_run_open = false;
        let mut prev_line_comment_end: Option<usize> = None;
        for token in rustc_lexer::tokenize(source) {
            let start = offset;
            let end = offset + token.len;
            match token.kind {
                TokenKind::LineComment => {
                    let text = &source[start..end];
                    if is_doc_line_comment(text) {
                        safety_run_open = false;
                    } else {
                        let is_continuation = is_safety_initiator(text)
                            || (safety_run_open
                                && prev_line_comment_end
                                    .is_some_and(|prev| contiguous(source, prev, start)));
                        if !is_continuation {
                            findings.push(plain_comment_finding(path, source, start, "line"));
                        }
                        safety_run_open = is_continuation;
                    }
                    prev_line_comment_end = Some(end);
                }
                TokenKind::BlockComment { .. } => {
                    let text = &source[start..end];
                    if !is_doc_block_comment(text) {
                        findings.push(plain_comment_finding(path, source, start, "block"));
                    }
                    safety_run_open = false;
                    prev_line_comment_end = None;
                }
                TokenKind::Whitespace => {}
                _ => {
                    safety_run_open = false;
                    prev_line_comment_end = None;
                }
            }
            offset = end;
        }
    }
}

fn is_doc_line_comment(text: &str) -> bool {
    text.starts_with("//!") || (text.starts_with("///") && !text.starts_with("////"))
}

fn is_doc_block_comment(text: &str) -> bool {
    text.starts_with("/*!") || text.starts_with("/**")
}

fn is_safety_initiator(text: &str) -> bool {
    text.strip_prefix("//")
        .is_some_and(|rest| rest.trim_start().starts_with("SAFETY:"))
}

/// Returns whether the bytes between the end of one line comment and the start
/// of the next contain exactly one line break, i.e. the two comments are on
/// consecutive lines with no blank line between them.
fn contiguous(source: &str, prev_end: usize, cur_start: usize) -> bool {
    source[prev_end..cur_start].matches('\n').count() == 1
}

fn plain_comment_finding(path: &Path, source: &str, start: usize, kind: &str) -> Finding {
    Finding {
        severity: Severity::Error,
        rule_id: RuleId::NoNonSafetyComment,
        path: path.to_path_buf(),
        line: byte_line(source, start),
        column: byte_column(source, start),
        message: format!("plain {kind} comment is not allowed"),
    }
}

fn byte_line(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset].split(|&b| b == b'\n').count()
}

fn byte_column(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(byte_offset + 1, |i| byte_offset - i)
}

#[cfg(test)]
mod tests {
    use super::{Finding, Linter, RuleId};
    use std::path::PathBuf;

    #[test]
    fn flags_plain_line_comment() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let findings = linter.check("// foo\npub fn fx() {}\n");
        assert_eq!(findings.len(), 1, "one finding expected");
        assert_eq!(findings[0].rule_id, RuleId::NoNonSafetyComment, "rule id");
        assert_eq!(
            (findings[0].line, findings[0].column),
            (1, 1),
            "plain comment at line 1 col 1"
        );
    }

    #[test]
    fn clean_source_returns_no_findings() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        assert!(linter.check("pub fn fx() {}").is_empty(), "clean source");
    }

    #[test]
    fn flags_plain_block_comment() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let findings = linter.check("/* foo */\npub fn fx() {}\n");
        assert_eq!(findings.len(), 1, "one finding expected");
        assert_eq!(findings[0].rule_id, RuleId::NoNonSafetyComment, "rule id");
        assert_eq!(
            (findings[0].line, findings[0].column),
            (1, 1),
            "block comment at line 1 col 1"
        );
    }

    #[test]
    fn allows_doc_comments() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "/// outer doc\n//! inner doc\n/** block doc */\n/*! inner block */\npub fn fx() {}\n";
        assert!(linter.check(source).is_empty(), "doc comments allowed");
    }

    #[test]
    fn allows_safety_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: this is the soundness argument\npub unsafe fn fx() {}\n";
        assert!(linter.check(source).is_empty(), "safety marker allowed");
    }

    #[test]
    fn ignores_comment_in_string_and_char_literal() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "let s = \"http://x\";\nlet c = '/';\n";
        assert!(linter.check(source).is_empty(), "literal slash not a comment");
    }

    #[test]
    fn allows_multiline_safety_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: first argument\n// SAFETY: second argument\npub fn fx() {}\n";
        assert!(linter.check(source).is_empty(), "each safety line passes");
    }

    #[test]
    fn allows_safety_line_span_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source =
            "// SAFETY: a\n// continues the reason b\n// and more c\nlet x = unsafe {};\n";
        assert!(
            linter.check(source).is_empty(),
            "contiguous line-span SAFETY continuation allowed"
        );
    }

    #[test]
    fn allows_indented_safety_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "    // SAFETY: a\n    // continues the reason b\n    let x = unsafe {};\n";
        assert!(
            linter.check(source).is_empty(),
            "indented contiguous continuation allowed"
        );
    }

    #[test]
    fn flags_orphan_comment_after_blank_line() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: a\n\n// orphan plain comment\nlet x = 0;\n";
        let findings = linter.check(source);
        assert_eq!(findings.len(), 1, "one finding for the orphan comment");
        assert_eq!(findings[0].rule_id, RuleId::NoNonSafetyComment, "rule id");
        assert_eq!(findings[0].line, 3, "orphan on line 3");
    }

    #[test]
    fn byte_position_helpers_are_one_based() {
        let source = "ab\ncd// comment\n";
        assert_eq!(super::byte_line(source, 5), 2, "second line");
        assert_eq!(super::byte_column(source, 5), 3, "third column");
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
