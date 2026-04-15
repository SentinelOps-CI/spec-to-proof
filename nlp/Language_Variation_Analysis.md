# Language Variation Handling in Spec-to-Proof System

The Spec-to-Proof system is designed to handle language variations in specifications while maintaining semantic consistency. This document explains how the system achieves this and provides detailed examples.

Note: examples in this document are illustrative normalization outcomes used for design discussion; exact extraction output depends on model version, prompt revisions, and post-processing rules.

## Core Problem

When multiple authors write specifications for the same system, they often use different:
- Phrasing and terminology
- Variable naming conventions
- Unit representations
- Mathematical notation styles
- Business vs. technical language

Without proper normalization, these variations could lead to:
- Duplicate invariants with different names
- Inconsistent formal expressions
- Difficulty in proving system properties
- Maintenance challenges

## Solution Architecture

The system uses a multi-layered approach to handle language variations:

### 1. NLP-Based Invariant Extraction
- Uses Claude AI to extract invariants from natural language
- Understands semantic meaning regardless of phrasing
- Identifies core requirements from various expressions

### 2. Post-Processing Normalization
- Standardizes variable names using regex patterns
- Normalizes units to consistent formats
- Converts mathematical notation to standard operators

### 3. Content-Based Addressing
- Uses SHA256 hashes to identify semantically equivalent content
- Prevents duplicate processing of the same invariant
- Enables efficient caching and retrieval

## Examples

### Example 1: User Authentication System

#### Input Variations

**Direct Statement:**
```
User ID must be a positive integer
Password length must be at least 8 characters
Response time must be under 500 milliseconds
Error rate must be less than 1%
```

**Alternative Phrasing:**
```
User identifier should be greater than zero
Password must contain no fewer than 8 characters
Response time cannot exceed 500 milliseconds
Error rate should remain below 1 percent
```

**Verbose Language:**
```
It is required that the user identifier be a positive integer value
The password must have a minimum length of at least 8 characters
The system response time must be maintained under 500 milliseconds
The error rate must be kept below 1 percent at all times
```

**Technical Jargon:**
```
UID shall be > 0
PWD length >= 8 chars
RT < 500ms
ER < 1%
```

**Business Language:**
```
User identification numbers are required to be positive
Passwords need to be at least 8 characters in length
System response times should not exceed 500 milliseconds
Error rates are expected to stay under 1 percent
```

#### Normalized Output

All variations produce the same normalized invariants:

```json
{
  "invariants": [
    {
      "description": "User ID must be a positive integer",
      "formal_expression": "user_id > 0",
      "variables": [
        {
          "name": "user_id",
          "type": "integer",
          "unit": "items"
        }
      ],
      "units": {"user_id": "items"},
      "confidence_score": 0.9,
      "priority": "HIGH"
    },
    {
      "description": "Password length must be at least 8 characters",
      "formal_expression": "password_length >= 8",
      "variables": [
        {
          "name": "password_length",
          "type": "integer",
          "unit": "items"
        }
      ],
      "units": {"password_length": "items"},
      "confidence_score": 0.9,
      "priority": "CRITICAL"
    },
    {
      "description": "Response time must be under 500 milliseconds",
      "formal_expression": "response_time < 500",
      "variables": [
        {
          "name": "response_time",
          "type": "integer",
          "unit": "milliseconds"
        }
      ],
      "units": {"response_time": "milliseconds"},
      "confidence_score": 0.9,
      "priority": "HIGH"
    },
    {
      "description": "Error rate must be less than 1%",
      "formal_expression": "error_rate < 0.01",
      "variables": [
        {
          "name": "error_rate",
          "type": "float",
          "unit": "ratio"
        }
      ],
      "units": {"error_rate": "ratio"},
      "confidence_score": 0.9,
      "priority": "HIGH"
    }
  ]
}
```

### Example 2: Payment Processing System

#### Input Variations

**Technical Specification:**
```
Transaction amount > 0
Currency code ∈ valid_iso_codes
Processing time < 2000ms
Success rate > 99.5%
```

**Business Requirements:**
```
All monetary transactions must have positive values
Currency codes must conform to ISO 4217 standards
Payment processing must complete within 2 seconds
System reliability must exceed 99.5 percent
```

**Regulatory Language:**
```
Transaction amounts are required to be greater than zero
Currency codes shall be valid three-letter ISO codes
Processing times cannot exceed 2000 milliseconds
Success rates must remain above 99.5 percent
```

#### Normalized Output

```json
{
  "invariants": [
    {
      "description": "Transaction amount must be positive",
      "formal_expression": "transaction_amount > 0",
      "variables": [
        {
          "name": "transaction_amount",
          "type": "decimal",
          "unit": "currency_units"
        }
      ],
      "units": {"transaction_amount": "currency_units"},
      "confidence_score": 0.95,
      "priority": "CRITICAL"
    },
    {
      "description": "Currency code must be valid ISO code",
      "formal_expression": "currency_code in valid_iso_codes",
      "variables": [
        {
          "name": "currency_code",
          "type": "string",
          "unit": "code"
        }
      ],
      "units": {"currency_code": "code"},
      "confidence_score": 0.9,
      "priority": "HIGH"
    },
    {
      "description": "Processing time must be under 2 seconds",
      "formal_expression": "processing_time < 2000",
      "variables": [
        {
          "name": "processing_time",
          "type": "integer",
          "unit": "milliseconds"
        }
      ],
      "units": {"processing_time": "milliseconds"},
      "confidence_score": 0.9,
      "priority": "HIGH"
    },
    {
      "description": "Success rate must exceed 99.5%",
      "formal_expression": "success_rate > 0.995",
      "variables": [
        {
          "name": "success_rate",
          "type": "float",
          "unit": "ratio"
        }
      ],
      "units": {"success_rate": "ratio"},
      "confidence_score": 0.9,
      "priority": "CRITICAL"
    }
  ]
}
```

## Normalization Patterns

### Variable Name Normalization

The system uses regex patterns to normalize common variable names:

```rust
variable_name_patterns = [
    (r"(?i)user\s*id", "user_id"),
    (r"(?i)system\s*status", "system_status"),
    (r"(?i)request\s*count", "request_count"),
    (r"(?i)response\s*time", "response_time"),
    (r"(?i)error\s*rate", "error_rate"),
    (r"(?i)memory\s*usage", "memory_usage"),
    (r"(?i)cpu\s*usage", "cpu_usage"),
    (r"(?i)connection\s*count", "connection_count"),
]
```

**Examples:**
- "User ID" → "user_id"
- "user_id" → "user_id"
- "USER_ID" → "user_id"
- "userId" → "user_id"
- "user-id" → "user_id"
- "User Identifier" → "user_identifier"

### Unit Standardization

Units are normalized to consistent formats:

```rust
unit_standardization = {
    // Time units
    "ms" => "milliseconds",
    "milliseconds" => "milliseconds",
    "s" => "seconds",
    "seconds" => "seconds",
    
    // Size units
    "B" => "bytes",
    "KB" => "kilobytes",
    "MB" => "megabytes",
    "GB" => "gigabytes",
    
    // Count units
    "count" => "items",
    "requests" => "items",
    "connections" => "items",
    
    // Percentage units
    "%" => "ratio",
    "percent" => "ratio",
    "percentage" => "ratio",
}
```

**Examples:**
- "ms" → "milliseconds"
- "500ms" → "500 milliseconds"
- "1%" → "ratio"
- "99.5%" → "99.5 ratio"

### Mathematical Notation Normalization

Mathematical expressions are normalized to standard operators:

```rust
replacements = {
    '≤' => '<=',
    '≥' => '>=',
    '≠' => '!=',
    '∧' => '&&',
    '∨' => '||',
    '¬' => '!',
    '∀' => 'forall',
    '∃' => 'exists',
    '∈' => 'in',
    '∉' => 'not_in',
    '⊆' => 'subset',
    '⊂' => 'proper_subset',
}
```

**Examples:**
- "x ≤ 10" → "x <= 10"
- "y ≥ 0" → "y >= 0"
- "z ∈ valid_values" → "z in valid_values"

## Benefits of This Approach

### 1. Consistency Across Authors
- Different team members can write specifications in their preferred style
- System automatically normalizes to consistent format
- Reduces confusion and maintenance overhead

### 2. Semantic Preservation
- Core meaning is preserved during normalization
- Mathematical relationships remain intact
- Business logic is not altered

### 3. Efficient Processing
- Content-based addressing prevents duplicate work
- Caching improves performance
- Standardized format enables automated analysis

### 4. Proof Generation
- Consistent variable names enable theorem proving
- Standardized units support mathematical reasoning
- Normalized expressions work with formal verification tools

## Implementation Details

### Post-Processing Pipeline

```rust
pub async fn process_invariants(
    &self,
    invariants: Vec<ExtractedInvariant>,
) -> Result<Vec<ExtractedInvariant>, Box<dyn Error>> {
    let mut processed_invariants = Vec::new();

    for mut invariant in invariants {
        // Normalize variable names
        for variable in &mut invariant.variables {
            variable.name = self.normalize_variable_name(&variable.name);
        }

        // Standardize units
        let mut normalized_units = HashMap::new();
        for (var_name, unit) in &invariant.units {
            let normalized_unit = self.standardize_unit(unit);
            normalized_units.insert(var_name.clone(), normalized_unit);
        }
        invariant.units = normalized_units;

        // Normalize formal expression
        invariant.formal_expression = 
            self.normalize_formal_expression(&invariant.formal_expression);

        processed_invariants.push(invariant);
    }

    Ok(processed_invariants)
}
```

### Cache Key Generation

```rust
fn generate_cache_key(&self, request: &ExtractInvariantsRequest) -> String {
    let content_hash = calculate_sha256(&request.content);
    let title_hash = calculate_sha256(&request.title);
    let source_hash = calculate_sha256(&request.source_system);
    
    format!("{}_{}_{}_{}", 
        content_hash, title_hash, source_hash, 
        request.confidence_threshold)
}
```

## Testing and Validation

The system includes comprehensive tests to ensure normalization works correctly:

```rust
#[tokio::test]
async fn test_language_variation_handling() {
    let test_cases = vec![
        ("direct-statement", "User ID must be positive"),
        ("alternative-phrasing", "User identifier should be greater than zero"),
        ("verbose-language", "It is required that the user identifier be positive"),
        ("technical-jargon", "UID > 0"),
        ("business-language", "User identification numbers are required to be positive"),
    ];
    
    for (case_id, content) in test_cases {
        let invariants = extract_invariants(content).await?;
        verify_normalized_invariants(&invariants, case_id);
    }
}
```