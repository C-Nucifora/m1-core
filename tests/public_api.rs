//! Compile-time guard that the public API is usable from outside the crate.
//! `Cst::reparse` takes `&Edit`; without the `Edit` re-export the method was
//! uncallable by consumers (#54). An integration test sees the crate exactly
//! as an external consumer does, so this fails to compile if the export drops.

#[test]
fn reparse_is_callable_with_the_exported_edit_type() {
    let cst = m1_core::parse("local x = 1;\n");
    // Replace `1` with `42`.
    let new_src = "local x = 42;\n";
    let edit = m1_core::Edit {
        start_byte: 10,
        old_end_byte: 11,
        new_end_byte: 12,
    };
    let reparsed = cst.reparse(&edit, new_src);
    assert_eq!(reparsed.source(), new_src);
    // Identical to a from-scratch parse of the new source.
    let fresh = m1_core::parse(new_src);
    let shape = |cst: &m1_core::Cst| {
        cst.root()
            .descendants()
            .map(|n| (n.kind(), n.byte_range()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        shape(&reparsed),
        shape(&fresh),
        "incremental reparse must agree with a full parse"
    );
}

/// The comment/trivia classifier must be reachable both as the free
/// `is_comment(Kind)` predicate (alongside the operator predicates) and as the
/// `Node::is_comment` convenience method, so downstream tools stop hand-rolling
/// `matches!(k, Kind::LineComment | Kind::BlockComment)`.
#[test]
fn is_comment_is_usable_from_outside_the_crate() {
    let cst = m1_core::parse("// note\nlocal x = 1;\n");
    let comment = cst
        .root()
        .descendants()
        .find(|n| n.is_comment())
        .expect("a comment node");
    assert!(m1_core::is_comment(comment.kind()));
    assert!(comment.is_comment());

    let decl = cst
        .root()
        .children()
        .into_iter()
        .find(|n| n.kind() == m1_core::Kind::LocalDeclaration)
        .expect("the local declaration");
    assert!(!m1_core::is_comment(decl.kind()));
    assert!(!decl.is_comment());
}

/// The assignment-operator classifier must be reachable from outside the crate
/// so consumers stop hand-writing `Kind::Assign | is_compound_assign(k)`. It
/// covers the grammar's full `_assignment_operator` family: plain `=` plus the
/// ten compound forms.
#[test]
fn is_assignment_op_is_usable_from_outside_the_crate() {
    let cst = m1_core::parse("Engine.Speed += 1;\n");
    let op = cst
        .root()
        .descendants()
        .find(|n| n.kind() == m1_core::Kind::PlusEq)
        .expect("the compound-assignment operator");
    assert!(m1_core::is_assignment_op(op.kind()));

    // Plain `=` is also part of the family; a comparison `==` is not.
    assert!(m1_core::is_assignment_op(m1_core::Kind::Assign));
    assert!(!m1_core::is_assignment_op(m1_core::Kind::EqEq));
}

/// The `@m1:` namespace marker must be reachable from outside the crate so
/// downstream tools (m1-fmt, m1-lint, ...) reference the single source of truth
/// instead of hard-coding the `"@m1:"` literal — which would silently drift if
/// the marker ever changed. The `annotation` module is private, so the const is
/// only reachable via the re-export in `lib.rs`.
#[test]
fn marker_is_usable_from_outside_the_crate() {
    assert_eq!(m1_core::MARKER, "@m1:");
}

#[test]
fn malformed_empty_arguments_never_become_blanket_suppression() {
    for args in [",", " , , ", ",\t,"] {
        let src = format!("// @m1:allow({args})\nx = 1;\n");
        let cst = m1_core::parse(&src);
        let annotations = m1_core::annotations(&cst, &m1_core::Registry::seed());
        assert_eq!(annotations.diagnostics().len(), 1);
        assert!(annotations.all().is_empty());
        for code in ["L010", "T030"] {
            assert!(!annotations.is_allowed(code, src.find("x = 1").unwrap()));
        }
    }
    for annotation in ["@m1:allow", "@m1:allow()", "@m1:allow( )"] {
        let src = format!("// {annotation}\nx = 1;\n");
        let cst = m1_core::parse(&src);
        let annotations = m1_core::annotations(&cst, &m1_core::Registry::seed());
        assert!(annotations.diagnostics().is_empty());
        assert!(annotations.is_allowed("L010", src.find("x = 1").unwrap()));
    }
}

#[test]
fn adjacent_annotation_suffix_is_not_a_blanket_allow() {
    for annotation in ["@m1:allow()L010", "@m1:allow()(L010)"] {
        let src = format!("// {annotation}\nx = 1;\n");
        let annotations = m1_core::annotations(&m1_core::parse(&src), &m1_core::Registry::seed());
        assert!(annotations.all().is_empty());
        assert_eq!(annotations.diagnostics().len(), 1);
    }
    let src = "// @m1:allow(L010) explanation\nx = 1;\n";
    let annotations = m1_core::annotations(&m1_core::parse(src), &m1_core::Registry::seed());
    assert!(annotations.diagnostics().is_empty());
    assert!(annotations.all()[0].has_positional("L010"));
}

#[test]
fn large_annotation_stacks_preserve_targets_order_and_indexes() {
    for tail in ["x = 1;\n", ""] {
        let src = format!(
            "{}{tail}",
            "// @m1:trace\n// ordinary comment\n".repeat(2000)
        );
        let tree = m1_core::parse(&src);
        let annotations = m1_core::annotations(&tree, &m1_core::Registry::seed());
        assert_eq!(annotations.all().len(), 2000);
        assert!(annotations.diagnostics().is_empty());
        let expected = src.find("x = 1;").map(|start| start..start + 6);
        for (i, annotation) in annotations.all().iter().enumerate() {
            assert_eq!(annotation.target_byte_range, expected);
            if i > 0 {
                assert!(
                    annotation.comment_byte_range.start
                        > annotations.all()[i - 1].comment_byte_range.start
                );
            }
        }
        if let Some(target) = expected {
            assert_eq!(annotations.for_target_start(target.start).len(), 2000);
        }
    }
}

#[test]
fn long_line_annotations_preserve_unicode_positions() {
    let src = format!(
        "/* é🙂 */ x=1; {}\n/* @m1:allow(,) */",
        "/* @m1:trace */ ".repeat(4000)
    );
    let cst = m1_core::parse(&src);
    let anns = m1_core::annotations(&cst, &m1_core::Registry::seed());
    assert_eq!(anns.all().len(), 4000);
    for a in anns.all().iter().step_by(127) {
        assert_eq!(
            a.range.start,
            m1_core::byte_to_position(&src, a.comment_byte_range.start)
        );
        assert_eq!(
            a.range.end,
            m1_core::byte_to_position(&src, a.comment_byte_range.end)
        );
        assert_eq!(a.target_byte_range, Some(13..17));
    }
    assert_eq!(anns.diagnostics().len(), 1);
    let diagnostic = &anns.diagnostics()[0];
    assert_eq!(
        diagnostic.range.start,
        m1_core::byte_to_position(&src, diagnostic.byte_range.start)
    );
    let statements = m1_core::parse(&"x=1; ".repeat(16000));
    assert!(
        m1_core::annotations(&statements, &m1_core::Registry::seed())
            .all()
            .is_empty()
    );
}
