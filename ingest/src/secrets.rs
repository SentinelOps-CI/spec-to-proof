use aws_sdk_secretsmanager::Client as SecretsClient;
use aws_sdk_kms::Client as KmsClient;
use serde::{Deserialize, Serialize};

/// AES-256-GCM nonce length in bytes (96 bits, the standard/recommended size).
const GCM_NONCE_LEN: usize = 12;

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
            .key_spec(aws_sdk_kms::types::DataKeySpec::Aes256)
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
            encrypted_key: encrypted_key.as_ref().to_vec(),
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
            .ciphertext_blob(aws_sdk_kms::primitives::Blob::new(
                encrypted_secret.encrypted_key.clone(),
            ))
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
        encrypt_aes_256_gcm(data, key)
    }

    fn decrypt_with_key(&self, encrypted_data: &[u8], key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        decrypt_aes_256_gcm(encrypted_data, key)
    }

    pub async fn list_secrets(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let response = self.secrets_client
            .list_secrets()
            .send()
            .await?;

        let secret_names = response
            .secret_list()
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

/// Encrypt `data` under a 32-byte AES-256 key using AES-256-GCM.
///
/// A fresh random 96-bit nonce is generated for every call and prepended to
/// the returned ciphertext (`nonce || ciphertext || tag`). Reusing a nonce
/// with GCM is catastrophic, so a per-message random nonce is used rather than
/// a hard-coded value.
fn encrypt_aes_256_gcm(data: &[u8], key: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use rand::RngCore;

    if key.len() != 32 {
        return Err(format!("AES-256 key must be 32 bytes, got {}", key.len()).into());
    }

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; GCM_NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let mut out = Vec::with_capacity(GCM_NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a payload produced by [`encrypt_aes_256_gcm`], i.e. a
/// `nonce || ciphertext || tag` blob under a 32-byte AES-256 key.
fn decrypt_aes_256_gcm(
    encrypted: &[u8],
    key: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};

    if key.len() != 32 {
        return Err(format!("AES-256 key must be 32 bytes, got {}", key.len()).into());
    }
    if encrypted.len() < GCM_NONCE_LEN {
        return Err("ciphertext too short to contain a nonce".into());
    }

    let (nonce_bytes, ciphertext) = encrypted.split_at(GCM_NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_round_trip() {
        let credentials = OAuth2Credentials {
            client_id: "test_client_id".to_string(),
            client_secret: "test_client_secret".to_string(),
            redirect_uri: "https://example.com/callback".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            token_endpoint: "https://example.com/token".to_string(),
            auth_endpoint: "https://example.com/auth".to_string(),
        };

        let plaintext = serde_json::to_vec(&credentials).unwrap();
        let key = [7u8; 32];

        let ciphertext = encrypt_aes_256_gcm(&plaintext, &key).unwrap();
        // Ciphertext must differ from plaintext and carry the nonce + GCM tag.
        assert_ne!(ciphertext, plaintext);
        assert!(ciphertext.len() > plaintext.len() + GCM_NONCE_LEN);

        let recovered = decrypt_aes_256_gcm(&ciphertext, &key).unwrap();
        assert_eq!(recovered, plaintext);

        let round_tripped: OAuth2Credentials = serde_json::from_slice(&recovered).unwrap();
        assert_eq!(round_tripped.client_id, credentials.client_id);
        assert_eq!(round_tripped.client_secret, credentials.client_secret);
    }

    #[test]
    fn test_encrypt_uses_random_nonce() {
        let key = [3u8; 32];
        let data = b"same plaintext";
        let a = encrypt_aes_256_gcm(data, &key).unwrap();
        let b = encrypt_aes_256_gcm(data, &key).unwrap();
        // Distinct random nonces mean identical plaintext encrypts differently.
        assert_ne!(a, b);
        assert_eq!(decrypt_aes_256_gcm(&a, &key).unwrap(), data);
        assert_eq!(decrypt_aes_256_gcm(&b, &key).unwrap(), data);
    }

    #[test]
    fn test_decrypt_rejects_wrong_key() {
        let data = b"secret";
        let ciphertext = encrypt_aes_256_gcm(data, &[1u8; 32]).unwrap();
        assert!(decrypt_aes_256_gcm(&ciphertext, &[2u8; 32]).is_err());
    }

    #[test]
    fn test_encrypt_rejects_bad_key_length() {
        assert!(encrypt_aes_256_gcm(b"x", &[0u8; 16]).is_err());
    }
} 