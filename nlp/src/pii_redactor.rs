use std::collections::HashSet;
use regex::Regex;

pub struct PiiRedactor {
    email_pattern: Regex,
    phone_pattern: Regex,
    ssn_pattern: Regex,
    credit_card_pattern: Regex,
    ip_address_pattern: Regex,
    url_pattern: Regex,
    name_patterns: Vec<Regex>,
}

impl PiiRedactor {
    pub fn new() -> Self {
        Self {
            email_pattern: Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap(),
            phone_pattern: Regex::new(r"\b(\+\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b").unwrap(),
            ssn_pattern: Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(),
            credit_card_pattern: Regex::new(r"\b\d{4}[-.\s]?\d{4}[-.\s]?\d{4}[-.\s]?\d{4}\b").unwrap(),
            ip_address_pattern: Regex::new(r"\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b").unwrap(),
            url_pattern: Regex::new(r"https?://[^\s]+").unwrap(),
            name_patterns: vec![
                Regex::new(r"\b[A-Z][a-z]+ [A-Z][a-z]+\b").unwrap(), // First Last
                Regex::new(r"\b[A-Z][a-z]+ [A-Z]\. [A-Z][a-z]+\b").unwrap(), // First M. Last
            ],
        }
    }

    pub fn redact(&self, content: &str) -> (String, bool, Vec<String>) {
        let mut redacted_content = content.to_string();
        let mut pii_detected = false;
        let mut redacted_fields = HashSet::new();

        // Redact email addresses
        if self.email_pattern.is_match(&redacted_content) {
            redacted_content = self.email_pattern
                .replace_all(&redacted_content, "[EMAIL_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("email".to_string());
        }

        // Redact phone numbers
        if self.phone_pattern.is_match(&redacted_content) {
            redacted_content = self.phone_pattern
                .replace_all(&redacted_content, "[PHONE_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("phone".to_string());
        }

        // Redact SSNs
        if self.ssn_pattern.is_match(&redacted_content) {
            redacted_content = self.ssn_pattern
                .replace_all(&redacted_content, "[SSN_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("ssn".to_string());
        }

        // Redact credit card numbers
        if self.credit_card_pattern.is_match(&redacted_content) {
            redacted_content = self.credit_card_pattern
                .replace_all(&redacted_content, "[CREDIT_CARD_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("credit_card".to_string());
        }

        // Redact IP addresses
        if self.ip_address_pattern.is_match(&redacted_content) {
            redacted_content = self.ip_address_pattern
                .replace_all(&redacted_content, "[IP_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("ip_address".to_string());
        }

        // Redact URLs (but keep domain names for context)
        if self.url_pattern.is_match(&redacted_content) {
            redacted_content = self.url_pattern
                .replace_all(&redacted_content, "[URL_REDACTED]")
                .to_string();
            pii_detected = true;
            redacted_fields.insert("url".to_string());
        }

        // Redact names (but be conservative to avoid false positives)
        for pattern in &self.name_patterns {
            if pattern.is_match(&redacted_content) {
                redacted_content = pattern
                    .replace_all(&redacted_content, "[NAME_REDACTED]")
                    .to_string();
                pii_detected = true;
                redacted_fields.insert("name".to_string());
            }
        }

        // Additional context-aware redactions
        redacted_content = self.redact_contextual_pii(&redacted_content, &mut pii_detected, &mut redacted_fields);

        (redacted_content, pii_detected, redacted_fields.into_iter().collect())
    }

    fn redact_contextual_pii(&self, content: &str, pii_detected: &mut bool, redacted_fields: &mut HashSet<String>) -> String {
        let mut redacted = content.to_string();

        // Redact common PII patterns in technical documents
        let patterns = vec![
            (Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap(), "[EMAIL_REDACTED]"),
            (Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(), "[SSN_REDACTED]"),
            (Regex::new(r"\b[A-Z]{2}\d{2}\s?\d{4}\s?\d{4}\s?\d{4}\s?\d{2}\b").unwrap(), "[PASSPORT_REDACTED]"),
            (Regex::new(r"\b\d{1,2}/\d{1,2}/\d{2,4}\b").unwrap(), "[DATE_REDACTED]"),
        ];

        for (pattern, replacement) in patterns {
            if pattern.is_match(&redacted) {
                redacted = pattern.replace_all(&redacted, replacement).to_string();
                *pii_detected = true;
                redacted_fields.insert("contextual_pii".to_string());
            }
        }

        redacted
    }

    pub fn is_pii_present(&self, content: &str) -> bool {
        self.email_pattern.is_match(content) ||
        self.phone_pattern.is_match(content) ||
        self.ssn_pattern.is_match(content) ||
        self.credit_card_pattern.is_match(content) ||
        self.ip_address_pattern.is_match(content) ||
        self.url_pattern.is_match(content) ||
        self.name_patterns.iter().any(|pattern| pattern.is_match(content))
    }
}

impl Default for PiiRedactor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_email_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Contact us at john.doe@example.com for support";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[EMAIL_REDACTED]"));
        assert!(!redacted.contains("john.doe@example.com"));
        assert!(fields.contains(&"email".to_string()));
    }

    #[test]
    fn test_phone_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Call us at (555) 123-4567";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[PHONE_REDACTED]"));
        assert!(!redacted.contains("(555) 123-4567"));
        assert!(fields.contains(&"phone".to_string()));
    }

    #[test]
    fn test_ssn_redaction() {
        let redactor = PiiRedactor::new();
        let content = "SSN: 123-45-6789";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[SSN_REDACTED]"));
        assert!(!redacted.contains("123-45-6789"));
        assert!(fields.contains(&"ssn".to_string()));
    }

    #[test]
    fn test_credit_card_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Card: 1234-5678-9012-3456";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[CREDIT_CARD_REDACTED]"));
        assert!(!redacted.contains("1234-5678-9012-3456"));
        assert!(fields.contains(&"credit_card".to_string()));
    }

    #[test]
    fn test_ip_address_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Server IP: 192.168.1.1";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[IP_REDACTED]"));
        assert!(!redacted.contains("192.168.1.1"));
        assert!(fields.contains(&"ip_address".to_string()));
    }

    #[test]
    fn test_url_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Visit https://example.com/path?token=secret";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[URL_REDACTED]"));
        assert!(!redacted.contains("https://example.com/path?token=secret"));
        assert!(fields.contains(&"url".to_string()));
    }

    #[test]
    fn test_name_redaction() {
        let redactor = PiiRedactor::new();
        let content = "Contact John Smith for details";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[NAME_REDACTED]"));
        assert!(!redacted.contains("John Smith"));
        assert!(fields.contains(&"name".to_string()));
    }

    #[test]
    fn test_no_pii_detection() {
        let redactor = PiiRedactor::new();
        let content = "This is a technical specification with no PII";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(!detected);
        assert_eq!(redacted, content);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_multiple_pii_types() {
        let redactor = PiiRedactor::new();
        let content = "Contact John Smith at john@example.com or call (555) 123-4567";
        let (redacted, detected, fields) = redactor.redact(content);
        
        assert!(detected);
        assert!(redacted.contains("[EMAIL_REDACTED]"));
        assert!(redacted.contains("[PHONE_REDACTED]"));
        assert!(redacted.contains("[NAME_REDACTED]"));
        assert!(fields.contains(&"email".to_string()));
        assert!(fields.contains(&"phone".to_string()));
        assert!(fields.contains(&"name".to_string()));
    }
} 