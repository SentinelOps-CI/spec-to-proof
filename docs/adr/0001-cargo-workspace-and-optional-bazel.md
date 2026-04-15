# ADR 0001: Cargo workspace and optional Bazel

## Status

Accepted

## Context

The repository ships multiple Rust crates (`proto`, `nlp`, `proof`) and a Next.js UI. Earlier automation assumed a Bazel graph that was not present in the tree, which blocked reproducible local and CI builds.

## Decision

- **Cargo** is the source of truth for Rust: a single workspace at the repository root, shared dependency policy, `rust-toolchain.toml`, and a committed `Cargo.lock`.
- **Bazel** is not required for day-to-day development or default CI. If a hermetic multi-language graph is needed later, it should be introduced as an explicit milestone with real `BUILD` files and a valid workspace, not as a stub.
- **Supply chain**: `cargo-deny` at the root enforces license policy; `cargo audit` can be run in CI or locally against the advisory database.

## Consequences

- Developers run `cargo test --workspace` and `cargo clippy` without installing Bazel.
- CI is driven by GitHub Actions jobs that mirror these commands.
- Additional Rust crates can be onboarded by adding explicit workspace membership rather than relying on implicit repository structure.
