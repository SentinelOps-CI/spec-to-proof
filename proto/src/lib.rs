pub mod spec_to_proof {
    pub mod v1 {
        tonic::include_proto!("spec_to_proof.v1");
    }

    pub mod proof {
        pub mod v1 {
            tonic::include_proto!("spec_to_proof.proof.v1");
        }
    }
}

pub mod nlp {
    pub mod v1 {
        tonic::include_proto!("nlp.v1");
    }
}

use chrono::{DateTime, Utc};
use prost_types::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

fn datetime_to_timestamp(dt: DateTime<Utc>) -> Timestamp {
    Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

fn timestamp_to_datetime(ts: &Timestamp) -> Result<DateTime<Utc>, Box<dyn std::error::Error>> {
    DateTime::from_timestamp(ts.seconds, ts.nanos as u32).ok_or_else(|| "invalid timestamp".into())
}

/// Generated protobuf types (`spec_to_proof.v1`).
pub use spec_to_proof::v1 as spec_v1;
/// Generated proof-service protobuf types (`spec_to_proof.proof.v1`).
pub use spec_to_proof::proof::v1 as proof_v1;

// Rust domain models with conversion traits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecDocumentModel {
    pub id: String,
    pub content_sha256: String,
    pub source_system: String,
    pub source_id: String,
    pub title: String,
    pub content: String,
    pub url: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
    pub version: i32,
    pub status: DocumentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantModel {
    pub id: String,
    pub content_sha256: String,
    pub description: String,
    pub formal_expression: String,
    pub natural_language: String,
    pub variables: Vec<VariableModel>,
    pub units: HashMap<String, String>,
    pub confidence_score: f64,
    pub source_document_id: String,
    pub extracted_at: DateTime<Utc>,
    pub status: InvariantStatus,
    pub tags: Vec<String>,
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariableModel {
    pub name: String,
    pub var_type: String,
    pub description: String,
    pub unit: String,
    pub constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantSetModel {
    pub id: String,
    pub content_sha256: String,
    pub name: String,
    pub description: String,
    pub invariants: Vec<InvariantModel>,
    pub source_document_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub modified_at: DateTime<Utc>,
    pub status: InvariantSetStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeanTheoremModel {
    pub id: String,
    pub content_sha256: String,
    pub theorem_name: String,
    pub lean_code: String,
    pub source_invariant_id: String,
    pub generated_at: DateTime<Utc>,
    pub status: TheoremStatus,
    pub compilation_errors: Vec<String>,
    pub proof_strategy: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofArtifactModel {
    pub id: String,
    pub content_sha256: String,
    pub theorem_id: String,
    pub invariant_id: String,
    pub status: ProofStatus,
    pub attempted_at: DateTime<Utc>,
    pub duration_ms: i64,
    pub output: String,
    pub logs: Vec<String>,
    pub resource_usage: ResourceUsageModel,
    pub proof_strategy: String,
    pub confidence_score: f64,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsageModel {
    pub cpu_seconds: f64,
    pub memory_bytes: i64,
    pub disk_bytes: i64,
    pub network_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeStatusModel {
    pub id: String,
    pub content_sha256: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub pr_number: i32,
    pub commit_sha: String,
    pub state: BadgeState,
    pub description: String,
    pub target_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub proof_artifact_ids: Vec<String>,
    pub coverage_percentage: f64,
    pub invariants_proven: i32,
    pub total_invariants: i32,
}

// Rust enums for better type safety
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DocumentStatus {
    Unspecified,
    Draft,
    Published,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvariantStatus {
    Unspecified,
    Extracted,
    Confirmed,
    Rejected,
    Proven,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum InvariantSetStatus {
    Unspecified,
    Draft,
    Review,
    Approved,
    Proven,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TheoremStatus {
    Unspecified,
    Generated,
    Compiling,
    Compiled,
    Proving,
    Proven,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProofStatus {
    Unspecified,
    Pending,
    Running,
    Success,
    Failed,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BadgeState {
    Unspecified,
    Pending,
    Success,
    Failure,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Priority {
    Unspecified,
    Low,
    Medium,
    High,
    Critical,
}

// Conversion traits
pub trait ToProto {
    type ProtoType;
    fn to_proto(&self) -> Self::ProtoType;
}

pub trait FromProto {
    type ProtoType;
    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>>
    where
        Self: Sized;
}

// Implementation of conversion traits
impl ToProto for SpecDocumentModel {
    type ProtoType = spec_v1::SpecDocument;

    fn to_proto(&self) -> Self::ProtoType {
        spec_v1::SpecDocument {
            id: self.id.clone(),
            content_sha256: self.content_sha256.clone(),
            source_system: self.source_system.clone(),
            source_id: self.source_id.clone(),
            title: self.title.clone(),
            content: self.content.clone(),
            url: self.url.clone(),
            author: self.author.clone(),
            created_at: Some(datetime_to_timestamp(self.created_at)),
            modified_at: Some(datetime_to_timestamp(self.modified_at)),
            metadata: self.metadata.clone(),
            version: self.version,
            status: self.status.to_proto(),
        }
    }
}

impl FromProto for SpecDocumentModel {
    type ProtoType = spec_v1::SpecDocument;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(SpecDocumentModel {
            id: proto.id,
            content_sha256: proto.content_sha256,
            source_system: proto.source_system,
            source_id: proto.source_id,
            title: proto.title,
            content: proto.content,
            url: proto.url,
            author: proto.author,
            created_at: timestamp_to_datetime(proto.created_at.as_ref().unwrap_or(&Timestamp::default()))?,
            modified_at: timestamp_to_datetime(proto.modified_at.as_ref().unwrap_or(&Timestamp::default()))?,
            metadata: proto.metadata,
            version: proto.version,
            status: DocumentStatus::from_proto(proto.status)?,
        })
    }
}

impl ToProto for InvariantModel {
    type ProtoType = spec_v1::Invariant;

    fn to_proto(&self) -> Self::ProtoType {
        spec_v1::Invariant {
            id: self.id.clone(),
            content_sha256: self.content_sha256.clone(),
            description: self.description.clone(),
            formal_expression: self.formal_expression.clone(),
            natural_language: self.natural_language.clone(),
            variables: self.variables.iter().map(|v| v.to_proto()).collect(),
            units: self.units.clone(),
            confidence_score: self.confidence_score,
            source_document_id: self.source_document_id.clone(),
            extracted_at: Some(datetime_to_timestamp(self.extracted_at)),
            status: self.status.to_proto(),
            tags: self.tags.clone(),
            priority: self.priority.to_proto(),
        }
    }
}

impl FromProto for InvariantModel {
    type ProtoType = spec_v1::Invariant;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(InvariantModel {
            id: proto.id,
            content_sha256: proto.content_sha256,
            description: proto.description,
            formal_expression: proto.formal_expression,
            natural_language: proto.natural_language,
            variables: proto
                .variables
                .into_iter()
                .map(|v| VariableModel::from_proto(v))
                .collect::<Result<Vec<_>, _>>()?,
            units: proto.units,
            confidence_score: proto.confidence_score,
            source_document_id: proto.source_document_id,
            extracted_at: timestamp_to_datetime(proto.extracted_at.as_ref().unwrap_or(&Timestamp::default()))?,
            status: InvariantStatus::from_proto(proto.status)?,
            tags: proto.tags,
            priority: Priority::from_proto(proto.priority)?,
        })
    }
}

impl ToProto for VariableModel {
    type ProtoType = spec_v1::Variable;

    fn to_proto(&self) -> Self::ProtoType {
        spec_v1::Variable {
            name: self.name.clone(),
            r#type: self.var_type.clone(),
            description: self.description.clone(),
            unit: self.unit.clone(),
            constraints: self.constraints.clone(),
        }
    }
}

impl FromProto for VariableModel {
    type ProtoType = spec_v1::Variable;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(VariableModel {
            name: proto.name,
            var_type: proto.r#type,
            description: proto.description,
            unit: proto.unit,
            constraints: proto.constraints,
        })
    }
}

// Enum conversion implementations
impl ToProto for DocumentStatus {
    type ProtoType = i32;

    fn to_proto(&self) -> Self::ProtoType {
        match self {
            DocumentStatus::Unspecified => 0,
            DocumentStatus::Draft => 1,
            DocumentStatus::Published => 2,
            DocumentStatus::Archived => 3,
        }
    }
}

impl FromProto for DocumentStatus {
    type ProtoType = i32;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(match proto {
            0 => DocumentStatus::Unspecified,
            1 => DocumentStatus::Draft,
            2 => DocumentStatus::Published,
            3 => DocumentStatus::Archived,
            _ => DocumentStatus::Unspecified,
        })
    }
}

impl ToProto for InvariantStatus {
    type ProtoType = i32;

    fn to_proto(&self) -> Self::ProtoType {
        match self {
            InvariantStatus::Unspecified => 0,
            InvariantStatus::Extracted => 1,
            InvariantStatus::Confirmed => 2,
            InvariantStatus::Rejected => 3,
            InvariantStatus::Proven => 4,
            InvariantStatus::Failed => 5,
        }
    }
}

impl FromProto for InvariantStatus {
    type ProtoType = i32;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(match proto {
            0 => InvariantStatus::Unspecified,
            1 => InvariantStatus::Extracted,
            2 => InvariantStatus::Confirmed,
            3 => InvariantStatus::Rejected,
            4 => InvariantStatus::Proven,
            5 => InvariantStatus::Failed,
            _ => InvariantStatus::Unspecified,
        })
    }
}

impl ToProto for Priority {
    type ProtoType = i32;

    fn to_proto(&self) -> Self::ProtoType {
        match self {
            Priority::Unspecified => 0,
            Priority::Low => 1,
            Priority::Medium => 2,
            Priority::High => 3,
            Priority::Critical => 4,
        }
    }
}

impl FromProto for Priority {
    type ProtoType = i32;

    fn from_proto(proto: Self::ProtoType) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(match proto {
            0 => Priority::Unspecified,
            1 => Priority::Low,
            2 => Priority::Medium,
            3 => Priority::High,
            4 => Priority::Critical,
            _ => Priority::Unspecified,
        })
    }
}

// Utility functions for SHA256 hashing
pub fn calculate_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_id() -> String {
    Uuid::new_v4().to_string()
}

// JSON Schema generation
pub fn generate_json_schema() -> serde_json::Value {
    serde_json::json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "Spec-to-Proof Domain Models",
        "type": "object",
        "properties": {
            "spec_document": {
                "$ref": "#/definitions/SpecDocument"
            },
            "invariant": {
                "$ref": "#/definitions/Invariant"
            },
            "invariant_set": {
                "$ref": "#/definitions/InvariantSet"
            },
            "lean_theorem": {
                "$ref": "#/definitions/LeanTheorem"
            },
            "proof_artifact": {
                "$ref": "#/definitions/ProofArtifact"
            },
            "badge_status": {
                "$ref": "#/definitions/BadgeStatus"
            }
        },
        "definitions": {
            "SpecDocument": {
                "type": "object",
                "required": ["id", "content_sha256", "source_system", "title", "content"],
                "properties": {
                    "id": {"type": "string"},
                    "content_sha256": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
                    "source_system": {"type": "string"},
                    "source_id": {"type": "string"},
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "url": {"type": "string", "format": "uri"},
                    "author": {"type": "string"},
                    "created_at": {"type": "string", "format": "date-time"},
                    "modified_at": {"type": "string", "format": "date-time"},
                    "metadata": {"type": "object", "additionalProperties": {"type": "string"}},
                    "version": {"type": "integer", "minimum": 1},
                    "status": {"$ref": "#/definitions/DocumentStatus"}
                }
            },
            "DocumentStatus": {
                "type": "string",
                "enum": ["unspecified", "draft", "published", "archived"]
            },
            "Invariant": {
                "type": "object",
                "required": ["id", "content_sha256", "description", "formal_expression"],
                "properties": {
                    "id": {"type": "string"},
                    "content_sha256": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"},
                    "description": {"type": "string"},
                    "formal_expression": {"type": "string"},
                    "natural_language": {"type": "string"},
                    "variables": {"type": "array", "items": {"$ref": "#/definitions/Variable"}},
                    "units": {"type": "object", "additionalProperties": {"type": "string"}},
                    "confidence_score": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                    "source_document_id": {"type": "string"},
                    "extracted_at": {"type": "string", "format": "date-time"},
                    "status": {"$ref": "#/definitions/InvariantStatus"},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "priority": {"$ref": "#/definitions/Priority"}
                }
            },
            "Variable": {
                "type": "object",
                "required": ["name", "type"],
                "properties": {
                    "name": {"type": "string"},
                    "type": {"type": "string"},
                    "description": {"type": "string"},
                    "unit": {"type": "string"},
                    "constraints": {"type": "array", "items": {"type": "string"}}
                }
            },
            "InvariantStatus": {
                "type": "string",
                "enum": ["unspecified", "extracted", "confirmed", "rejected", "proven", "failed"]
            },
            "Priority": {
                "type": "string",
                "enum": ["unspecified", "low", "medium", "high", "critical"]
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn test_sha256_hashing(content: String) {
            let hash1 = calculate_sha256(&content);
            let hash2 = calculate_sha256(&content);
            assert_eq!(hash1, hash2);
            assert_eq!(hash1.len(), 64);
        }
    }
} 