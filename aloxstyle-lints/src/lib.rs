//! A source-level lint checker enforcing the `AloxStyle` prose rules.
//!
//! This crate is the engine for the `aloxstyle-lints` CLI. It models a
//! [`Finding`] and exposes a [`Linter`] that runs a rule set over a source
//! string and returns every finding the rules produce. Source that is not
//! parseable is reported as [`CheckOutcome::Indeterminate`] (a fail-closed
//! gate, never a clean pass).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rustc_lexer::TokenKind;
use syn::spanned::Spanned;
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
    /// The source could not be parsed, so no rule result is trustworthy. The
    /// failing file is carried so the caller can name it: a fail-closed gate is
    /// only actionable when the operator can find the offending file.
    Indeterminate(PathBuf),
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
    allowlist: Allowlist,
}

impl Linter {
    /// Creates a linter bound to `path`, the location findings are attributed to,
    /// with an empty allowlist.
    #[must_use]
    pub const fn new(path: PathBuf) -> Self {
        Self::with_allowlist(path, Allowlist::empty())
    }

    /// Creates a linter bound to `path` that suppresses findings matched by
    /// `allowlist`.
    #[must_use]
    pub const fn with_allowlist(path: PathBuf, allowlist: Allowlist) -> Self {
        Self { path, allowlist }
    }

    /// Runs every registered rule over `source`, returning the outcome.
    ///
    /// The file is parsed once at this boundary. If parsing fails the outcome is
    /// [`CheckOutcome::Indeterminate`]; otherwise every rule runs on the parse
    /// and the allowlist suppresses any matching finding.
    #[must_use]
    pub fn check(&self, source: &str) -> CheckOutcome {
        let Ok(ast) = syn::parse_file(source) else {
            return CheckOutcome::Indeterminate(self.path.clone());
        };
        let mut findings = Vec::new();
        for rule in rules() {
            rule.check(source, &ast, &self.path, &mut findings);
        }
        let findings = self.allowlist.apply(findings, source);
        CheckOutcome::Findings(findings)
    }
}

fn rules() -> &'static [&'static dyn Rule] {
    &[&NO_NON_SAFETY_COMMENT, &UNSAFE_NEEDS_SAFETY_MARKER, &UNDERFLOW]
}

const NO_NON_SAFETY_COMMENT: NoNonSafetyComment = NoNonSafetyComment;
const UNSAFE_NEEDS_SAFETY_MARKER: UnsafeNeedsSafetyMarker = UnsafeNeedsSafetyMarker;
const UNDERFLOW: Underflow = Underflow;

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
        let Some(line_index) = line.checked_sub(1) else {
            return source.len();
        };
        let Some(&line_start) = self.line_starts.get(line_index) else {
            return source.len();
        };
        let line_end = self.line_starts.get(line_index + 1).copied().unwrap_or(source.len());
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
        column: char_column(source, offset),
        message: format!("unsafe {kind} is not preceded by a // SAFETY: marker"),
    }
}

/// A reason-bearing allowlist of suppressed findings.
///
/// An entry suppresses a finding when its `path` and `rule` match and either
/// the finding's line equals `line`, or (when `line` is absent) the finding's
/// source line contains `snippet`. The `reason` is mandatory: `AloxStyle` gives
/// every lint exemption a specific rationale. The reader accepts a restricted
/// TOML subset (a `[[allow]]` array of tables) so the file is a real
/// `.aloxstyle-lints.toml` without pulling a TOML dependency.
#[derive(Debug, Clone, Default)]
pub struct Allowlist {
    entries: Vec<AllowEntry>,
}

#[derive(Debug, Clone)]
struct AllowEntry {
    path: String,
    rule: String,
    line: Option<usize>,
    snippet: Option<String>,
    reason: String,
}

impl Allowlist {
    /// Returns an empty allowlist.
    #[must_use]
    pub const fn empty() -> Self {
        Self { entries: Vec::new() }
    }

    /// Parses an allowlist file body.
    ///
    /// # Errors
    ///
    /// Returns a message describing the offending line when the body is not the
    /// expected shape or an entry is missing its mandatory `reason`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut entries: Vec<AllowEntry> = Vec::new();
        let mut current: Option<AllowEntry> = None;
        for (idx, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[allow]]" {
                if let Some(previous) = current.take() {
                    entries.push(previous);
                }
                current = Some(AllowEntry {
                    path: String::new(),
                    rule: String::new(),
                    line: None,
                    snippet: None,
                    reason: String::new(),
                });
                continue;
            }
            let entry = current.as_mut().ok_or_else(|| {
                format!("line {}: key outside a [[allow]] table", idx + 1)
            })?;
            let (key, value) =
                parse_config_kv(line).ok_or_else(|| format!("line {}: expected `key = value`", idx + 1))?;
            match key.as_str() {
                "path" => entry.path = value,
                "rule" => entry.rule = value,
                "line" => {
                    entry.line = Some(value.parse().map_err(|_| {
                        format!("line {}: `line` must be an integer", idx + 1)
                    })?);
                }
                "snippet" => entry.snippet = Some(value),
                "reason" => entry.reason = value,
                _ => return Err(format!("line {}: unknown key `{key}`", idx + 1)),
            }
        }
        if let Some(last) = current.take() {
            entries.push(last);
        }
        for entry in &entries {
            if entry.reason.trim().is_empty() {
                return Err(format!(
                    "allowlist entry for {}:{} has no `reason`",
                    entry.path, entry.rule
                ));
            }
        }
        Ok(Allowlist { entries })
    }

    /// Applies the allowlist to `findings`, dropping any that match an entry.
    #[must_use]
    pub fn apply(&self, findings: Vec<Finding>, source: &str) -> Vec<Finding> {
        findings
            .into_iter()
            .filter(|finding| !self.is_suppressed(finding, source))
            .collect()
    }

    fn is_suppressed(&self, finding: &Finding, source: &str) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.suppresses(finding, source))
    }
}

impl AllowEntry {
    /// Returns true when this entry suppresses `finding` within `source`.
    ///
    /// A `line` entry matches the finding's 1-based line exactly. A `snippet`
    /// entry matches when the finding's source line contains the snippet as a
    /// literal substring, or when the snippet matches after whitespace is
    /// collapsed on both sides — so a snippet copied from a finding message
    /// (which strips whitespace, e.g. `b"GET / HTT"` shows as `b"GET/HTT"`)
    /// still suppresses.
    fn suppresses(&self, finding: &Finding, source: &str) -> bool {
        if self.rule != finding.rule_id.as_str() || !path_suffix_match(&finding.path, &self.path) {
            return false;
        }
        if let Some(line) = self.line {
            return line == finding.line;
        }
        if let Some(snippet) = &self.snippet {
            if snippet.is_empty() {
                return false;
            }
            let line_text = source.lines().nth(finding.line.saturating_sub(1));
            let needle = snippet.split_whitespace().collect::<String>();
            return line_text.is_some_and(|line_text| {
                line_text.contains(snippet.as_str())
                    || line_text.split_whitespace().collect::<String>().contains(&needle)
            });
        }
        false
    }
}

fn path_suffix_match(path: &Path, suffix: &str) -> bool {
    let path_str = path.to_string_lossy();
    let suffix = suffix.trim_start_matches("./");
    path_str == suffix
        || path_str
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('/') || prefix.is_empty())
}

fn parse_config_kv(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once('=')?;
    let key = key.trim().to_string();
    let value = rest.trim();
    let value = if value.starts_with('"') {
        value
            .strip_prefix('"')?
            .strip_suffix('"')?
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
    } else {
        value.to_string()
    };
    Some((key, value))
}

/// The rule that flags integer subtractions and decrements whose subtrahend may
/// exceed the minuend, unless the site is constant, a float-literal operation, a
/// guarded or counter decrement, or allowlisted.
///
/// Discovery walks the parsed [`syn::File`] for `Expr::Binary` with `BinOp::Sub`
/// (`a - b`) and `BinOp::SubAssign` (`a -= b`). A site is
/// skipped when both operands are constant-foldable (a literal or a
/// `SCREAMING_SNAKE` const path / `.len()` on a const), when either operand is a
/// float literal, when it is a length-counter decrement by one, when it is
/// guarded by a `while x > 0` loop, an immediately-preceding
/// `if subtrahend >= minuend { return }`/`assert!`, an empty-queue
/// `if length == 0 { return }`, or when the result wraps under a `& MASK`.
/// Otherwise it is flagged: conservative, so the author hardens or allowlists.
///
/// Time-type subtractions (`Duration - Duration`, `Instant - Instant`) are NOT
/// skipped: their `Sub` impls panic on underflow, so they are flagged like any
/// other risky subtraction. Only float literals are exempt, because float
/// arithmetic is saturating rather than panicking.
pub struct Underflow;

impl Rule for Underflow {
    fn check(&self, source: &str, ast: &syn::File, path: &Path, findings: &mut Vec<Finding>) {
        let line_map = LineMap::new(source);
        let mut visitor = UnderflowVisitor {
            source,
            path,
            findings,
            line_map: &line_map,
            guards: Vec::new(),
            loop_active: Vec::new(),
            masked: HashSet::new(),
            in_const: false,
        };
        visitor.visit_file(ast);
    }
}

enum Guard {
    Bounded { minuend: String, subtrahend: String },
    NonEmpty { expr: String },
}

struct UnderflowVisitor<'a> {
    source: &'a str,
    path: &'a Path,
    findings: &'a mut Vec<Finding>,
    line_map: &'a LineMap,
    guards: Vec<Guard>,
    loop_active: Vec<String>,
    masked: HashSet<usize>,
    in_const: bool,
}

impl<'a> Visit<'a> for UnderflowVisitor<'a> {
    fn visit_expr_binary(&mut self, node: &'a syn::ExprBinary) {
        match node.op {
            syn::BinOp::Sub(_) => {
                if self.should_skip(&node.left, &node.right, false) {
                    return;
                }
                self.push_underflow(&node.left, &node.right, false);
            }
            syn::BinOp::SubAssign(_) => {
                if self.should_skip(&node.left, &node.right, true) {
                    return;
                }
                self.push_underflow(&node.left, &node.right, true);
            }
            syn::BinOp::BitAnd(_) => {
                for side in [&node.left, &node.right] {
                    if let syn::Expr::Binary(b) = strip_ref_parens(side)
                        && matches!(b.op, syn::BinOp::Sub(_))
                    {
                        let offset = self.offset_of(b.span());
                        self.masked.insert(offset);
                    }
                }
            }
            _ => {}
        }
        syn::visit::visit_expr_binary(self, node);
    }

    fn visit_expr_while(&mut self, node: &'a syn::ExprWhile) {
        if let Some(counter) = self.loop_counter(&node.cond) {
            self.loop_active.push(counter);
        }
        syn::visit::visit_expr_while(self, node);
        self.loop_active.pop();
    }

    fn visit_block(&mut self, node: &'a syn::Block) {
        let depth = self.guards.len();
        for stmt in &node.stmts {
            self.visit_stmt(stmt);
            self.guards.extend(self.guards_of_stmt(stmt));
        }
        self.guards.truncate(depth);
    }

    fn visit_item_const(&mut self, node: &'a syn::ItemConst) {
        let was = std::mem::replace(&mut self.in_const, true);
        syn::visit::visit_item_const(self, node);
        self.in_const = was;
    }

    fn visit_expr_const(&mut self, node: &'a syn::ExprConst) {
        let was = std::mem::replace(&mut self.in_const, true);
        syn::visit::visit_expr_const(self, node);
        self.in_const = was;
    }
}

impl<'a> UnderflowVisitor<'a> {
    fn should_skip(&self, lhs: &syn::Expr, rhs: &syn::Expr, is_assign: bool) -> bool {
        if self.in_const {
            return true;
        }
        if self.masked.contains(&self.offset_of(lhs.span())) {
            return true;
        }
        if is_float(lhs) || is_float(rhs) {
            return true;
        }
        if self.is_const_ish(lhs) && self.is_const_ish(rhs) {
            return true;
        }
        if is_assign && self.is_counter_decrement(lhs, rhs) {
            return true;
        }
        self.is_guarded(lhs, rhs)
    }

    fn is_counter_decrement(&self, lhs: &syn::Expr, rhs: &syn::Expr) -> bool {
        if !is_const_one(rhs) {
            return false;
        }
        self.loop_active.contains(&self.expr_key(lhs))
    }

    /// Returns true when a subtraction is provably bounded by a preceding guard.
    ///
    /// A `Guard::Bounded` proves `minuend >= subtrahend` directly. A
    /// `Guard::NonEmpty` (`if expr == 0 { return }`) proves only `expr != 0`;
    /// that bounds just `expr - 1` (const-one subtrahend) on the guarded
    /// expression itself, never a sibling field's subtraction.
    fn is_guarded(&self, lhs: &syn::Expr, rhs: &syn::Expr) -> bool {
        let min_key = self.expr_key(lhs);
        let sub_key = self.expr_key(rhs);
        if self.guards.iter().any(|g| {
            matches!(g, Guard::Bounded { minuend, subtrahend }
                if *minuend == min_key && *subtrahend == sub_key)
        }) {
            return true;
        }
        is_const_one(rhs)
            && self
                .guards
                .iter()
                .any(|g| matches!(g, Guard::NonEmpty { expr } if *expr == min_key))
    }

    fn push_underflow(&mut self, lhs: &syn::Expr, rhs: &syn::Expr, is_assign: bool) {
        let offset = self.offset_of(lhs.span());
        let op = if is_assign { "-=" } else { "-" };
        let message = format!(
            "subtraction `{} {op} {}`: potential underflow/overflow-panic hazard; use checked_sub/saturating_sub or add a proven bound",
            self.expr_key(lhs),
            self.expr_key(rhs),
        );
        self.findings.push(Finding {
            severity: Severity::Error,
            rule_id: RuleId::Underflow,
            path: self.path.to_path_buf(),
            line: byte_line(self.source, offset),
            column: char_column(self.source, offset),
            message,
        });
    }

    fn offset_of(&self, span: proc_macro2::Span) -> usize {
        let lc = span.start();
        self.line_map.byte_offset(self.source, lc.line, lc.column)
    }

    fn end_offset_of(&self, span: proc_macro2::Span) -> usize {
        let lc = span.end();
        self.line_map.byte_offset(self.source, lc.line, lc.column)
    }

    fn expr_key(&self, expr: &syn::Expr) -> String {
        let start = self.offset_of(expr.span());
        let end = self.end_offset_of(expr.span());
        let text = &self.source[start..end.min(self.source.len())];
        text.split_whitespace().collect()
    }

    fn is_const_ish(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Lit(l) => matches!(&l.lit, syn::Lit::Int(_)),
            syn::Expr::Unary(u) => self.is_const_ish(&u.expr),
            syn::Expr::Binary(b) => self.is_const_ish(&b.left) && self.is_const_ish(&b.right),
            syn::Expr::Cast(c) => self.is_const_ish(&c.expr),
            syn::Expr::Paren(p) => self.is_const_ish(&p.expr),
            syn::Expr::Group(g) => self.is_const_ish(&g.expr),
            _ => false,
        }
    }

    fn loop_counter(&self, cond: &syn::Expr) -> Option<String> {
        if let syn::Expr::Binary(b) = cond
            && matches!(b.op, syn::BinOp::Gt(_))
            && is_zero(&b.right)
        {
            return Some(self.expr_key(&b.left));
        }
        None
    }

    fn guards_of_stmt(&self, stmt: &syn::Stmt) -> Vec<Guard> {
        let expr = match stmt {
            syn::Stmt::Expr(e, _) => e,
            syn::Stmt::Macro(m) => {
                return if m.mac.path.is_ident("assert") {
                    self.assert_guard(&m.mac).into_iter().collect()
                } else {
                    Vec::new()
                };
            }
            _ => return Vec::new(),
        };
        match expr {
            syn::Expr::If(ifex) => {
                let then_block = &ifex.then_branch;
                if !ends_in_control_flow(then_block) {
                    return Vec::new();
                }
                self.collect_return_guards(&ifex.cond)
            }
            syn::Expr::Macro(mac) if mac.mac.path.is_ident("assert") => {
                self.assert_guard(&mac.mac).into_iter().collect()
            }
            _ => Vec::new(),
        }
    }

    fn collect_return_guards(&self, cond: &syn::Expr) -> Vec<Guard> {
        if let syn::Expr::Binary(b) = cond {
            match b.op {
                syn::BinOp::Or(_) => {
                    let mut guards = self.collect_return_guards(&b.left);
                    guards.extend(self.collect_return_guards(&b.right));
                    return guards;
                }
                syn::BinOp::Ge(_) => {
                    return vec![Guard::Bounded {
                        minuend: self.expr_key(&b.right),
                        subtrahend: self.expr_key(&b.left),
                    }];
                }
                syn::BinOp::Eq(_) => {
                    if is_zero(&b.right) {
                        return vec![Guard::NonEmpty { expr: self.expr_key(&b.left) }];
                    }
                    return Vec::new();
                }
                _ => return Vec::new(),
            }
        }
        Vec::new()
    }

    fn assert_guard(&self, mac: &syn::Macro) -> Option<Guard> {
        let tokens = mac.tokens.clone();
        let parsed = syn::parse2::<syn::Expr>(tokens).ok()?;
        let expr = strip_parens(parsed);
        if let syn::Expr::Binary(b) = expr
            && matches!(b.op, syn::BinOp::Le(_))
        {
            return Some(Guard::Bounded {
                minuend: self.expr_key(&b.right),
                subtrahend: self.expr_key(&b.left),
            });
        }
        None
    }
}

fn ends_in_control_flow(block: &syn::Block) -> bool {
    let Some(stmt) = block.stmts.last() else {
        return false;
    };
    let syn::Stmt::Expr(expr, _) = stmt else {
        return false;
    };
    matches!(expr, syn::Expr::Return(_) | syn::Expr::Break(_) | syn::Expr::Continue(_))
}

const fn is_float(expr: &syn::Expr) -> bool {
    matches!(expr, syn::Expr::Lit(l) if matches!(&l.lit, syn::Lit::Float(_)))
}

fn is_const_one(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Lit(l) => matches!(&l.lit, syn::Lit::Int(i) if i.base10_digits() == "1"),
        _ => false,
    }
}

fn is_zero(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Lit(l) => matches!(&l.lit, syn::Lit::Int(i) if i.base10_digits() == "0"),
        _ => false,
    }
}

fn strip_parens(expr: syn::Expr) -> syn::Expr {
    match expr {
        syn::Expr::Paren(p) => strip_parens(*p.expr),
        syn::Expr::Group(g) => strip_parens(*g.expr),
        other => other,
    }
}

fn strip_ref_parens(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Paren(p) => strip_ref_parens(&p.expr),
        syn::Expr::Group(g) => strip_ref_parens(&g.expr),
        other => other,
    }
}

fn plain_comment_finding(path: &Path, source: &str, start: usize, kind: &str) -> Finding {
    Finding {
        severity: Severity::Error,
        rule_id: RuleId::NoNonSafetyComment,
        path: path.to_path_buf(),
        line: byte_line(source, start),
        column: char_column(source, start),
        message: format!("plain {kind} comment is not allowed"),
    }
}

fn byte_line(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset].split(|&b| b == b'\n').count()
}

fn char_column(source: &str, byte_offset: usize) -> usize {
    source.as_bytes()[..byte_offset]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or_else(
            || source[..byte_offset].chars().count() + 1,
            |i| source[i + 1..byte_offset].chars().count() + 1,
        )
}

#[cfg(test)]
mod tests {
    use super::{Allowlist, CheckOutcome, Finding, Linter, RuleId};
    use std::path::PathBuf;

    fn findings(linter: &Linter, source: &str) -> Vec<Finding> {
        match linter.check(source) {
            CheckOutcome::Findings(findings) => findings,
            CheckOutcome::Indeterminate(_) => {
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
        assert!(
            matches!(outcome, CheckOutcome::Indeterminate(ref p) if p == &PathBuf::from("fx.rs")),
            "unparseable source is indeterminate and names the file"
        );
    }

    #[test]
    fn position_helpers_are_one_based() {
        let source = "ab\ncd// comment\n";
        assert_eq!(super::byte_line(source, 5), 2, "second line");
        assert_eq!(super::char_column(source, 5), 3, "third character column");
    }

    #[test]
    fn char_column_counts_characters_not_bytes() {
        let source = "é  - 1\n";
        assert_eq!(super::char_column(source, 4), 4, "char column of `-`");
        assert_eq!(super::byte_line(source, 4), 1, "still on line 1");
    }

    #[test]
    fn underflow_finding_line_and_column_handle_crlf() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f() {\r\n    a - b\r\n}\r\n";
        let findings = findings(&linter, source);
        assert_eq!(findings.len(), 1, "one unbounded subtraction");
        assert_eq!(findings[0].line, 2, "CRLF line number is not off by one");
        assert_eq!(findings[0].column, 5, "CRLF column counts the four leading spaces");
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

    #[test]
    fn flags_unbounded_runtime_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n";
        let findings = findings(&linter, source);
        assert_eq!(findings.len(), 1, "one unbounded subtraction");
        assert_eq!(findings[0].rule_id, RuleId::Underflow, "underflow rule id");
    }

    #[test]
    fn flags_assignop_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(a: u32, b: u32) {\n    let mut a = a;\n    a -= b;\n}\n";
        let findings = findings(&linter, source);
        assert_eq!(findings.len(), 1, "one unbounded `-=`");
        assert_eq!(findings[0].rule_id, RuleId::Underflow, "underflow rule id");
    }

    #[test]
    fn skips_literal_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f() -> u32 {\n    4 - 1\n}\n";
        assert!(findings(&linter, source).is_empty(), "literal - literal");
    }

    #[test]
    fn flags_const_path_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const QUEUE: usize = 16;\nfn f() -> usize {\n    QUEUE - 1\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "const-path is not a const proxy");
    }

    #[test]
    fn flags_method_len_const_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const R: &[u8] = b\"foo\";\nfn f() -> usize {\n    R.len() - 1\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "`.len()` is not a const proxy");
    }

    #[test]
    fn skips_const_item_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const X: u32 = 4 - 1;\n";
        assert!(findings(&linter, source).is_empty(), "const item value is const");
    }

    #[test]
    fn skips_const_block_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const X: u32 = { 4 - 1 };\n";
        assert!(findings(&linter, source).is_empty(), "const block is const");
    }

    #[test]
    fn flags_const_fn_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const fn f(a: u32) -> u32 {\n    a - 1\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "const fn is runtime-callable; a - 1 can underflow");
    }

    #[test]
    fn skips_guarded_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(a: u32, b: u32) -> u32 {\n    if b >= a {\n        return 0;\n    }\n    a - b\n}\n";
        assert!(findings(&linter, source).is_empty(), "guarded by `if b >= a`");
    }

    #[test]
    fn skips_assert_guard_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(a: u32, b: u32) -> u32 {\n    assert!(b <= a);\n    a - b\n}\n";
        assert!(findings(&linter, source).is_empty(), "guarded by assert!");
    }

    #[test]
    fn flags_unproved_counter_decrement() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "struct S { len: u32 }\nimpl S {\n    fn dec(&mut self) {\n        self.len -= 1;\n    }\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "counter name is not a proof");
    }

    #[test]
    fn flags_unproved_local_decrement() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(mut count: u32) {\n    count -= 1;\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "local -= 1 needs a proof");
    }

    #[test]
    fn skips_while_index_decrement() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(mut index: u32) {\n    while index > 0 {\n        index -= 1;\n    }\n}\n";
        assert!(findings(&linter, source).is_empty(), "while index > 0 decrement");
    }

    #[test]
    fn flags_ge_loop_decrement() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(mut x: u32) {\n    while x >= 0 {\n        x -= 1;\n    }\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "`while x >= 0` is not a sound unsigned bound");
    }

    #[test]
    fn skips_same_expr_nonempty_decrement() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(mut x: u32) {\n    if x == 0 {\n        return;\n    }\n    x -= 1;\n}\n";
        assert!(findings(&linter, source).is_empty(), "guard on the guarded expr itself");
    }

    #[test]
    fn skips_mask_wrapped_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "const M: usize = 15;\nfn f(x: usize) -> usize {\n    (x - 1) & M\n}\n";
        assert!(findings(&linter, source).is_empty(), "mask-wrapped subtraction");
    }

    #[test]
    fn skips_saturating_and_checked_sub() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let sat = "fn f(a: u32, b: u32) -> u32 {\n    a.saturating_sub(b)\n}\n";
        let checked = "fn f(a: u32, b: u32) -> Option<u32> {\n    a.checked_sub(b)\n}\n";
        assert!(findings(&linter, sat).is_empty(), "saturating_sub is not a `-`");
        assert!(findings(&linter, checked).is_empty(), "checked_sub is not a `-`");
    }

    #[test]
    fn flags_time_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let time = "fn f(a: std::time::Duration, b: std::time::Duration) -> std::time::Duration {\n    a - b\n}\n";
        assert_eq!(findings(&linter, time).len(), 1, "Duration - Duration panics on underflow");
    }

    #[test]
    fn skips_float_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let float = "fn f(a: f64) -> f64 {\n    a - 1.5\n}\n";
        assert!(findings(&linter, float).is_empty(), "float arithmetic does not underflow-panic");
    }

    #[test]
    fn flags_sibling_field_subtraction() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "struct Q { lengths: [usize; 8], head: usize, off: usize, len: usize }\n\
            impl Q {\n\
            fn consume(&self) -> usize {\n\
            if self.len == 0 {\n\
            return 0;\n\
            }\n\
            self.lengths[self.head] - self.off\n\
            }\n\
            }\n";
        assert_eq!(findings(&linter, source).len(), 1, "sibling-field guard is not a proof");
    }

    #[test]
    fn allowlist_parse_requires_reason() {
        let body = "[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nline = 2\n";
        assert!(Allowlist::parse(body).is_err(), "missing reason is rejected");
    }

    #[test]
    fn allowlist_suppresses_line_match() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n";
        assert_eq!(findings(&linter, source).len(), 1, "flagged without allowlist");
        let allow = Allowlist::parse("[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nline = 2\nreason = \"test site is sized so b never exceeds a\"\n")
            .expect("parses");
        let linter = Linter::with_allowlist(PathBuf::from("fx.rs"), allow);
        assert!(findings(&linter, source).is_empty(), "suppressed by line match");
    }

    #[test]
    fn allowlist_suppresses_snippet_match() {
        let source = "fn f(a: u32, b: u32) -> u32 {\n    a - b\n}\n";
        let allow = Allowlist::parse("[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nsnippet = \"a - b\"\nreason = \"test site is sized so b never exceeds a\"\n")
            .expect("parses");
        let linter = Linter::with_allowlist(PathBuf::from("fx.rs"), allow);
        assert!(findings(&linter, source).is_empty(), "suppressed by snippet match");
    }

    #[test]
    fn allowlist_matches_whitespace_collapsed_snippet() {
        let source = "fn f() {\n    let n = HEAD_BYTES - b\"GET / HTT\".len();\n}\n";
        let allow = Allowlist::parse("[[allow]]\npath = \"fx.rs\"\nrule = \"underflow\"\nsnippet = \"HEAD_BYTES - b\\\"GET/HTT\\\".len()\"\nreason = \"buffer cap is >= the constant byte string length\"\n")
            .expect("parses");
        let linter = Linter::with_allowlist(PathBuf::from("fx.rs"), allow);
        assert!(findings(&linter, source).is_empty(), "whitespace-collapsed snippet matches");
    }

    #[test]
    fn byte_offset_guard_handles_zero_line() {
        let source = "ab\ncd\n";
        let map = super::LineMap::new(source);
        assert_eq!(
            map.byte_offset(source, 0, 0),
            source.len(),
            "a 0 line (synthetic span) must not panic; falls back to source.len()"
        );
    }

    #[test]
    fn plain_comment_column_is_char_column() {
        let linter = Linter::new(PathBuf::from("fx.rs"));
        let source = "fn f() { let s: &str = \"é\"; /* plain */ }\n";
        let findings = findings(&linter, source);
        assert_eq!(findings.len(), 1, "one plain block comment");
        assert_eq!(
            findings[0].column, 29,
            "comment column is the char column (é is one char), not the byte column"
        );
    }
}

