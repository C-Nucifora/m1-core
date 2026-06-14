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
