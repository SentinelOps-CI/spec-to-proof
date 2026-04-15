# Pull Request

## What changed

Describe the change and the motivation in 2-5 bullet points.

## Change type

- [ ] Bug fix
- [ ] Feature
- [ ] Refactor
- [ ] Documentation
- [ ] Security
- [ ] Breaking change

## Validation performed

Paste the commands you ran and summarize outcomes.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm ci
npm run lint
npm run type-check
npm test
```

If applicable:

```bash
cd proof/lean && lake build
cargo deny check
cargo audit
```

## Risk and rollout

- **Risk level**: low / medium / high
- **Fallback plan**: how to revert or disable safely
- **Data/compatibility impact**: migrations, API/contract changes, or none

## Security checklist

- [ ] No secrets committed (`.env`, tokens, private keys, credentials)
- [ ] Input validation and error handling reviewed
- [ ] Dependency changes reviewed for supply-chain risk
- [ ] Relevant threat surface considered (authz, data exposure, injection)

## Documentation checklist

- [ ] Updated docs for behavior/config changes (`README`, `CONTRIBUTING`, ADRs)
- [ ] Added or updated tests for the changed behavior

## Linked issues

Closes #(issue)

## Screenshots / logs (if relevant)

Include UI screenshots or terminal output snippets when they improve review clarity.