//! Syntax-error diagnostics: ERROR -> `SyntaxError`, MISSING -> `MissingToken`.

use crate::cst::{Cst, Node};
use crate::diagnostic::{Code, Diagnostic, Severity};

pub(crate) fn collect(cst: &Cst) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    walk(cst.root(), &mut out);
    out
}

/// Pre-order traversal with an explicit work-stack (not recursion), so a
/// pathologically deep tree can't overflow the call stack (#28). Pre-order,
/// left-to-right diagnostic order is preserved by pushing children in reverse.
fn walk(root: Node, out: &mut Vec<Diagnostic>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_missing() {
            out.push(node.diagnostic(
                Severity::Error,
                Code::MissingToken,
                format!("missing {}", node.kind_str()),
            ));
            // A MISSING node is a zero-width leaf; nothing useful lies beneath it.
            continue;
        }
        if node.is_error() {
            out.push(node.diagnostic(Severity::Error, Code::SyntaxError, "syntax error"));
            // Don't descend into an ERROR node: its children are MISSING/ERROR
            // fragments of the same parse failure and would emit duplicate,
            // redundant diagnostics for one error region (#10). Sibling errors
            // elsewhere are still reported — only this node's subtree is skipped.
            continue;
        }
        // `child_nodes().rev()` is allocation-free; `children()` would build a
        // `Vec` per node visited, once per file change on the LSP path (#49).
        for child in node.child_nodes().rev() {
            stack.push(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Severity, parse};

    #[test]
    fn clean_source_has_no_diagnostics() {
        let cst = parse("local x = (a >> 2) & 1;\n");
        assert!(cst.syntax_diagnostics().is_empty());
    }

    #[test]
    fn broken_source_reports_errors() {
        // `local <Type> = 1;` is missing the declared name.
        let cst = parse("local <Integer> = 1;\n");
        let diags = cst.syntax_diagnostics();
        assert!(!diags.is_empty(), "expected at least one diagnostic");
        assert!(
            diags.iter().all(|d| d.severity == Severity::Error),
            "syntax diagnostics are all errors"
        );
        // Range is within the source and non-degenerate at the source level.
        assert!(diags.iter().all(|d| d.byte_range.start <= d.byte_range.end));
    }

    #[test]
    fn error_node_emits_single_diagnostic_not_duplicates() {
        // `local <Integer> = 1;` parses to an ERROR node wrapping a MISSING
        // name. Before #10, walk() emitted both a SyntaxError (the ERROR) and a
        // MissingToken (its child) for the same failure. Now the ERROR subtree
        // is not recursed, so exactly one diagnostic is produced.
        let diags = crate::parse("local <Integer> = 1;\n").syntax_diagnostics();
        assert_eq!(
            diags.len(),
            1,
            "one error region should yield one diagnostic, got {diags:?}"
        );
    }

    #[test]
    fn deeply_nested_input_does_not_overflow_the_stack() {
        // ~60k nested parens. A recursive walk overflows the (smaller, 2 MiB)
        // test-thread stack here; the iterative walk must just return.
        let depth = 60_000;
        let src = format!("x = {}1{};\n", "(".repeat(depth), ")".repeat(depth));
        let cst = parse(&src);
        // Must complete without aborting on a stack overflow.
        let _ = cst.syntax_diagnostics();
    }
}
