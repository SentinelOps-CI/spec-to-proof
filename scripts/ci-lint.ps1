#!/usr/bin/env pwsh
# Local checks aligned with .github/workflows/ci.yml for Windows/PowerShell users.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
Set-Location $RepoRoot

Write-Host "==> Rust: fmt, clippy, test, deny"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

if (Get-Command cargo-deny -ErrorAction SilentlyContinue) {
    cargo deny check
} else {
    Write-Host "cargo-deny not installed; skip (install: cargo install cargo-deny)"
}

Write-Host "==> Node: lint, typecheck (UI lives under platform/ui; see package.json)"
npm ci
npm run lint
npm run type-check

Write-Host "==> Optional: Lake (proof/lean)"
if (Get-Command lake -ErrorAction SilentlyContinue) {
    Push-Location "proof/lean"
    lake build
    Pop-Location
} else {
    Write-Host "lake not in PATH; skip Lean build (install elan + lean from https://leanprover.github.io/lean4/doc/setup.html)"
}

Write-Host "==> Optional: Terraform"
if (Get-Command terraform -ErrorAction SilentlyContinue) {
    Push-Location "terraform"
    terraform fmt -check
    terraform init -backend=false
    terraform validate
    Pop-Location
} else {
    Write-Host "terraform not in PATH; skip"
}

Write-Host "==> Optional: Helm"
if (Get-Command helm -ErrorAction SilentlyContinue) {
    helm lint charts/spec-to-proof
    helm lint charts/lean-farm
} else {
    Write-Host "helm not in PATH; skip"
}

Write-Host "All requested checks passed."
