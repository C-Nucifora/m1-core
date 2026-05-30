//! The wrapped concrete syntax tree. The only module that depends on
//! `tree_sitter`; everything outside sees m1-core's own [`Cst`]/[`Node`].

use crate::diagnostic::{Position, Range};
use crate::kind::Kind;

/// A parsed M1 source file: the tree-sitter tree plus the owned source text.
pub struct Cst {
    tree: tree_sitter::Tree,
    source: String,
}

/// Parse M1 source into a [`Cst`]. Infallible: grammar load is a build
/// invariant and tree-sitter always returns a tree.
pub fn parse(src: &str) -> Cst {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_m1::LANGUAGE.into())
        .expect("load M1 grammar");
    let tree = parser
        .parse(src, None)
        .expect("tree-sitter always returns a tree");
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

    /// The root node (`source_file`).
    pub fn root(&self) -> Node<'_> {
        Node {
            inner: self.tree.root_node(),
            source: &self.source,
        }
    }
}

/// A node in the CST, wrapping a `tree_sitter::Node` plus a borrow of the
/// source so callers can get text and ranges without a separate handle.
#[derive(Clone, Copy)]
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
        &self.source[self.inner.byte_range()]
    }

    /// Byte offsets of this node within the source.
    pub fn byte_range(&self) -> std::ops::Range<usize> {
        self.inner.byte_range()
    }

    /// Line/column range (0-based; column is a byte offset within the line).
    pub fn range(&self) -> Range {
        let s = self.inner.start_position();
        let e = self.inner.end_position();
        Range {
            start: Position {
                line: s.row as u32,
                column: s.column as u32,
            },
            end: Position {
                line: e.row as u32,
                column: e.column as u32,
            },
        }
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

    /// All direct children (named and anonymous).
    pub fn children(&self) -> Vec<Node<'a>> {
        (0..self.inner.child_count())
            .filter_map(|i| self.inner.child(i))
            .map(|inner| Node {
                inner,
                source: self.source,
            })
            .collect()
    }

    /// Direct named children only (skips punctuation/keywords).
    pub fn named_children(&self) -> Vec<Node<'a>> {
        (0..self.inner.named_child_count())
            .filter_map(|i| self.inner.named_child(i))
            .map(|inner| Node {
                inner,
                source: self.source,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::{parse, Kind};

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
    fn multi_word_identifier_is_one_node() {
        // Exercises the external scanner through the m1-core boundary.
        let cst = parse("Vund Klee.Trilby Glonk = 1;\n");
        let assign = cst.root().children().into_iter().next().unwrap();
        let member = assign.named_children().into_iter().next().unwrap();
        assert_eq!(member.kind(), Kind::MemberExpression);
        let obj = member.named_children().into_iter().next().unwrap();
        assert_eq!(obj.text(), "Vund Klee");
    }
}
