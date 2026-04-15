use std::collections::HashMap;
use std::fs;

pub struct PromptTemplate {
    template: String,
}

impl PromptTemplate {
    pub fn load(template_name: &str) -> Self {
        let template_path = format!("prompts/{}", template_name);
        let template_content = fs::read_to_string(&template_path)
            .unwrap_or_else(|_| {
                tracing::warn!("Failed to load prompt template: {}, using default", template_name);
                Self::get_default_template()
            });

        Self {
            template: template_content,
        }
    }

    pub fn render(&self, variables: &HashMap<String, String>) -> String {
        let mut result = self.template.clone();

        for (key, value) in variables {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value.as_str());
        }

        result
    }

    fn get_default_template() -> String {
        r#"You are an expert software engineer and formal verification specialist. Your task is to extract formal invariants from software specification documents.

## Instructions

1. **Analyze the specification content** and identify potential invariants
2. **Extract formal mathematical expressions** for each invariant
3. **Normalize variable names** using consistent naming conventions
4. **Identify units** for all variables where applicable
5. **Assign confidence scores** based on clarity and completeness
6. **Categorize by priority** (LOW, MEDIUM, HIGH, CRITICAL)

## Output Format

Return a JSON array of invariants with the following structure:

```json
{
  "invariants": [
    {
      "description": "Human-readable description of the invariant",
      "formal_expression": "Mathematical expression using standard notation",
      "natural_language": "Natural language description",
      "variables": [
        {
          "name": "normalized_variable_name",
          "type": "data_type",
          "description": "Variable description",
          "unit": "unit_if_applicable",
          "constraints": ["constraint1", "constraint2"]
        }
      ],
      "units": {
        "variable_name": "unit"
      },
      "confidence_score": 0.95,
      "tags": ["safety", "data_integrity"],
      "priority": "HIGH"
    }
  ]
}
```

## Content to Analyze

```
{{content}}
```

## Response

Provide only the JSON response with no additional text or explanation."#.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_rendering() {
        let template = PromptTemplate {
            template: "Hello {{name}}, you are {{age}} years old.".to_string(),
        };

        let mut variables = HashMap::new();
        variables.insert("name".to_string(), "Alice".to_string());
        variables.insert("age".to_string(), "30".to_string());

        let result = template.render(&variables);
        assert_eq!(result, "Hello Alice, you are 30 years old.");
    }

    #[test]
    fn test_template_with_missing_variables() {
        let template = PromptTemplate {
            template: "Hello {{name}}, you are {{age}} years old.".to_string(),
        };

        let mut variables = HashMap::new();
        variables.insert("name".to_string(), "Alice".to_string());

        let result = template.render(&variables);
        assert_eq!(result, "Hello Alice, you are {{age}} years old.");
    }

    #[test]
    fn test_default_template_loading() {
        let template = PromptTemplate::load("nonexistent_template.md");
        assert!(!template.template.is_empty());
        assert!(template.template.contains("JSON"));
    }
} 