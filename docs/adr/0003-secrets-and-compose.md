# ADR 0003: Secrets and Docker Compose

## Status

Accepted

## Context

`docker-compose.yml` ships default database and cache passwords suitable only for local experimentation. Production deployments must never reuse those literals.

## Decision

- Document development-only defaults in comments and in `.env.example`.
- Expect real secrets (cloud credentials, API keys, strong DB passwords) to be supplied via environment variables or a secrets manager, not committed to git.
- The root `.env.example` lists common variable names without values for sensitive keys.
- Repository documentation should reference `SECURITY.md` for coordinated disclosure and secure reporting.

## Consequences

- Onboarding developers copy `.env.example` to `.env` and fill in local or staging values.
- Production runbooks should reference Kubernetes secrets, IAM roles, or the organization’s vault — not compose defaults.
