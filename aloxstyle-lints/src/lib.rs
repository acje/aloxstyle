//! A source-level lint checker enforcing the `AloxStyle` prose rules.
//!
//! This crate is the engine for the `aloxstyle-lints` CLI. It models a
//! [`Finding`] and exposes a [`Linter`] that runs a rule set over a source
//! string and returns every finding the rules produce. Source that is not
//! parseable is reported as [`CheckOutcome::Indeterminate`] (a fail-closed
//! gate, never a clean pass).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rustc_lexer::TokenKind;
use syn::visit::Visit;

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

/// The result of running the rule set over a source file.
///
/// A parseable file yields its findings; an unparseable file is
/// [`CheckOutcome::Indeterminate`], which the CLI maps to exit code 3. The
/// checker is fail-closed: malformed source is never a clean pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The source parsed and every rule ran; these are the findings.
    Findings(Vec<Finding>),
    /// The source could not be parsed, so no rule result is trustworthy.
    Indeterminate,
}

/// A single source-level lint rule.
pub trait Rule {
    /// Inspects `source` and pushes a [`Finding`] into `findings` for every
    /// violation found. Findings are attributed to `path`. `ast` is the parsed
    /// file (parsed once at the engine boundary); a rule may ignore it.
    fn check(&self, source: &str, ast: &syn::File, path: &Path, findings: &mut Vec<Finding>);
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

    /// Runs every registered rule over `source`, returning the outcome.
    ///
    /// The file is parsed once at this boundary. If parsing fails the outcome is
    /// [`CheckOutcome::Indeterminate`]; otherwise every rule runs on the parse.
    #[must_use]
    pub fn check(&self, source: &str) -> CheckOutcome {
        let Ok(ast) = syn::parse_file(source) else {
            return CheckOutcome::Indeterminate;
        };
        let mut findings = Vec::new();
        for rule in rules() {
            rule.check(source, &ast, &self.path, &mut findings);
        }
        CheckOutcome::Findings(findings)
    }
}

fn rules() -> &'static [&'static dyn Rule] {
    &[&NO_NON_SAFETY_COMMENT, &UNSAFE_NEEDS_SAFETY_MARKER]
}

const NO_NON_SAFETY_COMMENT: NoNonSafetyComment = NoNonSafetyComment;
const UNSAFE_NEEDS_SAFETY_MARKER: UnsafeNeedsSafetyMarker = UnsafeNeedsSafetyMarker;

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
    fn check(&self, source: &str, _ast: &syn::File, path: &Path, findings: &mut Vec<Finding>) {
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

/// A lexer token together with its byte range in the source.
struct Tok {
    kind: TokenKind,
    start: usize,
    end: usize,
}

fn tokenize_with_ranges(source: &str) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    for token in rustc_lexer::tokenize(source) {
        let start = offset;
        let end = offset + token.len;
        tokens.push(Tok { kind: token.kind, start, end });
        offset = end;
    }
    tokens
}

/// The rule that requires every `unsafe` site to be immediately preceded by a
/// `// SAFETY:` marker.
///
/// Genuine `unsafe` sites are discovered from the parsed [`syn::File`] AST so
/// that a non-site `unsafe` spelling is never reported: an `unsafe` keyword
/// inside an attribute (`#[unsafe(...)]`) or inside a macro token tree is not a
/// real `unsafe` construct and is excluded by construction. The sites are an
/// `unsafe` block, `fn`, `impl`, `trait`, `extern`, and any `unsafe fn` method
/// or trait item.
///
/// Between the marker run and the `unsafe` keyword the allowed tokens are
/// whitespace with no blank line, a `let`/`mut`/`const`/binding/type-name
/// identifier and the type-annotation punctuation that may interleave on the
/// statement prefix (`=`, `:`, `&`, `*`, `[`, `]`, `<`, `>`, `,`, a lifetime),
/// plus attribute groups (`#[...]`) and bare type brackets (`[...]`). The marker
/// is the line-span-continuation form from [`NoNonSafetyComment`]: a
/// `// SAFETY:` initiator plus its contiguous `//` continuation lines.
pub struct UnsafeNeedsSafetyMarker;

impl Rule for UnsafeNeedsSafetyMarker {
    fn check(&self, source: &str, ast: &syn::File, path: &Path, findings: &mut Vec<Finding>) {
        let sites = genuine_unsafe_sites(source, ast);
        let tokens = tokenize_with_ranges(source);
        for i in 0..tokens.len() {
            if !is_unsafe_keyword(&tokens, i, source) {
                continue;
            }
            let offset = tokens[i].start;
            let Some(&kind) = sites.get(&offset) else {
                continue;
            };
            if !has_safety_marker_before(&tokens, i, source) {
                findings.push(unsafe_finding(path, source, offset, kind));
            }
        }
    }
}

/// Discovers every genuine `unsafe` site in `ast`, returning a map from the
/// byte offset of the `unsafe` keyword to a human-readable site kind.
fn genuine_unsafe_sites(source: &str, ast: &syn::File) -> HashMap<usize, &'static str> {
    let line_map = LineMap::new(source);
    let mut collector = SiteCollector { source, line_map: &line_map, sites: HashMap::new() };
    collector.visit_file(ast);
    collector.sites
}

/// The collection of a site's `unsafe` keyword offset and its display kind.
struct SiteCollector<'a> {
    source: &'a str,
    line_map: &'a LineMap,
    sites: HashMap<usize, &'static str>,
}

impl<'a> Visit<'a> for SiteCollector<'a> {
    fn visit_expr_unsafe(&mut self, node: &'a syn::ExprUnsafe) {
        self.push(node.unsafe_token.span, "block");
        syn::visit::visit_expr_unsafe(self, node);
    }

    fn visit_item_fn(&mut self, node: &'a syn::ItemFn) {
        self.push_optional(&node.sig.unsafety, "function");
        syn::visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'a syn::ImplItemFn) {
        self.push_optional(&node.sig.unsafety, "function");
        syn::visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'a syn::TraitItemFn) {
        self.push_optional(&node.sig.unsafety, "function");
        syn::visit::visit_trait_item_fn(self, node);
    }

    fn visit_item_impl(&mut self, node: &'a syn::ItemImpl) {
        self.push_optional(&node.unsafety, "impl");
        syn::visit::visit_item_impl(self, node);
    }

    fn visit_item_trait(&mut self, node: &'a syn::ItemTrait) {
        self.push_optional(&node.unsafety, "trait");
        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_foreign_mod(&mut self, node: &'a syn::ItemForeignMod) {
        self.push_optional(&node.unsafety, "extern");
        syn::visit::visit_item_foreign_mod(self, node);
    }
}

impl<'a> SiteCollector<'a> {
    fn push(&mut self, span: proc_macro2::Span, kind: &'static str) {
        let lc = span.start();
        let offset = self.line_map.byte_offset(self.source, lc.line, lc.column);
        self.sites.entry(offset).or_insert(kind);
    }

    fn push_optional(&mut self, tok: &Option<syn::token::Unsafe>, kind: &'static str) {
        if let Some(tok) = tok {
            self.push(tok.span, kind);
        }
    }
}

/// Maps a 1-based line and 0-based character column to a byte offset, matching
/// the column semantics of [`proc_macro2::Span::start`] with `span-locations`.
struct LineMap {
    line_starts: Vec<usize>,
}

impl LineMap {
    fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    fn byte_offset(&self, source: &str, line: usize, column: usize) -> usize {
        let line_start = self.line_starts[line - 1];
        let line_end = self.line_starts.get(line).copied().unwrap_or(source.len());
        let line_text = &source[line_start..line_end];
        line_text
            .char_indices()
            .nth(column)
            .map_or(line_start + line_text.len(), |(byte, _)| line_start + byte)
    }
}

fn is_unsafe_keyword(tokens: &[Tok], i: usize, source: &str) -> bool {
    let tok = &tokens[i];
    tok.kind == TokenKind::Ident && &source[tok.start..tok.end] == "unsafe"
}

/// Returns whether a `// SAFETY:` marker run immediately precedes the `unsafe`
/// keyword at `tokens[unsafe_idx]`.
///
/// The scan walks backward over the statement prefix, allowing the type
/// annotation and binding tokens that may interleave before the marker; it stops
/// (returning false) at a blank line, block comment, statement boundary, or any
/// token that is not part of an allowed interleave. The marker is accepted only
/// when the run reached is the [`marker_run_first_line`] of a `// SAFETY:`
/// initiator.
fn has_safety_marker_before(tokens: &[Tok], unsafe_idx: usize, source: &str) -> bool {
    let mut i = unsafe_idx;
    while i > 0 {
        i -= 1;
        let tok = &tokens[i];
        match tok.kind {
            TokenKind::Whitespace => {
                if source[tok.start..tok.end].matches('\n').count() > 1 {
                    return false;
                }
            }
            TokenKind::Ident
            | TokenKind::RawIdent
            | TokenKind::Eq
            | TokenKind::Colon
            | TokenKind::And
            | TokenKind::Star
            | TokenKind::Lt
            | TokenKind::Gt
            | TokenKind::Comma
            | TokenKind::Lifetime { .. } => {}
            TokenKind::CloseBracket => match bracket_group_front(tokens, i) {
                Some(front) => i = front,
                None => return false,
            },
            TokenKind::LineComment => {
                return marker_run_first_line(tokens, i, source).is_some();
            }
            _ => return false,
        }
    }
    false
}

/// Walks back from a `CloseBracket` to the matching `OpenBracket`. If a `#`
/// immediately precedes it, the group is an attribute and its front delimiter is
/// the `#`; otherwise it is a bare type bracket and the front delimiter is the
/// `OpenBracket`. Returns the index of the front token.
fn bracket_group_front(tokens: &[Tok], close_idx: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = close_idx;
    loop {
        match tokens[i].kind {
            TokenKind::CloseBracket => depth += 1,
            TokenKind::OpenBracket => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    if i > 0 && tokens[i - 1].kind == TokenKind::Pound {
                        return Some(i - 1);
                    }
                    return Some(i);
                }
            }
            _ => {}
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn marker_run_first_line(tokens: &[Tok], mut idx: usize, source: &str) -> Option<usize> {
    let nearest = &source[tokens[idx].start..tokens[idx].end];
    if is_doc_line_comment(nearest) {
        return None;
    }
    while idx >= 2 {
        let ws = &tokens[idx - 1];
        let prev = &tokens[idx - 2];
        if ws.kind == TokenKind::Whitespace
            && source[ws.start..ws.end].matches('\n').count() == 1
            && prev.kind == TokenKind::LineComment
            && !is_doc_line_comment(&source[prev.start..prev.end])
        {
            idx -= 2;
        } else {
            break;
        }
    }
    let first = &source[tokens[idx].start..tokens[idx].end];
    is_safety_initiator(first).then_some(tokens[idx].start)
}

fn unsafe_finding(path: &Path, source: &str, offset: usize, kind: &str) -> Finding {
    Finding {
        severity: Severity::Error,
        rule_id: RuleId::UnsafeNeedsSafetyMarker,
        path: path.to_path_buf(),
        line: byte_line(source, offset),
        column: byte_column(source, offset),
        message: format!("unsafe {kind} is not preceded by a // SAFETY: marker"),
    }
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
    use super::{CheckOutcome, Finding, Linter, RuleId};
    use std::path::PathBuf;

    fn findings(linter: &Linter, source: &str) -> Vec<Finding> {
        match linter.check(source) {
            CheckOutcome::Findings(findings) => findings,
            CheckOutcome::Indeterminate => {
                unreachable!("fixture unexpectedly unparseable: {source:?}")
            }
        }
    }

    #[test]
    fn flags_plain_line_comment() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let findings = findings(&linter, "// foo\npub fn fx() {}\n");
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
        assert!(findings(&linter, "pub fn fx() {}").is_empty(), "clean source");
    }

    #[test]
    fn flags_plain_block_comment() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let findings = findings(&linter, "/* foo */\npub fn fx() {}\n");
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
        let source =
            "//! inner line doc\n/*! inner block doc */\n/// outer line doc\n/** outer block doc */\npub fn fx() {}\n";
        assert!(findings(&linter, source).is_empty(), "doc comments allowed");
    }

    #[test]
    fn allows_safety_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: this is the soundness argument\npub fn fx() {}\n";
        assert!(findings(&linter, source).is_empty(), "safety marker allowed");
    }

    #[test]
    fn ignores_comment_in_string_and_char_literal() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() { let s = \"http://x\"; let c = '/'; }\n";
        assert!(findings(&linter, source).is_empty(), "literal slash not a comment");
    }

    #[test]
    fn allows_multiline_safety_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: first argument\n// SAFETY: second argument\npub fn fx() {}\n";
        assert!(findings(&linter, source).is_empty(), "each safety line passes");
    }

    #[test]
    fn allows_safety_line_span_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\n// continues the reason b\n// and more c\nlet x = unsafe {};\n}\n";
        assert!(
            findings(&linter, source).is_empty(),
            "contiguous line-span SAFETY continuation allowed"
        );
    }

    #[test]
    fn allows_indented_safety_continuation() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n    // SAFETY: a\n    // continues the reason b\n    let x = unsafe {};\n}\n";
        assert!(
            findings(&linter, source).is_empty(),
            "indented contiguous continuation allowed"
        );
    }

    #[test]
    fn flags_orphan_comment_after_blank_line() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\n\n// orphan plain comment\nlet x = 0;\n}\n";
        let findings = findings(&linter, source);
        assert_eq!(findings.len(), 1, "one finding for the orphan comment");
        assert_eq!(findings[0].rule_id, RuleId::NoNonSafetyComment, "rule id");
        assert_eq!(findings[0].line, 4, "orphan on line 4");
    }

    #[test]
    fn flags_unsafe_without_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let findings = findings(&linter, "pub fn fx() { unsafe {} }\n");
        assert_eq!(findings.len(), 1, "one finding expected");
        assert_eq!(findings[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn allows_unsafe_with_safety_marker_and_binding() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\nlet x = unsafe {};\n}\n";
        assert!(findings(&linter, source).is_empty(), "marker + binding allowed");
    }

    #[test]
    fn allows_unsafe_with_attribute_before_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n#[inline]\n// SAFETY: a\nlet x = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "attribute before marker allowed: {f:?}");
    }

    #[test]
    fn allows_unsafe_with_attribute_in_interleave() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\n#[inline]\nlet x = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "attribute in interleave allowed: {f:?}");
    }

    #[test]
    fn flags_unsafe_preceded_by_plain_comment() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// plain comment\nlet x = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 2, "plain comment + unsafe both flagged: {f:?}");
        assert!(
            f.iter().any(|x| x.rule_id == RuleId::UnsafeNeedsSafetyMarker),
            "unsafe finding present: {f:?}"
        );
    }

    #[test]
    fn flags_unsafe_fn_with_doc_comment_only() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "/// doc comment\nunsafe fn f() {}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 1, "doc comment alone is not the marker: {f:?}");
        assert_eq!(f[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn flags_unsafe_with_blank_line_before_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\n\nlet x = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 1, "blank line breaks adjacency: {f:?}");
        assert_eq!(f[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn flags_unsafe_impl_without_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "unsafe impl Foo for Bar {}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 1, "unsafe impl needs a marker: {f:?}");
        assert_eq!(f[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn allows_unsafe_with_type_annotated_binding_and_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\nlet x: *const u8 = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "typed binding + marker allowed: {f:?}");
    }

    #[test]
    fn flags_unsafe_with_type_annotated_binding_without_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\nlet x: *const u8 = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 1, "typed binding without marker flagged: {f:?}");
        assert_eq!(f[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn ignores_unsafe_inside_attribute() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "#[unsafe(no_mangle)]\npub fn f() {}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "unsafe attribute is not a site: {f:?}");
    }

    #[test]
    fn ignores_unsafe_inside_macro_token_tree() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn f() {\nfoo!(unsafe {});\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "macro token-tree unsafe is not a site: {f:?}");
    }

    #[test]
    fn flags_unsafe_fn_without_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let f = findings(&linter, "unsafe fn f() {}\n");
        assert_eq!(f.len(), 1, "unsafe fn needs a marker: {f:?}");
        assert_eq!(
            f[0].message,
            "unsafe function is not preceded by a // SAFETY: marker",
            "kind message"
        );
    }

    #[test]
    fn allows_unsafe_with_lifetime_type_and_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "pub fn fx() {\n// SAFETY: a\nlet x: &'static [u8] = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "lifetime + slice binding + marker allowed: {f:?}");
    }

    #[test]
    fn allows_unsafe_impl_with_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "// SAFETY: a\nunsafe impl Foo for Bar {}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 0, "unsafe impl with marker allowed: {f:?}");
    }

    #[test]
    fn flags_unsafe_inside_unsafe_fn_body_without_own_marker() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source =
            "// SAFETY: fn soundness\nunsafe fn f() {\nlet z = unsafe {};\n}\n";
        let f = findings(&linter, source);
        assert_eq!(f.len(), 1, "inner unsafe block needs its own marker: {f:?}");
        assert_eq!(f[0].rule_id, RuleId::UnsafeNeedsSafetyMarker, "rule id");
    }

    #[test]
    fn check_is_indeterminate_on_unparseable_source() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let outcome = linter.check("fn broken( {\n");
        assert_eq!(outcome, CheckOutcome::Indeterminate, "unparseable source");
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

