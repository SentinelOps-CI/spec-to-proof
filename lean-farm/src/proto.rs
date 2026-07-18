//! Protobuf types re-exported from the shared `spec-to-proof-proto` crate.
//!
//! Keeping the re-export in one place lets the rest of the crate refer to
//! `crate::proto::spec_to_proof::v1::*` and `crate::proto::proof::v1::*` while
//! the actual generated code lives in the workspace proto crate.

pub mod spec_to_proof {
    pub mod v1 {
        pub use spec_to_proof_proto::spec_v1::*;
    }
}

pub mod proof {
    pub mod v1 {
        pub use spec_to_proof_proto::proof_v1::*;
    }
}
