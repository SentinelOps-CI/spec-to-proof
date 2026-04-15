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
