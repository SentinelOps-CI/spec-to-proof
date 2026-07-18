# ADR 0001: Cargo workspace and optional Bazel

## Status

Accepted

## Context

The repository ships multiple Rust crates (`proto`, `nlp`, `proof`) and a Next.js UI. Earlier automation assumed a Bazel graph that was not present in the tree, which blocked reproducible local and CI builds.

## Decision

- **Cargo** is the source of truth for Rust: a single workspace at the repository root, shared dependency policy, `rust-toolchain.toml`, and a committed `Cargo.lock`.
- **Bazel** is not required for day-to-day development or default CI. If a hermetic multi-language graph is needed later, it should be introduced as an explicit milestone with real `BUILD` files and a valid workspace, not as a stub.
- **Supply chain**: `cargo-deny` at the root enforces license policy; `cargo audit` can be run in CI or locally against the advisory database.

## Workspace membership and explicit exclusions

The root workspace members are `proto`, `nlp`, `proof`, and `platform/gh-app`.
The `ingest` and `lean-farm` crates live in the tree but are listed under
`[workspace].exclude` in the root `Cargo.toml`. They are standalone service
binaries that carry their own `Cargo.lock` with service-specific dependency
pins downgraded to satisfy the pinned MSRV (for example: `ingest` ->
async-nats/AES-GCM/AWS Secrets Manager+KMS, `lean-farm` -> aws-sdk-s3/axum).
Because those lockfiles pin AWS SDK transitive crates to older, MSRV-compatible
versions than the workspace resolves, excluding them keeps
`cargo <cmd> --workspace` reproducible.

Excluding them explicitly also avoids the "current package believes it's in a
workspace when it's not" error when running Cargo inside those directories. Each
excluded crate builds and tests on its own (`cargo test` from within the crate
directory). Onboarding one into the workspace is a deliberate step (add it to
`members`, reconcile its dependency pins with the workspace `Cargo.lock`, and
confirm `cargo test --workspace` stays green).

Note: the drift-detection orchestration in `platform/gh-app` originally
referenced a fictional `temporal-sdk` crate; it has been replaced by a
self-contained async activity/engine abstraction (`workflows.rs`), so `gh-app`
now builds as a normal workspace member with no unpublished dependencies.

## Consequences

- Developers run `cargo test --workspace` and `cargo clippy` without installing Bazel.
- CI is driven by GitHub Actions jobs that mirror these commands.
- Additional Rust crates can be onboarded by adding explicit workspace membership rather than relying on implicit repository structure.
