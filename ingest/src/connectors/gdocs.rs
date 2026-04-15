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
pub struct GoogleDriveFile {
    pub id: String,
    pub name: String,
    pub mimeType: String,
    pub createdTime: String,
    pub modifiedTime: String,
    pub owners: Vec<GoogleDriveUser>,
    pub lastModifyingUser: Option<GoogleDriveUser>,
    pub parents: Vec<String>,
    pub webViewLink: String,
    pub size: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDriveUser {
    pub displayName: String,
    pub emailAddress: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDriveFileList {
    pub files: Vec<GoogleDriveFile>,
    pub nextPageToken: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsDocument {
    pub documentId: String,
    pub title: String,
    pub body: GoogleDocsBody,
    pub revisionId: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsBody {
    pub content: Vec<GoogleDocsContent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsContent {
    pub startIndex: Option<i32>,
    pub endIndex: Option<i32>,
    pub paragraph: Option<GoogleDocsParagraph>,
    pub table: Option<GoogleDocsTable>,
    pub sectionBreak: Option<GoogleDocsSectionBreak>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsParagraph {
    pub elements: Vec<GoogleDocsElement>,
    pub paragraphStyle: Option<GoogleDocsParagraphStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsElement {
    pub startIndex: Option<i32>,
    pub endIndex: Option<i32>,
    pub textRun: Option<GoogleDocsTextRun>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTextRun {
    pub content: String,
    pub textStyle: Option<GoogleDocsTextStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTextStyle {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsParagraphStyle {
    pub namedStyleType: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTable {
    pub tableRows: Vec<GoogleDocsTableRow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTableRow {
    pub tableCells: Vec<GoogleDocsTableCell>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsTableCell {
    pub content: Vec<GoogleDocsContent>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsSectionBreak {
    pub sectionStyle: Option<GoogleDocsSectionStyle>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleDocsSectionStyle {
    pub columnSeparatorStyle: Option<String>,
}

pub struct GoogleDocsConnector {
    config: ConnectorConfig,
    http_client: Client,
    rate_limiter: RateLimiter,
    backoff: ExponentialBackoff,
    last_sync_timestamp: Option<i64>,
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
            last_sync_timestamp: None,
        }
    }

    pub async fn poll_documents(&mut self, token: &OAuth2Token) -> Result<Vec<SpecDocument>, Box<dyn std::error::Error>> {
        self.rate_limiter.acquire().await?;

        // First, search for Google Docs files
        let files = self.search_docs_files(token).await?;
        
        let mut documents = Vec::new();
        for file in files {
            if let Some(document) = self.convert_file_to_document(file, token).await? {
                documents.push(document);
            }
        }

        tracing::info!(
            "Polled {} Google Docs files, converted {} to documents",
            files.len(),
            documents.len()
        );

        Ok(documents)
    }

    async fn search_docs_files(&mut self, token: &OAuth2Token) -> Result<Vec<GoogleDriveFile>, Box<dyn std::error::Error>> {
        let url = "https://www.googleapis.com/drive/v3/files";
        
        let mut all_files = Vec::new();
        let mut page_token = None;

        loop {
            let response = self.backoff
                .execute_with_backoff(|| async {
                    let mut query_params = vec![
                        ("q", "mimeType='application/vnd.google-apps.document'"),
                        ("fields", "files(id,name,mimeType,createdTime,modifiedTime,owners,lastModifyingUser,parents,webViewLink,size,description)"),
                        ("pageSize", &self.config.batch_size.to_string()),
                    ];

                    if let Some(token) = &page_token {
                        query_params.push(("pageToken", token));
                    }

                    // Add search terms for specification-related documents
                    let search_query = "(name contains 'specification' OR name contains 'spec' OR description contains 'specification' OR description contains 'spec')";
                    query_params.push(("q", &format!("mimeType='application/vnd.google-apps.document' AND ({})", search_query)));

                    let response = self.http_client
                        .get(url)
                        .header("Authorization", format!("Bearer {}", token.access_token))
                        .query(&query_params)
                        .send()
                        .await?;

                    if !response.status().is_success() {
                        return Err(format!("Google Drive API error: {}", response.status()));
                    }

                    let file_list: GoogleDriveFileList = response.json().await?;
                    Ok(file_list)
                })
                .await?;

            all_files.extend(response.files);

            if let Some(next_token) = response.nextPageToken {
                page_token = Some(next_token);
            } else {
                break;
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

        let metadata = DocumentMetadata {
            source_id: file.id,
            title: file.name,
            url: file.webViewLink,
            author: file.owners.first()
                .map(|owner| owner.displayName.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
            created_at: self.parse_timestamp(&file.createdTime)?,
            modified_at: self.parse_timestamp(&file.modifiedTime)?,
            version: 1, // Google Docs doesn't expose version numbers
            status: "published".to_string(),
            metadata: self.extract_metadata(&file),
        };

        let mut document: SpecDocument = metadata.into();
        document.content = content;
        document.content_sha256 = content_sha256;
        document.source_system = "google_docs".to_string();

        Ok(Some(document))
    }

    async fn fetch_document_content(&self, document_id: &str, token: &OAuth2Token) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("https://docs.googleapis.com/v1/documents/{}", document_id);

        let response = self.backoff
            .execute_with_backoff(|| async {
                let response = self.http_client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", token.access_token))
                    .send()
                    .await?;

                if !response.status().is_success() {
                    return Err(format!("Google Docs API error: {}", response.status()));
                }

                let doc: GoogleDocsDocument = response.json().await?;
                Ok(doc)
            })
            .await?;

        self.extract_content_from_document(&response)
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
        if let Some(style) = &paragraph.paragraphStyle {
            if let Some(named_style) = &style.namedStyleType {
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
            if let Some(text_run) = &element.textRun {
                let mut text = text_run.content.clone();

                // Apply text styling
                if let Some(text_style) = &text_run.textStyle {
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

        for row in &table.tableRows {
            let mut row_content = String::new();
            
            for cell in &row.tableCells {
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
        
        metadata.insert("mime_type".to_string(), file.mimeType.clone());
        metadata.insert("size".to_string(), file.size.clone().unwrap_or_default());
        
        if let Some(description) = &file.description {
            metadata.insert("description".to_string(), description.clone());
        }

        // Add owner information
        if let Some(owner) = file.owners.first() {
            metadata.insert("owner_email".to_string(), owner.emailAddress.clone());
            metadata.insert("owner_display_name".to_string(), owner.displayName.clone());
        }

        // Add last modifying user information
        if let Some(last_modifier) = &file.lastModifyingUser {
            metadata.insert("last_modifier_email".to_string(), last_modifier.emailAddress.clone());
            metadata.insert("last_modifier_display_name".to_string(), last_modifier.displayName.clone());
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

    fn parse_google_timestamp(&self, timestamp_str: &str) -> Result<i64, Box<dyn std::error::Error>> {
        let timestamp = chrono::DateTime::parse_from_rfc3339(timestamp_str)?;
        Ok(timestamp.timestamp())
    }

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
        
        assert!(timestamp.seconds > 0);
        assert!(timestamp.nanos >= 0);
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
                    startIndex: Some(0),
                    endIndex: Some(4),
                    textRun: Some(GoogleDocsTextRun {
                        content: "Test".to_string(),
                        textStyle: Some(GoogleDocsTextStyle {
                            bold: Some(true),
                            italic: Some(false),
                            underline: Some(false),
                        }),
                    }),
                },
            ],
            paragraphStyle: None,
        };

        let content = connector.extract_paragraph_content(&paragraph);
        assert!(content.contains("**Test**"));
    }
} 