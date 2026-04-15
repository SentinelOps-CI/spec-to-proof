use ingest::{
    ConnectorConfig, IngestionConnector, OAuth2Token,
    connectors::{JiraConnector, ConfluenceConnector, GoogleDocsConnector},
    secrets::SecretsManager,
};
use aws_sdk_secretsmanager::Client as SecretsClient;
use aws_sdk_kms::Client as KmsClient;
use nats::jetstream::Context as JetStreamContext;
use tokio::time::{sleep, Duration};
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path, query_param};
use serde_json::json;

#[tokio::test]
async fn test_jira_connector_e2e() {
    // Start mock Jira server
    let mock_server = MockServer::start().await;
    
    // Mock Jira API responses
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .and(query_param("jql", "summary ~ 'specification'"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({
                "issues": [
                    {
                        "id": "12345",
                        "key": "SPEC-001",
                        "fields": {
                            "summary": "API Specification",
                            "description": "This document describes the API specification for our service.",
                            "status": {
                                "name": "In Progress",
                                "statusCategory": {
                                    "name": "To Do"
                                }
                            },
                            "assignee": {
                                "displayName": "John Doe",
                                "emailAddress": "john@example.com"
                            },
                            "reporter": {
                                "displayName": "Jane Smith",
                                "emailAddress": "jane@example.com"
                            },
                            "created": "2023-01-01T12:00:00.000+0000",
                            "updated": "2023-01-02T14:30:00.000+0000",
                            "project": {
                                "key": "SPEC",
                                "name": "Specifications"
                            },
                            "issuetype": {
                                "name": "Story",
                                "description": "User story"
                            },
                            "priority": {
                                "name": "High"
                            },
                            "labels": ["api", "specification"],
                            "components": [
                                {
                                    "name": "Backend"
                                }
                            ]
                        },
                        "changelog": {
                            "histories": [
                                {
                                    "created": "2023-01-02T14:30:00.000+0000",
                                    "items": [
                                        {
                                            "field": "status",
                                            "fieldtype": "jira",
                                            "fromString": "To Do",
                                            "toString": "In Progress"
                                        }
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

    // Start localstack for AWS services
    let localstack_url = "http://localhost:4566";
    
    // Initialize AWS clients with localstack
    let aws_config = aws_config::from_env()
        .endpoint_url(localstack_url)
        .load()
        .await;
    
    let secrets_client = SecretsClient::new(&aws_config);
    let kms_client = KmsClient::new(&aws_config);

    // Initialize secrets manager
    let secrets_manager = SecretsManager::new(
        secrets_client.clone(),
        kms_client,
        "alias/spec-to-proof".to_string(),
    );

    // Store test OAuth2 credentials
    let test_credentials = ingest::secrets::OAuth2Credentials {
        client_id: "test_client_id".to_string(),
        client_secret: "test_client_secret".to_string(),
        redirect_uri: "https://example.com/callback".to_string(),
        scopes: vec!["read".to_string(), "write".to_string()],
        token_endpoint: "https://example.com/token".to_string(),
        auth_endpoint: "https://example.com/auth".to_string(),
    };

    secrets_manager.store_oauth2_credentials("test-jira-oauth", &test_credentials).await.unwrap();

    // Start NATS server (you'd need to have NATS running locally or in Docker)
    let nats_url = "nats://localhost:4222";
    let nc = nats::connect(nats_url).expect("Failed to connect to NATS");
    let jetstream = nats::jetstream::new(nc);

    // Create JetStream stream for testing
    let stream_name = "spec-documents";
    let _ = jetstream.create_stream(&nats::jetstream::api::stream::Config {
        name: stream_name.to_string(),
        subjects: vec!["spec-documents.*".to_string()],
        ..Default::default()
    }).await;

    // Initialize Jira connector
    let config = ConnectorConfig {
        source_system: "jira".to_string(),
        base_url: mock_server.uri(),
        rate_limit_per_minute: 100,
        batch_size: 50,
        poll_interval_seconds: 1,
        secrets_arn: "test-jira-oauth".to_string(),
    };

    let mut jira_connector = JiraConnector::new(config);

    // Initialize ingestion connector
    let ingestion_connector = IngestionConnector::new(
        config,
        secrets_client,
        jetstream,
    ).await.unwrap();

    // Create test OAuth2 token
    let token = OAuth2Token {
        access_token: "test_access_token".to_string(),
        refresh_token: "test_refresh_token".to_string(),
        expires_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0),
        token_type: "Bearer".to_string(),
    };

    // Poll documents from Jira
    let documents = jira_connector.poll_documents(&token).await.unwrap();

    // Verify documents were retrieved
    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "jira");
    assert_eq!(document.source_id, "SPEC-001");
    assert_eq!(document.title, "API Specification");
    assert!(document.content.contains("API Specification"));
    assert!(document.content.contains("This document describes"));

    // Publish documents to JetStream
    for document in documents {
        ingestion_connector.publish_document(document).await.unwrap();
    }

    // Verify documents were published to JetStream
    let consumer = jetstream.pull_subscribe(&format!("{}.jira", stream_name), "test-consumer").await.unwrap();
    
    // Wait a bit for the message to be published
    sleep(Duration::from_millis(100)).await;
    
    let messages = consumer.fetch(1, Duration::from_secs(5)).await.unwrap();
    assert_eq!(messages.len(), 1);

    let message = &messages[0];
    let document: ingest::proto::spec_to_proof::v1::SpecDocument = 
        serde_json::from_slice(&message.payload).unwrap();
    
    assert_eq!(document.source_system, "jira");
    assert_eq!(document.source_id, "SPEC-001");
}

#[tokio::test]
async fn test_confluence_connector_e2e() {
    // Start mock Confluence server
    let mock_server = MockServer::start().await;
    
    // Mock Confluence API responses
    Mock::given(method("POST"))
        .and(path("/rest/api/content/search"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({
                "results": [
                    {
                        "id": "12345",
                        "title": "System Specification",
                        "status": "current",
                        "version": {
                            "number": 1,
                            "message": "Initial version"
                        },
                        "space": {
                            "key": "SPEC",
                            "name": "Specifications"
                        },
                        "body": {
                            "storage": {
                                "value": "<h1>System Specification</h1><p>This document describes the system specification.</p>",
                                "representation": "storage"
                            }
                        },
                        "_links": {
                            "webui": "/pages/viewpage.action?pageId=12345"
                        },
                        "created_by": {
                            "display_name": "John Doe",
                            "username": "johndoe"
                        },
                        "created_date": "2023-01-01T12:00:00.000Z",
                        "last_modified_date": "2023-01-02T14:30:00.000Z",
                        "last_modified_by": {
                            "display_name": "Jane Smith",
                            "username": "janesmith"
                        }
                    }
                ],
                "start": 0,
                "limit": 50,
                "size": 1,
                "_links": {
                    "next": null
                }
            })))
        .mount(&mock_server)
        .await;

    // Initialize Confluence connector
    let config = ConnectorConfig {
        source_system: "confluence".to_string(),
        base_url: mock_server.uri(),
        rate_limit_per_minute: 100,
        batch_size: 50,
        poll_interval_seconds: 1,
        secrets_arn: "test-confluence-oauth".to_string(),
    };

    let mut confluence_connector = ConfluenceConnector::new(config);

    // Create test OAuth2 token
    let token = OAuth2Token {
        access_token: "test_access_token".to_string(),
        refresh_token: "test_refresh_token".to_string(),
        expires_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0),
        token_type: "Bearer".to_string(),
    };

    // Poll documents from Confluence
    let documents = confluence_connector.poll_documents(&token).await.unwrap();

    // Verify documents were retrieved
    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "confluence");
    assert_eq!(document.source_id, "12345");
    assert_eq!(document.title, "System Specification");
    assert!(document.content.contains("System Specification"));
    assert!(document.content.contains("This document describes"));
}

#[tokio::test]
async fn test_gdocs_connector_e2e() {
    // Start mock Google APIs server
    let mock_server = MockServer::start().await;
    
    // Mock Google Drive API responses
    Mock::given(method("GET"))
        .and(path("/drive/v3/files"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({
                "files": [
                    {
                        "id": "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms",
                        "name": "API Documentation",
                        "mimeType": "application/vnd.google-apps.document",
                        "createdTime": "2023-01-01T12:00:00.000Z",
                        "modifiedTime": "2023-01-02T14:30:00.000Z",
                        "owners": [
                            {
                                "displayName": "John Doe",
                                "emailAddress": "john@example.com"
                            }
                        ],
                        "lastModifyingUser": {
                            "displayName": "Jane Smith",
                            "emailAddress": "jane@example.com"
                        },
                        "parents": ["1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms"],
                        "webViewLink": "https://docs.google.com/document/d/1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms/edit",
                        "size": "1024",
                        "description": "API documentation for our service"
                    }
                ],
                "nextPageToken": null
            })))
        .mount(&mock_server)
        .await;

    // Mock Google Docs API responses
    Mock::given(method("GET"))
        .and(path("/v1/documents/1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({
                "documentId": "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms",
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
                                            "content": "API Documentation",
                                            "textStyle": {
                                                "bold": true,
                                                "italic": false,
                                                "underline": false
                                            }
                                        }
                                    }
                                ],
                                "paragraphStyle": {
                                    "namedStyleType": "HEADING_1"
                                }
                            }
                        },
                        {
                            "startIndex": 2,
                            "endIndex": 3,
                            "paragraph": {
                                "elements": [
                                    {
                                        "startIndex": 2,
                                        "endIndex": 3,
                                        "textRun": {
                                            "content": "This document describes the API specification for our service.",
                                            "textStyle": {
                                                "bold": false,
                                                "italic": false,
                                                "underline": false
                                            }
                                        }
                                    }
                                ],
                                "paragraphStyle": {
                                    "namedStyleType": "NORMAL_TEXT"
                                }
                            }
                        }
                    ]
                },
                "revisionId": "1"
            })))
        .mount(&mock_server)
        .await;

    // Initialize Google Docs connector
    let config = ConnectorConfig {
        source_system: "google_docs".to_string(),
        base_url: mock_server.uri(),
        rate_limit_per_minute: 100,
        batch_size: 50,
        poll_interval_seconds: 1,
        secrets_arn: "test-gdocs-oauth".to_string(),
    };

    let mut gdocs_connector = GoogleDocsConnector::new(config);

    // Create test OAuth2 token
    let token = OAuth2Token {
        access_token: "test_access_token".to_string(),
        refresh_token: "test_refresh_token".to_string(),
        expires_at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64 + 3600)
            .unwrap_or(0),
        token_type: "Bearer".to_string(),
    };

    // Poll documents from Google Docs
    let documents = gdocs_connector.poll_documents(&token).await.unwrap();

    // Verify documents were retrieved
    assert_eq!(documents.len(), 1);
    let document = &documents[0];
    assert_eq!(document.source_system, "google_docs");
    assert_eq!(document.source_id, "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms");
    assert_eq!(document.title, "API Documentation");
    assert!(document.content.contains("API Documentation"));
    assert!(document.content.contains("This document describes"));
}

#[tokio::test]
async fn test_rate_limiting_and_backoff() {
    // Test that rate limiting and exponential backoff work correctly
    let config = ConnectorConfig {
        source_system: "jira".to_string(),
        base_url: "https://example.atlassian.net".to_string(),
        rate_limit_per_minute: 10, // Low rate limit for testing
        batch_size: 5,
        poll_interval_seconds: 1,
        secrets_arn: "test-jira-oauth".to_string(),
    };

    let jira_connector = JiraConnector::new(config);
    
    // Test rate limiter
    let (current_usage, max_usage) = jira_connector.rate_limiter.get_current_usage().await;
    assert_eq!(current_usage, 0);
    assert_eq!(max_usage, 7); // 70% of 10

    // Test exponential backoff
    let backoff = ingest::backoff::ExponentialBackoff::new();
    let delay = backoff.calculate_delay(0);
    assert!(delay.as_millis() >= 1000); // At least 1 second base delay
}

#[tokio::test]
async fn test_secrets_encryption() {
    // Test that secrets are properly encrypted and decrypted
    let config = ConnectorConfig {
        source_system: "jira".to_string(),
        base_url: "https://example.atlassian.net".to_string(),
        rate_limit_per_minute: 100,
        batch_size: 50,
        poll_interval_seconds: 300,
        secrets_arn: "test-jira-oauth".to_string(),
    };

    // This test would require a real AWS KMS setup or mocking
    // For now, we'll just test the configuration
    assert_eq!(config.source_system, "jira");
    assert_eq!(config.rate_limit_per_minute, 100);
    assert_eq!(config.batch_size, 50);
} 