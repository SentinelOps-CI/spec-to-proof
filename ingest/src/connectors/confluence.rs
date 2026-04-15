use crate::{
    ConnectorConfig, OAuth2Token, DocumentMetadata,
    rate_limiter::RateLimiter, backoff::ExponentialBackoff,
};
use crate::proto::spec_to_proof::v1::SpecDocument;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluencePage {
    pub id: String,
    pub title: String,
    pub status: String,
    pub version: ConfluenceVersion,
    pub space: ConfluenceSpace,
    pub body: ConfluenceBody,
    pub _links: ConfluenceLinks,
    pub created_by: ConfluenceUser,
    pub created_date: String,
    pub last_modified_date: String,
    pub last_modified_by: ConfluenceUser,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceVersion {
    pub number: i32,
    pub message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceSpace {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceBody {
    pub storage: ConfluenceStorage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceStorage {
    pub value: String,
    pub representation: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceLinks {
    pub webui: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceUser {
    pub display_name: String,
    pub username: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceSearchResponse {
    pub results: Vec<ConfluencePage>,
    pub start: i32,
    pub limit: i32,
    pub size: i32,
    pub _links: ConfluenceSearchLinks,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfluenceSearchLinks {
    pub next: Option<String>,
}

pub struct ConfluenceConnector {
    config: ConnectorConfig,
    http_client: Client,
    rate_limiter: RateLimiter,
    backoff: ExponentialBackoff,
    last_sync_timestamp: Option<i64>,
}

impl ConfluenceConnector {
    pub fn new(config: ConnectorConfig) -> Self {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let rate_limiter = RateLimiter::new(
            (config.rate_limit_per_minute as f64 * 0.7) as u32, // 70% of quota
            std::time::Duration::from_secs(60),
        );

        Self {
            config,
            http_client,
            rate_limiter,
            backoff: ExponentialBackoff::new(),
            last_sync_timestamp: None,
        }
    }

    pub async fn poll_documents(&mut self, token: &OAuth2Token) -> Result<Vec<SpecDocument>, Box<dyn std::error::Error>> {
        self.rate_limiter.acquire().await?;

        let url = format!("{}/rest/api/content/search", self.config.base_url);
        let cql = self.build_cql_query();

        let response = self.backoff
            .execute_with_backoff(|| async {
                let response = self.http_client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", token.access_token))
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json")
                    .json(&serde_json::json!({
                        "cql": cql,
                        "limit": self.config.batch_size,
                        "expand": "body.storage,version,space,history.lastUpdated,history.createdBy,history.lastUpdatedBy"
                    }))
                    .send()
                    .await?;

                if !response.status().is_success() {
                    return Err(format!("Confluence API error: {}", response.status()));
                }

                let search_response: ConfluenceSearchResponse = response.json().await?;
                Ok(search_response)
            })
            .await?;

        let mut documents = Vec::new();
        for page in response.results {
            if let Some(document) = self.convert_page_to_document(page).await? {
                documents.push(document);
            }
        }

        // Update last sync timestamp
        if let Some(last_page) = response.results.last() {
            if let Ok(timestamp) = self.parse_confluence_timestamp(&last_page.last_modified_date) {
                self.last_sync_timestamp = Some(timestamp);
            }
        }

        tracing::info!(
            "Polled {} Confluence pages, converted {} to documents",
            response.results.len(),
            documents.len()
        );

        Ok(documents)
    }

    fn build_cql_query(&self) -> String {
        let mut cql_parts = Vec::new();
        
        // Only fetch pages updated since last sync
        if let Some(last_sync) = self.last_sync_timestamp {
            let last_sync_str = self.format_confluence_timestamp(last_sync);
            cql_parts.push(format!("lastmodified >= '{}'", last_sync_str));
        }

        // Filter for specification-related pages
        cql_parts.push("(text ~ 'specification' OR text ~ 'spec' OR title ~ 'specification' OR title ~ 'spec')".to_string());
        
        // Only include pages (not comments, attachments, etc.)
        cql_parts.push("type = page".to_string());
        
        // Order by last modified
        cql_parts.push("ORDER BY lastmodified DESC".to_string());

        cql_parts.join(" AND ")
    }

    async fn convert_page_to_document(&self, page: ConfluencePage) -> Result<Option<SpecDocument>, Box<dyn std::error::Error>> {
        // Skip pages that don't have meaningful content
        if page.body.storage.value.is_empty() {
            return Ok(None);
        }

        let content = self.extract_content(&page)?;
        let content_sha256 = self.compute_content_hash(&content);

        let metadata = DocumentMetadata {
            source_id: page.id,
            title: page.title,
            url: format!("{}{}", self.config.base_url, page._links.webui),
            author: page.created_by.display_name,
            created_at: self.parse_timestamp(&page.created_date)?,
            modified_at: self.parse_timestamp(&page.last_modified_date)?,
            version: page.version.number,
            status: page.status,
            metadata: self.extract_metadata(&page),
        };

        let mut document: SpecDocument = metadata.into();
        document.content = content;
        document.content_sha256 = content_sha256;
        document.source_system = "confluence".to_string();

        Ok(Some(document))
    }

    fn extract_content(&self, page: &ConfluencePage) -> Result<String, Box<dyn std::error::Error>> {
        let mut content_parts = Vec::new();

        // Add title
        content_parts.push(format!("# {}", page.title));

        // Convert Confluence storage format to markdown
        let markdown_content = self.confluence_storage_to_markdown(&page.body.storage.value)?;
        content_parts.push(markdown_content);

        // Add space information
        content_parts.push(format!("\n## Space\n{} ({})", page.space.name, page.space.key));

        // Add version information
        if let Some(message) = &page.version.message {
            content_parts.push(format!("\n## Version\n{}", message));
        }

        Ok(content_parts.join("\n\n"))
    }

    fn confluence_storage_to_markdown(&self, storage_content: &str) -> Result<String, Box<dyn std::error::Error>> {
        // This is a simplified conversion. In production, you'd use a proper
        // Confluence storage format parser or the Confluence REST API's
        // export functionality to get clean markdown.
        
        // For now, we'll do basic HTML to markdown conversion
        let mut markdown = storage_content.to_string();
        
        // Basic HTML tag replacements
        markdown = markdown.replace("<p>", "").replace("</p>", "\n\n");
        markdown = markdown.replace("<h1>", "# ").replace("</h1>", "\n");
        markdown = markdown.replace("<h2>", "## ").replace("</h2>", "\n");
        markdown = markdown.replace("<h3>", "### ").replace("</h3>", "\n");
        markdown = markdown.replace("<strong>", "**").replace("</strong>", "**");
        markdown = markdown.replace("<em>", "*").replace("</em>", "*");
        markdown = markdown.replace("<code>", "`").replace("</code>", "`");
        markdown = markdown.replace("<pre>", "```\n").replace("</pre>", "\n```");
        
        // Clean up extra whitespace
        markdown = markdown.lines()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join("\n");

        Ok(markdown)
    }

    fn extract_metadata(&self, page: &ConfluencePage) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        
        metadata.insert("space_key".to_string(), page.space.key.clone());
        metadata.insert("space_name".to_string(), page.space.name.clone());
        metadata.insert("version_number".to_string(), page.version.number.to_string());
        metadata.insert("created_by_username".to_string(), page.created_by.username.clone());
        metadata.insert("modified_by_username".to_string(), page.last_modified_by.username.clone());
        metadata.insert("modified_by_display_name".to_string(), page.last_modified_by.display_name.clone());

        if let Some(message) = &page.version.message {
            metadata.insert("version_message".to_string(), message.clone());
        }

        metadata
    }

    fn compute_content_hash(&self, content: &str) -> String {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn parse_timestamp(&self, timestamp_str: &str) -> Result<DateTime<Utc>, Box<dyn std::error::Error>> {
        // Confluence timestamps are in format: "2023-01-01T12:00:00.000Z"
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)?;
        Ok(timestamp.with_timezone(&Utc))
    }

    fn parse_confluence_timestamp(&self, timestamp_str: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)?;
        Ok(timestamp.timestamp())
    }

    fn format_confluence_timestamp(&self, timestamp: i64) -> String {
        let dt = chrono::DateTime::from_timestamp(timestamp, 0)
            .unwrap_or_else(|| chrono::Utc::now());
        dt.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_cql_query() {
        let config = ConnectorConfig {
            source_system: "confluence".to_string(),
            base_url: "https://example.atlassian.net/wiki".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:confluence-oauth".to_string(),
        };

        let connector = ConfluenceConnector::new(config);
        
        // Test without last sync timestamp
        let cql = connector.build_cql_query();
        assert!(cql.contains("specification"));
        assert!(cql.contains("type = page"));
        assert!(cql.contains("ORDER BY lastmodified DESC"));
    }

    #[test]
    fn test_compute_content_hash() {
        let config = ConnectorConfig {
            source_system: "confluence".to_string(),
            base_url: "https://example.atlassian.net/wiki".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:confluence-oauth".to_string(),
        };

        let connector = ConfluenceConnector::new(config);
        let hash = connector.compute_content_hash("test content");
        
        // SHA256 hash should be 64 characters long
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_parse_timestamp() {
        let config = ConnectorConfig {
            source_system: "confluence".to_string(),
            base_url: "https://example.atlassian.net/wiki".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:confluence-oauth".to_string(),
        };

        let connector = ConfluenceConnector::new(config);
        let timestamp = connector.parse_timestamp("2023-01-01T12:00:00.000Z").unwrap();
        
        assert!(timestamp.seconds > 0);
        assert!(timestamp.nanos >= 0);
    }

    #[test]
    fn test_confluence_storage_to_markdown() {
        let config = ConnectorConfig {
            source_system: "confluence".to_string(),
            base_url: "https://example.atlassian.net/wiki".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:confluence-oauth".to_string(),
        };

        let connector = ConfluenceConnector::new(config);
        let html = "<h1>Title</h1><p>This is <strong>bold</strong> text.</p>";
        let markdown = connector.confluence_storage_to_markdown(html).unwrap();
        
        assert!(markdown.contains("# Title"));
        assert!(markdown.contains("**bold**"));
    }
} 