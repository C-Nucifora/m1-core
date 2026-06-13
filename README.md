# m1-core

Shared foundation for the MoTeC M1 (`.m1scr`) tooling: CST helpers, diagnostics,
and (later) a `.m1prj`/`.m1cfg` symbol model. Downstream tools (`m1-fmt`,
`m1-lint`, `m1-typecheck`, `m1-lsp`) depend on this crate and never import
tree-sitter directly.

## Layering: type inference lives in `m1-typecheck` (#8)

`m1-core` deliberately exposes only *structural* CST access — `kind`, `children`,
`field`, `byte_range`, and friends — and **no type-query API on `Node`**. Asking
"what type does this expression resolve to?" requires the project symbol model
(`.m1prj`/`.m1cfg`), which lives in the layer **above** this crate, in
`m1-typecheck`. Putting an `infer_type(node, …)` here would invert that
dependency. So type information has a single canonical home: downstream tools get
it from `m1-typecheck` — `resolve(path, scope)` and `typer::type_of(node, scope)`
— rather than from `m1-core`. (Resolution of #8, option (a).)

## Workspace layout

The M1 toolchain lives in **six separate repositories** coupled through Cargo
**path** dependencies. They are not published to crates.io, so this crate does
**not** build from a standalone clone — check out the whole set as siblings under
one parent directory:

```
<parent>/
├── tree-sitter-m1/   # grammar (root)
├── m1-core/          # this crate
├── m1-lint/          # depends on ../m1-core
├── m1-fmt/           # depends on ../m1-core
├── m1-typecheck/     # depends on ../m1-core
└── m1-lsp/           # depends on the four above
```

**`m1-core` depends on `../tree-sitter-m1`** (`tree-sitter-m1 = { path =
"../tree-sitter-m1" }`) and generates `src/kind.rs` / `src/field.rs` from that
crate's `node-types.json` — so a clean build, and the `kind_rs_is_fresh` /
`field_rs_is_fresh` tests, require the matching `tree-sitter-m1` checked out
alongside it. It is in turn depended on by `m1-lint`, `m1-fmt`, `m1-typecheck`,
and `m1-lsp`.

Because the repos are independent on GitHub, this coupling is **not visible
there**: each repo's CI and PRs see only itself. Build/merge ordering across the
stack is a manual, local-workspace concern.

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

CST traversal helpers used by downstream tools:

```rust
use m1_core::{Field, Kind, MAX_RECURSION_DEPTH};
use m1_core::{is_binary_op, is_unary_op, is_compound_assign};

// Guard against adversarially deep trees before recursing.
if cst.root().max_depth() > MAX_RECURSION_DEPTH { /* bail */ }

// Named-field access (returns Option<Node>).
if let Some(cond) = node.child_by_field(Field::Condition) { /* ... */ }

// Kind predicates — avoid a long match arm per operator.
if is_binary_op(node.kind()) { /* ... */ }
if is_unary_op(node.kind())  { /* ... */ }
if is_compound_assign(node.kind()) { /* ... */ }

// Lazy depth-first descendant iterator (no Vec allocation per level).
for desc in node.descendants() { /* ... */ }

// Byte-offset → LSP-style line/column conversion.
let pos = m1_core::byte_to_position(src, byte_offset);
```

## Annotations (`// @m1:<kind>(args)`)

Comment-embedded attributes — the M1 analogue of Rust attributes / `// eslint-disable` —
parsed once here and consumed by every downstream tool. They ride inside ordinary
`//` (and `/* */`) comments, so they are valid M1 and need no grammar change.

```rust
// // @m1:allow(L010, T030)   ← suppress L010/T030 on the following statement
// Front Torque = 1; // @m1:safety-critical   ← trailing form attaches to this statement
let reg = m1_core::Registry::seed();           // or build your own with only the kinds you consume
let anns = m1_core::annotations(&cst, &reg);
for a in anns.all() { /* a.kind, a.args, a.target_byte_range */ }
let warnings = anns.diagnostics();             // unknown-kind warnings (Code::Annotation)
let suppressed = anns.is_allowed("L010", byte_offset); // honour @allow when filtering diagnostics
```

An annotation attaches to a **construct**: a comment trailing a statement on the
same line attaches to that statement; otherwise it is *leading* and attaches to
the next statement (so annotations stack on consecutive lines above their target).
A tool registers the kinds it owns; m1-core emits a `Severity::Warning`
(`Code::Annotation`) for any `@m1:` kind not in the registry — an unknown attribute.

## Codegen

`src/kind.rs` and `src/field.rs` are both generated from `tree-sitter-m1`'s
`node-types.json`. After a grammar change, regenerate both with one command:

    cargo run -p xtask -- gen-kinds

Two freshness tests (`xtask`'s `kind_rs_is_fresh` and `field_rs_is_fresh`) fail
if either committed file is stale — a stale `field.rs` silently drops named-field
accessors, so both guards must stay green.

## Test

    cargo test

## Known limitations (v1)

These are deliberate v1 scoping/behavior choices, tracked for later increments:

- **No symbol model yet.** `.m1prj`/`.m1cfg` loading, name resolution, and types
  are deferred until `m1-typecheck` drives their shape.
- **Generic ERROR message.** ERROR nodes report `"syntax error"` without the
  offending token text.
- **Iterative tree walk.** `syntax_diagnostics()` traverses with an explicit
  work-stack, so even pathologically deep input cannot overflow the call stack (#28).
- **Byte-column positions.** `Position::column` is a byte offset; UTF-16/LSP
  position conversion is the responsibility of `m1-lsp`.

## License

Licensed under the GNU General Public License v3.0 or later (GPL-3.0-or-later) — see [LICENSE](LICENSE).

Copyright (C) 2026 The M1 Tools authors.

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
