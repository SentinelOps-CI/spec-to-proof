import Lake
open Lake DSL

package «spec-to-proof-lean» where
  version := v!"0.1.0"

@[default_target]
lean_lib «SpecToProof» where
  roots := #[`TrivialExample]
