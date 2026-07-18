//! End-to-end integration tests for the ingestion connectors.
//!
//! These tests exercise the full poll -> convert -> publish pipeline without
//! requiring any external infrastructure: upstream APIs (Jira/Confluence/Google
//! Docs) are mocked with `wiremock`, and publishing is verified against the
//! in-memory [`DocumentPublisher`] implementation. This keeps the suite
//! hermetic and runnable in CI.

use std::sync::Arc;

use ingest::{
    connectors::{ConfluenceConnector, GoogleDocsConnector, JiraConnector},
    proto::spec_to_proof::v1::SpecDocument,
    publisher::InMemoryPublisher,
    ConnectorConfig, IngestionConnector, OAuth2Token,
};
use prost::Message;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_token() -> OAuth2Token {
    OAuth2Token {
        access_token: "test_access_token".to_string(),
        refresh_token: "test_refresh_token".to_string(),
        expires_at_unix: 4_102_444_800, // year 2100, comfortably in the future
        token_type: "Bearer".to_string(),
    }
}

fn config_for(source_system: &str, base_url: String) -> ConnectorConfig {
    ConnectorConfig {
        source_system: source_system.to_string(),
        base_url,
        rate_limit_per_minute: 100,
        batch_size: 50,
        poll_interval_seconds: 1,
        secrets_arn: format!("test-{}-oauth", source_system),
    }
}

#[tokio::test]
async fn test_jira_connector_polls_and_publishes() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "issues": [
                {
                    "id": "12345",
                    "key": "SPEC-001",
                    "fields": {
                        "summary": "API Specification",
                        "description": "This document describes the API specification for our service.",
                        "status": { "name": "In Progress", "statusCategory": { "name": "To Do" } },
                        "assignee": { "displayName": "John Doe", "emailAddress": "john@example.com" },
                        "reporter": { "displayName": "Jane Smith", "emailAddress": "jane@example.com" },
                        "created": "2023-01-01T12:00:00.000+0000",
                        "updated": "2023-01-02T14:30:00.000+0000",
                        "project": { "key": "SPEC", "name": "Specifications" },
                        "issuetype": { "name": "Story", "description": "User story" },
                        "priority": { "name": "High" },
                        "labels": ["api", "specification"],
                        "components": [ { "name": "Backend" } ]
                    },
                    "changelog": {
                        "histories": [
                            {
                                "created": "2023-01-02T14:30:00.000+0000",
                                "items": [
                                    { "field": "status", "fieldtype": "jira", "fromString": "To Do", "toString": "In Progress" }
                                ]
                            }
                        ]
                    }
                }
            ],
            "total": 1,
            "maxResults": 50,
            "startAt": 0
        })))
        .mount(&mock_server)
        .await;

    let config = config_for("jira", mock_server.uri());
    let mut jira_connector = JiraConnector::new(config.clone());
    let token = test_token();

    let documents = jira_connector.poll_documents(&token).await.unwrap();

    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "jira");
    assert_eq!(document.source_id, "SPEC-001");
    assert_eq!(document.title, "API Specification");
    assert!(document.content.contains("API Specification"));
    assert!(document.content.contains("This document describes"));

    // Publish through the ingestion connector using the in-memory transport and
    // verify the encoded document round-trips.
    let publisher = Arc::new(InMemoryPublisher::new());
    let ingestion_connector = IngestionConnector::new(
        config,
        secrets_client().await,
        publisher.clone(),
    )
    .await
    .unwrap();

    for document in documents {
        ingestion_connector.publish_document(document).await.unwrap();
    }

    let messages = publisher.messages();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0, "spec-documents.jira");
    let decoded = SpecDocument::decode(messages[0].1.as_slice()).unwrap();
    assert_eq!(decoded.source_system, "jira");
    assert_eq!(decoded.source_id, "SPEC-001");
}

#[tokio::test]
async fn test_confluence_connector_polls_and_converts() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/rest/api/content/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {
                    "id": "12345",
                    "title": "System Specification",
                    "status": "current",
                    "version": { "number": 1, "message": "Initial version" },
                    "space": { "key": "SPEC", "name": "Specifications" },
                    "body": {
                        "storage": {
                            "value": "<h1>System Specification</h1><p>This document describes the system specification.</p>",
                            "representation": "storage"
                        }
                    },
                    "_links": { "webui": "/pages/viewpage.action?pageId=12345" },
                    "created_by": { "display_name": "John Doe", "username": "johndoe" },
                    "created_date": "2023-01-01T12:00:00.000Z",
                    "last_modified_date": "2023-01-02T14:30:00.000Z",
                    "last_modified_by": { "display_name": "Jane Smith", "username": "janesmith" }
                }
            ],
            "start": 0,
            "limit": 50,
            "size": 1,
            "_links": { "next": null }
        })))
        .mount(&mock_server)
        .await;

    let config = config_for("confluence", mock_server.uri());
    let mut confluence_connector = ConfluenceConnector::new(config);
    let token = test_token();

    let documents = confluence_connector.poll_documents(&token).await.unwrap();

    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "confluence");
    assert_eq!(document.source_id, "12345");
    assert_eq!(document.title, "System Specification");
    assert!(document.content.contains("System Specification"));
    assert!(document.content.contains("This document describes"));
}

#[tokio::test]
async fn test_gdocs_connector_polls_and_converts() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [
                {
                    "id": "doc-1",
                    "name": "API Documentation",
                    "mimeType": "application/vnd.google-apps.document",
                    "createdTime": "2023-01-01T12:00:00.000Z",
                    "modifiedTime": "2023-01-02T14:30:00.000Z",
                    "owners": [ { "displayName": "John Doe", "emailAddress": "john@example.com" } ],
                    "lastModifyingUser": { "displayName": "Jane Smith", "emailAddress": "jane@example.com" },
                    "parents": ["folder-1"],
                    "webViewLink": "https://docs.google.com/document/d/doc-1/edit",
                    "size": "1024",
                    "description": "API documentation for our service"
                }
            ],
            "nextPageToken": null
        })))
        .mount(&mock_server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v1/documents/doc-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "documentId": "doc-1",
            "title": "API Documentation",
            "body": {
                "content": [
                    {
                        "startIndex": 1,
                        "endIndex": 2,
                        "paragraph": {
                            "elements": [
                                {
                                    "startIndex": 1,
                                    "endIndex": 2,
                                    "textRun": {
                                        "content": "This document describes the API specification for our service.",
                                        "textStyle": { "bold": false, "italic": false, "underline": false }
                                    }
                                }
                            ],
                            "paragraphStyle": { "namedStyleType": "NORMAL_TEXT" }
                        }
                    }
                ]
            },
            "revisionId": "1"
        })))
        .mount(&mock_server)
        .await;

    let config = config_for("google_docs", mock_server.uri());
    // Point both the Drive and Docs API roots at the mock server.
    let mut gdocs_connector =
        GoogleDocsConnector::new(config).with_endpoints(mock_server.uri(), mock_server.uri());

    let documents = gdocs_connector.poll_documents(&test_token()).await.unwrap();

    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "google_docs");
    assert_eq!(document.source_id, "doc-1");
    assert_eq!(document.title, "API Documentation");
    assert!(document.content.contains("API Documentation"));
    assert!(document.content.contains("This document describes"));
}

#[tokio::test]
async fn test_rate_limiting_accounting() {
    let config = ConnectorConfig {
        rate_limit_per_minute: 10,
        ..config_for("jira", "https://example.atlassian.net".to_string())
    };

    let jira_connector = JiraConnector::new(config);
    let (current_usage, max_usage) = jira_connector.rate_limiter().get_current_usage().await;
    assert_eq!(current_usage, 0);
    assert_eq!(max_usage, 7); // 70% of 10

    let backoff = ingest::backoff::ExponentialBackoff::new();
    let delay = backoff.calculate_delay(0);
    assert!(delay.as_millis() >= 1); // jittered, but always positive
}

/// Build a Secrets Manager client that does not require live AWS credentials.
/// It is never actually called in these tests (publishing uses the in-memory
/// transport), but `IngestionConnector::new` requires a client value.
async fn secrets_client() -> aws_sdk_secretsmanager::Client {
    let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new("us-east-1"))
        .load()
        .await;
    aws_sdk_secretsmanager::Client::new(&config)
}
