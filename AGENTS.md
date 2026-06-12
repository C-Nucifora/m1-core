# AGENTS.md — m1-core

Guidance for coding agents working in this repository.

## What this is

The CST layer of the M1 toolchain. Wraps `tree-sitter-m1` behind a typed API
(`Cst`/`Node`, `Kind`/`Field` enums), emits syntax diagnostics, parses
`@m1:` comment annotations, and exposes operator predicates. Consumed by
`m1-fmt`, `m1-lint`, `m1-typecheck`, and `m1-lsp` — none of which import
tree-sitter directly.

## Hard rules

- **Structural API only.** No type queries on `Node`; type inference lives in
  `m1-typecheck`. Don't add `infer_type`-shaped APIs here.
- **Versioned git-tag deps only.** `tree-sitter-m1` is pinned by tag in
  `Cargo.toml`. Never use `branch`, `path`, or `[patch]` — the repo must build
  exactly like a public clone.
- **Iterative traversal.** Public tree walks use explicit work-stacks
  (`MAX_RECURSION_DEPTH` guards the rest); don't introduce unbounded recursion
  over user input.

## Codegen

`src/kind.rs` and `src/field.rs` are generated from the grammar's
`node-types.json`:

```sh
cargo run -p xtask -- gen-kinds
```

Run this after every `tree-sitter-m1` tag bump. The `kind_rs_is_fresh` /
`field_rs_is_fresh` tests fail CI if the committed files are stale. A stale
generated file is also the classic cause of confusing downstream behaviour
(operators tokenising as ERROR) — check freshness before debugging deeper.

## Build / test gate

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

CI also runs Docs (rustdoc `-D warnings`), Security Audit, and an MSRV job
pinned to Rust 1.88. The `dtolnay/rust-toolchain@1.88` pin in
`.github/workflows/ci.yml` must stay in sync with `rust-version` in
`Cargo.toml`; Dependabot is configured to ignore that action.

## Releases and the cascade

- Release = bump `version` in `Cargo.toml` + `Cargo.lock` on `main`;
  `release.yml` tags it. Library repo: the tag is the deliverable.
- Every consumer in one lockfile must pin the **same** m1-core tag or the
  build fails with E0308 (two distinct `Node` types). After releasing, open
  the consumer bump PRs (m1-fmt, m1-lint, m1-typecheck, m1-lsp) immediately —
  Dependabot daily is the backstop, not the propagation path.
- A grammar change flows: tree-sitter-m1 release → bump tag here → gen-kinds →
  m1-core release → consumer bumps.

## Conventions

- Conventional commit messages (`feat:`, `fix:`, `chore:`, `docs:`).
- No AI attribution or `Co-Authored-By` trailers in commits or PRs.
- The M1 Development Manual (in the private corpus repo) is the language
  source of truth — when grammar/semantics questions arise, the manual wins
  over current tool behaviour.
