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
