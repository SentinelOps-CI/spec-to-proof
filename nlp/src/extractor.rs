use anyhow::Result;
use serde::Deserialize;
use crate::proto::nlp::v1::{
    ExtractInvariantsRequest, ExtractedInvariant, Variable, Priority, TokenUsage
};
use crate::claude_client::ClaudeClient;
use crate::prompts::PromptTemplate;

#[derive(Debug, Deserialize)]
struct ClaudeInvariantResponse {
    invariants: Vec<RawExtractedInvariant>,
}

#[derive(Debug, Deserialize)]
struct RawExtractedInvariant {
    description: String,
    formal_expression: String,
    natural_language: String,
    variables: Vec<RawVariable>,
    units: std::collections::HashMap<String, String>,
    confidence_score: f64,
    tags: Vec<String>,
    priority: String,
}

#[derive(Debug, Deserialize)]
struct RawVariable {
    name: String,
    #[serde(rename = "type")]
    var_type: String,
    description: String,
    unit: String,
    constraints: Vec<String>,
}

pub struct InvariantExtractor {
    claude_client: ClaudeClient,
    prompt_template: PromptTemplate,
    max_retries: u32,
    retry_delay_ms: u64,
    cost_per_1k_tokens: f64,
}

impl InvariantExtractor {
    pub fn new(config: &crate::InvariantExtractionConfig) -> Self {
        Self {
            claude_client: ClaudeClient::new(&config.claude_api_key, &config.claude_model),
            prompt_template: PromptTemplate::load("invariant_extraction.md"),
            max_retries: config.max_retries,
            retry_delay_ms: config.retry_delay_ms,
            cost_per_1k_tokens: config.cost_per_1k_tokens,
        }
    }

    pub async fn extract_invariants(
        &self,
        request: &ExtractInvariantsRequest,
        redacted_content: &str,
    ) -> Result<ExtractionResult> {
        // Build the prompt from template
        let prompt = self.build_prompt(request, redacted_content);
        
        // Call Claude API
        let (response_text, input_tokens, output_tokens) = self.claude_client
            .generate_response(&prompt, self.max_retries, self.retry_delay_ms)
            .await?;

        // Parse the response
        let claude_response: ClaudeInvariantResponse = serde_json::from_str(&response_text)
            .map_err(|e| anyhow::anyhow!("Failed to parse Claude response: {}", e))?;

        // Convert to protobuf format
        let invariants: Vec<ExtractedInvariant> = claude_response.invariants
            .into_iter()
            .map(|raw_inv| self.convert_invariant(raw_inv))
            .collect();

        // Calculate cost
        let estimated_cost = self.claude_client.estimate_cost(
            input_tokens,
            output_tokens,
            self.cost_per_1k_tokens
        );

        let token_usage = TokenUsage {
            input_tokens: input_tokens as i32,
            output_tokens: output_tokens as i32,
            total_tokens: (input_tokens + output_tokens) as i32,
            estimated_cost_usd: estimated_cost,
        };

        Ok(ExtractionResult {
            invariants,
            token_usage: Some(token_usage),
        })
    }

    fn build_prompt(&self, request: &ExtractInvariantsRequest, redacted_content: &str) -> String {
        let mut template_vars = std::collections::HashMap::new();
        template_vars.insert("source_system".to_string(), request.source_system.clone());
        template_vars.insert("title".to_string(), request.title.clone());
        template_vars.insert("document_id".to_string(), request.document_id.clone());
        template_vars.insert("content".to_string(), redacted_content.to_string());

        self.prompt_template.render(&template_vars)
    }

    fn convert_invariant(&self, raw: RawExtractedInvariant) -> ExtractedInvariant {
        let variables: Vec<Variable> = raw.variables
            .into_iter()
            .map(|raw_var| Variable {
                name: raw_var.name,
                r#type: raw_var.var_type,
                description: raw_var.description,
                unit: raw_var.unit,
                constraints: raw_var.constraints,
            })
            .collect();

        let priority = match raw.priority.to_uppercase().as_str() {
            "CRITICAL" => Priority::Critical as i32,
            "HIGH" => Priority::High as i32,
            "MEDIUM" => Priority::Medium as i32,
            "LOW" => Priority::Low as i32,
            _ => Priority::Unspecified as i32,
        };

        ExtractedInvariant {
            description: raw.description,
            formal_expression: raw.formal_expression,
            natural_language: raw.natural_language,
            variables,
            units: raw.units,
            confidence_score: raw.confidence_score,
            tags: raw.tags,
            priority,
            extraction_metadata: None, // Will be set by the service
        }
    }
}

pub struct ExtractionResult {
    pub invariants: Vec<ExtractedInvariant>,
    pub token_usage: Option<TokenUsage>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::nlp::v1::ExtractInvariantsRequest;

    #[test]
    fn test_priority_conversion() {
        let extractor = InvariantExtractor::new(&crate::InvariantExtractionConfig::default());
        
        let raw_inv = RawExtractedInvariant {
            description: "Test".to_string(),
            formal_expression: "x > 0".to_string(),
            natural_language: "x is positive".to_string(),
            variables: vec![],
            units: std::collections::HashMap::new(),
            confidence_score: 0.9,
            tags: vec![],
            priority: "HIGH".to_string(),
        };

        let converted = extractor.convert_invariant(raw_inv);
        assert_eq!(converted.priority, Priority::High as i32);
    }

    #[test]
    fn test_prompt_building() {
        let extractor = InvariantExtractor::new(&crate::InvariantExtractionConfig::default());
        
        let request = ExtractInvariantsRequest {
            document_id: "test-123".to_string(),
            content: "Test content".to_string(),
            title: "Test Document".to_string(),
            source_system: "jira".to_string(),
            invariant_types: vec![],
            confidence_threshold: 0.5,
        };

        let prompt = extractor.build_prompt(&request, "Redacted content");
        assert!(prompt.contains("jira"));
        assert!(prompt.contains("Test Document"));
        assert!(prompt.contains("test-123"));
        assert!(prompt.contains("Redacted content"));
    }
} 