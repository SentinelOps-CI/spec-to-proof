use crate::{
    ConnectorConfig, IngestionConnector, OAuth2Token, DocumentMetadata,
    rate_limiter::RateLimiter, backoff::ExponentialBackoff,
};
use crate::proto::spec_to_proof::v1::SpecDocument;
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraIssue {
    pub id: String,
    pub key: String,
    pub fields: JiraFields,
    pub changelog: Option<JiraChangelog>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraFields {
    pub summary: String,
    pub description: Option<String>,
    pub status: JiraStatus,
    pub assignee: Option<JiraUser>,
    pub reporter: Option<JiraUser>,
    pub created: String,
    pub updated: String,
    pub project: JiraProject,
    pub issuetype: JiraIssueType,
    pub priority: Option<JiraPriority>,
    pub labels: Vec<String>,
    pub components: Vec<JiraComponent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraStatus {
    pub name: String,
    pub statusCategory: JiraStatusCategory,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraStatusCategory {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraUser {
    pub displayName: String,
    pub emailAddress: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraProject {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraIssueType {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraPriority {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraComponent {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraChangelog {
    pub histories: Vec<JiraHistory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraHistory {
    pub created: String,
    pub items: Vec<JiraChangeItem>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraChangeItem {
    pub field: String,
    pub fieldtype: String,
    pub fromString: Option<String>,
    pub toString: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JiraSearchResponse {
    pub issues: Vec<JiraIssue>,
    pub total: i32,
    pub maxResults: i32,
    pub startAt: i32,
}

pub struct JiraConnector {
    config: ConnectorConfig,
    http_client: Client,
    rate_limiter: RateLimiter,
    backoff: ExponentialBackoff,
    last_sync_timestamp: Option<i64>,
}

impl JiraConnector {
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

        let jql = self.build_jql_query();
        let url = format!("{}/rest/api/3/search", self.config.base_url);

        let response = self.backoff
            .execute_with_backoff(|| async {
                let response = self.http_client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token.access_token))
                    .header("Accept", "application/json")
                    .query(&[
                        ("jql", &jql),
                        ("maxResults", &self.config.batch_size.to_string()),
                        ("fields", &"summary,description,status,assignee,reporter,created,updated,project,issuetype,priority,labels,components".to_string()),
                        ("expand", &"changelog".to_string()),
                    ])
                    .send()
                    .await?;

                if !response.status().is_success() {
                    return Err(format!("Jira API error: {}", response.status()));
                }

                let search_response: JiraSearchResponse = response.json().await?;
                Ok(search_response)
            })
            .await?;

        let mut documents = Vec::new();
        for issue in response.issues {
            if let Some(document) = self.convert_issue_to_document(issue).await? {
                documents.push(document);
            }
        }

        // Update last sync timestamp
        if let Some(last_issue) = response.issues.last() {
            if let Ok(timestamp) = self.parse_jira_timestamp(&last_issue.fields.updated) {
                self.last_sync_timestamp = Some(timestamp);
            }
        }

        tracing::info!(
            "Polled {} Jira issues, converted {} to documents",
            response.issues.len(),
            documents.len()
        );

        Ok(documents)
    }

    fn build_jql_query(&self) -> String {
        let mut jql_parts = Vec::new();
        
        // Only fetch issues updated since last sync
        if let Some(last_sync) = self.last_sync_timestamp {
            let last_sync_str = self.format_jira_timestamp(last_sync);
            jql_parts.push(format!("updated >= '{}'", last_sync_str));
        }

        // Filter for specification-related issues
        jql_parts.push("(summary ~ 'specification' OR summary ~ 'spec' OR description ~ 'specification' OR description ~ 'spec')".to_string());
        
        // Order by last updated
        jql_parts.push("ORDER BY updated DESC".to_string());

        jql_parts.join(" AND ")
    }

    async fn convert_issue_to_document(&self, issue: JiraIssue) -> Result<Option<SpecDocument>, Box<dyn std::error::Error>> {
        // Skip issues that don't have meaningful content
        if issue.fields.description.is_none() && issue.fields.summary.is_empty() {
            return Ok(None);
        }

        let content = self.extract_content(&issue)?;
        let content_sha256 = self.compute_content_hash(&content);

        let metadata = DocumentMetadata {
            source_id: issue.key,
            title: issue.fields.summary,
            url: format!("{}/browse/{}", self.config.base_url, issue.key),
            author: issue.fields.reporter
                .map(|r| r.displayName)
                .unwrap_or_else(|| "Unknown".to_string()),
            created_at: self.parse_timestamp(&issue.fields.created)?,
            modified_at: self.parse_timestamp(&issue.fields.updated)?,
            version: 1, // Jira doesn't have explicit versions
            status: issue.fields.status.name,
            metadata: self.extract_metadata(&issue),
        };

        let mut document: SpecDocument = metadata.into();
        document.content = content;
        document.content_sha256 = content_sha256;
        document.source_system = "jira".to_string();

        Ok(Some(document))
    }

    fn extract_content(&self, issue: &JiraIssue) -> Result<String, Box<dyn std::error::Error>> {
        let mut content_parts = Vec::new();

        // Add summary
        if !issue.fields.summary.is_empty() {
            content_parts.push(format!("# {}", issue.fields.summary));
        }

        // Add description
        if let Some(description) = &issue.fields.description {
            content_parts.push(description.clone());
        }

        // Add labels as tags
        if !issue.fields.labels.is_empty() {
            content_parts.push(format!("\n## Labels\n{}", issue.fields.labels.join(", ")));
        }

        // Add components
        if !issue.fields.components.is_empty() {
            let component_names: Vec<String> = issue.fields.components
                .iter()
                .map(|c| c.name.clone())
                .collect();
            content_parts.push(format!("\n## Components\n{}", component_names.join(", ")));
        }

        // Add changelog for recent changes
        if let Some(changelog) = &issue.changelog {
            let recent_changes = self.extract_recent_changes(changelog);
            if !recent_changes.is_empty() {
                content_parts.push(format!("\n## Recent Changes\n{}", recent_changes));
            }
        }

        Ok(content_parts.join("\n\n"))
    }

    fn extract_recent_changes(&self, changelog: &JiraChangelog) -> String {
        let mut changes = Vec::new();
        
        for history in &changelog.histories {
            for item in &history.items {
                if let (Some(from), Some(to)) = (&item.fromString, &item.toString) {
                    changes.push(format!("- {}: {} → {}", item.field, from, to));
                }
            }
        }

        changes.join("\n")
    }

    fn extract_metadata(&self, issue: &JiraIssue) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        
        metadata.insert("project_key".to_string(), issue.fields.project.key.clone());
        metadata.insert("project_name".to_string(), issue.fields.project.name.clone());
        metadata.insert("issue_type".to_string(), issue.fields.issuetype.name.clone());
        
        if let Some(priority) = &issue.fields.priority {
            metadata.insert("priority".to_string(), priority.name.clone());
        }
        
        if let Some(assignee) = &issue.fields.assignee {
            metadata.insert("assignee".to_string(), assignee.displayName.clone());
            if let Some(email) = &assignee.emailAddress {
                metadata.insert("assignee_email".to_string(), email.clone());
            }
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
        // Jira timestamps are in format: "2023-01-01T12:00:00.000+0000"
        let timestamp = chrono::DateTime::parse_from_str(timestamp_str, "%Y-%m-%dT%H:%M:%S.%3f%z")?;
        Ok(timestamp.with_timezone(&Utc))
    }

    fn parse_jira_timestamp(&self, timestamp_str: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let timestamp = chrono::DateTime::parse_from_str(timestamp_str, "%Y-%m-%dT%H:%M:%S.%3f%z")?;
        Ok(timestamp.timestamp())
    }

    fn format_jira_timestamp(&self, timestamp: i64) -> String {
        let dt = chrono::DateTime::from_timestamp(timestamp, 0)
            .unwrap_or_else(|| chrono::Utc::now());
        dt.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost_types::Timestamp;

    #[test]
    fn test_build_jql_query() {
        let config = ConnectorConfig {
            source_system: "jira".to_string(),
            base_url: "https://example.atlassian.net".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:jira-oauth".to_string(),
        };

        let connector = JiraConnector::new(config);
        
        // Test without last sync timestamp
        let jql = connector.build_jql_query();
        assert!(jql.contains("specification"));
        assert!(jql.contains("ORDER BY updated DESC"));
    }

    #[test]
    fn test_compute_content_hash() {
        let config = ConnectorConfig {
            source_system: "jira".to_string(),
            base_url: "https://example.atlassian.net".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:jira-oauth".to_string(),
        };

        let connector = JiraConnector::new(config);
        let hash = connector.compute_content_hash("test content");
        
        // SHA256 hash should be 64 characters long
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_parse_timestamp() {
        let config = ConnectorConfig {
            source_system: "jira".to_string(),
            base_url: "https://example.atlassian.net".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:jira-oauth".to_string(),
        };

        let connector = JiraConnector::new(config);
        let timestamp = connector.parse_timestamp("2023-01-01T12:00:00.000+0000").unwrap();
        
        assert!(timestamp.seconds > 0);
        assert!(timestamp.nanos >= 0);
    }
} 