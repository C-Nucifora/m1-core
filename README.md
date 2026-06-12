# m1-core

Shared foundation for the [MoTeC M1](https://www.motec.com.au/) (`.m1scr`)
tooling: CST access, syntax diagnostics, and comment-embedded annotations.
Downstream tools (`m1-fmt`, `m1-lint`, `m1-typecheck`, `m1-lsp`) depend on this
crate and never import tree-sitter directly.

## What it provides

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

- **CST access** — `parse`, `Cst`, `Node` (`kind`, `children`, `field`,
  `byte_range`, …), typed `Kind`/`Field` enums generated from the grammar, and
  iterator helpers (`Children`, `Descendants`).
- **Incremental reparse** — `Edit` + `Cst::reparse` for editor-style updates
  without reparsing from scratch.
- **Diagnostics** — `syntax_diagnostics()` with `Diagnostic`, `Severity`,
  `Code`, `Range`, and `byte_to_position` for offset → line/column conversion.
  Traversal is iterative (explicit work-stack), so pathologically deep input
  cannot overflow the call stack.
- **Operator predicates** — `is_binary_op`, `is_unary_op`,
  `is_compound_assign`, shared by the formatter and linter.
- **Annotations** — see below.

`Position::column` is a byte offset; UTF-16/LSP position conversion is the
responsibility of `m1-lsp`.

## Layering: type inference lives in `m1-typecheck`

`m1-core` deliberately exposes only *structural* CST access and **no type-query
API on `Node`**. Asking "what type does this expression resolve to?" requires
the project symbol model (`.m1prj`/`.m1cfg`), which lives in the layer above
this crate, in `m1-typecheck` (`resolve(path, scope)` and
`typer::type_of(node, scope)`). Putting an `infer_type(node, …)` here would
invert that dependency.

## Annotations (`// @m1:<kind>(args)`)

Comment-embedded attributes — the M1 analogue of Rust attributes /
`// eslint-disable` — parsed once here and consumed by every downstream tool.
They ride inside ordinary `//` (and `/* */`) comments, so they are valid M1 and
need no grammar change.

```rust
// // @m1:allow(L010, T030)   ← suppress L010/T030 on the following statement
// Front Torque = 1; // @m1:safety-critical   ← trailing form attaches to this statement
let reg = m1_core::Registry::seed();           // or build your own with only the kinds you consume
let anns = m1_core::annotations(&cst, &reg);
for a in anns.all() { /* a.kind, a.args, a.target_byte_range */ }
let warnings = anns.diagnostics();             // unknown-kind warnings (Code::Annotation)
let suppressed = anns.is_allowed("L010", byte_offset); // honour @allow when filtering diagnostics
```

An annotation attaches to a **construct**: a comment trailing a statement on
the same line attaches to that statement; otherwise it is *leading* and
attaches to the next statement (so annotations stack on consecutive lines above
their target). A tool registers the kinds it owns; m1-core emits a
`Severity::Warning` (`Code::Annotation`) for any `@m1:` kind not in the
registry — an unknown attribute.

## Usage

Not published to crates.io; consumed via a versioned git tag (the whole
toolchain uses this scheme, and Dependabot keeps consumers current). Pin the
[latest release](https://github.com/C-Nucifora/m1-core/releases):

```toml
[dependencies]
m1-core = { git = "https://github.com/C-Nucifora/m1-core.git", tag = "v0.10.0" }
```

The grammar dependency (`tree-sitter-m1`) is itself a versioned git tag, so the
crate builds from a standalone clone.

## Codegen

`src/kind.rs` and `src/field.rs` are generated from `tree-sitter-m1`'s
`node-types.json`. After a grammar release, bump the `tree-sitter-m1` tag and
regenerate:

```sh
cargo run -p xtask -- gen-kinds
```

Freshness tests (`kind_rs_is_fresh` / `field_rs_is_fresh`) fail if the
committed files are stale.

## Development

The CI gate is `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --all -- --check`, on stable and on the MSRV (Rust 1.88). Releases
are cut by bumping `version` in `Cargo.toml` on `main`; the tag is the
deliverable (source-only — consumers build from the tag).

## License

Licensed under the GNU General Public License v3.0 or later
(GPL-3.0-or-later) — see [LICENSE](LICENSE).

Copyright (C) 2026 The M1 Tools authors.

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
