//! Shared diagnostic types emitted by every M1 tool.

/// A 0-based source position. `column` counts **characters** (Unicode scalar
/// values) within `line` — the one column unit everywhere in m1-core, whether
/// the position came from [`byte_to_position`] or `Node::range()`. Tools that
/// need a different encoding (m1-lsp's UTF-16/UTF-8 LSP positions) convert
/// from byte offsets instead, via `Diagnostic::byte_range`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// Clamp `byte` to `s.len()`, then round it **down** to the nearest UTF-8 char
/// boundary (returning `byte` itself when it already lands on one).
///
/// Slicing `s` at a byte offset that falls inside a multi-byte character panics
/// (`"byte index N is not a char boundary"`), and offsets sourced from byte
/// edits or external positions can land anywhere. Run them through this first
/// so the following `s[..b]` slice is always valid. This is the one home for
/// that clamping contract — every position helper shares it instead of carrying
/// its own copy of the loop. We deliberately do **not** use std's
/// `str::floor_char_boundary`: it stabilised after this crate's MSRV (1.88), so
/// calling it would break the MSRV gate.
pub(crate) fn floor_char_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Convert a byte offset into `source` to a 0-based [`Position`].
///
/// Lines are counted by `'\n'`; the column is the number of **characters**
/// (codepoints) between the start of the current line and the offset, so it is
/// correct for multi-byte input. An offset that lands inside a multi-byte
/// character is rounded down to the enclosing char boundary; an offset past the
/// end of `source` is clamped to its end. The canonical helper for turning a
/// lint/typecheck byte offset into a line/column, replacing per-rule copies.
pub fn byte_to_position(source: &str, byte_offset: usize) -> Position {
    let offset = floor_char_boundary(source, byte_offset);
    let line = source[..offset].bytes().filter(|&b| b == b'\n').count();
    let line_start = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let column = source[line_start..offset].chars().count();
    Position {
        line: line as u32,
        column: column as u32,
    }
}

/// A half-open range between two [`Position`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Stable diagnostic code. Grows as new producers are added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    SyntaxError,
    MissingToken,
    /// Type-checker findings (m1-typecheck).
    TypeError,
    /// Lint-rule findings (m1-lint).
    LintError,
    /// Semantic findings that aren't strictly type errors (resolution, flow).
    SemanticError,
    /// `@m1:` annotation findings (m1-core): unknown kind, malformed args.
    Annotation,
}

/// A single diagnostic with both line/column and byte ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub range: Range,
    pub byte_range: std::ops::Range<usize>,
    pub severity: Severity,
    pub code: Code,
    pub message: String,
}

impl Diagnostic {
    /// Build a diagnostic from explicit ranges.
    pub fn new(
        severity: Severity,
        code: Code,
        range: Range,
        byte_range: std::ops::Range<usize>,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            range,
            byte_range,
            severity,
            code,
            message: message.into(),
        }
    }

    /// Build an [`Severity::Error`] diagnostic.
    pub fn error(
        code: Code,
        range: Range,
        byte_range: std::ops::Range<usize>,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(Severity::Error, code, range, byte_range, message)
    }

    /// Build a [`Severity::Warning`] diagnostic.
    pub fn warning(
        code: Code,
        range: Range,
        byte_range: std::ops::Range<usize>,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(Severity::Warning, code, range, byte_range, message)
    }

    /// Build a diagnostic spanning a CST node, deriving both the line/column
    /// `range` and the `byte_range` from the node — so consumers don't repeat
    /// that extraction at every call site.
    pub fn at_node(
        node: crate::Node<'_>,
        severity: Severity,
        code: Code,
        message: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic::new(severity, code, node.range(), node.byte_range(), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn at_node_derives_ranges_from_node() {
        let cst = crate::parse("local x = 1;\n");
        let root = cst.root();
        let d = Diagnostic::at_node(root, Severity::Warning, Code::TypeError, "hi");
        assert_eq!(d.code, Code::TypeError);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.byte_range, root.byte_range());
        assert_eq!(d.range, root.range());
    }

    #[test]
    fn byte_to_position_basic_and_multiline() {
        let src = "ab\ncde\nf";
        assert_eq!(byte_to_position(src, 0), Position { line: 0, column: 0 });
        assert_eq!(byte_to_position(src, 1), Position { line: 0, column: 1 });
        // Just after the first '\n' is the start of line 1.
        assert_eq!(byte_to_position(src, 3), Position { line: 1, column: 0 });
        assert_eq!(byte_to_position(src, 5), Position { line: 1, column: 2 });
        assert_eq!(byte_to_position(src, 7), Position { line: 2, column: 0 });
    }

    #[test]
    fn byte_to_position_counts_chars_not_bytes() {
        // 'é' is two bytes; "x = é" — the char after 'é' is at column 5,
        // even though it sits at byte offset 6.
        let src = "x = é!";
        let bang = src.find('!').unwrap();
        assert_eq!(bang, 6); // byte offset
        assert_eq!(
            byte_to_position(src, bang),
            Position { line: 0, column: 5 } // char column
        );
    }

    #[test]
    fn byte_to_position_mid_codepoint_rounds_down() {
        let src = "é"; // bytes 0..2
        // Offset 1 is inside 'é' -> rounds down to the boundary at 0.
        assert_eq!(byte_to_position(src, 1), Position { line: 0, column: 0 });
    }

    #[test]
    fn byte_to_position_past_end_is_clamped() {
        let src = "abc\nde";
        assert_eq!(byte_to_position(src, 999), Position { line: 1, column: 2 });
        // Empty source never panics.
        assert_eq!(byte_to_position("", 0), Position { line: 0, column: 0 });
        assert_eq!(byte_to_position("", 5), Position { line: 0, column: 0 });
    }

    #[test]
    fn floor_char_boundary_rounds_down_clamps_and_guards() {
        // 'é' is two bytes (0..2); a 1-byte 'x' follows at byte 2.
        let src = "éx";
        assert_eq!(src.len(), 3);
        // Already on a boundary -> unchanged.
        assert_eq!(floor_char_boundary(src, 0), 0);
        assert_eq!(floor_char_boundary(src, 2), 2);
        assert_eq!(floor_char_boundary(src, 3), 3);
        // Mid-codepoint offset 1 rounds down to the enclosing boundary at 0.
        assert_eq!(floor_char_boundary(src, 1), 0);
        // Past the end is clamped to the length.
        assert_eq!(floor_char_boundary(src, 999), 3);
        // The `> 0` guard: empty input never underflows.
        assert_eq!(floor_char_boundary("", 0), 0);
        assert_eq!(floor_char_boundary("", 5), 0);
    }

    #[test]
    fn diagnostic_is_constructible() {
        let d = Diagnostic {
            range: Range {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 3 },
            },
            byte_range: 0..3,
            severity: Severity::Error,
            code: Code::SyntaxError,
            message: "syntax error".to_string(),
        };
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.range.end.column, 3);
    }

    #[test]
    fn constructors_match_struct_literal() {
        let range = Range {
            start: Position { line: 0, column: 0 },
            end: Position { line: 0, column: 3 },
        };
        let expected = Diagnostic {
            range,
            byte_range: 0..3,
            severity: Severity::Error,
            code: Code::SyntaxError,
            message: "boom".to_string(),
        };
        assert_eq!(
            Diagnostic::new(Severity::Error, Code::SyntaxError, range, 0..3, "boom"),
            expected
        );
        assert_eq!(
            Diagnostic::error(Code::SyntaxError, range, 0..3, "boom"),
            expected
        );
        assert_eq!(
            Diagnostic::warning(Code::SyntaxError, range, 0..3, "boom").severity,
            Severity::Warning
        );
    }
}
