# ADR 0002: CI job split (Rust, Node, Lean, infra, Docker)

## Status

Accepted

## Context

Prior workflows mixed Bazel, third-party SaaS scanners without secrets, and coverage steps that could not succeed on a clean checkout.

## Decision

CI is split into parallel jobs:

1. **Rust** — `cargo fmt`, `cargo clippy`, `cargo test`, `cargo deny` (toolchain from `rust-toolchain.toml` via `dtolnay/rust-toolchain`).
2. **Node** — `npm ci`, UI lint and typecheck via root scripts, Jest tests.
3. **Lean** — `lake build` in `proof/lean` with elan.
4. **Infra** — `terraform fmt -check` / `validate` and `helm lint` where manifests exist.
5. **Docker** — build `lean-farm` image locally in CI; Trivy scan is informational (`continue-on-error`) until policies are finalized.

Optional or paid scanners (Snyk, FOSSA, mutation testing) are not run by default; add them behind repository secrets when ready.

## Consequences

- PR checks match what contributors can run locally (`./scripts/ci-lint.sh` and the commands in `CONTRIBUTING.md`).
- A PowerShell equivalent (`./scripts/ci-lint.ps1`) keeps local verification parity for Windows contributors.
- Heavy or credential-gated steps do not block merges.
