use std::error::Error;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
struct ClaudeRequest {
    model: String,
    max_tokens: u32,
    temperature: f32,
    messages: Vec<ClaudeMessage>,
    tools: Option<Vec<ClaudeTool>>,
    tool_choice: Option<String>,
    seed: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ClaudeTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: ClaudeFunction,
}

#[derive(Debug, Serialize)]
struct ClaudeFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContent>,
    usage: ClaudeUsage,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClaudeContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
    tool_calls: Option<Vec<ClaudeToolCall>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClaudeToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ClaudeFunctionCall,
}

#[derive(Debug, Deserialize)]
struct ClaudeFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    input_tokens: u32,
    output_tokens: u32,
}

pub struct ClaudeClient {
    api_key: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    http_client: Client,
    base_url: String,
}

impl ClaudeClient {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens: 8000,
            temperature: 0.0,
            http_client: Client::new(),
            base_url: "https://api.anthropic.com/v1/messages".to_string(),
        }
    }

    pub async fn generate_lean_theorem(
        &self,
        invariant: &str,
        proof_strategy: &str,
        seed: u64,
    ) -> Result<(String, u32, u32), Box<dyn Error>> {
        let prompt = self.build_lean_prompt(invariant, proof_strategy);
        
        let tools = vec![
            ClaudeTool {
                tool_type: "function".to_string(),
                function: ClaudeFunction {
                    name: "generate_lean_theorem".to_string(),
                    description: "Generate a Lean 4 theorem from an invariant specification".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "theorem_name": {
                                "type": "string",
                                "description": "The name of the theorem in Lean"
                            },
                            "lean_code": {
                                "type": "string",
                                "description": "The complete Lean 4 code for the theorem"
                            },
                            "imports": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Required Lean imports"
                            },
                            "dependencies": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "Additional Lean dependencies needed"
                            },
                            "proof_strategy": {
                                "type": "string",
                                "description": "The proof strategy used"
                            }
                        },
                        "required": ["theorem_name", "lean_code"]
                    }),
                },
            },
        ];

        let request = ClaudeRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            tools: Some(tools),
            tool_choice: Some("auto".to_string()),
            seed: Some(seed),
        };

        let response = self.make_request(&request).await?;
        Ok((response.0, response.1, response.2))
    }

    pub async fn generate_proof(
        &self,
        theorem_code: &str,
        proof_strategy: &str,
        seed: u64,
    ) -> Result<(String, u32, u32), Box<dyn Error>> {
        let prompt = self.build_proof_prompt(theorem_code, proof_strategy);
        
        let tools = vec![
            ClaudeTool {
                tool_type: "function".to_string(),
                function: ClaudeFunction {
                    name: "complete_proof".to_string(),
                    description: "Complete the proof for a Lean theorem".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": {
                            "proof_code": {
                                "type": "string",
                                "description": "The complete proof code for the theorem"
                            },
                            "proof_strategy": {
                                "type": "string",
                                "description": "The proof strategy used"
                            },
                            "tactics_used": {
                                "type": "array",
                                "items": {"type": "string"},
                                "description": "List of Lean tactics used in the proof"
                            },
                            "difficulty": {
                                "type": "string",
                                "enum": ["easy", "medium", "hard"],
                                "description": "Difficulty level of the proof"
                            }
                        },
                        "required": ["proof_code", "proof_strategy"]
                    }),
                },
            },
        ];

        let request = ClaudeRequest {
            model: self.model.clone(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            messages: vec![ClaudeMessage {
                role: "user".to_string(),
                content: prompt,
            }],
            tools: Some(tools),
            tool_choice: Some("auto".to_string()),
            seed: Some(seed),
        };

        let response = self.make_request(&request).await?;
        Ok((response.0, response.1, response.2))
    }

    fn build_lean_prompt(&self, invariant: &str, proof_strategy: &str) -> String {
        format!(
            r#"You are an expert Lean 4 theorem prover. Convert the following invariant specification into a Lean 4 theorem.

Invariant Specification:
{}

Proof Strategy: {}

Requirements:
1. Generate a complete, compilable Lean 4 theorem
2. Include all necessary imports
3. Use proper Lean 4 syntax and conventions
4. Make the theorem name descriptive and follow Lean naming conventions
5. Include type annotations where helpful
6. Use the specified proof strategy

Generate the theorem using the generate_lean_theorem function."#,
            invariant, proof_strategy
        )
    }

    fn build_proof_prompt(&self, theorem_code: &str, proof_strategy: &str) -> String {
        format!(
            r#"You are an expert Lean 4 theorem prover. Complete the proof for the following Lean theorem.

Theorem Code:
{}

Proof Strategy: {}

Requirements:
1. Complete the proof using Lean 4 tactics
2. Follow the specified proof strategy
3. Make the proof clear and readable
4. Use appropriate tactics for the theorem type
5. Ensure the proof compiles and runs successfully

Complete the proof using the complete_proof function."#,
            theorem_code, proof_strategy
        )
    }

    async fn make_request(
        &self,
        request: &ClaudeRequest,
    ) -> Result<(String, u32, u32), Box<dyn Error>> {
        let response = self
            .http_client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(format!("Claude API error: {}", error_text).into());
        }

        let claude_response: ClaudeResponse = response.json().await?;
        
        // Extract the tool call response
        let mut lean_code = String::new();
        for content in &claude_response.content {
            if let Some(tool_calls) = &content.tool_calls {
                for tool_call in tool_calls {
                    if tool_call.function.name == "generate_lean_theorem" || 
                       tool_call.function.name == "complete_proof" {
                        let args: Value = serde_json::from_str(&tool_call.function.arguments)?;
                        
                        if tool_call.function.name == "generate_lean_theorem" {
                            lean_code = args["lean_code"].as_str().unwrap_or("").to_string();
                        } else {
                            lean_code = args["proof_code"].as_str().unwrap_or("").to_string();
                        }
                        break;
                    }
                }
            }
        }

        if lean_code.is_empty() {
            return Err("No valid tool call response received".into());
        }

        Ok((
            lean_code,
            claude_response.usage.input_tokens,
            claude_response.usage.output_tokens,
        ))
    }

    pub fn estimate_cost(&self, input_tokens: u32, output_tokens: u32, cost_per_1k_tokens: f64) -> f64 {
        let total_tokens = input_tokens + output_tokens;
        (total_tokens as f64 / 1000.0) * cost_per_1k_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_claude_client_creation() {
        let client = ClaudeClient::new("test-key", "claude-3-opus-20240229");
        assert_eq!(client.model, "claude-3-opus-20240229");
        assert_eq!(client.temperature, 0.0);
    }

    #[test]
    fn test_cost_estimation() {
        let client = ClaudeClient::new("test-key", "claude-3-opus-20240229");
        let cost = client.estimate_cost(1000, 500, 0.015);
        assert_eq!(cost, 0.0225); // (1500 / 1000) * 0.015
    }

    #[test]
    fn test_prompt_building() {
        let client = ClaudeClient::new("test-key", "claude-3-opus-20240229");
        let lean_prompt = client.build_lean_prompt("test invariant", "induction");
        assert!(lean_prompt.contains("test invariant"));
        assert!(lean_prompt.contains("induction"));
        
        let proof_prompt = client.build_proof_prompt("test theorem", "tactics");
        assert!(proof_prompt.contains("test theorem"));
        assert!(proof_prompt.contains("tactics"));
    }
} 