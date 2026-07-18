# Proof Service (`proof`)

The `proof` crate turns extracted invariants into Lean artifacts and serves proof workflows over gRPC.

## Scope

- Generate Lean theorem stubs and proof attempts from invariant data
- Apply guarded prompts and retry logic when calling LLM backends
- Persist generated artifacts to S3-compatible storage
- Expose proof APIs via `tonic` gRPC service

## Run locally

From repository root:

```bash
cargo run -p proof --bin lean_compiler
```

The service reads configuration from environment variables.

## Key environment variables

| Variable | Default | Notes |
|---|---|---|
| `CLAUDE_API_KEY` | required | API key for model calls |
| `CLAUDE_MODEL` | `claude-3-opus-20240229` | model identifier |
| `MAX_TOKENS` | `8000` | per-call token cap |
| `TEMPERATURE` | `0.0` | deterministic generation baseline |
| `MAX_RETRIES` | `3` | proof retry attempts |
| `RETRY_DELAY_MS` | `1000` | base retry delay |
| `COST_PER_1K_TOKENS` | `0.015` | cost accounting |
| `S3_BUCKET` | `spec-to-proof-lean` | artifact bucket |
| `S3_REGION` | `us-east-1` | bucket region |
| `S3_KEY_PREFIX` | `theorems/` | object key prefix |
| `KMS_KEY_ID` | optional | SSE-KMS key |

## Development commands

```bash
# Build
cargo build -p proof

# Lint
cargo clippy -p proof --all-targets -- -D warnings

# Test
cargo test -p proof
```

## Lean project

Lean files used for local theorem checking live under `proof/lean/`:

```bash
cd proof/lean
lake build
```

## Notes

- Retry backoff is exponential with saturation guards.
- Prompt sanitization and input validation are part of the crate tests.
- For repository-wide quality gates, use root-level `./scripts/ci-lint.sh` (or `./scripts/ci-lint.ps1` on Windows).

## Metrics and confidence scoring

- Compilation and proof responses report **real token usage and estimated cost**,
  aggregated from the per-theorem counters returned by the model client and
  priced via `COST_PER_1K_TOKENS`. The proof-generation path also reports the
  **actual number of attempts** taken by the retry loop.
- Each proof artifact carries a **confidence score in `[0, 1]`**. This is a
  conservative heuristic (capped at `0.95`) intended purely for prioritisation:
  proofs that defer their goal with `sorry`/`admit` score `0.1`, and the score
  otherwise scales with estimated difficulty and tactic richness. It is **not** a
  substitute for machine-checking the proof with Lean, which the lean-farm runner
  performs.
- `ResourceUsage` on a proof artifact records the payload bytes exchanged with
  the model (`network_bytes`), the on-disk size of the generated artifact
  (`disk_bytes`), and the CPU time of local post-processing (`cpu_seconds`);
  heavy Lean-checking resource accounting is owned by the lean-farm runner.