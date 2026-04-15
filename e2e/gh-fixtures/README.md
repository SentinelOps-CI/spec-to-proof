# GitHub Fixture Data (`e2e/gh-fixtures`)

This directory contains fixture content used by end-to-end tests for GitHub-driven workflows (PR/spec detection/badge flows).

## Purpose

- provide deterministic sample spec files and payloads
- support repeatable local and CI E2E runs
- avoid external repository dependencies in tests

## Guidelines

- keep fixture inputs small and representative
- use sanitized, non-sensitive test data only
- prefer additive fixture updates over editing existing canonical fixtures
- document new fixture sets in this file when introducing them

## Running tests

Run repository tests from the root:

```bash
npm test
```

If a specific test suite consumes these fixtures, include that command in the corresponding test file or package script to keep this README stable as tooling evolves.