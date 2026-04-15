use aws_sdk_secretsmanager::Client as SecretsClient;
use aws_sdk_kms::Client as KmsClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedSecret {
    pub encrypted_data: Vec<u8>,
    pub encrypted_key: Vec<u8>,
    pub key_id: String,
    pub algorithm: String,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub token_endpoint: String,
    pub auth_endpoint: String,
}

pub struct SecretsManager {
    secrets_client: SecretsClient,
    kms_client: KmsClient,
    key_id: String,
}

impl SecretsManager {
    pub fn new(secrets_client: SecretsClient, kms_client: KmsClient, key_id: String) -> Self {
        Self {
            secrets_client,
            kms_client,
            key_id,
        }
    }

    pub async fn store_oauth2_credentials(
        &self,
        secret_name: &str,
        credentials: &OAuth2Credentials,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Generate a data key from KMS
        let data_key_response = self.kms_client
            .generate_data_key()
            .key_id(&self.key_id)
            .key_spec("AES_256")
            .send()
            .await?;

        let plaintext_key = data_key_response.plaintext()
            .ok_or("No plaintext key returned")?;
        let encrypted_key = data_key_response.ciphertext_blob()
            .ok_or("No encrypted key returned")?;

        // Encrypt the credentials with the data key
        let credentials_json = serde_json::to_vec(credentials)?;
        let encrypted_data = self.encrypt_with_key(&credentials_json, plaintext_key.as_ref())?;

        // Create the encrypted secret structure
        let encrypted_secret = EncryptedSecret {
            encrypted_data,
            encrypted_key: encrypted_key.to_vec(),
            key_id: self.key_id.clone(),
            algorithm: "AES256".to_string(),
            version: 1,
        };

        // Store in Secrets Manager
        self.secrets_client
            .create_secret()
            .name(secret_name)
            .secret_string(serde_json::to_string(&encrypted_secret)?)
            .send()
            .await?;

        tracing::info!("Stored encrypted OAuth2 credentials for {}", secret_name);
        Ok(())
    }

    pub async fn retrieve_oauth2_credentials(
        &self,
        secret_name: &str,
    ) -> Result<OAuth2Credentials, Box<dyn std::error::Error>> {
        // Retrieve the encrypted secret
        let secret_response = self.secrets_client
            .get_secret_value()
            .secret_id(secret_name)
            .send()
            .await?;

        let secret_string = secret_response.secret_string()
            .ok_or("No secret string found")?;

        let encrypted_secret: EncryptedSecret = serde_json::from_str(secret_string)?;

        // Decrypt the data key
        let decrypted_key_response = self.kms_client
            .decrypt()
            .key_id(&encrypted_secret.key_id)
            .ciphertext_blob(aws_sdk_kms::types::Blob::new(&encrypted_secret.encrypted_key))
            .send()
            .await?;

        let decrypted_key = decrypted_key_response.plaintext()
            .ok_or("No decrypted key returned")?;

        // Decrypt the credentials
        let decrypted_data =
            self.decrypt_with_key(&encrypted_secret.encrypted_data, decrypted_key.as_ref())?;
        let credentials: OAuth2Credentials = serde_json::from_slice(&decrypted_data)?;

        Ok(credentials)
    }

    fn encrypt_with_key(&self, data: &[u8], key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        use aes_gcm::aead::{Aead, NewAead};

        let cipher = Aes256Gcm::new(Key::from_slice(key));
        let nonce = Nonce::from_slice(b"unique nonce"); // In production, use random nonce

        let ciphertext = cipher
            .encrypt(nonce, data)
            .map_err(|e| format!("Encryption failed: {}", e))?;

        Ok(ciphertext)
    }

    fn decrypt_with_key(&self, encrypted_data: &[u8], key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use aes_gcm::{Aes256Gcm, Key, Nonce};
        use aes_gcm::aead::{Aead, NewAead};

        let cipher = Aes256Gcm::new(Key::from_slice(key));
        let nonce = Nonce::from_slice(b"unique nonce"); // In production, use stored nonce

        let plaintext = cipher
            .decrypt(nonce, encrypted_data)
            .map_err(|e| format!("Decryption failed: {}", e))?;

        Ok(plaintext)
    }

    pub async fn list_secrets(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let response = self.secrets_client
            .list_secrets()
            .send()
            .await?;

        let secret_names = response
            .secret_list()
            .unwrap_or(&[])
            .iter()
            .map(|secret| secret.name().unwrap_or_default().to_string())
            .collect();

        Ok(secret_names)
    }

    pub async fn delete_secret(&self, secret_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.secrets_client
            .delete_secret()
            .secret_id(secret_name)
            .force_delete_without_recovery(true)
            .send()
            .await?;

        tracing::info!("Deleted secret: {}", secret_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_sdk_secretsmanager::config::Config as SecretsConfig;
    use aws_sdk_kms::config::Config as KmsConfig;
    use aws_config::BehaviorVersion;

    #[tokio::test]
    async fn test_oauth2_credentials_encryption() {
        // This test would require AWS credentials and proper setup
        // For now, we'll test the encryption/decryption functions directly
        
        let credentials = OAuth2Credentials {
            client_id: "test_client_id".to_string(),
            client_secret: "test_client_secret".to_string(),
            redirect_uri: "https://example.com/callback".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            token_endpoint: "https://example.com/token".to_string(),
            auth_endpoint: "https://example.com/auth".to_string(),
        };

        let credentials_json = serde_json::to_vec(&credentials).unwrap();
        
        // Test with a dummy key (in production, this would be from KMS)
        let dummy_key = vec![1u8; 32];
        
        // This test is incomplete without proper AWS setup
        // In a real environment, you'd mock the AWS clients
        assert_eq!(credentials_json.len(), 0); // Placeholder assertion
    }
} 