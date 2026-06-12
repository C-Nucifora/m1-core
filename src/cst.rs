//! The wrapped concrete syntax tree. The only module that depends on
//! `tree_sitter`; everything outside sees m1-core's own [`Cst`]/[`Node`].

use crate::diagnostic::{Code, Diagnostic, Position, Range, Severity};
use crate::field::Field;
use crate::kind::Kind;

/// A conservative nesting-depth bound for consumers that recurse over the tree.
///
/// tree-sitter builds an unbounded-depth tree from deeply nested input, so the
/// natural `for c in node.children() { recurse(c) }` pattern can stack-overflow
/// (an *uncatchable* abort, fatal to the long-lived LSP) on adversarial source.
/// Real M1 code nests only a handful of levels — the reference corpus tops out
/// at 6 — so a tree deeper than this is adversarial. Consumers that must recurse
/// (type inference, pretty-printing) should compare [`Node::max_depth`] against
/// this once and bail with a single diagnostic instead of recursing. The value
/// is far above any real input yet far below the observed ~24k-frame crash
/// threshold, and safe even on the LSP's smaller worker-thread stacks (#35).
pub const MAX_RECURSION_DEPTH: usize = 1024;

/// A parsed M1 source file: the tree-sitter tree plus the owned source text.
#[derive(Debug)]
pub struct Cst {
    tree: tree_sitter::Tree,
    source: String,
}

/// A single contiguous text edit, for incremental reparsing. Byte range
/// `start_byte..old_end_byte` of the previous source was replaced by content
/// ending at `new_end_byte` in the new source. (Byte offsets, not char.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
}

fn make_parser() -> tree_sitter::Parser {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_m1::LANGUAGE.into())
        .expect("load M1 grammar");
    parser
}

thread_local! {
    // One parser per thread, reused across parses: `Parser::set_language`
    // installs the compiled grammar automaton, which is too expensive to redo
    // on every `parse`/`reparse` call — the LSP hits `reparse` per keystroke
    // (#50). `tree_sitter::Parser` is not `Sync`, so thread-local is the
    // correct sharing granularity.
    static PARSER: std::cell::RefCell<tree_sitter::Parser> =
        std::cell::RefCell::new(make_parser());
}

/// Run `f` with the thread's cached parser.
fn with_parser<T>(f: impl FnOnce(&mut tree_sitter::Parser) -> T) -> T {
    PARSER.with(|p| f(&mut p.borrow_mut()))
}

/// tree-sitter `Point` (row + byte-column) of `byte` within `src`.
fn point_at(src: &str, byte: usize) -> tree_sitter::Point {
    let mut b = byte.min(src.len());
    // Round down to a UTF-8 char boundary: an `Edit` byte offset that lands
    // inside a multibyte character would otherwise panic the `src[..b]` slice
    // below ("byte index N is not a char boundary"). A column off by a couple of
    // bytes within one codepoint is harmless; a panic in the reparse path is not
    // (it would crash the LSP once incremental reparse is wired up) (#36).
    while b > 0 && !src.is_char_boundary(b) {
        b -= 1;
    }
    let row = src.as_bytes()[..b].iter().filter(|&&c| c == b'\n').count();
    let column = b - src[..b].rfind('\n').map(|i| i + 1).unwrap_or(0);
    tree_sitter::Point { row, column }
}

/// Convert a tree-sitter `Point` (row + byte-column) at `byte` into a
/// [`Position`] with a **character** column, the single column unit for
/// `Position` (#57). Cheaper than [`crate::byte_to_position`]: the line start
/// is already known from the point, so only the current line is scanned.
fn char_position(source: &str, byte: usize, point: tree_sitter::Point) -> Position {
    let line_start = byte.saturating_sub(point.column);
    let column = source
        .get(line_start..byte)
        .map(|line| line.chars().count())
        // Defensive: out-of-range or non-boundary slices (incremental-reparse
        // edges) fall back to the byte column rather than panicking.
        .unwrap_or(point.column);
    Position {
        line: point.row as u32,
        column: column as u32,
    }
}

/// Parse M1 source into a [`Cst`]. Infallible: grammar load is a build
/// invariant and tree-sitter always returns a tree.
pub fn parse(src: &str) -> Cst {
    let tree = with_parser(|p| p.parse(src, None)).expect("tree-sitter always returns a tree");
    Cst {
        tree,
        source: src.to_string(),
    }
}

impl Cst {
    /// The original source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Incrementally reparse after `edit`, reusing the previous tree for the
    /// unaffected subtrees (#9). `new_src` is the full updated source. The
    /// result is identical to `parse(new_src)` but only the nodes touched by the
    /// edit are rebuilt — the fast path for editor keystrokes.
    pub fn reparse(&self, edit: &Edit, new_src: &str) -> Cst {
        let input_edit = tree_sitter::InputEdit {
            start_byte: edit.start_byte,
            old_end_byte: edit.old_end_byte,
            new_end_byte: edit.new_end_byte,
            start_position: point_at(&self.source, edit.start_byte),
            old_end_position: point_at(&self.source, edit.old_end_byte),
            new_end_position: point_at(new_src, edit.new_end_byte),
        };
        let mut old_tree = self.tree.clone();
        old_tree.edit(&input_edit);
        let tree = with_parser(|p| p.parse(new_src, Some(&old_tree)))
            .expect("tree-sitter always returns a tree");
        Cst {
            tree,
            source: new_src.to_string(),
        }
    }

    /// All syntax-error diagnostics (ERROR and MISSING nodes) in this tree.
    pub fn syntax_diagnostics(&self) -> Vec<crate::diagnostic::Diagnostic> {
        crate::syntax::collect(self)
    }

    /// The root node (`source_file`).
    pub fn root(&self) -> Node<'_> {
        Node {
            inner: self.tree.root_node(),
            source: &self.source,
        }
    }

    /// Clamp a query offset for `*_node_at_offset`. Interior offsets are left
    /// untouched. An offset at or past end-of-file is pulled back to the last
    /// non-whitespace byte, so an EOF cursor (typically sitting just after a
    /// trailing newline) resolves to the last real token rather than the root
    /// `source_file` node (see #12).
    fn clamp_query_offset(&self, offset: usize) -> usize {
        let len = self.source.len();
        if offset < len {
            return offset;
        }
        let bytes = self.source.as_bytes();
        let mut off = len.saturating_sub(1);
        while off > 0 && bytes[off].is_ascii_whitespace() {
            off -= 1;
        }
        off
    }

    /// The smallest node whose byte span contains `offset` (any node). Offsets
    /// at or past end-of-file resolve to the last token rather than the root
    /// node; always returns a node.
    pub fn node_at_offset(&self, offset: usize) -> Node<'_> {
        let off = self.clamp_query_offset(offset);
        let inner = self
            .tree
            .root_node()
            .descendant_for_byte_range(off, off)
            .unwrap_or_else(|| self.tree.root_node());
        Node {
            inner,
            source: &self.source,
        }
    }

    /// The smallest *named* node whose byte span contains `offset`. Offsets at
    /// or past end-of-file resolve to the last token; always returns a node.
    pub fn named_node_at_offset(&self, offset: usize) -> Node<'_> {
        let off = self.clamp_query_offset(offset);
        let inner = self
            .tree
            .root_node()
            .named_descendant_for_byte_range(off, off)
            .unwrap_or_else(|| self.tree.root_node());
        Node {
            inner,
            source: &self.source,
        }
    }
}

/// A node in the CST, wrapping a `tree_sitter::Node` plus a borrow of the
/// source so callers can get text and ranges without a separate handle.
#[derive(Debug, Clone, Copy)]
pub struct Node<'a> {
    inner: tree_sitter::Node<'a>,
    source: &'a str,
}

impl<'a> Node<'a> {
    /// The typed node kind.
    pub fn kind(&self) -> Kind {
        Kind::from_kind_str(self.inner.kind())
    }

    /// The raw tree-sitter kind string (escape hatch / `Other` recovery).
    pub fn kind_str(&self) -> &'a str {
        self.inner.kind()
    }

    /// The source text this node spans.
    pub fn text(&self) -> &'a str {
        // Defensive: a node's byte range is normally within source, but guard
        // against an out-of-range range (e.g. an incremental-reparse edge) by
        // returning "" rather than panicking on a bad slice.
        self.source.get(self.inner.byte_range()).unwrap_or("")
    }

    /// Byte offsets of this node within the source.
    pub fn byte_range(&self) -> std::ops::Range<usize> {
        self.inner.byte_range()
    }

    /// Line/column range (0-based; column counts **characters** within the
    /// line, matching [`crate::byte_to_position`] — see [`Position`]).
    pub fn range(&self) -> Range {
        let s = self.inner.start_position();
        let e = self.inner.end_position();
        Range {
            start: char_position(self.source, self.inner.start_byte(), s),
            end: char_position(self.source, self.inner.end_byte(), e),
        }
    }

    /// Build a diagnostic spanning exactly this node.
    pub fn diagnostic(
        &self,
        severity: Severity,
        code: Code,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(severity, code, self.range(), self.byte_range(), message)
    }

    /// True if this is an ERROR node.
    pub fn is_error(&self) -> bool {
        self.inner.is_error()
    }

    /// True if this is a zero-width MISSING node inserted during recovery.
    pub fn is_missing(&self) -> bool {
        self.inner.is_missing()
    }

    /// The parent node, if any.
    pub fn parent(&self) -> Option<Node<'a>> {
        self.inner.parent().map(|inner| Node {
            inner,
            source: self.source,
        })
    }

    /// The next sibling in the parent's child list, if any.
    pub fn next_sibling(&self) -> Option<Node<'a>> {
        self.inner.next_sibling().map(|inner| Node {
            inner,
            source: self.source,
        })
    }

    /// The previous sibling in the parent's child list, if any.
    pub fn prev_sibling(&self) -> Option<Node<'a>> {
        self.inner.prev_sibling().map(|inner| Node {
            inner,
            source: self.source,
        })
    }

    /// All direct children (named and anonymous), collected into a `Vec`.
    ///
    /// Allocates a `Vec` on every call. For hot loops that only iterate the
    /// children once, prefer the allocation-free [`Node::child_nodes`].
    pub fn children(&self) -> Vec<Node<'a>> {
        let mut cursor = self.inner.walk();
        self.inner
            .children(&mut cursor)
            .map(|inner| Node {
                inner,
                source: self.source,
            })
            .collect()
    }

    /// Direct named children only (skips punctuation/keywords), collected into
    /// a `Vec`.
    ///
    /// Allocates a `Vec` on every call. For hot loops that only iterate the
    /// children once, prefer the allocation-free [`Node::named_child_nodes`].
    pub fn named_children(&self) -> Vec<Node<'a>> {
        let mut cursor = self.inner.walk();
        self.inner
            .named_children(&mut cursor)
            .map(|inner| Node {
                inner,
                source: self.source,
            })
            .collect()
    }

    /// Lazy iterator over all direct children (named and anonymous).
    pub fn child_nodes(&self) -> Children<'a> {
        Children {
            parent: self.inner,
            source: self.source,
            index: 0,
            count: self.inner.child_count(),
            named_only: false,
        }
    }

    /// Lazy iterator over direct named children only.
    pub fn named_child_nodes(&self) -> Children<'a> {
        Children {
            parent: self.inner,
            source: self.source,
            index: 0,
            count: self.inner.named_child_count(),
            named_only: true,
        }
    }

    /// Pre-order iterator over this node and all of its descendants.
    ///
    /// This is the recursion-safe way to visit every node in a subtree: it uses
    /// a single heap work-list, so it does not stack-overflow on a
    /// pathologically deep tree the way a naive recursive `children()` walk
    /// would. Prefer it over hand-rolled recursion wherever a traversal only
    /// needs to see each node.
    pub fn descendants(&self) -> Descendants<'a> {
        Descendants {
            stack: vec![self.inner],
            source: self.source,
        }
    }

    /// The maximum nesting depth of the subtree rooted at this node, counting
    /// this node as depth 1.
    ///
    /// Computed iteratively (an explicit heap work-stack, never recursion), so
    /// it is safe to call on an arbitrarily deep tree. A consumer that *must*
    /// recurse over expression nesting can call this once on the root and bail
    /// with an "input too deeply nested" diagnostic when it exceeds
    /// [`MAX_RECURSION_DEPTH`], turning an uncatchable stack-overflow abort on
    /// adversarial input into a clean diagnostic (#35).
    pub fn max_depth(&self) -> usize {
        let mut max = 0;
        let mut stack: Vec<(tree_sitter::Node<'a>, usize)> = vec![(self.inner, 1)];
        while let Some((node, depth)) = stack.pop() {
            if depth > max {
                max = depth;
            }
            let count = node.child_count();
            for i in 0..count {
                if let Some(child) = node.child(i as u32) {
                    stack.push((child, depth + 1));
                }
            }
        }
        max
    }

    /// The child filling the given grammar field, if present.
    pub fn child_by_field(&self, field: Field) -> Option<Node<'a>> {
        self.inner
            .child_by_field_name(field.as_str())
            .map(|inner| Node {
                inner,
                source: self.source,
            })
    }
}

/// Iterator over a node's direct children, yielded lazily. When `named_only`
/// is set, only named children are visited. Allocates nothing per element.
pub struct Children<'a> {
    parent: tree_sitter::Node<'a>,
    source: &'a str,
    index: usize,
    count: usize,
    named_only: bool,
}

impl<'a> Iterator for Children<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        while self.index < self.count {
            let i = self.index;
            self.index += 1;
            let child = if self.named_only {
                self.parent.named_child(i as u32)
            } else {
                self.parent.child(i as u32)
            };
            if let Some(inner) = child {
                return Some(Node {
                    inner,
                    source: self.source,
                });
            }
        }
        None
    }
}

impl<'a> DoubleEndedIterator for Children<'a> {
    // Reverse iteration without collecting: work-stack traversals push children
    // in reverse to pop them in source order, and `child_nodes().rev()` lets
    // them do so allocation-free instead of via `children()` (#49).
    fn next_back(&mut self) -> Option<Node<'a>> {
        while self.index < self.count {
            self.count -= 1;
            let i = self.count;
            let child = if self.named_only {
                self.parent.named_child(i as u32)
            } else {
                self.parent.child(i as u32)
            };
            if let Some(inner) = child {
                return Some(Node {
                    inner,
                    source: self.source,
                });
            }
        }
        None
    }
}

/// Pre-order iterator over a node and all of its descendants (node first, then
/// each child's subtree, left to right). Uses a single worklist for the whole
/// traversal rather than allocating a child vector per node.
pub struct Descendants<'a> {
    stack: Vec<tree_sitter::Node<'a>>,
    source: &'a str,
}

impl<'a> Iterator for Descendants<'a> {
    type Item = Node<'a>;

    fn next(&mut self) -> Option<Node<'a>> {
        let inner = self.stack.pop()?;
        // Push children in reverse so the leftmost is popped next (pre-order).
        let count = inner.child_count();
        for i in (0..count).rev() {
            if let Some(child) = inner.child(i as u32) {
                self.stack.push(child);
            }
        }
        Some(Node {
            inner,
            source: self.source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, Node};
    use crate::{Kind, parse};

    /// Flatten a tree to `(kind, start, end)` for every node, depth-first.
    fn shape(root: Node) -> Vec<(Kind, usize, usize)> {
        fn walk(n: Node, out: &mut Vec<(Kind, usize, usize)>) {
            let r = n.byte_range();
            out.push((n.kind(), r.start, r.end));
            for c in n.children() {
                walk(c, out);
            }
        }
        let mut out = Vec::new();
        walk(root, &mut out);
        out
    }

    /// Incremental reparse must yield the identical tree a full parse would.
    #[test]
    fn reparse_matches_full_parse_on_insert() {
        let old = "local x = 1 + 2;\nRatio = x;\n";
        // Insert " * 3" after "2": bytes [14..14) -> "... 2 * 3;".
        let new = "local x = 1 + 2 * 3;\nRatio = x;\n";
        let at = old.find("2;").unwrap() + 1; // byte right after the `2`
        let cst = parse(old);
        let edit = Edit {
            start_byte: at,
            old_end_byte: at,
            new_end_byte: at + " * 3".len(),
        };
        let inc = cst.reparse(&edit, new);
        assert_eq!(inc.source(), new);
        assert_eq!(shape(inc.root()), shape(parse(new).root()));
    }

    #[test]
    fn reparse_matches_full_parse_on_multiline_delete() {
        let old = "local a = 1;\nlocal b = 2;\nlocal c = 3;\n";
        // Delete the middle line entirely.
        let start = old.find("local b").unwrap();
        let end = old.find("local c").unwrap();
        let new = format!("{}{}", &old[..start], &old[end..]);
        let cst = parse(old);
        let edit = Edit {
            start_byte: start,
            old_end_byte: end,
            new_end_byte: start, // deletion: nothing inserted
        };
        let inc = cst.reparse(&edit, &new);
        assert_eq!(inc.source(), new);
        assert_eq!(shape(inc.root()), shape(parse(&new).root()));
    }

    #[test]
    fn parses_and_walks() {
        let cst = parse("local x = 1;\n");
        let root = cst.root();
        assert_eq!(root.kind(), Kind::SourceFile);

        let decl = root.children().into_iter().next().unwrap();
        assert_eq!(decl.kind(), Kind::LocalDeclaration);
        assert_eq!(decl.kind_str(), "local_declaration");
    }

    #[test]
    fn node_text_and_range_round_trip() {
        let src = "Ratio = 2;\n";
        let cst = parse(src);
        let assign = cst.root().children().into_iter().next().unwrap();
        let target = assign.named_children().into_iter().next().unwrap();
        assert_eq!(target.kind(), Kind::Identifier);
        assert_eq!(target.text(), "Ratio");
        assert_eq!(target.range().start.line, 0);
        assert_eq!(target.range().start.column, 0);
        assert_eq!(target.range().end.column, 5);
        assert_eq!(&src[target.byte_range()], "Ratio");
    }

    #[test]
    fn range_columns_count_chars_after_multibyte() {
        // `°` is two bytes. Both declarations share line 0, so the second
        // one's column must agree with `byte_to_position` (characters, not
        // bytes) — the single column invariant for `Position` (#57).
        let src = "local a = \"°°\"; local b = 1;\n";
        let cst = parse(src);
        let second = cst.root().children().into_iter().nth(1).unwrap();
        assert_eq!(second.text(), "local b = 1;");
        let start_byte = second.byte_range().start;
        assert_eq!(
            second.range().start,
            crate::byte_to_position(src, start_byte)
        );
        assert_eq!(
            second.range().start.column as usize,
            src[..start_byte].chars().count()
        );
    }

    #[test]
    fn multi_word_identifier_is_one_node() {
        // Exercises the external scanner through the m1-core boundary.
        // (Identifiers here are synthetic placeholders, not from any real project.)
        let cst = parse("Vund Klee.Trilby Glonk = 1;\n");
        let assign = cst.root().children().into_iter().next().unwrap();
        let member = assign.named_children().into_iter().next().unwrap();
        assert_eq!(member.kind(), Kind::MemberExpression);
        let obj = member.named_children().into_iter().next().unwrap();
        assert_eq!(obj.text(), "Vund Klee");
    }

    #[test]
    fn child_by_field_finds_roles() {
        use crate::Field;
        let cst = parse("x = a + b;\n");
        let stmt = cst.root().children().into_iter().next().unwrap();
        assert_eq!(stmt.kind(), Kind::AssignmentStatement);
        let target = stmt.child_by_field(Field::Target).unwrap();
        assert_eq!(target.text(), "x");

        let value = stmt.child_by_field(Field::Value).unwrap();
        assert_eq!(value.kind(), Kind::BinaryExpression);
        assert_eq!(value.child_by_field(Field::Left).unwrap().text(), "a");
        assert_eq!(value.child_by_field(Field::Operator).unwrap().text(), "+");
        assert_eq!(value.child_by_field(Field::Right).unwrap().text(), "b");

        // Absent field -> None.
        assert!(stmt.child_by_field(Field::Condition).is_none());
    }

    #[test]
    fn is_pattern_list_shape() {
        // Grammar v0.5.0: `is (A or B)` is a compile-time pattern list, not a
        // boolean binary_expression. Single-pattern is-clauses keep the plain
        // expression shape; `or` outside an is-clause stays BinaryExpression.
        let src = "when (Mode)\n{\n\tis (State.A or State.B)\n\t{\n\t\tx = 1;\n\t}\n\tis (Off)\n\t{\n\t\tx = 2;\n\t}\n}\ny = a or b;\n";
        let cst = parse(src);
        let clauses: Vec<Node> = cst
            .root()
            .descendants()
            .filter(|n| n.kind() == Kind::IsClause)
            .collect();
        assert_eq!(clauses.len(), 2);

        let multi = clauses[0].child_by_field(crate::Field::State).unwrap();
        assert_eq!(multi.kind(), Kind::IsPatternList);
        let patterns: Vec<Node> = multi
            .named_children()
            .into_iter()
            .filter(|c| c.kind() == Kind::MemberExpression)
            .collect();
        assert_eq!(patterns.len(), 2, "two member-path patterns");
        // The first pattern is reachable via the repeated `pattern` field too.
        assert_eq!(
            multi.child_by_field(crate::Field::Pattern).unwrap().kind(),
            Kind::MemberExpression
        );

        let single = clauses[1].child_by_field(crate::Field::State).unwrap();
        assert_eq!(single.kind(), Kind::Identifier, "single pattern unchanged");

        let or_expr = cst
            .root()
            .descendants()
            .find(|n| n.kind() == Kind::BinaryExpression)
            .expect("y = a or b stays a BinaryExpression");
        assert!(or_expr.text().contains("or"));
    }

    #[test]
    fn sibling_navigation() {
        let cst = parse("x = a + b;\n");
        let stmt = cst.root().children().into_iter().next().unwrap();
        let value = {
            use crate::Field;
            stmt.child_by_field(Field::Value).unwrap()
        };
        // children of the binary expression: a, +, b
        let left = value.children().into_iter().next().unwrap();
        assert_eq!(left.text(), "a");
        let op = left.next_sibling().unwrap();
        assert_eq!(op.text(), "+");
        let right = op.next_sibling().unwrap();
        assert_eq!(right.text(), "b");
        assert!(right.next_sibling().is_none());
        assert_eq!(op.prev_sibling().unwrap().text(), "a");
        assert!(left.prev_sibling().is_none());
    }

    #[test]
    fn child_iterators_match_vec_accessors() {
        let cst = parse("if x { y = 1; }\n");
        let if_stmt = cst.root().children().into_iter().next().unwrap();

        let iter_all: Vec<_> = if_stmt.child_nodes().map(|n| n.kind()).collect();
        let vec_all: Vec<_> = if_stmt.children().iter().map(|n| n.kind()).collect();
        assert_eq!(iter_all, vec_all);

        let iter_named: Vec<_> = if_stmt.named_child_nodes().map(|n| n.kind()).collect();
        let vec_named: Vec<_> = if_stmt.named_children().iter().map(|n| n.kind()).collect();
        assert_eq!(iter_named, vec_named);

        // Iterators are non-empty for a node with children and byte-faithful.
        assert!(iter_all.len() >= iter_named.len());
        let first = if_stmt.child_nodes().next().unwrap();
        assert_eq!(first.byte_range(), if_stmt.children()[0].byte_range());
    }

    #[test]
    fn child_nodes_rev_matches_reversed_children() {
        let cst = parse("if (a > 1)\n{\n\tb = 2;\n}\n");
        let if_stmt = cst.root().children().into_iter().next().unwrap();

        let rev_iter: Vec<_> = if_stmt
            .child_nodes()
            .rev()
            .map(|n| n.byte_range())
            .collect();
        let mut rev_vec: Vec<_> = if_stmt.children().iter().map(|n| n.byte_range()).collect();
        rev_vec.reverse();
        assert_eq!(rev_iter, rev_vec);

        let rev_named: Vec<_> = if_stmt
            .named_child_nodes()
            .rev()
            .map(|n| n.byte_range())
            .collect();
        let mut named_vec: Vec<_> = if_stmt
            .named_children()
            .iter()
            .map(|n| n.byte_range())
            .collect();
        named_vec.reverse();
        assert_eq!(rev_named, named_vec);

        // Meeting in the middle yields each child exactly once.
        let n = if_stmt.children().len();
        let mut iter = if_stmt.child_nodes();
        let mut seen = 0;
        while iter.next().is_some() {
            seen += 1;
            if iter.next_back().is_some() {
                seen += 1;
            }
        }
        assert_eq!(seen, n);
    }

    #[test]
    fn descendants_preorder_matches_recursive_walk() {
        let cst = parse("x = a + b;\n");
        let root = cst.root();

        // Reference: recursive children() walk (node first, then each subtree).
        fn rec<'a>(n: crate::Node<'a>, out: &mut Vec<(Kind, std::ops::Range<usize>)>) {
            out.push((n.kind(), n.byte_range()));
            for c in n.children() {
                rec(c, out);
            }
        }
        let mut reference = Vec::new();
        rec(root, &mut reference);

        let via_iter: Vec<_> = root
            .descendants()
            .map(|n| (n.kind(), n.byte_range()))
            .collect();
        assert_eq!(via_iter, reference);

        // Root is yielded first.
        assert_eq!(root.descendants().next().unwrap().kind(), Kind::SourceFile);
    }

    #[test]
    fn node_at_offset_finds_token() {
        let src = "Ratio = 2;\n";
        let cst = parse(src);
        // Offset 2 is inside "Ratio".
        let n = cst.node_at_offset(2);
        assert_eq!(n.kind(), Kind::Identifier);
        assert_eq!(n.text(), "Ratio");

        // Named lookup at the "2" literal.
        let two_at = src.find('2').unwrap();
        let named = cst.named_node_at_offset(two_at);
        assert_eq!(named.kind(), Kind::Number);
        assert_eq!(named.text(), "2");

        // Past EOF clamps and never panics; returns some node.
        let past = cst.node_at_offset(src.len() + 100);
        let _ = past.kind();
    }

    #[test]
    fn node_at_eof_returns_last_token_not_root() {
        // Cursor exactly at source.len() must resolve to the last token, not
        // the root SourceFile (regression for #12).
        let src = "Ratio = 2;\n";
        let cst = parse(src);
        let at_eof = cst.node_at_offset(src.len());
        assert_ne!(at_eof.kind(), Kind::SourceFile);
        let named_eof = cst.named_node_at_offset(src.len());
        assert_ne!(named_eof.kind(), Kind::SourceFile);

        // No trailing newline: EOF sits right after ';'.
        let src2 = "Ratio = 2;";
        let cst2 = parse(src2);
        assert_ne!(cst2.node_at_offset(src2.len()).kind(), Kind::SourceFile);

        // Empty source must still not panic and returns a node.
        let empty = parse("");
        let _ = empty.node_at_offset(0).kind();
    }

    #[test]
    fn node_diagnostic_spans_node() {
        use crate::{Code, Severity};
        let src = "Ratio = 2;\n";
        let cst = parse(src);
        let target = cst
            .root()
            .children()
            .into_iter()
            .next()
            .unwrap()
            .named_children()
            .into_iter()
            .next()
            .unwrap();
        let d = target.diagnostic(Severity::Warning, Code::SyntaxError, "hi");
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.code, Code::SyntaxError);
        assert_eq!(d.message, "hi");
        assert_eq!(d.byte_range, target.byte_range());
        assert_eq!(d.range, target.range());
    }

    #[test]
    fn point_at_handles_mid_codepoint_offset() {
        // Regression for #36: a byte offset inside a multibyte char must not
        // panic the internal `src[..b]` slice; it rounds down to the boundary.
        let src = "x = é;\n"; // 'é' occupies bytes 4..6
        let p = super::point_at(src, 5); // 5 is *inside* 'é'
        assert_eq!(p.row, 0);
        assert_eq!(p.column, 4); // rounded down to the char boundary at byte 4
        // Boundary and past-EOF offsets still behave.
        assert_eq!(super::point_at(src, 6).column, 6);
        let _ = super::point_at(src, 999);
    }

    #[test]
    fn reparse_with_mid_codepoint_edit_does_not_panic() {
        // The only caller of `point_at` is `reparse`; an edit whose offsets land
        // inside a multibyte char must not crash it (#36).
        let old = "x = é;\n";
        let new = "x = à;\n";
        let edit = Edit {
            start_byte: 5, // inside 'é'
            old_end_byte: 5,
            new_end_byte: 5,
        };
        let inc = parse(old).reparse(&edit, new);
        assert_eq!(inc.source(), new);
    }

    #[test]
    fn max_depth_is_iterative_and_safe_on_deep_input() {
        // #35: a pathologically nested expression must compute its depth without
        // recursing (no stack overflow) and report well above the safe bound, so
        // a recursive consumer can bail instead of aborting.
        let depth = 50_000;
        let src = format!("x = {}1{};\n", "(".repeat(depth), ")".repeat(depth));
        let cst = parse(&src);
        let d = cst.root().max_depth();
        assert!(
            d > super::MAX_RECURSION_DEPTH,
            "expected depth > {} on {depth} nested parens, got {d}",
            super::MAX_RECURSION_DEPTH
        );
        // The iterative descendants() walk must also survive the same input.
        assert!(cst.root().descendants().count() > depth);
    }
}
