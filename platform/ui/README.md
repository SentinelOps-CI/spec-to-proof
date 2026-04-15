# Platform UI (`platform/ui`)

This directory contains the Next.js frontend for Spec-to-Proof.

## How to run

Use root-level scripts (recommended) so commands are consistent with CI:

```bash
# from repository root
npm ci
npm run dev
npm run lint
npm run type-check
npm test
```

These scripts delegate into `platform/ui` via `scripts/run-in-platform-ui.mjs`.

## Build

```bash
npm run build
npm run start
```

## Tech stack

- Next.js 14
- TypeScript 5
- tRPC + React Query
- Jest + Testing Library

## Testing expectations

- Keep UI tests deterministic (no external network dependency).
- Prefer behavior-focused tests over snapshot-heavy tests.
- Keep lint and type-check warnings at zero.

## Environment

Example public variables:

```bash
NEXT_PUBLIC_API_URL=http://localhost:3001/trpc
NEXT_PUBLIC_APP_URL=http://localhost:3000
```

For full project setup, see root `README.md` and `CONTRIBUTING.md`.