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
let diagnostics = cst.syntax_diagnostics();
```

- **CST access** — a typed tree (`Cst`/`Node` with `Kind`/`Field` enums
  generated from the grammar), plus incremental reparse for editor-style
  updates.
- **Syntax diagnostics** — error/warning extraction with offset → line/column
  conversion. Traversal is iterative, so pathologically deep input cannot
  overflow the call stack.
- **Annotations** — `@m1:` comment attributes, parsed once here and consumed
  by every downstream tool (see below).
- Small shared helpers the formatter and linter would otherwise duplicate,
  such as operator classification.

Full API documentation is in the rustdoc (`cargo doc --open`).

## Layering: type inference lives in `m1-typecheck`

`m1-core` deliberately exposes only *structural* CST access and **no type-query
API on `Node`**. Asking "what type does this expression resolve to?" requires
the project symbol model (`.m1prj`/`.m1cfg`), which lives in the layer above
this crate, in `m1-typecheck`. Putting type inference here would invert that
dependency.

## Annotations (`// @m1:<kind>(args)`)

Comment-embedded attributes — the M1 analogue of Rust attributes /
`// eslint-disable`. They ride inside ordinary comments, so they are valid M1
and need no grammar change:

```c
// @m1:allow(L010, T030)        suppresses those diagnostics on the next statement
Front Torque = 1; // @m1:safety-critical    trailing form attaches to this statement
```

A comment trailing a statement attaches to that statement; otherwise it
attaches to the next one (so annotations stack on consecutive lines above
their target). Each tool registers the annotation kinds it owns, and m1-core
warns on any `@m1:` kind no tool recognises.

## Usage

Not published to crates.io; consumed via a versioned git tag (the whole
toolchain uses this scheme). Pin the
[latest release](https://github.com/C-Nucifora/m1-core/releases):

```toml
[dependencies]
m1-core = { git = "https://github.com/C-Nucifora/m1-core.git", tag = "v0.14.1" }
```

The grammar dependency (`tree-sitter-m1`) is itself a versioned git tag, so the
crate builds from a standalone clone.

## Development

The CI gate is `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo fmt --all -- --check`, on stable and on the MSRV. Releases are cut by
bumping `version` in `Cargo.toml` on `main`; the tag is the deliverable
(source-only — consumers build from the tag).

`src/kind.rs` and `src/field.rs` are generated from the grammar; after a
`tree-sitter-m1` bump, regenerate with `cargo run -p xtask -- gen-kinds`
(freshness tests fail CI if the committed files are stale).

## License

GPL-3.0-or-later — see [LICENSE](LICENSE).

## Trademark

Independent, community-built open-source tooling for the MoTeC® M1 script
language. Not affiliated with, authorised, or endorsed by MoTeC Pty Ltd.
"MoTeC" and "M1" are trademarks of MoTeC Pty Ltd.
