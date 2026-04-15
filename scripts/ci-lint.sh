#!/usr/bin/env bash
# Local checks aligned with .github/workflows/ci.yml (Cargo + npm + optional Lake/Terraform/Helm).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> Rust: fmt, clippy, test, deny"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check
else
  echo "cargo-deny not installed; skip (install: cargo install cargo-deny)"
fi

echo "==> Node: lint, typecheck (UI lives under platform/ui; see package.json)"
npm ci
npm run lint
npm run type-check

echo "==> Optional: Lake (proof/lean)"
if command -v lake >/dev/null 2>&1; then
  (cd proof/lean && lake build)
else
  echo "lake not in PATH; skip Lean build (install elan + lean from https://leanprover.github.io/lean4/doc/setup.html)"
fi

echo "==> Optional: Terraform"
if command -v terraform >/dev/null 2>&1; then
  (cd terraform && terraform fmt -check && terraform init -backend=false && terraform validate)
else
  echo "terraform not in PATH; skip"
fi

echo "==> Optional: Helm"
if command -v helm >/dev/null 2>&1; then
  helm lint charts/spec-to-proof
  helm lint charts/lean-farm
else
  echo "helm not in PATH; skip"
fi

echo "All requested checks passed."
