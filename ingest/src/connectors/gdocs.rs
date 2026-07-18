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
#[serde(rename_all = "camelCase")]
pub struct GoogleDriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub created_time: String,
    pub modified_time: String,
    pub owners: Vec<GoogleDriveUser>,
    pub last_modifying_user: Option<GoogleDriveUser>,
    pub parents: Vec<String>,
    pub web_view_link: String,
    pub size: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDriveUser {
    pub display_name: String,
    pub email_address: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDriveFileList {
    pub files: Vec<GoogleDriveFile>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsDocument {
    pub document_id: String,
    pub title: String,
    pub body: GoogleDocsBody,
    pub revision_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsBody {
    pub content: Vec<GoogleDocsContent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsContent {
    pub start_index: Option<i32>,
    pub end_index: Option<i32>,
    pub paragraph: Option<GoogleDocsParagraph>,
    pub table: Option<GoogleDocsTable>,
    pub section_break: Option<GoogleDocsSectionBreak>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsParagraph {
    pub elements: Vec<GoogleDocsElement>,
    pub paragraph_style: Option<GoogleDocsParagraphStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsElement {
    pub start_index: Option<i32>,
    pub end_index: Option<i32>,
    pub text_run: Option<GoogleDocsTextRun>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsTextRun {
    pub content: String,
    pub text_style: Option<GoogleDocsTextStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTextStyle {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsParagraphStyle {
    pub named_style_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsTable {
    pub table_rows: Vec<GoogleDocsTableRow>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsTableRow {
    pub table_cells: Vec<GoogleDocsTableCell>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTableCell {
    pub content: Vec<GoogleDocsContent>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsSectionBreak {
    pub section_style: Option<GoogleDocsSectionStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleDocsSectionStyle {
    pub column_separator_style: Option<String>,
}

/// Default Google Drive API root (`.../drive/v3/files` is appended).
const DEFAULT_DRIVE_API_BASE: &str = "https://www.googleapis.com";
/// Default Google Docs API root (`.../v1/documents/{id}` is appended).
const DEFAULT_DOCS_API_BASE: &str = "https://docs.googleapis.com";

pub struct GoogleDocsConnector {
    config: ConnectorConfig,
    http_client: Client,
    rate_limiter: RateLimiter,
    backoff: ExponentialBackoff,
    drive_api_base: String,
    docs_api_base: String,
}

impl GoogleDocsConnector {
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
            drive_api_base: DEFAULT_DRIVE_API_BASE.to_string(),
            docs_api_base: DEFAULT_DOCS_API_BASE.to_string(),
        }
    }

    /// Override the Drive and Docs API roots. Enables pointing the connector at
    /// a mock server for integration testing without changing production
    /// behaviour (which uses Google's real endpoints).
    pub fn with_endpoints(
        mut self,
        drive_api_base: impl Into<String>,
        docs_api_base: impl Into<String>,
    ) -> Self {
        self.drive_api_base = drive_api_base.into();
        self.docs_api_base = docs_api_base.into();
        self
    }

    pub async fn poll_documents(&mut self, token: &OAuth2Token) -> Result<Vec<SpecDocument>, Box<dyn std::error::Error>> {
        self.rate_limiter.acquire().await?;

        // First, search for Google Docs files
        let files = self.search_docs_files(token).await?;
        let file_count = files.len();

        let mut documents = Vec::new();
        for file in files {
            if let Some(document) = self.convert_file_to_document(file, token).await? {
                documents.push(document);
            }
        }

        tracing::info!(
            "Polled {} Google Docs files, converted {} to documents",
            file_count,
            documents.len()
        );

        Ok(documents)
    }

    async fn search_docs_files(&self, token: &OAuth2Token) -> Result<Vec<GoogleDriveFile>, Box<dyn std::error::Error>> {
        let url = format!("{}/drive/v3/files", self.drive_api_base);
        let url = url.as_str();

        // Combine the document-type filter and the specification search terms
        // into a single Drive `q` expression (a request must carry only one
        // `q` parameter).
        let query = "mimeType='application/vnd.google-apps.document' AND \
             (name contains 'specification' OR name contains 'spec' OR \
             fullText contains 'specification' OR fullText contains 'spec')";
        let fields = "nextPageToken,files(id,name,mimeType,createdTime,modifiedTime,owners,lastModifyingUser,parents,webViewLink,size,description)";
        let page_size = self.config.batch_size.to_string();
        let http_client = &self.http_client;

        let mut all_files = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let page_token_ref = page_token.clone();
            let file_list: GoogleDriveFileList = self.backoff
                .execute_with_backoff_async(|| async {
                    let mut request = http_client
                        .get(url)
                        .header("Authorization", format!("Bearer {}", token.access_token))
                        .query(&[
                            ("q", query),
                            ("fields", fields),
                            ("pageSize", page_size.as_str()),
                        ]);

                    if let Some(pt) = page_token_ref.as_deref() {
                        request = request.query(&[("pageToken", pt)]);
                    }

                    let response = request
                        .send()
                        .await
                        .map_err(|e| format!("Google Drive request failed: {}", e))?;

                    if !response.status().is_success() {
                        return Err(format!("Google Drive API error: {}", response.status()));
                    }

                    response
                        .json::<GoogleDriveFileList>()
                        .await
                        .map_err(|e| format!("Failed to parse Google Drive response: {}", e))
                })
                .await?;

            all_files.extend(file_list.files);

            match file_list.next_page_token {
                Some(next_token) => page_token = Some(next_token),
                None => break,
            }
        }

        Ok(all_files)
    }

    async fn convert_file_to_document(&self, file: GoogleDriveFile, token: &OAuth2Token) -> Result<Option<SpecDocument>, Box<dyn std::error::Error>> {
        // Skip files that don't have meaningful content
        if file.name.is_empty() {
            return Ok(None);
        }

        // Fetch the document content
        let content = self.fetch_document_content(&file.id, token).await?;
        let content_sha256 = self.compute_content_hash(&content);

        // Extract metadata (which borrows `file`) before consuming its fields.
        let extra_metadata = self.extract_metadata(&file);
        let created_at = self.parse_timestamp(&file.created_time)?;
        let modified_at = self.parse_timestamp(&file.modified_time)?;
        let author = file.owners.first()
            .map(|owner| owner.display_name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let metadata = DocumentMetadata {
            source_id: file.id,
            title: file.name,
            url: file.web_view_link,
            author,
            created_at,
            modified_at,
            version: 1, // Google Docs doesn't expose version numbers
            status: "published".to_string(),
            metadata: extra_metadata,
        };

        let mut document: SpecDocument = metadata.into();
        document.content = content;
        document.content_sha256 = content_sha256;
        document.source_system = "google_docs".to_string();

        Ok(Some(document))
    }

    async fn fetch_document_content(&self, document_id: &str, token: &OAuth2Token) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{}/v1/documents/{}", self.docs_api_base, document_id);

        let http_client = &self.http_client;
        let doc: GoogleDocsDocument = self.backoff
            .execute_with_backoff_async(|| async {
                let response = http_client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token.access_token))
                    .send()
                    .await
                    .map_err(|e| format!("Google Docs request failed: {}", e))?;

                if !response.status().is_success() {
                    return Err(format!("Google Docs API error: {}", response.status()));
                }

                response
                    .json::<GoogleDocsDocument>()
                    .await
                    .map_err(|e| format!("Failed to parse Google Docs response: {}", e))
            })
            .await?;

        self.extract_content_from_document(&doc)
    }

    fn extract_content_from_document(&self, doc: &GoogleDocsDocument) -> Result<String, Box<dyn std::error::Error>> {
        let mut content_parts = Vec::new();

        // Add title
        content_parts.push(format!("# {}", doc.title));

        // Extract content from body
        let body_content = self.extract_body_content(&doc.body)?;
        content_parts.push(body_content);

        Ok(content_parts.join("\n\n"))
    }

    fn extract_body_content(&self, body: &GoogleDocsBody) -> Result<String, Box<dyn std::error::Error>> {
        let mut content = String::new();

        for element in &body.content {
            if let Some(paragraph) = &element.paragraph {
                content.push_str(&self.extract_paragraph_content(paragraph));
                content.push('\n');
            } else if let Some(table) = &element.table {
                content.push_str(&self.extract_table_content(table));
                content.push('\n');
            }
        }

        Ok(content)
    }

    fn extract_paragraph_content(&self, paragraph: &GoogleDocsParagraph) -> String {
        let mut content = String::new();

        // Check if this is a heading
        if let Some(style) = &paragraph.paragraph_style {
            if let Some(named_style) = &style.named_style_type {
                match named_style.as_str() {
                    "HEADING_1" => content.push_str("# "),
                    "HEADING_2" => content.push_str("## "),
                    "HEADING_3" => content.push_str("### "),
                    _ => {}
                }
            }
        }

        // Extract text from elements
        for element in &paragraph.elements {
            if let Some(text_run) = &element.text_run {
                let mut text = text_run.content.clone();

                // Apply text styling
                if let Some(text_style) = &text_run.text_style {
                    if text_style.bold.unwrap_or(false) {
                        text = format!("**{}**", text);
                    }
                    if text_style.italic.unwrap_or(false) {
                        text = format!("*{}*", text);
                    }
                }

                content.push_str(&text);
            }
        }

        content
    }

    fn extract_table_content(&self, table: &GoogleDocsTable) -> String {
        let mut content = String::new();

        for row in &table.table_rows {
            let mut row_content = String::new();
            
            for cell in &row.table_cells {
                let cell_text = self.extract_cell_content(cell);
                row_content.push_str(&format!("| {}", cell_text));
            }
            
            row_content.push_str(" |");
            content.push_str(&row_content);
            content.push('\n');
        }

        content
    }

    fn extract_cell_content(&self, cell: &GoogleDocsTableCell) -> String {
        let mut content = String::new();
        
        for element in &cell.content {
            if let Some(paragraph) = &element.paragraph {
                content.push_str(&self.extract_paragraph_content(paragraph));
            }
        }

        content.trim().to_string()
    }

    fn extract_metadata(&self, file: &GoogleDriveFile) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        
        metadata.insert("mime_type".to_string(), file.mime_type.clone());
        metadata.insert("size".to_string(), file.size.clone().unwrap_or_default());
        
        if let Some(description) = &file.description {
            metadata.insert("description".to_string(), description.clone());
        }

        // Add owner information
        if let Some(owner) = file.owners.first() {
            metadata.insert("owner_email".to_string(), owner.email_address.clone());
            metadata.insert("owner_display_name".to_string(), owner.display_name.clone());
        }

        // Add last modifying user information
        if let Some(last_modifier) = &file.last_modifying_user {
            metadata.insert("last_modifier_email".to_string(), last_modifier.email_address.clone());
            metadata.insert("last_modifier_display_name".to_string(), last_modifier.display_name.clone());
        }

        // Add parent folder information
        if let Some(parent) = file.parents.first() {
            metadata.insert("parent_folder_id".to_string(), parent.clone());
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
        // Google API timestamps are in RFC3339 format: "2023-01-01T12:00:00.000Z"
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)?;
        Ok(timestamp.with_timezone(&Utc))
    }

    #[allow(dead_code)]
    fn parse_google_timestamp(&self, timestamp_str: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)?;
        Ok(timestamp.timestamp())
    }

    #[allow(dead_code)]
    fn format_google_timestamp(&self, timestamp: i64) -> String {
        let dt = chrono::DateTime::from_timestamp(timestamp, 0)
            .unwrap_or_else(|| chrono::Utc::now());
        dt.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_content_hash() {
        let config = ConnectorConfig {
            source_system: "google_docs".to_string(),
            base_url: "https://docs.googleapis.com".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:gdocs-oauth".to_string(),
        };

        let connector = GoogleDocsConnector::new(config);
        let hash = connector.compute_content_hash("test content");
        
        // SHA256 hash should be 64 characters long
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_parse_timestamp() {
        let config = ConnectorConfig {
            source_system: "google_docs".to_string(),
            base_url: "https://docs.googleapis.com".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:gdocs-oauth".to_string(),
        };

        let connector = GoogleDocsConnector::new(config);
        let timestamp = connector.parse_timestamp("2023-01-01T12:00:00.000Z").unwrap();

        assert!(timestamp.timestamp() > 0);
        assert_eq!(timestamp.date_naive().to_string(), "2023-01-01");
    }

    #[test]
    fn test_extract_paragraph_content() {
        let config = ConnectorConfig {
            source_system: "google_docs".to_string(),
            base_url: "https://docs.googleapis.com".to_string(),
            rate_limit_per_minute: 100,
            batch_size: 50,
            poll_interval_seconds: 300,
            secrets_arn: "arn:aws:secretsmanager:us-east-1:123456789012:secret:gdocs-oauth".to_string(),
        };

        let connector = GoogleDocsConnector::new(config);
        
        let paragraph = GoogleDocsParagraph {
            elements: vec![
                GoogleDocsElement {
                    start_index: Some(0),
                    end_index: Some(4),
                    text_run: Some(GoogleDocsTextRun {
                        content: "Test".to_string(),
                        text_style: Some(GoogleDocsTextStyle {
                            bold: Some(true),
                            italic: Some(false),
                            underline: Some(false),
                        }),
                    }),
                },
            ],
            paragraph_style: None,
        };

        let content = connector.extract_paragraph_content(&paragraph);
        assert!(content.contains("**Test**"));
    }
} 