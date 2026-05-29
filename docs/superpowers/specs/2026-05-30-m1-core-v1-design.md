# m1-core v1 — design

Date: 2026-05-30
Status: approved (pending written-spec review)

## Purpose

`m1-core` is the shared Rust library between `tree-sitter-m1` (the raw parser)
and the downstream tools (`m1-fmt`, `m1-lint`, `m1-typecheck`, `m1-lsp`). Its
eventual scope is CST helpers + a `.m1prj`/`.m1cfg` symbol model + shared
diagnostics.

**v1 builds the syntactic foundation only:** a CST-helper layer that wraps the
tree-sitter tree behind m1-core's own types, plus a shared `Diagnostic` type and
a first producer of diagnostics (syntax errors). This unblocks `m1-fmt` and
`m1-lint`, which are mostly syntactic. The symbol model is deliberately deferred
until `m1-typecheck` gives it a concrete shape to satisfy.

## Decisions (from brainstorming)

- **First increment: CST helpers + diagnostics** (not the symbol model).
- **CST is exposed as wrapped tree-sitter nodes** — m1-core defines its own
  lightweight `Cst`/`Node`/`Kind` types; `tree-sitter` is *not* re-exported and
  downstream tools never import it. Full source fidelity (trivia, exact ranges)
  is retained because we wrap rather than convert.
- **`Kind` is generated, not hand-written** — from `tree-sitter-m1`'s
  `src/node-types.json`, via **committed codegen with a freshness test** (option
  #2). The generated `src/kind.rs` is committed and reviewable; a test fails if
  it drifts from the grammar.

## Architecture

Rust library crate `m1-core`. Dependencies:
- `tree-sitter` 0.25 (behind the `cst` boundary; not re-exported)
- `tree-sitter-m1` (path dependency; provides `LANGUAGE` and node-types JSON)

The library deliberately has **no** `serde_json` dependency (see Codegen).

### Modules / units

Each unit has one purpose, a clear interface, and is testable in isolation.

- **`diagnostic`** — pure data, no dependencies.
  - `Position { line: u32, column: u32 }` — 0-based line; `column` is a **byte**
    offset within the line. (UTF-16/LSP position encoding is deferred to
    `m1-lsp`; documented as a known conversion point.)
  - `Range { start: Position, end: Position }`
  - `Diagnostic { range: Range, byte_range: std::ops::Range<usize>, severity:
    Severity, code: Code, message: String }`
  - `Severity { Error, Warning, Info, Hint }`
  - `Code` — small enum that grows per producer. v1: `SyntaxError`,
    `MissingToken`.

- **`cst`** — the wrapped tree; the only unit that touches `tree-sitter`.
  - `Cst` owns the tree-sitter `Tree` and the source `String`. Constructed by
    `parse`. Methods: `root() -> Node`, `source() -> &str`,
    `syntax_diagnostics() -> Vec<Diagnostic>` (see `syntax`).
  - `Node<'a>` wraps `tree_sitter::Node<'a>` plus a `&'a str` source handle.
    Methods: `kind() -> Kind`, `kind_str() -> &str`, `text() -> &'a str`,
    `range() -> Range`, `byte_range()`, `children()`, `named_children()`,
    `parent() -> Option<Node>`, `is_error()`, `is_missing()`.
  - `Kind` — **generated** enum over the grammar's node types (named nodes such
    as `LocalDeclaration`, `IfStatement`, `WhenStatement`, `IsClause`,
    `ExpandStatement`, `BinaryExpression`, `MemberExpression`, `CallExpression`,
    `Identifier`, `Number`, `String`, …, plus anonymous tokens like operators
    and punctuation, plus an `Other` fallback for forward-compatibility). Lives
    in committed `src/kind.rs`.

- **`parse`** — `parse(src: &str) -> Cst`. Sets the language to
  `tree_sitter_m1::LANGUAGE` and parses. Infallible: grammar-load failure is a
  build invariant (`expect`), and tree-sitter always returns a tree.

- **`syntax`** — `Cst::syntax_diagnostics()`: walks the tree and emits one
  `Diagnostic` per ERROR node (`code = SyntaxError`) and per MISSING node
  (`code = MissingToken`). v1's first real Diagnostic producer; proves the type
  end-to-end against the corpus.

### Data flow

```
&str ──parse()──▶ Cst ──syntax_diagnostics()──▶ Vec<Diagnostic>
                   │
                   └─ root()/children() … ▶ consumers (m1-fmt, m1-lint) walk nodes
```

## Codegen (Kind)

- `tree-sitter-m1` exposes its node-types JSON as a const so the generator reads
  it path-independently:
  `pub const NODE_TYPES_JSON: &str = include_str!("../../src/node-types.json");`
  (mirrors the existing `HIGHLIGHTS_QUERY` etc. consts). Committed in the
  `tree-sitter-m1` repo.
- m1-core becomes a tiny cargo **workspace** with two members: the library
  (`.`) and `xtask` (a bin). `xtask` depends on `serde_json` + `tree-sitter-m1`;
  this keeps `serde_json` out of the library's dependency tree.
- `cargo run -p xtask -- gen-kinds` parses `NODE_TYPES_JSON` and writes the
  committed `src/kind.rs` via a pure function `generate_kind_rs(json) -> String`.
- **Freshness test** (in `xtask`): `assert_eq!(generate_kind_rs(NODE_TYPES_JSON),
  fs::read_to_string("../src/kind.rs"))`. Runs under `cargo test`; fails if the
  committed file drifts from the grammar. No shelling out, no build-script magic.

## Error handling

- `parse` is infallible (see `parse`).
- `syntax_diagnostics` never fails; an empty vec means a clean parse.
- `Node` accessors return `Option` where the underlying tree-sitter call can
  (e.g. `parent`), and never panic on well-formed trees.

## Testing

Corpus-driven, mirroring the grammar's rhythm:
- **Unit tests** on small snippets: `kind()` mapping, `range()`/`byte_range()`,
  `text()` round-trips, `children()`/`named_children()` shape.
- **Corpus regression**: every `.m1scr` under
  `../m1-example/UQR-EV/01.00/Scripts` parses and yields **zero**
  `syntax_diagnostics()` (the Rust counterpart of `tree-sitter-m1`'s
  `check-corpus.sh`).
- **Negative test**: a deliberately-broken snippet yields exactly the expected
  `SyntaxError`/`MissingToken` diagnostics with correct ranges.
- **Freshness test** for `kind.rs` (see Codegen).

## Out of scope for v1 (YAGNI)

- Symbol model (`.m1prj`/`.m1cfg` loading, name resolution, types).
- Any formatting or lint rules (those live in `m1-fmt`/`m1-lint`).
- LSP position encoding (UTF-16) and incremental reparsing.

## Downstream impact

- `m1-fmt` and `m1-lint` can begin against `m1_core::{parse, Cst, Node, Kind,
  Diagnostic}` without importing tree-sitter.
- `tree-sitter-m1` gains one additive const (`NODE_TYPES_JSON`); no grammar
  change.
