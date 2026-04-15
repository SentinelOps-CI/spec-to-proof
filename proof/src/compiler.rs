use std::collections::HashMap;
use std::error::Error;
use std::time::Instant;
use serde_json::Value;
use sha2::{Sha256, Digest};

use crate::claude_client::ClaudeClient;
use crate::proto::proof::v1::*;
use crate::proto::spec_to_proof::v1::*;
use crate::ProofConfig;

pub struct LeanCompiler {
    claude_client: ClaudeClient,
    #[allow(dead_code)] // Retained for planned compiler features using full config.
    config: ProofConfig,
}

impl LeanCompiler {
    pub fn new(config: &ProofConfig) -> Self {
        let claude_client = ClaudeClient::new(&config.claude_api_key, &config.claude_model);
        
        Self {
            claude_client,
            config: config.clone(),
        }
    }

    pub async fn compile_invariant_to_theorem(
        &self,
        invariant: &Invariant,
        options: &CompilationOptions,
    ) -> Result<LeanTheorem, Box<dyn Error>> {
        let start_time = Instant::now();
        
        // Convert invariant to string representation
        let invariant_str = self.invariant_to_string(invariant);
        
        // Generate Lean theorem using Claude
        let (lean_code, input_tokens, output_tokens) = self.claude_client
            .generate_lean_theorem(&invariant_str, &options.proof_strategy, options.seed)
            .await?;

        // Parse the response to extract theorem name and imports
        let parsed_response = self.parse_lean_response(&lean_code)?;
        
        // Generate theorem ID and content hash
        let theorem_id = self.generate_theorem_id(invariant);
        let content_hash = self.compute_content_hash(&lean_code);
        
        // Create metadata
        let mut metadata = HashMap::new();
        metadata.insert("input_tokens".to_string(), input_tokens.to_string());
        metadata.insert("output_tokens".to_string(), output_tokens.to_string());
        metadata.insert("compilation_time_ms".to_string(), start_time.elapsed().as_millis().to_string());
        metadata.insert("proof_strategy".to_string(), options.proof_strategy.clone());
        metadata.insert("temperature".to_string(), options.temperature.to_string());
        metadata.insert("seed".to_string(), options.seed.to_string());
        
        if let Some(imports) = parsed_response.get("imports") {
            metadata.insert("imports".to_string(), serde_json::to_string(imports)?);
        }
        
        if let Some(dependencies) = parsed_response.get("dependencies") {
            metadata.insert("dependencies".to_string(), serde_json::to_string(dependencies)?);
        }

        let theorem = LeanTheorem {
            id: theorem_id,
            content_sha256: content_hash,
            theorem_name: parsed_response.get("theorem_name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown_theorem")
                .to_string(),
            lean_code,
            source_invariant_id: invariant.id.clone(),
            generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            status: TheoremStatus::Generated as i32,
            compilation_errors: Vec::new(),
            proof_strategy: options.proof_strategy.clone(),
            metadata,
        };

        Ok(theorem)
    }

    pub async fn generate_proof(
        &self,
        theorem: &LeanTheorem,
        options: &ProofOptions,
    ) -> Result<(LeanTheorem, ProofArtifact), Box<dyn Error>> {
        let start_time = Instant::now();
        
        // Generate proof using Claude
        let strategy = theorem.proof_strategy.clone();
        let (proof_code, input_tokens, output_tokens) = self.claude_client
            .generate_proof(&theorem.lean_code, &strategy, options.seed)
            .await?;

        // Parse the proof response
        let parsed_proof = self.parse_proof_response(&proof_code)?;
        
        // Combine original theorem with proof
        let complete_lean_code = format!("{}\n\n{}", theorem.lean_code, proof_code);
        let new_content_hash = self.compute_content_hash(&complete_lean_code);
        
        // Create proven theorem
        let mut proven_theorem = theorem.clone();
        proven_theorem.lean_code = complete_lean_code;
        proven_theorem.content_sha256 = new_content_hash;
        proven_theorem.status = TheoremStatus::Proven as i32;
        
        // Update metadata
        let mut metadata = HashMap::new();
        metadata.insert("proof_input_tokens".to_string(), input_tokens.to_string());
        metadata.insert("proof_output_tokens".to_string(), output_tokens.to_string());
        metadata.insert("proof_generation_time_ms".to_string(), start_time.elapsed().as_millis().to_string());
        metadata.insert("proof_strategy".to_string(), strategy.clone());
        metadata.insert("attempts".to_string(), "1".to_string());
        
        if let Some(tactics) = parsed_proof.get("tactics_used") {
            metadata.insert("tactics_used".to_string(), serde_json::to_string(tactics)?);
        }
        
        if let Some(difficulty) = parsed_proof.get("difficulty") {
            metadata.insert("difficulty".to_string(), difficulty.as_str().unwrap_or("unknown").to_string());
        }
        
        proven_theorem.metadata = metadata;

        // Create proof artifact
        let proof_artifact = ProofArtifact {
            id: self.generate_proof_artifact_id(theorem),
            content_sha256: self.compute_content_hash(&proof_code),
            theorem_id: theorem.id.clone(),
            invariant_id: theorem.source_invariant_id.clone(),
            status: ProofStatus::Success as i32,
            attempted_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            duration_ms: start_time.elapsed().as_millis() as i64,
            output: proof_code,
            logs: vec!["Proof generated successfully".to_string()],
            resource_usage: Some(ResourceUsage {
                cpu_seconds: 0.0, // TODO: Track actual CPU usage
                memory_bytes: 0,   // TODO: Track actual memory usage
                disk_bytes: 0,     // TODO: Track actual disk usage
                network_bytes: 0,  // TODO: Track actual network usage
            }),
            proof_strategy: strategy,
            confidence_score: 1.0, // TODO: Implement confidence scoring
            metadata: HashMap::new(),
        };

        Ok((proven_theorem, proof_artifact))
    }

    fn invariant_to_string(&self, invariant: &Invariant) -> String {
        let mut parts = Vec::new();
        
        // Add description
        parts.push(format!("Description: {}", invariant.description));
        
        // Add formal expression
        parts.push(format!("Formal Expression: {}", invariant.formal_expression));
        
        // Add natural language
        parts.push(format!("Natural Language: {}", invariant.natural_language));
        
        // Add variables
        if !invariant.variables.is_empty() {
            let var_strings: Vec<String> = invariant.variables
                .iter()
                .map(|v| format!("- {}: {} ({})", v.name, v.r#type, v.description))
                .collect();
            parts.push(format!("Variables:\n{}", var_strings.join("\n")));
        }
        
        // Add units
        if !invariant.units.is_empty() {
            let unit_strings: Vec<String> = invariant.units
                .iter()
                .map(|(k, v)| format!("- {}: {}", k, v))
                .collect();
            parts.push(format!("Units:\n{}", unit_strings.join("\n")));
        }
        
        // Add tags
        if !invariant.tags.is_empty() {
            parts.push(format!("Tags: {}", invariant.tags.join(", ")));
        }
        
        // Add priority
        parts.push(format!("Priority: {}", invariant.priority));
        
        parts.join("\n\n")
    }

    fn parse_lean_response(&self, lean_code: &str) -> Result<Value, Box<dyn Error>> {
        // In a real implementation, this would parse the Lean code more intelligently
        // For now, we'll extract basic information
        
        let mut result = HashMap::new();
        
        // Extract theorem name (simplified)
        if let Some(name) = self.extract_theorem_name(lean_code) {
            result.insert("theorem_name".to_string(), Value::String(name));
        }
        
        // Extract imports (simplified)
        let imports = self.extract_imports(lean_code);
        if !imports.is_empty() {
            result.insert("imports".to_string(), Value::Array(
                imports.into_iter().map(Value::String).collect()
            ));
        }
        
        // Extract dependencies (simplified)
        let dependencies = self.extract_dependencies(lean_code);
        if !dependencies.is_empty() {
            result.insert("dependencies".to_string(), Value::Array(
                dependencies.into_iter().map(Value::String).collect()
            ));
        }
        
        Ok(Value::Object(result.into_iter().collect()))
    }

    fn parse_proof_response(&self, proof_code: &str) -> Result<Value, Box<dyn Error>> {
        // In a real implementation, this would parse the proof code more intelligently
        // For now, we'll extract basic information
        
        let mut result = HashMap::new();
        
        // Extract tactics used (simplified)
        let tactics = self.extract_tactics(proof_code);
        if !tactics.is_empty() {
            result.insert("tactics_used".to_string(), Value::Array(
                tactics.into_iter().map(Value::String).collect()
            ));
        }
        
        // Estimate difficulty based on proof length and complexity
        let difficulty = self.estimate_difficulty(proof_code);
        result.insert("difficulty".to_string(), Value::String(difficulty));
        
        Ok(Value::Object(result.into_iter().collect()))
    }

    fn extract_theorem_name(&self, lean_code: &str) -> Option<String> {
        // Simple regex-like extraction for theorem names
        for line in lean_code.lines() {
            let line = line.trim();
            if line.starts_with("theorem") || line.starts_with("lemma") {
                if let Some(name) = line.split_whitespace().nth(1) {
                    return Some(name.to_string());
                }
            }
        }
        None
    }

    fn extract_imports(&self, lean_code: &str) -> Vec<String> {
        let mut imports = Vec::new();
        for line in lean_code.lines() {
            let line = line.trim();
            if line.starts_with("import") {
                if let Some(import) = line.split_whitespace().nth(1) {
                    imports.push(import.to_string());
                }
            }
        }
        imports
    }

    fn extract_dependencies(&self, _lean_code: &str) -> Vec<String> {
        // In a real implementation, this would analyze the Lean code
        // to determine dependencies. For now, return empty.
        Vec::new()
    }

    fn extract_tactics(&self, proof_code: &str) -> Vec<String> {
        let mut tactics = Vec::new();
        let common_tactics = [
            "rw", "simp", "apply", "exact", "intro", "cases", "induction",
            "refine", "constructor", "split", "left", "right", "assumption",
            "contradiction", "exfalso", "by_contra", "have", "let", "calc"
        ];
        
        for tactic in &common_tactics {
            if proof_code.contains(tactic) {
                tactics.push(tactic.to_string());
            }
        }
        
        tactics
    }

    fn estimate_difficulty(&self, proof_code: &str) -> String {
        let lines = proof_code.lines().count();
        let tactics = self.extract_tactics(proof_code).len();
        
        match (lines, tactics) {
            (0..=10, 0..=3) => "easy".to_string(),
            (11..=30, 4..=8) => "medium".to_string(),
            _ => "hard".to_string(),
        }
    }

    fn generate_theorem_id(&self, invariant: &Invariant) -> String {
        format!("theorem_{}", invariant.id)
    }

    fn generate_proof_artifact_id(&self, theorem: &LeanTheorem) -> String {
        format!("proof_{}", theorem.id)
    }

    fn compute_content_hash(&self, content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invariant_to_string() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let invariant = Invariant {
            id: "test_invariant".to_string(),
            content_sha256: "hash".to_string(),
            description: "Test invariant".to_string(),
            formal_expression: "∀x, P(x)".to_string(),
            natural_language: "For all x, P holds".to_string(),
            variables: vec![
                Variable {
                    name: "x".to_string(),
                    r#type: "Nat".to_string(),
                    description: "Natural number".to_string(),
                    unit: "".to_string(),
                    constraints: vec![],
                }
            ],
            units: HashMap::new(),
            confidence_score: 0.9,
            source_document_id: "doc1".to_string(),
            extracted_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            status: InvariantStatus::Extracted as i32,
            tags: vec!["test".to_string()],
            priority: Priority::Medium as i32,
        };
        
        let result = compiler.invariant_to_string(&invariant);
        assert!(result.contains("Test invariant"));
        assert!(result.contains("∀x, P(x)"));
        assert!(result.contains("For all x, P holds"));
    }

    #[test]
    fn test_extract_theorem_name() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let lean_code = r#"
            import Mathlib.Data.Nat.Basic
            
            theorem test_theorem (n : Nat) : n + 0 = n := by
                simp
        "#;
        
        let name = compiler.extract_theorem_name(lean_code);
        assert_eq!(name, Some("test_theorem".to_string()));
    }

    #[test]
    fn test_extract_imports() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let lean_code = r#"
            import Mathlib.Data.Nat.Basic
            import Mathlib.Algebra.Ring.Basic
            
            theorem test_theorem (n : Nat) : n + 0 = n := by
                simp
        "#;
        
        let imports = compiler.extract_imports(lean_code);
        assert_eq!(imports, vec![
            "Mathlib.Data.Nat.Basic".to_string(),
            "Mathlib.Algebra.Ring.Basic".to_string()
        ]);
    }

    #[test]
    fn test_extract_tactics() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let proof_code = r#"
            by
                simp
                apply Nat.add_zero
                exact rfl
        "#;
        
        let tactics = compiler.extract_tactics(proof_code);
        assert!(tactics.contains(&"simp".to_string()));
        assert!(tactics.contains(&"apply".to_string()));
        assert!(tactics.contains(&"exact".to_string()));
    }

    #[test]
    fn test_estimate_difficulty() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let easy_proof = "by simp";
        assert_eq!(compiler.estimate_difficulty(easy_proof), "easy");
        
        let medium_proof = r#"by
  simp
  apply Nat.add_zero
  exact rfl
  rw [add_assoc]
  simp
  apply add_comm
  exact rfl
  intro x
  cases x
  simp
"#;
        assert_eq!(compiler.estimate_difficulty(medium_proof), "medium");
        
        // Nine distinct tactic tokens triggers the fallback "hard" bucket (tactic count > 8).
        let hard_proof = r#"induction
rw
simp
apply
exact
intro
cases
refine
constructor
split
"#;
        assert_eq!(compiler.estimate_difficulty(hard_proof), "hard");
    }

    #[test]
    fn test_compute_content_hash() {
        let config = ProofConfig::default();
        let compiler = LeanCompiler::new(&config);
        
        let content = "test content";
        let hash = compiler.compute_content_hash(content);
        
        let expected = "6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72";
        assert_eq!(hash, expected);
    }
} 