use std::collections::HashMap;

use anyhow::Result;
use regex::Regex;
use crate::proto::nlp::v1::ExtractedInvariant;

pub struct PostProcessor {
    variable_name_patterns: Vec<(Regex, String)>,
    unit_standardization: HashMap<String, String>,
}

impl PostProcessor {
    pub fn new() -> Self {
        // Common variable name normalizations
        let variable_name_patterns = vec![
            (
                Regex::new(r"(?i)user\s*id").unwrap(),
                "user_id".to_string(),
            ),
            (
                Regex::new(r"(?i)system\s*status").unwrap(),
                "system_status".to_string(),
            ),
            (
                Regex::new(r"(?i)request\s*count").unwrap(),
                "request_count".to_string(),
            ),
            (
                Regex::new(r"(?i)response\s*time").unwrap(),
                "response_time".to_string(),
            ),
            (
                Regex::new(r"(?i)error\s*rate").unwrap(),
                "error_rate".to_string(),
            ),
            (
                Regex::new(r"(?i)memory\s*usage").unwrap(),
                "memory_usage".to_string(),
            ),
            (
                Regex::new(r"(?i)cpu\s*usage").unwrap(),
                "cpu_usage".to_string(),
            ),
            (
                Regex::new(r"(?i)connection\s*count").unwrap(),
                "connection_count".to_string(),
            ),
        ];

        let mut unit_standardization = HashMap::new();
        
        // Time units
        unit_standardization.insert("ms".to_string(), "milliseconds".to_string());
        unit_standardization.insert("milliseconds".to_string(), "milliseconds".to_string());
        unit_standardization.insert("s".to_string(), "seconds".to_string());
        unit_standardization.insert("seconds".to_string(), "seconds".to_string());
        unit_standardization.insert("min".to_string(), "minutes".to_string());
        unit_standardization.insert("minutes".to_string(), "minutes".to_string());
        
        // Size units (keys are lowercased in standardize_unit)
        unit_standardization.insert("B".to_string(), "bytes".to_string());
        unit_standardization.insert("bytes".to_string(), "bytes".to_string());
        unit_standardization.insert("KB".to_string(), "kilobytes".to_string());
        unit_standardization.insert("kb".to_string(), "kilobytes".to_string());
        unit_standardization.insert("kilobytes".to_string(), "kilobytes".to_string());
        unit_standardization.insert("MB".to_string(), "megabytes".to_string());
        unit_standardization.insert("mb".to_string(), "megabytes".to_string());
        unit_standardization.insert("megabytes".to_string(), "megabytes".to_string());
        unit_standardization.insert("GB".to_string(), "gigabytes".to_string());
        unit_standardization.insert("gb".to_string(), "gigabytes".to_string());
        unit_standardization.insert("gigabytes".to_string(), "gigabytes".to_string());
        
        // Count units
        unit_standardization.insert("count".to_string(), "items".to_string());
        unit_standardization.insert("items".to_string(), "items".to_string());
        unit_standardization.insert("requests".to_string(), "items".to_string());
        unit_standardization.insert("connections".to_string(), "items".to_string());
        
        // Percentage units
        unit_standardization.insert("%".to_string(), "ratio".to_string());
        unit_standardization.insert("percent".to_string(), "ratio".to_string());
        unit_standardization.insert("percentage".to_string(), "ratio".to_string());

        Self {
            variable_name_patterns,
            unit_standardization,
        }
    }

    pub async fn process_invariants(
        &self,
        invariants: Vec<ExtractedInvariant>,
    ) -> Result<Vec<ExtractedInvariant>> {
        let mut processed_invariants = Vec::new();

        for mut invariant in invariants {
            // Normalize variable names
            for variable in &mut invariant.variables {
                variable.name = self.normalize_variable_name(&variable.name);
            }

            // Standardize units; keys follow the same normalization as variable names
            let mut normalized_units = HashMap::new();
            for (var_name, unit) in &invariant.units {
                let key = self.normalize_variable_name(var_name);
                normalized_units.insert(key, self.standardize_unit(unit));
            }
            invariant.units = normalized_units;

            // Also normalize units in variable definitions
            for variable in &mut invariant.variables {
                variable.unit = self.standardize_unit(&variable.unit);
            }

            // Normalize formal expression
            invariant.formal_expression = self.normalize_formal_expression(&invariant.formal_expression);

            processed_invariants.push(invariant);
        }

        Ok(processed_invariants)
    }

    fn normalize_variable_name(&self, name: &str) -> String {
        let mut normalized = name.to_lowercase();
        
        // Apply regex patterns
        for (pattern, replacement) in &self.variable_name_patterns {
            normalized = pattern.replace_all(&normalized, replacement).to_string();
        }

        // Replace spaces and special characters with underscores
        normalized = normalized
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect::<String>();

        // Remove consecutive underscores
        while normalized.contains("__") {
            normalized = normalized.replace("__", "_");
        }

        // Remove leading/trailing underscores
        normalized = normalized.trim_matches('_').to_string();

        if normalized.is_empty() {
            "unnamed_variable".to_string()
        } else {
            normalized
        }
    }

    fn standardize_unit(&self, unit: &str) -> String {
        let normalized = unit.to_lowercase().trim().to_string();
        self.unit_standardization.get(&normalized)
            .cloned()
            .unwrap_or(normalized)
    }

    fn normalize_formal_expression(&self, expression: &str) -> String {
        let mut normalized = expression.to_string();
        
        // Normalize common mathematical operators
        normalized = normalized.replace("≤", "<=");
        normalized = normalized.replace("≥", ">=");
        normalized = normalized.replace("≠", "!=");
        normalized = normalized.replace("∧", "&&");
        normalized = normalized.replace("∨", "||");
        normalized = normalized.replace("¬", "!");
        normalized = normalized.replace("∀", "forall");
        normalized = normalized.replace("∃", "exists");
        normalized = normalized.replace("∈", "in");
        normalized = normalized.replace("∉", "not_in");
        normalized = normalized.replace("⊆", "subset");
        normalized = normalized.replace("⊂", "proper_subset");
        
        // Normalize variable names in expressions
        for (pattern, replacement) in &self.variable_name_patterns {
            normalized = pattern.replace_all(&normalized, replacement).to_string();
        }

        normalized
    }
}

impl Default for PostProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::nlp::v1::{Variable, Priority};

    #[tokio::test]
    async fn test_variable_name_normalization() {
        let processor = PostProcessor::new();
        
        assert_eq!(processor.normalize_variable_name("User ID"), "user_id");
        assert_eq!(processor.normalize_variable_name("System Status"), "system_status");
        assert_eq!(processor.normalize_variable_name("Request Count"), "request_count");
        assert_eq!(processor.normalize_variable_name("Response Time"), "response_time");
        assert_eq!(processor.normalize_variable_name("Error Rate"), "error_rate");
        assert_eq!(processor.normalize_variable_name("Memory Usage"), "memory_usage");
        assert_eq!(processor.normalize_variable_name("CPU Usage"), "cpu_usage");
        assert_eq!(processor.normalize_variable_name("Connection Count"), "connection_count");
    }

    #[tokio::test]
    async fn test_unit_standardization() {
        let processor = PostProcessor::new();
        
        assert_eq!(processor.standardize_unit("ms"), "milliseconds");
        assert_eq!(processor.standardize_unit("seconds"), "seconds");
        assert_eq!(processor.standardize_unit("KB"), "kilobytes");
        assert_eq!(processor.standardize_unit("MB"), "megabytes");
        assert_eq!(processor.standardize_unit("%"), "ratio");
        assert_eq!(processor.standardize_unit("count"), "items");
    }

    #[tokio::test]
    async fn test_formal_expression_normalization() {
        let processor = PostProcessor::new();
        
        assert_eq!(processor.normalize_formal_expression("x ≤ 10"), "x <= 10");
        assert_eq!(processor.normalize_formal_expression("y ≥ 0"), "y >= 0");
        assert_eq!(processor.normalize_formal_expression("z ≠ null"), "z != null");
        assert_eq!(processor.normalize_formal_expression("a ∧ b"), "a && b");
        assert_eq!(processor.normalize_formal_expression("c ∨ d"), "c || d");
    }

    #[tokio::test]
    async fn test_invariant_processing() {
        let processor = PostProcessor::new();
        
        let invariant = ExtractedInvariant {
            description: "Test invariant".to_string(),
            formal_expression: "User ID > 0".to_string(),
            natural_language: "User ID must be positive".to_string(),
            variables: vec![
                Variable {
                    name: "User ID".to_string(),
                    r#type: "integer".to_string(),
                    description: "User identifier".to_string(),
                    unit: "count".to_string(),
                    constraints: vec![],
                }
            ],
            units: {
                let mut map = HashMap::new();
                map.insert("User ID".to_string(), "count".to_string());
                map
            },
            confidence_score: 0.9,
            tags: vec!["test".to_string()],
            priority: Priority::High as i32,
            extraction_metadata: None,
        };

        let processed = processor.process_invariants(vec![invariant]).await.unwrap();
        let processed_inv = &processed[0];
        
        assert_eq!(processed_inv.variables[0].name, "user_id");
        assert_eq!(processed_inv.variables[0].unit, "items");
        assert_eq!(processed_inv.units["user_id"], "items");
        assert_eq!(processed_inv.formal_expression, "user_id > 0");
    }
} 