# Lean Farm (`lean-farm`)

`lean-farm` is the execution component for Lean jobs and proof artifact processing.

## What is in this directory

- Rust service code (`src/`)
- crate tests (`tests/`)
- hardened container image (`Dockerfile`)
- local security benchmark script (`scripts/security-benchmark.sh`)

## Build and run

From repository root:

```bash
# Build crate
cargo build -p lean-farm

# Run tests
cargo test -p lean-farm

# Strict lint
cargo clippy -p lean-farm --all-targets -- -D warnings
```

Build the container image used in CI:

```bash
docker build -f lean-farm/Dockerfile -t spec-to-proof/lean-farm:local lean-farm
```

## Security baseline

- run as non-root where possible
- keep container filesystem/read permissions minimal
- scan built images in CI (Trivy informational today)
- avoid embedding credentials in image layers; use environment or secret stores

## Operational notes

- Kubernetes deployment and policy live in repository charts and infra folders.
- Keep this README implementation-focused; architecture-level decisions belong in `docs/adr/`.
