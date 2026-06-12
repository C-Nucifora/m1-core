# AGENTS.md — m1-core

Guidance for coding agents working in this repository.

## Purpose

The CST layer of the M1 toolchain. It wraps the `tree-sitter-m1` grammar
behind a typed Rust API, emits syntax diagnostics, and parses `@m1:` comment
annotations. The downstream tools (`m1-fmt`, `m1-lint`, `m1-typecheck`,
`m1-lsp`) all build on this crate and never import tree-sitter directly —
that's the point: one place owns the grammar binding, so a grammar change is
absorbed here once instead of four times.

## Things that are deliberate (don't "fix" them)

- **Structural API only.** This crate answers "what is the shape of the
  code?", never "what type is this expression?". Type questions need the
  project symbol model (`.m1prj`/`.m1cfg`), which lives a layer up in
  `m1-typecheck`. Adding type queries here would invert the dependency.
- **Versioned git-tag deps only.** `tree-sitter-m1` is pinned by tag. Never
  use `branch`, `path`, or `[patch]` — the repo must build exactly like a
  public clone, and every consumer in one lockfile must pin the *same*
  m1-core tag or the build breaks with two distinct `Node` types.
- **Iterative traversal.** Public tree walks avoid unbounded recursion over
  user input; scripts can be pathologically deep.

## Generated code

`src/kind.rs` and `src/field.rs` are generated from the grammar
(`cargo run -p xtask -- gen-kinds`); regenerate after every `tree-sitter-m1`
bump. Freshness tests fail CI if the committed files are stale — and a stale
generated file is the classic cause of confusing downstream behaviour (new
operators tokenising as ERROR), so check freshness before debugging deeper.

## Build / test gate

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI also runs rustdoc with `-D warnings`, a security audit, and an MSRV job.
The MSRV pin in CI (`dtolnay/rust-toolchain@<version>`) must stay in sync with
`rust-version` in `Cargo.toml` — never bump one without the other.

## Releases

A release is a version bump on `main`; `release.yml` tags it, and the tag is
the deliverable. After releasing, open the consumer bump PRs (m1-fmt, m1-lint,
m1-typecheck, m1-lsp) immediately rather than waiting for Dependabot. A
grammar change flows: tree-sitter-m1 release → bump tag here → regenerate →
m1-core release → consumer bumps.

When grammar or semantics questions arise, the M1 Development Manual is the
source of truth — the manual wins over current tool behaviour.
