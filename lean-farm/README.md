# Lean Farm (`lean-farm`)

`lean-farm` is the execution component for Lean jobs and proof artifact processing.

## What is in this directory

- Rust service code (`src/`)
- crate tests (`tests/`)
- hardened container image (`Dockerfile`)
- local security benchmark script (`scripts/security-benchmark.sh`)

## Build and run

`lean-farm` is a standalone crate that is intentionally excluded from the root
Cargo workspace (it carries its own `Cargo.lock` with MSRV-compatible AWS SDK
pins). Run Cargo commands from inside this directory:

```bash
cd lean-farm

# Build crate
cargo build

# Run tests (unit + hermetic integration tests)
cargo test

# Strict lint
cargo clippy --all-targets -- -D warnings

# Run the service (defaults shown; see --help for all flags)
cargo run -- --config /etc/lean-farm/config.yaml --log-format json
```

Build the container image used in CI (from the repository root):

```bash
docker build -f lean-farm/Dockerfile -t spec-to-proof/lean-farm:local lean-farm
```

## Modules

- `config` — YAML-backed configuration with defaults and validation (security,
  storage, `lean`, `sandbox`, job, metrics).
- `security` — runtime/sandbox posture detection and validation.
- `storage` — S3 (code bundles) and S3-compatible/MinIO (artifacts) access,
  with typed `NotConfigured` fallbacks when buckets are unset.
- `lean` — deterministic Lean proof-source generation (imports configurable;
  stdlib-only by default so proofs check offline).
- `sandbox` — **real** Lean execution: runs a generated source inside a hardened
  `docker`/`runsc` (gVisor) container with resource caps, timeouts and
  read-only, non-root, no-network isolation, returning a structured outcome.
- `artifact` — the cross-service `ProofArtifactReference` record and directory/S3
  sink consumed by the GitHub App badge service.
- `metrics` — shared counters plus a Prometheus text exposition endpoint.
- `job_runner` — worker pool orchestration and the per-job lifecycle
  (generate → sandbox execute → artifact → upload → reference emit).

## End-to-end proving

The proving hot path is real: for each job the farm generates a self-contained
Lean source, executes it with the Lean toolchain inside a hardened container,
and derives the artifact status from the actual exit code. A proof is `Success`
only if `lean` elaborates the source without error.

### 1. Build the sandbox runner image (once)

There is no maintained official Lean Docker image, so the farm ships a
purpose-built one with a pinned toolchain baked in for fully offline checking:

```bash
docker build -f lean-farm/lean-runner.Dockerfile \
  --build-arg LEAN_TOOLCHAIN=leanprover/lean4:v4.22.0 \
  -t spec-to-proof/lean-runner:v4.22.0 lean-farm
```

### 2. Run the real proving tests (requires Docker)

Docker-backed tests are gated behind `E2E_DOCKER=1` so CI stays green where
Docker is unavailable, but exercise the real path when it is present:

```powershell
# Windows PowerShell
$env:E2E_DOCKER = "1"; cargo test --test e2e_proving_test -- --nocapture
```

```bash
# POSIX
E2E_DOCKER=1 cargo test --test e2e_proving_test -- --nocapture
```

These prove a true theorem (`decide`/`rfl` over the stdlib), reject a false one
with real diagnostics, and assert that a `ProofArtifactReference` is mirrored for
the badge service.

### 3. Local artifact storage (AWS-free) with MinIO

From the repository root, bring up MinIO and create the artifact bucket, then
run the storage round-trip:

```bash
docker compose -f docker-compose.e2e.yml up -d

# In lean-farm/ with credentials for the local MinIO:
#   AWS_ACCESS_KEY_ID=minioadmin AWS_SECRET_ACCESS_KEY=minioadmin AWS_REGION=us-east-1
E2E_MINIO=1 cargo test --test minio_storage_test -- --nocapture

docker compose -f docker-compose.e2e.yml down -v
```

### 4. Run the service locally

```bash
cargo run -- --config config/local-e2e.yaml --log-format text
```

`config/local-e2e.yaml` points the artifact store at the local MinIO and mirrors
badge references to `./.lean-farm/references`. Set the GitHub App's
`PROOF_REFERENCE_DIR` (or `proof_reference_dir` in its config) to that directory
to serve real proof outcomes on the badge.

The hermetic unit/integration suite (`cargo test`) exercises configuration,
queueing, code generation, sandbox argument construction and artifact mapping
without Docker or object storage.

## Security baseline

- run as non-root where possible
- keep container filesystem/read permissions minimal
- scan built images in CI (Trivy informational today)
- avoid embedding credentials in image layers; use environment or secret stores

## Operational notes

- Kubernetes deployment and policy live in repository charts and infra folders.
- Keep this README implementation-focused; architecture-level decisions belong in `docs/adr/`.
