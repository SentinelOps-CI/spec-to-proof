use std::collections::HashMap;
use std::error::Error;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::types::ServerSideEncryption;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_kms::Client as KmsClient;

use crate::proto::proof::v1::*;
use crate::proto::spec_to_proof::v1::*;
use crate::ProofConfig;

pub struct S3Storage {
    s3_client: S3Client,
    kms_client: Option<KmsClient>,
    config: ProofConfig,
}

impl S3Storage {
    pub async fn new(config: &ProofConfig) -> Result<Self, Box<dyn Error>> {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        
        let s3_client = S3Client::new(&aws_config);
        let kms_client = if config.kms_key_id.is_some() {
            Some(KmsClient::new(&aws_config))
        } else {
            None
        };

        Ok(Self {
            s3_client,
            kms_client,
            config: config.clone(),
        })
    }

    pub async fn upload_theorem(
        &self,
        theorem: &LeanTheorem,
        version: &str,
        s3_config: &S3Config,
    ) -> Result<String, Box<dyn Error>> {
        let key = self.generate_s3_key(theorem, version, s3_config);
        
        // Prepare encryption settings
        let encryption_config = self.build_encryption_config(s3_config).await?;
        
        // Upload the theorem code
        let body = ByteStream::from(bytes::Bytes::copy_from_slice(theorem.lean_code.as_bytes()));
        
        let mut upload_request = self.s3_client
            .put_object()
            .bucket(&s3_config.bucket_name)
            .key(&key)
            .body(body)
            .content_type("text/plain");

        // Apply encryption if configured
        if let Some(encryption) = encryption_config {
            upload_request = upload_request.set_server_side_encryption(Some(encryption));
        }

        // Add metadata
        let mut metadata = HashMap::new();
        metadata.insert("theorem_id".to_string(), theorem.id.clone());
        metadata.insert("theorem_name".to_string(), theorem.theorem_name.clone());
        metadata.insert("source_invariant_id".to_string(), theorem.source_invariant_id.clone());
        metadata.insert("content_hash".to_string(), theorem.content_sha256.clone());
        metadata.insert("version".to_string(), version.to_string());
        metadata.insert("proof_strategy".to_string(), theorem.proof_strategy.clone());
        metadata.insert("uploaded_at".to_string(), std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string());

        upload_request = upload_request.set_metadata(Some(metadata));

        // Execute upload
        let _result = upload_request.send().await?;
        
        // Generate S3 location URL
        let s3_location = format!(
            "s3://{}/{}",
            s3_config.bucket_name,
            key
        );

        tracing::info!("Successfully uploaded theorem {} to {}", theorem.theorem_name, s3_location);

        Ok(s3_location)
    }

    pub async fn download_theorem(
        &self,
        s3_location: &str,
    ) -> Result<LeanTheorem, Box<dyn Error>> {
        // Parse S3 location to extract bucket and key
        let (bucket, key) = self.parse_s3_location(s3_location)?;
        
        // Download the object
        let result = self.s3_client
            .get_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await?;

        // Read the content
        let metadata = result.metadata().cloned().unwrap_or_default();

        let body = result.body.collect().await?;
        let lean_code = String::from_utf8(body.into_bytes().to_vec())?;
        
        // Reconstruct LeanTheorem (simplified - in real implementation, you'd store full proto)
        let theorem = LeanTheorem {
            id: metadata.get("theorem_id").unwrap_or(&"unknown".to_string()).clone(),
            content_sha256: metadata.get("content_hash").unwrap_or(&"".to_string()).clone(),
            theorem_name: metadata.get("theorem_name").unwrap_or(&"unknown".to_string()).clone(),
            lean_code,
            source_invariant_id: metadata.get("source_invariant_id").unwrap_or(&"".to_string()).clone(),
            generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            status: TheoremStatus::Generated as i32,
            compilation_errors: Vec::new(),
            proof_strategy: metadata.get("proof_strategy").unwrap_or(&"".to_string()).clone(),
            metadata: HashMap::new(),
        };

        Ok(theorem)
    }

    pub async fn list_theorems(
        &self,
        prefix: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let result = self.s3_client
            .list_objects_v2()
            .bucket(&self.config.s3_bucket)
            .prefix(prefix)
            .send()
            .await?;

        let keys: Vec<String> = result.contents()
            .iter()
            .map(|obj| obj.key().unwrap_or("").to_string())
            .collect();

        Ok(keys)
    }

    pub async fn delete_theorem(
        &self,
        s3_location: &str,
    ) -> Result<(), Box<dyn Error>> {
        let (bucket, key) = self.parse_s3_location(s3_location)?;
        
        self.s3_client
            .delete_object()
            .bucket(bucket)
            .key(key)
            .send()
            .await?;

        tracing::info!("Successfully deleted theorem from {}", s3_location);

        Ok(())
    }

    fn generate_s3_key(
        &self,
        theorem: &LeanTheorem,
        version: &str,
        s3_config: &S3Config,
    ) -> String {
        let prefix = if s3_config.key_prefix.is_empty() {
            "theorems/"
        } else {
            s3_config.key_prefix.as_str()
        };
        let invariant_hash = &theorem.content_sha256[..8]; // Use first 8 chars for readability
        
        format!(
            "{}{}/{}/{}.lean",
            prefix,
            invariant_hash,
            version,
            theorem.theorem_name
        )
    }

    async fn build_encryption_config(
        &self,
        s3_config: &S3Config,
    ) -> Result<Option<ServerSideEncryption>, Box<dyn Error>> {
        if let Some(encryption) = &s3_config.encryption {
            match encryption.sse_algorithm.as_str() {
                "AES256" => {
                    Ok(Some(ServerSideEncryption::Aes256))
                }
                "aws:kms" => {
                    if !encryption.kms_key_id.is_empty() {
                        let key_id = encryption.kms_key_id.clone();
                        if let Some(kms_client) = &self.kms_client {
                            kms_client
                                .describe_key()
                                .key_id(&key_id)
                                .send()
                                .await?;
                        }
                        
                        Ok(Some(ServerSideEncryption::AwsKms))
                    } else {
                        Err("KMS key ID required for aws:kms encryption".into())
                    }
                }
                _ => {
                    Err(format!("Unsupported encryption algorithm: {}", encryption.sse_algorithm).into())
                }
            }
        } else {
            Ok(None)
        }
    }

    fn parse_s3_location(&self, s3_location: &str) -> Result<(String, String), Box<dyn Error>> {
        if !s3_location.starts_with("s3://") {
            return Err("Invalid S3 location format".into());
        }
        
        let path = &s3_location[5..]; // Remove "s3://"
        let parts: Vec<&str> = path.splitn(2, '/').collect();
        
        if parts.len() != 2 {
            return Err("Invalid S3 location format".into());
        }
        
        Ok((parts[0].to_string(), parts[1].to_string()))
    }

    pub async fn create_bucket_if_not_exists(&self) -> Result<(), Box<dyn Error>> {
        // Check if bucket exists
        match self.s3_client
            .head_bucket()
            .bucket(&self.config.s3_bucket)
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!("S3 bucket {} already exists", self.config.s3_bucket);
                Ok(())
            }
            Err(_) => {
                // Create bucket
                let mut create_request = self.s3_client
                    .create_bucket()
                    .bucket(&self.config.s3_bucket);

                // Set region if specified
                if !self.config.s3_region.is_empty() {
                    create_request = create_request.create_bucket_configuration(
                        aws_sdk_s3::types::CreateBucketConfiguration::builder()
                            .location_constraint(aws_sdk_s3::types::BucketLocationConstraint::from(
                                self.config.s3_region.as_str()
                            ))
                            .build()
                    );
                }

                create_request.send().await?;
                tracing::info!("Created S3 bucket {}", self.config.s3_bucket);
                Ok(())
            }
        }
    }

    pub async fn enable_versioning(&self) -> Result<(), Box<dyn Error>> {
        self.s3_client
            .put_bucket_versioning()
            .bucket(&self.config.s3_bucket)
            .versioning_configuration(
                aws_sdk_s3::types::VersioningConfiguration::builder()
                    .status(aws_sdk_s3::types::BucketVersioningStatus::Enabled)
                    .build()
            )
            .send()
            .await?;

        tracing::info!("Enabled versioning for S3 bucket {}", self.config.s3_bucket);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_config::BehaviorVersion;

    fn test_sdk_config() -> aws_config::SdkConfig {
        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(aws_config::defaults(BehaviorVersion::latest()).load())
    }

    #[test]
    fn test_generate_s3_key() {
        let config = ProofConfig::default();
        let storage = S3Storage {
            s3_client: S3Client::new(&test_sdk_config()),
            kms_client: None,
            config,
        };

        let theorem = LeanTheorem {
            id: "test_theorem".to_string(),
            content_sha256: "a1b2c3d4e5f6789012345678901234567890abcdef1234567890abcdef1234".to_string(),
            theorem_name: "test_theorem".to_string(),
            lean_code: "test".to_string(),
            source_invariant_id: "inv1".to_string(),
            generated_at: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
            status: TheoremStatus::Generated as i32,
            compilation_errors: Vec::new(),
            proof_strategy: "induction".to_string(),
            metadata: HashMap::new(),
        };

        let s3_config = S3Config {
            bucket_name: "test-bucket".to_string(),
            key_prefix: "theorems/".to_string(),
            region: "us-east-1".to_string(),
            encryption: None,
        };

        let key = storage.generate_s3_key(&theorem, "v1", &s3_config);
        assert!(key.starts_with("theorems/a1b2c3d4/v1/test_theorem.lean"));
    }

    #[test]
    fn test_parse_s3_location() {
        let config = ProofConfig::default();
        let storage = S3Storage {
            s3_client: S3Client::new(&test_sdk_config()),
            kms_client: None,
            config,
        };

        let s3_location = "s3://test-bucket/theorems/test.lean";
        let (bucket, key) = storage.parse_s3_location(s3_location).unwrap();
        
        assert_eq!(bucket, "test-bucket");
        assert_eq!(key, "theorems/test.lean");
    }

    #[test]
    fn test_parse_s3_location_invalid() {
        let config = ProofConfig::default();
        let storage = S3Storage {
            s3_client: S3Client::new(&test_sdk_config()),
            kms_client: None,
            config,
        };

        let invalid_location = "invalid-location";
        let result = storage.parse_s3_location(invalid_location);
        assert!(result.is_err());
    }
} 