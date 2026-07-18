# GitHub App / Webhook Service (`gh-app`)

This crate hosts the GitHub App surface for spec-to-proof: it authenticates as a
GitHub App, receives webhooks (GitHub, Jira, Confluence, Google Docs), triggers
drift-detection workflows, publishes proof badges, and exposes Sigstore
verification and cost-governance endpoints.

## Build status and requirements

This crate **is** part of the root Cargo workspace (see
[docs/adr/0001](../../docs/adr/0001-cargo-workspace-and-optional-bazel.md)) and
builds with the pinned public dependency set:

```
cargo check -p gh-app --all-features
cargo test  -p gh-app
```

The drift-detection orchestration originally referenced a fictional
`temporal-sdk` crate; it has been replaced by a self-contained async
activity/engine abstraction (`workflows.rs`) with no unpublished dependencies.
All remaining dependencies (AWS SDK, Redis, OpenTelemetry) are declared in
`Cargo.toml` and resolve from crates.io. Feature-gated integrations (for example
`aws-governance`, which enables the CloudWatch Events / Cost Explorer / SES
handlers) are compiled with `--all-features` or the specific feature flag.

## What is implemented

- **Webhook signature verification** (`webhook_handlers.rs`,
  `auth.rs::validate_webhook_signature`): real HMAC-SHA256 over the raw request
  body, comparing against the `sha256=<hex>` header using a constant-time check
  (`hmac::Mac::verify_slice`). A missing/empty secret is treated as a hard
  failure rather than silently accepting the payload.
- **GitHub App authentication** (`auth.rs`): RS256 app/installation JWT minting
  with short-lived caching, and installation-token exchange against the GitHub
  API.
- **Idempotency key derivation** (`webhook_handlers.rs`): deterministic
  SHA-256 over `source:event_id`.
- **Health/uptime** (`lib.rs`, `server.rs`): the `/health` endpoint reports real
  process uptime and a `github_app_auth` readiness check derived from the JWT
  manager (it actually attempts to mint an app JWT), rather than a hardcoded
  status.
- **Cost governance** (`cost_governance.rs`): a Redis-backed token bucket
  (atomic Lua script) wired to a real `redis::Client` held by the manager.

## Known limitations (intentional, documented)

The following paths are **not yet wired to their downstream systems**. Rather
than panicking with `todo!()`, they return a clear `... is not configured` error
(or are marked as ignored integration tests), so they fail cleanly instead of
aborting the process:

- Temporal activities in `workflows.rs`: `fetch_spec_document`,
  `get_last_proven_hash`, `enqueue_proof_job`, `update_drift_status`,
  `send_drift_alert`.
- Cost-governance event handlers in `cloudwatch_events.rs`
  (`handle_governance_event`).
- Alert transports (`SlackAlertChannel`, `EmailAlertChannel`) currently log the
  alert; wiring an HTTP/SMTP transport is pending.
- `auth.rs`: `auth_middleware` performs a presence check on the bearer token and
  `extract_public_key_from_private` is a placeholder; full JWT verification for
  inbound API calls and public-key extraction are pending. These are not on the
  webhook ingress path (which is HMAC-verified as described above).
- `sigstore.rs` and `badge.rs`: the Fulcio certificate retrieval, Rekor
  inclusion/consistency proofs, and proof-farm badge artifacts currently return
  mock values. The request plumbing and types are in place, but the external
  Sigstore/proof-farm integrations are not yet wired in.

## Required configuration (environment variables)

| Variable | Purpose |
| --- | --- |
| `GITHUB_WEBHOOK_SECRET` | HMAC secret for GitHub webhook verification |
| `JIRA_WEBHOOK_SECRET` / `CONFLUENCE_WEBHOOK_SECRET` / `GDOCS_WEBHOOK_SECRET` | Per-source webhook HMAC secrets |
| `SLACK_WEBHOOK_URL` | Optional Slack alert transport |
| `SMTP_HOST` / `SMTP_PORT` / `SMTP_USERNAME` / `SMTP_PASSWORD` | Optional email alert transport |

## Development commands

```bash
# gh-app is a root workspace member, so these can run from the repo root
# (add `-p gh-app`) or from this directory:
cargo build -p gh-app
cargo clippy -p gh-app --all-targets -- -D warnings
cargo test -p gh-app
```
