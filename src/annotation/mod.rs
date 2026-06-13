//! Comment-embedded annotations: `// @m1:<kind>[(args)]`.
//!
//! The M1 analogue of Rust attributes / `// eslint-disable`: a reusable
//! mechanism, parsed once here in m1-core and consumed by every downstream tool
//! (lint, typecheck, lsp). Annotations ride inside ordinary `//` line comments
//! (and `/* */` block comments), so they are valid M1 — the compiler ignores
//! them — and need no grammar support beyond the existing comment tokens.
//!
//! ## Syntax
//!
//! ```text
//! // @m1:<kind>[(arg, key=value, ...)]
//! ```
//!
//! The `@m1:` marker (rather than a bare `@`) namespaces toolchain annotations
//! so an ordinary `@`-containing comment never collides. A `kind` is an
//! identifier (`allow`, `requires-finite`, `safety-critical`); arguments are an
//! optional parenthesised, comma-separated list of positional values (`L010`,
//! `"a message"`, `-100`) and/or `key=value` pairs.
//!
//! ## Attachment
//!
//! An annotation applies to a *construct*:
//! - a comment **on the same line as, and after,** a statement is *trailing* and
//!   attaches to that statement;
//! - otherwise the annotation is *leading* and attaches to the next statement
//!   (skipping intervening comment lines, so annotations stack on consecutive
//!   lines above their target).
//!
//! The target is always a *named statement* node, never an anonymous
//! punctuation token (`{`, `}`, `)`, `+`) — attaching to such a token is a
//! silent no-op (m1-core#61). A comment buried inside an expression climbs to
//! its enclosing statement; one with no statement to attach to is *dangling*
//! (`target_byte_range` is `None`).
//!
//! ## Registry + unknown kinds
//!
//! Each tool owns the set of kinds it consumes; [`Registry`] is that set.
//! m1-core emits a [`Severity::Warning`] for any `@m1:` annotation whose kind is
//! not registered — an unknown attribute, like an unknown `#[...]` in Rust.
//!
//! [`Registry::seed`] carries the toolchain-defined kinds: `allow` (suppress a
//! lint/typecheck code), `deprecated`, `unit`, `range`, `trace`,
//! `requires-finite`, `safety-critical`, `source`, `external`, `sanitizes`,
//! `clears` (m1-core#33), and `fmt` — m1-fmt's format-off markers
//! `@m1:fmt(off)` / `@m1:fmt(on)` (m1-fmt#102).

mod args;

use crate::cst::{Cst, Node};
use crate::diagnostic::{Code, Diagnostic, Range, Severity};
use crate::kind::Kind;
use std::collections::{HashMap, HashSet};
use std::ops::Range as ByteRange;

/// The `@m1:` namespace marker distinguishing a toolchain annotation from an
/// ordinary comment.
pub const MARKER: &str = "@m1:";

/// One argument to an annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationArg {
    /// A bare value: `L010`, `"a message"` (quotes stripped), `-100`.
    Positional(String),
    /// A `key=value` pair (quotes stripped from the value).
    Named { key: String, value: String },
}

impl AnnotationArg {
    /// The argument's value — the whole positional, or the value half of a
    /// `key=value` pair. Surrounding double quotes are already stripped.
    pub fn value(&self) -> &str {
        match self {
            AnnotationArg::Positional(v) => v,
            AnnotationArg::Named { value, .. } => value,
        }
    }
}

/// One parsed `@m1:` annotation and the construct it applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    /// The kind, without the `@m1:` marker (`allow`, `requires-finite`, …).
    pub kind: String,
    /// The parsed argument list (empty if no parentheses were present).
    pub args: Vec<AnnotationArg>,
    /// Line/column range of the annotation comment itself.
    pub range: Range,
    /// Byte range of the annotation comment itself.
    pub comment_byte_range: ByteRange<usize>,
    /// Byte range of the construct this annotation applies to: the following
    /// statement (leading) or the preceding statement (trailing). `None` for a
    /// dangling annotation with no target construct.
    pub target_byte_range: Option<ByteRange<usize>>,
}

impl Annotation {
    /// Whether the annotation has a positional argument equal to `v` (e.g. a
    /// diagnostic code in `@allow(L010, T030)`).
    pub fn has_positional(&self, v: &str) -> bool {
        self.args
            .iter()
            .any(|a| matches!(a, AnnotationArg::Positional(p) if p == v))
    }

    /// The value of the first `key=value` argument with key `key`, if any.
    pub fn named(&self, key: &str) -> Option<&str> {
        self.args.iter().find_map(|a| match a {
            AnnotationArg::Named { key: k, value } if k == key => Some(value.as_str()),
            _ => None,
        })
    }

    /// Iterate the positional argument values.
    pub fn positionals(&self) -> impl Iterator<Item = &str> {
        self.args.iter().filter_map(|a| match a {
            AnnotationArg::Positional(p) => Some(p.as_str()),
            _ => None,
        })
    }
}

/// The set of annotation kinds a tool recognises. m1-core warns on any `@m1:`
/// annotation whose kind is not registered.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    kinds: HashSet<String>,
}

impl Registry {
    /// An empty registry — every kind is "unknown" until registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry seeded from an iterator of kind names.
    pub fn with_kinds<I, S>(kinds: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Registry {
            kinds: kinds.into_iter().map(Into::into).collect(),
        }
    }

    /// Register a kind. Returns `&mut self` for chaining.
    pub fn register(&mut self, kind: impl Into<String>) -> &mut Self {
        self.kinds.insert(kind.into());
        self
    }

    /// Whether `kind` is registered.
    pub fn knows(&self, kind: &str) -> bool {
        self.kinds.contains(kind)
    }

    /// The toolchain's seed set of kinds (the table in m1-core#33). A convenient
    /// "know every defined kind" registry; a tool can also build its own with
    /// only the kinds it consumes.
    pub fn seed() -> Self {
        Self::with_kinds([
            "allow",
            "deprecated",
            "unit",
            "range",
            "trace",
            "requires-finite",
            "safety-critical",
            "source",
            "external",
            "sanitizes",
            "clears",
            "fmt",
        ])
    }
}

/// All annotations parsed from a [`Cst`], with an index for fast per-node
/// lookup and any diagnostics produced while parsing (unknown kinds).
#[derive(Debug, Clone, Default)]
pub struct Annotations {
    items: Vec<Annotation>,
    diagnostics: Vec<Diagnostic>,
    by_target_start: HashMap<usize, Vec<usize>>,
}

impl Annotations {
    /// Every parsed annotation, in source order.
    pub fn all(&self) -> &[Annotation] {
        &self.items
    }

    /// Diagnostics produced during parsing (currently: unknown-kind warnings).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The annotations attached to `node` (by its start byte). Empty if none.
    pub fn for_node(&self, node: &Node) -> Vec<&Annotation> {
        self.for_target_start(node.byte_range().start)
    }

    /// The annotations whose target construct starts at byte `start`.
    pub fn for_target_start(&self, start: usize) -> Vec<&Annotation> {
        self.by_target_start
            .get(&start)
            .map(|ix| ix.iter().map(|&i| &self.items[i]).collect())
            .unwrap_or_default()
    }

    /// Whether a diagnostic carrying `code` at `byte_offset` is suppressed by an
    /// `@allow` annotation whose target construct contains the offset.
    ///
    /// `@allow(L010, T030)` suppresses only the listed codes; a bare `@allow`
    /// (no args) suppresses every code on its target.
    pub fn is_allowed(&self, code: &str, byte_offset: usize) -> bool {
        self.items.iter().any(|a| {
            a.kind == "allow"
                && a.target_byte_range
                    .as_ref()
                    .is_some_and(|t| t.contains(&byte_offset))
                && (a.args.is_empty() || a.has_positional(code))
        })
    }
}

/// Parse every `@m1:` annotation in `cst`, attaching each to its target
/// construct and validating kinds against `registry`.
pub fn annotations(cst: &Cst, registry: &Registry) -> Annotations {
    let mut out = Annotations::default();
    for node in cst.root().descendants() {
        if !matches!(node.kind(), Kind::LineComment | Kind::BlockComment) {
            continue;
        }
        let Some((kind, args)) = parse_comment(node.text()) else {
            continue;
        };
        if !registry.knows(&kind) {
            out.diagnostics.push(Diagnostic::at_node(
                node,
                Severity::Warning,
                Code::Annotation,
                format!("unknown annotation kind `@m1:{kind}`"),
            ));
        }
        let target = attachment_target(&node).map(|t| t.byte_range());
        let idx = out.items.len();
        if let Some(t) = &target {
            out.by_target_start.entry(t.start).or_default().push(idx);
        }
        out.items.push(Annotation {
            kind,
            args,
            range: node.range(),
            comment_byte_range: node.byte_range(),
            target_byte_range: target,
        });
    }
    out
}

/// Resolve the construct a comment annotation applies to.
///
/// The target must be a *named statement* node — never an anonymous punctuation
/// token (`{`, `}`, `)`, `+`). Attaching to such a token is a silent no-op:
/// `is_allowed` checks whether the target byte range contains the diagnostic
/// offset, and a one-byte `{` never contains the statement it precedes
/// (m1-core#61). So we skip anonymous siblings, and if a comment is buried
/// inside an expression (no usable statement sibling) we climb to the enclosing
/// statement instead. Returns `None` when there is no statement to attach to.
fn attachment_target<'a>(comment: &Node<'a>) -> Option<Node<'a>> {
    // Trailing: a named statement on the same line as, and before, the comment.
    if let Some(prev) = comment.prev_sibling()
        && is_statement(&prev)
        && prev.range().end.line == comment.range().start.line
    {
        return Some(prev);
    }
    // Leading: the next named statement sibling, skipping comments and anonymous
    // tokens (so stacked annotation/comment lines all resolve to the same
    // following statement, and a trailing `{`/`)` token is not mistaken for one).
    let mut next = comment.next_sibling();
    while let Some(n) = next {
        if is_statement(&n) {
            return Some(n);
        }
        next = n.next_sibling();
    }
    // Buried inside an expression (e.g. `1 + /* @m1:allow */ 2`): fall back to
    // the nearest enclosing statement. This applies only when the comment is NOT
    // a direct child of a statement container (`block` / `source_file`): a
    // trailing comment with no following statement in such a container is
    // genuinely dangling and must stay `None` (m1-core#61), not silently scope
    // the whole enclosing block.
    if !comment.parent().is_some_and(|p| is_statement_container(&p)) {
        let mut ancestor = comment.parent();
        while let Some(a) = ancestor {
            if is_statement(&a) {
                return Some(a);
            }
            ancestor = a.parent();
        }
    }
    None
}

/// Whether `node` is a named statement-level construct an annotation can attach
/// to — the unit `is_allowed` tests its byte range against. Anonymous tokens
/// and sub-statement expression fragments (`number`, `identifier`, `+`) are
/// excluded; a `block` is included so an annotation can scope a whole body.
fn is_statement(node: &Node) -> bool {
    node.is_named()
        && matches!(
            node.kind(),
            Kind::AssignmentStatement
                | Kind::LocalDeclaration
                | Kind::ExpressionStatement
                | Kind::EmptyStatement
                | Kind::IfStatement
                | Kind::WhenStatement
                | Kind::ExpandStatement
                | Kind::Block
        )
}

/// Whether `node` holds statements directly (a `block` body or the file root).
/// A comment whose parent is one of these is in statement position: if no
/// following statement exists it is a genuinely dangling annotation (`None`),
/// not an expression fragment needing the enclosing-statement fallback.
fn is_statement_container(node: &Node) -> bool {
    matches!(node.kind(), Kind::Block | Kind::SourceFile)
}

/// Parse a comment's source text into `(kind, args)` if it is an `@m1:`
/// annotation, else `None`.
fn parse_comment(text: &str) -> Option<(String, Vec<AnnotationArg>)> {
    let body = args::strip_comment_markers(text).trim_start();
    let rest = body.strip_prefix(MARKER)?;
    // Kind: a leading identifier `[A-Za-z][A-Za-z0-9_-]*`.
    let kind_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .unwrap_or(rest.len());
    let kind = &rest[..kind_end];
    if kind.is_empty() || !kind.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let after = rest[kind_end..].trim_start();
    let parsed = match after.strip_prefix('(') {
        Some(inner) => {
            let close = args::find_close_paren(inner).unwrap_or(inner.len());
            args::parse_args(&inner[..close])
        }
        None => Vec::new(),
    };
    Some((kind.to_string(), parsed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn parse_anns(src: &str) -> Annotations {
        annotations(&parse(src), &Registry::seed())
    }

    fn is_comment(node: &Node) -> bool {
        matches!(node.kind(), Kind::LineComment | Kind::BlockComment)
    }

    /// The first statement-ish node (skips comment siblings, which are named).
    fn first_stmt(cst: &Cst) -> Node<'_> {
        cst.root()
            .named_children()
            .into_iter()
            .find(|n| !is_comment(n))
            .unwrap()
    }

    #[test]
    fn parses_kind_and_positional_args() {
        let anns = parse_anns("// @m1:allow(L010, T030)\nlocal x = 1;\n");
        assert_eq!(anns.all().len(), 1);
        let a = &anns.all()[0];
        assert_eq!(a.kind, "allow");
        assert!(a.has_positional("L010"));
        assert!(a.has_positional("T030"));
        assert!(!a.has_positional("L999"));
    }

    #[test]
    fn parses_quoted_and_named_args() {
        let anns = parse_anns("// @m1:deprecated(\"use Foo instead\")\nlocal x = 1;\n");
        let a = &anns.all()[0];
        assert_eq!(a.kind, "deprecated");
        assert_eq!(
            a.args,
            vec![AnnotationArg::Positional("use Foo instead".into())]
        );

        let anns = parse_anns("// @m1:range(min=-100, max=100)\nlocal x = 1;\n");
        let a = &anns.all()[0];
        assert_eq!(a.named("min"), Some("-100"));
        assert_eq!(a.named("max"), Some("100"));
    }

    #[test]
    fn bare_kind_without_parens() {
        let anns = parse_anns("// @m1:safety-critical\nFront Torque = 1;\n");
        let a = &anns.all()[0];
        assert_eq!(a.kind, "safety-critical");
        assert!(a.args.is_empty());
    }

    #[test]
    fn ordinary_comment_is_not_an_annotation() {
        let anns = parse_anns("// just a comment, not @m1 anything\nlocal x = 1;\n");
        assert!(anns.all().is_empty());
        assert!(anns.diagnostics().is_empty());
    }

    #[test]
    fn leading_annotation_attaches_to_following_statement() {
        let src = "// @m1:allow(L010)\nlocal x = 1;\n";
        let anns = parse_anns(src);
        let target = anns.all()[0].target_byte_range.clone().unwrap();
        assert_eq!(&src[target], "local x = 1;");
    }

    #[test]
    fn stacked_annotations_attach_to_same_statement() {
        let src = "// @m1:requires-finite\n// @m1:safety-critical\nFront Torque = 1;\n";
        let anns = parse_anns(src);
        assert_eq!(anns.all().len(), 2);
        let t0 = anns.all()[0].target_byte_range.clone().unwrap();
        let t1 = anns.all()[1].target_byte_range.clone().unwrap();
        assert_eq!(t0, t1);
        assert_eq!(&src[t0], "Front Torque = 1;");
    }

    #[test]
    fn trailing_annotation_attaches_to_preceding_statement() {
        let src = "Ratio = 2; // @m1:allow(L010)\n";
        let anns = parse_anns(src);
        let target = anns.all()[0].target_byte_range.clone().unwrap();
        assert_eq!(&src[target], "Ratio = 2;");
    }

    #[test]
    fn for_node_returns_attached_annotations() {
        let src = "// @m1:allow(T030)\nlocal x = 1;\n";
        let cst = parse(src);
        let anns = annotations(&cst, &Registry::seed());
        let stmt = first_stmt(&cst);
        let attached = anns.for_node(&stmt);
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].kind, "allow");
    }

    #[test]
    fn unknown_kind_warns() {
        let cst = parse("// @m1:bogus(1)\nlocal x = 1;\n");
        let anns = annotations(&cst, &Registry::seed());
        assert_eq!(anns.diagnostics().len(), 1);
        let d = &anns.diagnostics()[0];
        assert_eq!(d.code, Code::Annotation);
        assert_eq!(d.severity, Severity::Warning);
        assert!(d.message.contains("bogus"));
        // The annotation is still parsed and available, just flagged.
        assert_eq!(anns.all()[0].kind, "bogus");
    }

    #[test]
    fn registered_kind_does_not_warn() {
        let cst = parse("// @m1:allow(L010)\nlocal x = 1;\n");
        let anns = annotations(&cst, &Registry::seed());
        assert!(anns.diagnostics().is_empty());
    }

    #[test]
    fn fmt_kind_is_seeded() {
        // m1-fmt's format-off markers (m1-fmt#102) must not trip the
        // unknown-kind warning in tools running with the seed registry.
        for src in [
            "// @m1:fmt(off)\nlocal x = 1;\n",
            "// @m1:fmt(on)\nlocal x = 1;\n",
            "// @m1:fmt(skip)\nlocal x = 1;\n",
        ] {
            let anns = annotations(&parse(src), &Registry::seed());
            assert!(anns.diagnostics().is_empty(), "warned on {src:?}");
            assert_eq!(anns.all()[0].kind, "fmt");
        }
    }

    #[test]
    fn is_allowed_suppresses_listed_code_on_target() {
        let src = "// @m1:allow(L010)\nlocal x = 1;\n";
        let cst = parse(src);
        let anns = annotations(&cst, &Registry::seed());
        let stmt = first_stmt(&cst);
        let mid = stmt.byte_range().start + 2;
        assert!(anns.is_allowed("L010", mid));
        // A different code is not suppressed by this @allow.
        assert!(!anns.is_allowed("T030", mid));
        // Outside the target construct, nothing is suppressed.
        assert!(!anns.is_allowed("L010", 0));
    }

    #[test]
    fn bare_allow_suppresses_every_code_on_target() {
        let src = "// @m1:allow\nlocal x = 1;\n";
        let cst = parse(src);
        let anns = annotations(&cst, &Registry::seed());
        let stmt = first_stmt(&cst);
        let mid = stmt.byte_range().start + 2;
        assert!(anns.is_allowed("L010", mid));
        assert!(anns.is_allowed("T030", mid));
    }

    #[test]
    fn annotation_inside_a_block_attaches_locally() {
        let src = "if x {\n\t// @m1:allow(L010)\n\tValue = 1;\n}\n";
        let cst = parse(src);
        let anns = annotations(&cst, &Registry::seed());
        assert_eq!(anns.all().len(), 1);
        let target = anns.all()[0].target_byte_range.clone().unwrap();
        assert_eq!(&src[target], "Value = 1;");
    }

    #[test]
    fn dangling_annotation_has_no_target() {
        // An annotation with nothing after it (end of file, no statement).
        let src = "local x = 1;\n// @m1:trace\n";
        let anns = parse_anns(src);
        // `trace` parsed, but there is no following construct.
        let a = anns.all().iter().find(|a| a.kind == "trace").unwrap();
        assert!(a.target_byte_range.is_none());
    }

    #[test]
    fn annotation_arg_with_nested_parens() {
        // A positional argument that is itself a parenthesised expression must
        // be captured whole — the inner `)` must not end the arg list, and the
        // inner `,` must not split the argument (regression for nested parens).
        let anns = parse_anns("// @m1:trace(scale(x, 2), raw)\nlocal x = 1;\n");
        assert_eq!(anns.all().len(), 1);
        let a = &anns.all()[0];
        assert_eq!(a.kind, "trace");
        assert_eq!(
            a.args,
            vec![
                AnnotationArg::Positional("scale(x, 2)".into()),
                AnnotationArg::Positional("raw".into()),
            ]
        );
    }

    #[test]
    fn block_comment_annotation() {
        let anns = parse_anns("/* @m1:allow(L010) */\nlocal x = 1;\n");
        assert_eq!(anns.all().len(), 1);
        assert_eq!(anns.all()[0].kind, "allow");
        assert!(anns.all()[0].has_positional("L010"));
    }

    /// m1-core#61: an annotation must never attach to an anonymous punctuation
    /// token (`{`, `}`, `)`, `+`). Doing so was a silent no-op — `is_allowed`
    /// tests the target byte range against the diagnostic offset, and a 1-byte
    /// `{` never contains the statement it precedes. Each repro must now either
    /// suppress correctly (target a named statement that contains the offset) or
    /// cleanly no-op (`None`).
    #[test]
    fn annotation_after_open_brace_targets_following_statement() {
        // `{ // @m1:allow(...)` — prev sibling is the anonymous `{`; the comment
        // must skip it and attach to the following `Value = 1;`.
        let src = "if x\n{ // @m1:allow(L010)\n\tValue = 1;\n}\n";
        let anns = parse_anns(src);
        let a = anns.all().iter().find(|a| a.kind == "allow").unwrap();
        let target = a.target_byte_range.clone().unwrap();
        assert_eq!(&src[target.clone()], "Value = 1;");
        // The diagnostic offset inside the statement is suppressed.
        assert!(anns.is_allowed("L010", target.start + 1));
    }

    #[test]
    fn annotation_before_close_brace_is_dangling() {
        // A comment as the last item in a block has no following statement and
        // its prev sibling is on a different line: it must dangle (`None`), not
        // attach to the anonymous `}`.
        let src = "if x\n{\n\tValue = 1;\n\t// @m1:allow(L010)\n}\n";
        let anns = parse_anns(src);
        let a = anns.all().iter().find(|a| a.kind == "allow").unwrap();
        assert!(
            a.target_byte_range.is_none(),
            "expected dangling, got {:?}",
            a.target_byte_range
        );
    }

    #[test]
    fn annotation_after_condition_paren_scopes_the_body() {
        // `if (x) // @m1:allow(...)` — prev sibling is the anonymous `)`; the
        // comment must skip it and attach to the following named `block`, whose
        // range contains the statement inside.
        let src = "if (x) // @m1:allow(L010)\n{\n\tValue = 1;\n}\n";
        let anns = parse_anns(src);
        let a = anns.all().iter().find(|a| a.kind == "allow").unwrap();
        let target = a.target_byte_range.clone().unwrap();
        // Target is the block; the inner statement offset is suppressed.
        let stmt_off = src.find("Value = 1;").unwrap();
        assert!(target.contains(&stmt_off));
        assert!(anns.is_allowed("L010", stmt_off));
        // It must not be the bare anonymous `)`.
        assert_ne!(&src[target], ")");
    }

    #[test]
    fn annotation_inside_expression_climbs_to_enclosing_statement() {
        // `1 + /* @m1:allow */ 2` — the comment sits between the anonymous `+`
        // and `2` inside a binary expression. It must climb to the enclosing
        // `assignment_statement`, not target `+`, `2`, or `1`.
        let src = "Value = 1 + /* @m1:allow(L010) */ 2;\n";
        let anns = parse_anns(src);
        let a = anns.all().iter().find(|a| a.kind == "allow").unwrap();
        let target = a.target_byte_range.clone().unwrap();
        assert_eq!(&src[target.clone()], "Value = 1 + /* @m1:allow(L010) */ 2;");
        assert!(anns.is_allowed("L010", target.start + 1));
    }

    #[test]
    fn working_trailing_allow_still_targets_statement() {
        // The control case from the issue must keep working unchanged.
        let src = "y = 1; // @m1:allow(L010)\n";
        let anns = parse_anns(src);
        let a = anns.all().iter().find(|a| a.kind == "allow").unwrap();
        let target = a.target_byte_range.clone().unwrap();
        assert_eq!(&src[target.clone()], "y = 1;");
        assert!(anns.is_allowed("L010", target.start + 1));
    }
}
