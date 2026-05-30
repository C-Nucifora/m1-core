# m1-core v2 — Design Specification

**Date:** 2026-05-31
**Status:** Approved for implementation
**Scope:** v2 — field accessors, allocation-free traversal iterators, sibling
navigation, node-by-byte-offset lookup, and diagnostic construction helpers
**Spec (v1):** `docs/superpowers/specs/2026-05-30-m1-core-v1-design.md`

> **Note:** Example identifiers and snippets in this document are synthetic
> placeholders, not drawn from any real project. The corpus path used by the
> integration tests is resolved via the `M1_CORPUS_PATH` env var (falling back
> to the sibling `m1-example` example project); corpus tests are skipped when the
> directory is absent.

---

## 1. Purpose

v1 shipped the syntactic foundation of `m1-core`: `parse(&str) -> Cst`, the
wrapped `Cst`/`Node` types, the generated `Kind` enum (with the `xtask`
`gen-kinds` codegen and the `kind_rs_is_fresh` freshness test), and the shared
`Diagnostic`/`Range`/`Position`/`Severity`/`Code` types plus a first diagnostic
producer (`syntax_diagnostics`). Four downstream tools now consume it:
`m1-fmt`, `m1-lint`, `m1-typecheck`, and `m1-lsp`.

Reviewing how those consumers actually use the v1 surface surfaces three
recurring pains, all of which v2 removes **without** touching the symbol model
and **without** breaking any existing consumer:

1. **Children are accessed by guesswork, not by grammar role.** The M1 grammar
   names its children — `binary_expression` has `left`/`operator`/`right`,
   `if_statement` has `condition`/`consequence`, `local_declaration` has
   `name`/`value`, and so on. v1 exposes none of this, so consumers reach for
   `node.named_children().into_iter().find(|c| c.kind() == Kind::Identifier)`
   (see `m1-typecheck/src/rules/mod.rs`, `t002_float_eq.rs`, `t010_local_prefix.rs`)
   — brittle, positional, and wrong when a node has two identifier children.
   v2 adds **field accessors** so consumers ask for `node.child_by_field(Field::Name)`.

2. **Every consumer hand-rolls a recursive, `Vec`-allocating tree walk.**
   `m1-lint`'s `Runner::walk`, `m1-typecheck`'s `walk`/`collect_locals`, and
   `m1-fmt`'s printer all recurse over `node.children()` — which allocates a
   fresh `Vec<Node>` at every node visited. v2 adds **borrowing iterators**
   (`child_nodes()`, `named_child_nodes()`) and a pre-order **`descendants()`**
   iterator so a whole-tree walk allocates nothing per node.

3. **No way to find the node under a cursor.** `m1-lsp` parses to a `Cst` and
   emits diagnostics, but to grow hover / go-to-definition it needs "what node
   is at byte offset N?". tree-sitter provides this; v1 does not surface it.
   v2 adds **`Cst::node_at_offset` / `named_node_at_offset`**.

A fourth, smaller pain: **building a `Diagnostic` is verbose.** Every producer
(`m1-lint`'s rules, `m1-typecheck`'s `diagnostics::make`, the v1 `syntax`
module) constructs `Diagnostic { range, byte_range, severity, code, message }`
and derives `range`/`byte_range` from a node by hand. v2 adds small **builder
constructors** and a **`Diagnostic::at_node`** helper.

`m1-core` continues to depend only on `tree-sitter` + `tree-sitter-m1`. v2 adds
**no new dependencies**, does **not** re-export `tree-sitter`, and keeps
`serde_json` confined to `xtask`. The `kind.rs` codegen + freshness mechanism is
preserved exactly, and is extended to also generate a companion `Field` enum
from the same `node-types.json`.

---

## 2. What v1 Already Provides (build on this, do not re-invent)

The relevant existing public surface, with actual signatures:

- `m1_core::parse(src: &str) -> Cst`.
- `m1_core::Cst` — `#[derive(Debug)]`; `source(&self) -> &str`,
  `root(&self) -> Node<'_>`, `syntax_diagnostics(&self) -> Vec<Diagnostic>`.
  Internally holds `tree: tree_sitter::Tree` and `source: String` (both private).
- `m1_core::Node<'a>` — `#[derive(Debug, Clone, Copy)]`; wraps a private
  `inner: tree_sitter::Node<'a>` and `source: &'a str`. Methods: `kind() -> Kind`,
  `kind_str() -> &'a str`, `text() -> &'a str`, `byte_range() -> Range<usize>`,
  `range() -> m1_core::Range`, `is_error() -> bool`, `is_missing() -> bool`,
  `parent() -> Option<Node<'a>>`, `children() -> Vec<Node<'a>>`,
  `named_children() -> Vec<Node<'a>>`.
- `m1_core::Kind` — generated `#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]`
  enum in `src/kind.rs` with one variant per grammar node type (named nodes such
  as `LocalDeclaration`, `IfStatement`, `BinaryExpression`, `MemberExpression`,
  operator/punctuation tokens such as `EqEq`, `AmpAmp`, `Plus`, plus `Other`),
  and `Kind::from_kind_str(s: &str) -> Kind`.
- `m1_core::{Position, Range, Severity, Code, Diagnostic}` — pure data:
  - `Position { line: u32, column: u32 }` (0-based; `column` is a byte offset).
  - `Range { start: Position, end: Position }`.
  - `Severity { Error, Warning, Info, Hint }`.
  - `Code { SyntaxError, MissingToken }`.
  - `Diagnostic { range: Range, byte_range: std::ops::Range<usize>, severity:
    Severity, code: Code, message: String }`.
- The `xtask` workspace member: `generate_kind_rs(node_types_json: &str) -> String`,
  the `gen-kinds` bin, and the `kind_rs_is_fresh` test. `tree_sitter_m1::NODE_TYPES_JSON`
  is the codegen input. `serde_json` lives only in `xtask`.

v2 adds methods, two new modules' worth of types, and one new generated enum,
but does not change or remove any of the above. `children()`/`named_children()`
(returning `Vec`) are retained for backward compatibility; the new iterators are
additive.

---

## 3. Key Decisions

### 3.1 Field accessors — a generated `Field` enum, mirroring `Kind`

The M1 grammar's `node-types.json` declares **20 distinct field names** across
12 named node types:

| Node | Fields |
|------|--------|
| `assignment_statement` | `operator`, `target`, `value` |
| `binary_expression` | `left`, `operator`, `right` |
| `call_expression` | `arguments`, `function` |
| `expand_statement` | `end`, `start`, `variable` |
| `if_statement` | `condition`, `consequence` |
| `is_clause` | `body`, `state` |
| `local_declaration` | `name`, `value` |
| `member_expression` | `object`, `property` |
| `ternary_expression` | `alternative`, `condition`, `consequence` |
| `type_annotation` | `type` |
| `unary_expression` | `operator` |
| `when_statement` | `subject` |

The full set of field names is: `alternative`, `arguments`, `body`, `condition`,
`consequence`, `end`, `function`, `left`, `name`, `object`, `operator`,
`property`, `right`, `start`, `state`, `subject`, `target`, `type`, `value`,
`variable`.

**Decision:** generate a `Field` enum the same way `Kind` is generated — from
`node-types.json`, committed as `src/field.rs`, guarded by the same freshness
mechanism. This keeps the type honest against the grammar (a new field added to
the grammar fails the freshness test until regenerated) and avoids a hand-written
list drifting out of date.

`Field` carries the grammar field-name string so it can feed tree-sitter's
`Node::child_by_field_name`:

```rust
// src/field.rs (generated)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Alternative, Arguments, Body, Condition, Consequence, End, Function,
    Left, Name, Object, Operator, Property, Right, Start, State, Subject,
    Target, Type, Value, Variable,
}

impl Field {
    pub fn as_str(self) -> &'static str { /* match -> "alternative", ... */ }
}
```

`Node` gains:

```rust
pub fn child_by_field(&self, field: Field) -> Option<Node<'a>>;
```

So `m1-typecheck`'s `collect_locals` becomes
`decl.child_by_field(Field::Name)` instead of a positional
`named_children().find(|c| c.kind() == Kind::Identifier)`, and `t002_float_eq`
can ask `bin.child_by_field(Field::Operator)` directly.

**Note on `type`:** the grammar field literally named `type` becomes the
`Field::Type` variant (Rust keyword avoided by PascalCase; the variant carries
`"type"` as its `as_str`). This is handled in the generator's PascalCase path —
`type` is a valid identifier segment, so no special-casing is needed beyond the
existing `pascal_case` already used for `Kind`.

### 3.2 Traversal iterators — borrow the parent, allocate nothing per node

v1's `children()`/`named_children()` allocate a `Vec` every call; a recursive
whole-tree walk therefore allocates O(nodes) vectors. v2 adds lazy iterators
that hold a `tree_sitter::TreeCursor` and yield wrapped `Node`s:

```rust
pub fn child_nodes(&self) -> Children<'a>;        // all direct children
pub fn named_child_nodes(&self) -> Children<'a>;  // named direct children only
pub fn descendants(&self) -> Descendants<'a>;     // self + all descendants, pre-order
```

`Children<'a>` and `Descendants<'a>` are public opaque iterator structs
(`impl Iterator<Item = Node<'a>>`) defined in the `cst` module. `descendants()`
yields the node itself first, then every descendant in pre-order (matching the
order the consumers' hand-rolled `walk` functions already use), so a rule walk
becomes:

```rust
for node in cst.root().descendants() {
    rule.check_node(&node, ...);
}
```

The `Vec`-returning `children()`/`named_children()` are kept (some call sites
index into them or take `.len()`), so this is purely additive.

Two sibling helpers round out navigation (needed by formatter spacing logic and
future fixers):

```rust
pub fn next_sibling(&self) -> Option<Node<'a>>;
pub fn prev_sibling(&self) -> Option<Node<'a>>;
```

### 3.3 Node-by-byte-offset lookup — for the LSP

tree-sitter exposes `descendant_for_byte_range` and
`named_descendant_for_byte_range`. v2 surfaces both on `Cst`, taking a single
byte offset (the common "cursor position" case) and querying the zero-width
range `[offset, offset)`:

```rust
impl Cst {
    /// The smallest node whose byte span contains `offset` (any node).
    pub fn node_at_offset(&self, offset: usize) -> Node<'_>;

    /// The smallest *named* node whose byte span contains `offset`.
    pub fn named_node_at_offset(&self, offset: usize) -> Node<'_>;
}
```

These return a `Node` (never `Option`): tree-sitter's descendant lookup always
yields at least the root for an in-bounds offset, and offsets past EOF clamp to
the last node. This unblocks `m1-lsp` hover / go-to-definition without exposing
tree-sitter and without an incremental-reparse commitment (deferred to v3).

### 3.4 Diagnostic construction helpers

Every diagnostic producer writes the same five-field struct literal and derives
`range`/`byte_range` from a node. v2 adds, on `Diagnostic` (in `diagnostic.rs`,
still dependency-free):

```rust
impl Diagnostic {
    /// Build a diagnostic with an explicit range + byte range.
    pub fn new(severity: Severity, code: Code, range: Range,
               byte_range: std::ops::Range<usize>, message: impl Into<String>) -> Diagnostic;

    /// Convenience: severity-specific constructors.
    pub fn error(code: Code, range: Range, byte_range: std::ops::Range<usize>,
                 message: impl Into<String>) -> Diagnostic;
    pub fn warning(code: Code, range: Range, byte_range: std::ops::Range<usize>,
                   message: impl Into<String>) -> Diagnostic;
}
```

Because `diagnostic.rs` must stay free of any `cst`/`tree-sitter` dependency
(it is the leaf module every other crate's diagnostic type wraps), the
node-spanning helper lives on `Node` instead, in `cst.rs`:

```rust
impl<'a> Node<'a> {
    /// Build a diagnostic spanning exactly this node.
    pub fn diagnostic(&self, severity: Severity, code: Code,
                      message: impl Into<String>) -> Diagnostic;
}
```

So the v1 `syntax` module's two struct literals collapse to
`node.diagnostic(Severity::Error, Code::MissingToken, format!("missing {}", node.kind_str()))`.

### 3.5 Codegen: one generator pass, two files

The `xtask` `gen-kinds` command (renamed conceptually to "generate" but keeping
the `gen-kinds` subcommand name for compatibility with existing docs) now writes
**both** `src/kind.rs` and `src/field.rs` from the same `NODE_TYPES_JSON`. A new
pure function `generate_field_rs(node_types_json: &str) -> String` mirrors
`generate_kind_rs`. The freshness test gains a `field_rs_is_fresh` case
asserting the committed `src/field.rs` matches the generator output.

Field names are always grammar identifiers (snake_case, ASCII), so the `Field`
generator reuses the existing `pascal_case` helper and needs no `SYMBOL_MAP`.

---

## 4. Architecture & Module Layout (after v2)

```
m1-core/
  Cargo.toml            # unchanged (no new deps)
  src/
    lib.rs              # + `mod field; pub use field::Field;` and the new re-exports
    kind.rs             # GENERATED (unchanged shape)
    field.rs            # NEW — GENERATED Field enum
    diagnostic.rs       # + Diagnostic::{new,error,warning}
    cst.rs              # + Field accessor, iterators, sibling nav, node_at_offset,
                        #   Node::diagnostic
    syntax.rs           # refactored to use Node::diagnostic (behaviour unchanged)
  tests/
    corpus.rs           # + a v2 traversal/field invariant test over the corpus
  xtask/
    src/main.rs         # + generate_field_rs, write field.rs, field_rs_is_fresh test
```

No new crate, no new dependency, no change to the dependency graph.

### Public API delta (additive)

| Item | Kind | Module |
|------|------|--------|
| `Field` (enum) + `Field::as_str` | new | `field` (generated) |
| `Node::child_by_field(Field)` | new method | `cst` |
| `Node::child_nodes()` / `named_child_nodes()` | new methods | `cst` |
| `Node::descendants()` | new method | `cst` |
| `Node::next_sibling()` / `prev_sibling()` | new methods | `cst` |
| `Node::diagnostic(Severity, Code, msg)` | new method | `cst` |
| `Children<'a>`, `Descendants<'a>` | new iterator types | `cst` |
| `Cst::node_at_offset(usize)` / `named_node_at_offset(usize)` | new methods | `cst` |
| `Diagnostic::{new, error, warning}` | new ctors | `diagnostic` |

---

## 5. Invariants (must hold; verified by tests)

- **tree-sitter is never re-exported.** `Field`, the iterators, and the offset
  lookups all take/return only m1-core types (`Node`, `Field`, `usize`). No
  `tree_sitter::*` appears in any public signature.
- **`serde_json` stays in `xtask`.** The library crate's `Cargo.toml` gains no
  dependency. `generate_field_rs` lives in `xtask`.
- **Freshness preserved & extended.** `kind_rs_is_fresh` still passes; a new
  `field_rs_is_fresh` guards `src/field.rs`. `cargo run -p xtask -- gen-kinds`
  regenerates both files.
- **Iterators agree with the `Vec` accessors.** For any node,
  `child_nodes().collect::<Vec<_>>()` equals `children()`, and
  `named_child_nodes().collect()` equals `named_children()` (kind + byte_range).
- **`descendants()` order matches the existing hand-rolled walk** (node first,
  then each child's subtree, left to right), so consumers swapping to it get
  identical diagnostic ordering.
- **`node_at_offset` is total** for in-bounds offsets and clamps past EOF;
  never panics on the corpus.

---

## 6. Testing

- **Unit (`field.rs` via `cst.rs` tests):** `child_by_field` returns the right
  child for `binary_expression` (`Left`/`Operator`/`Right`),
  `local_declaration` (`Name`/`Value`), `member_expression`
  (`Object`/`Property`), and returns `None` for an absent field.
- **Unit (iterators):** `child_nodes`/`named_child_nodes` equal the `Vec`
  accessors; `descendants()` of a small tree yields the expected pre-order kind
  sequence and includes the root first.
- **Unit (siblings):** `next_sibling`/`prev_sibling` walk the operator between
  two operands.
- **Unit (offset):** `node_at_offset` inside an identifier returns that
  identifier; an offset past EOF returns a node (no panic);
  `named_node_at_offset` skips anonymous tokens.
- **Unit (diagnostic ctors + `Node::diagnostic`):** constructed values match the
  equivalent struct literal.
- **Freshness:** `kind_rs_is_fresh` and the new `field_rs_is_fresh` both pass.
- **Corpus (`tests/corpus.rs`):** in addition to the v1 zero-syntax-error check,
  a new test walks every corpus file via `descendants()` and asserts it visits
  the same node count as a recursive `children()` walk, and that
  `node_at_offset` never panics for a sampling of offsets — proving the new
  traversal is faithful and total over real input.

---

## 7. Downstream impact (why each consumer benefits)

- **m1-typecheck** replaces positional `named_children().find(...)` in
  `collect_locals`, `t002_float_eq`, `t003_int_float_mix`, `t010_local_prefix`,
  `t011_prefix_mismatch` with `child_by_field(Field::…)`, and can swap its
  `walk`/`collect_locals` recursion for `descendants()`.
- **m1-lint** replaces `Runner::walk`'s allocating recursion with
  `root().descendants()`, and `l005`/`l007` operator handling can use
  `child_by_field(Field::Operator)` / sibling navigation.
- **m1-fmt** can use `child_by_field` in its printer to dispatch on roles
  instead of positional scans, and `next_sibling`/`prev_sibling` for spacing.
- **m1-lsp** gains the missing primitive (`node_at_offset`) it needs for hover /
  go-to-definition, still without importing tree-sitter.

All four keep building unchanged; every v2 addition is additive.

---

## 8. Deferred to v3 (YAGNI)

| Item | Why deferred |
|------|--------------|
| Incremental / edit reparse (`Tree::edit` + reparse with old tree) | No consumer needs it yet; m1-lsp re-parses whole documents and that is fast enough. Surfacing edits cleanly without leaking `tree_sitter::InputEdit` needs its own design. |
| `.m1prj` / `.m1cfg` symbol model + name resolution | Already deferred in v1; `m1-typecheck` owns a local model for now. Belongs in its own spec. |
| UTF-16 / LSP position encoding in `m1-core` | Stays in `m1-lsp`'s `LineIndex`; m1-core remains byte-oriented per the v1 decision. |
| tree-sitter query (`.scm`) API surfaced through m1-core | No consumer needs pattern queries yet; `descendants()` + `Kind`/`Field` cover current traversal needs. |
| Mutable / owned CST or red-green trees | Out of scope; the wrap-and-borrow model is sufficient. |
| Typed wrapper structs per node kind (e.g. `BinaryExpression { left, op, right }`) | `Kind` + `Field` accessors deliver the ergonomics at a fraction of the surface area; revisit only if accessor churn proves painful. |
