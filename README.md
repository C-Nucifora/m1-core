# m1-core

Shared foundation for the MoTeC M1 (`.m1scr`) tooling: CST helpers, diagnostics,
and (later) a `.m1prj`/`.m1cfg` symbol model. Downstream tools (`m1-fmt`,
`m1-lint`, `m1-typecheck`, `m1-lsp`) depend on this crate and never import
tree-sitter directly.

## v1 surface

```rust
let cst = m1_core::parse(src);
for node in cst.root().children() {
    match node.kind() {
        m1_core::Kind::IfStatement => { /* ... */ }
        _ => {}
    }
}
let diagnostics = cst.syntax_diagnostics(); // Vec<m1_core::Diagnostic>
```

## Codegen

`src/kind.rs` is generated from `tree-sitter-m1`'s `node-types.json`. After a
grammar change, regenerate it:

    cargo run -p xtask -- gen-kinds

A test (`xtask`'s `kind_rs_is_fresh`) fails if the committed file is stale.

## Test

    cargo test

## Known limitations (v1)

These are deliberate v1 scoping/behavior choices, tracked for later increments:

- **No symbol model yet.** `.m1prj`/`.m1cfg` loading, name resolution, and types
  are deferred until `m1-typecheck` drives their shape.
- **Diagnostic double-reporting.** `syntax_diagnostics()` emits a `MissingToken`
  for a MISSING node *in addition to* the `SyntaxError` for its enclosing ERROR
  node, so a single mistake can surface as two overlapping diagnostics. Acceptable
  for v1; revisit before wiring diagnostics into `m1-lsp`.
- **Generic ERROR message.** ERROR nodes report `"syntax error"` without the
  offending token text.
- **Recursive tree walk.** `syntax_diagnostics()` recurses; pathologically deep
  malformed input could overflow the stack. Not a concern for real M1 scripts.
- **Byte-column positions.** `Position::column` is a byte offset; UTF-16/LSP
  position conversion is the responsibility of `m1-lsp`.
