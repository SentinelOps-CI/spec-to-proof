//! Protobuf types for proof service (re-exported from `spec-to-proof-proto`).
pub mod proof {
    pub use spec_to_proof_proto::proof_v1 as v1;
}

pub mod spec_to_proof {
    pub use spec_to_proof_proto::spec_v1 as v1;
}
