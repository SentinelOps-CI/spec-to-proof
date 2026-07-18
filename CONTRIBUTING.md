# Contributing

## Prerequisites

- Rust via [rustup](https://rustup.rs/); this repo pins the toolchain in `rust-toolchain.toml`.
- Node.js 20+ and npm 9+ (see `package.json` `engines`).
- For Lean: [elan](https://github.com/leanprover/elan) and Lake (used under `proof/lean/`).

## Local checks

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check   # requires: cargo install cargo-deny

npm ci
npm run lint
npm run type-check
npm test
```

Or run the aggregated script (POSIX shell):

```bash
./scripts/ci-lint.sh
```

On Windows/PowerShell, use:

```powershell
./scripts/ci-lint.ps1
```

## UI commands

Next.js lives under `platform/ui/`. Root `package.json` scripts delegate there via `scripts/run-in-platform-ui.mjs` so `npm run lint` works from the repository root on Linux, macOS, and Windows.

## Lean / Lake

```bash
cd proof/lean
lake build
```

## End-to-end Lean proving (`lean-farm`)

`lean-farm` runs Lean proofs for real inside a hardened container. It is a
standalone crate (excluded from the root workspace), so run its commands from
its own directory.

```bash
cd lean-farm

# Hermetic suite (no Docker/object store required):
cargo test

# Build the pinned Lean sandbox image once (from the repository root):
cd .. && docker build -f lean-farm/lean-runner.Dockerfile \
  --build-arg LEAN_TOOLCHAIN=leanprover/lean4:v4.22.0 \
  -t spec-to-proof/lean-runner:v4.22.0 lean-farm && cd lean-farm

# Real Docker-backed proving (gated by E2E_DOCKER so CI stays green without Docker):
#   POSIX:      E2E_DOCKER=1 cargo test --test e2e_proving_test -- --nocapture
#   PowerShell: $env:E2E_DOCKER = "1"; cargo test --test e2e_proving_test -- --nocapture
```

Local, AWS-free artifact storage uses MinIO from the repository root:

```bash
docker compose -f docker-compose.e2e.yml up -d
# then, in lean-farm/ with AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin AWS_REGION=us-east-1
E2E_MINIO=1 cargo test --test minio_storage_test -- --nocapture
docker compose -f docker-compose.e2e.yml down -v
```

See [`lean-farm/README.md`](lean-farm/README.md) for the full E2E workflow and
how to wire proof outcomes into the GitHub App badge (`PROOF_REFERENCE_DIR`).

## Optional NLP integration tests

Rust integration tests under `nlp/tests/` are gated behind `--features legacy-integration-tests` until they are refreshed for the current AWS SDK and fixtures:

```bash
cargo test -p nlp --features legacy-integration-tests
```

## Windows linker note (LNK1104)

If `cargo test` fails on Windows with `LINK : fatal error LNK1104` for a file under `target/debug/deps/*.exe`, this is typically a local file lock (antivirus/indexer/another process). Common mitigations:

- close running binaries and duplicate test processes
- add `target/` to antivirus real-time scan exclusions
- retry after `cargo clean`
- run tests in WSL if locks persist

## Security

For responsible disclosure of vulnerabilities, see [`SECURITY.md`](SECURITY.md).

## Architectural notes

See `docs/adr/` for decisions (Cargo workspace, CI layout, secrets, observability).

## Pull requests

- Keep changes focused and tested (`cargo test --workspace`, `npm test`).
- Follow existing style; match formatting (`cargo fmt`, Prettier/ESLint as configured).
- For docs-only changes, still run at least `cargo clippy --workspace --all-targets -- -D warnings` and `npm run lint` before opening a PR.
