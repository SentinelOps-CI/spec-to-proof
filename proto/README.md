# Protocol Models (`proto`)

The `proto` crate is the shared schema layer for Spec-to-Proof.

## What lives here

- `.proto` contracts for cross-service APIs and domain models
- Rust code generation via `tonic-build` (`build.rs`)
- Shared conversion utilities and model helpers in `src/lib.rs`
- Buf config for schema linting and compatibility checks

## Common workflows

From repository root:

```bash
# Build generated Rust bindings and crate
cargo build -p spec-to-proof-proto

# Run tests for the crate
cargo test -p spec-to-proof-proto

# Strict lint for the crate
cargo clippy -p spec-to-proof-proto --all-targets -- -D warnings
```

From `proto/`:

```bash
# Schema linting
buf lint

# Breaking-change check against main
buf breaking --against '.git#branch=main'

# Regenerate outputs configured in buf.gen.yaml
buf generate
```

## Updating schemas safely

1. Update `.proto` definitions.
2. Regenerate code (`buf generate`) when needed.
3. Run `buf lint` and `buf breaking`.
4. Run `cargo test -p spec-to-proof-proto`.
5. Verify downstream crates (`nlp`, `proof`) still compile and pass clippy/tests.

## Notes

- Prefer additive, backward-compatible field evolution.
- Reserve removed field numbers/names in `.proto` files when deprecating.
- Keep model documentation near the schema to avoid drift.