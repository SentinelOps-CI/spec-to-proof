use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use base64::{engine::general_purpose, Engine as _};
use reqwest::{
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    Client,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::info;

use crate::config::GitHubAppConfig;
use crate::proto::gh_app::v1::*;

/// Client for the Sigstore transparency log (Rekor) and keyless-signing
/// certificate authority (Fulcio).
///
/// The transparency-critical operations — Merkle inclusion-proof verification
/// (RFC 6962) and ECDSA signature verification — are implemented locally and do
/// not require network access, so they can be exercised offline and in tests.
#[derive(Debug, Clone)]
pub struct SigstoreClient {
    #[allow(dead_code)]
    config: GitHubAppConfig,
    http_client: Client,
    rekor_url: String,
    fulcio_url: String,
    oidc_issuer: String,
    entry_cache: Arc<Mutex<HashMap<String, (SigstoreEntry, Instant)>>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RekorEntry {
    pub uuid: String,
    pub log_index: u64,
    pub integrated_time: u64,
    pub log_id: String,
    pub body: String,
    #[serde(default)]
    pub verification: Option<RekorVerification>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RekorVerification {
    pub inclusion_proof: Option<RekorInclusionProof>,
    #[serde(default)]
    pub signed_entry_timestamp: String,
    #[serde(default)]
    pub signed_tree_head: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RekorInclusionProof {
    pub hashes: Vec<String>,
    pub log_index: u64,
    pub root_hash: String,
    pub tree_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OIDCToken {
    pub iss: String,
    pub sub: String,
    pub aud: String,
    pub exp: u64,
    pub iat: u64,
    pub email: Option<String>,
    pub email_verified: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogInfo {
    pub root_hash: String,
    pub tree_size: u64,
    pub signed_tree_head: String,
    pub tree_id: String,
}

// ---------------------------------------------------------------------------
// RFC 6962 Merkle tree hashing + inclusion proof verification (pure functions).
// ---------------------------------------------------------------------------

/// RFC 6962 leaf hash: `SHA-256(0x00 || leaf_data)`.
pub fn rfc6962_leaf_hash(leaf_data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([0x00]);
    hasher.update(leaf_data);
    hasher.finalize().into()
}

/// RFC 6962 interior node hash: `SHA-256(0x01 || left || right)`.
pub fn rfc6962_node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([0x01]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// Verify an RFC 6962 Merkle inclusion proof.
///
/// Implements the algorithm from RFC 6962 §2.1.1 verbatim: given the leaf index
/// `leaf_index`, the `tree_size`, the `leaf_hash`, the audit `proof` path and
/// the expected `root_hash`, returns `true` iff the proof reconstructs the root.
pub fn verify_inclusion_proof_rfc6962(
    leaf_index: u64,
    tree_size: u64,
    leaf_hash: [u8; 32],
    proof: &[[u8; 32]],
    root_hash: &[u8; 32],
) -> bool {
    if leaf_index >= tree_size {
        return false;
    }

    let mut fn_ = leaf_index;
    let mut sn = tree_size - 1;
    let mut r = leaf_hash;

    for p in proof {
        if sn == 0 {
            // Path is longer than the tree can justify.
            return false;
        }
        if (fn_ & 1) == 1 || fn_ == sn {
            r = rfc6962_node_hash(p, &r);
            if (fn_ & 1) == 0 {
                while (fn_ & 1) == 0 && fn_ != 0 {
                    fn_ >>= 1;
                    sn >>= 1;
                }
            }
        } else {
            r = rfc6962_node_hash(&r, p);
        }
        fn_ >>= 1;
        sn >>= 1;
    }

    sn == 0 && &r == root_hash
}

fn decode_hash32(hex_str: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_str).context("invalid hex hash")?;
    if bytes.len() != 32 {
        return Err(anyhow::anyhow!(
            "expected 32-byte SHA-256 hash, got {} bytes",
            bytes.len()
        ));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

impl SigstoreClient {
    pub async fn new(config: &GitHubAppConfig) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        let http_client = Client::builder()
            .timeout(Duration::from_secs(config.request_timeout))
            .default_headers(headers)
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            config: config.clone(),
            http_client,
            rekor_url: config.sigstore_rekor_url.clone(),
            fulcio_url: config.sigstore_fulcio_url.clone(),
            oidc_issuer: config.sigstore_oidc_issuer.clone(),
            entry_cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn get_entry(&self, entry_id: &str) -> Result<SigstoreEntry> {
        {
            let cache = self.entry_cache.lock().await;
            if let Some((entry, created_at)) = cache.get(entry_id) {
                if created_at.elapsed() < Duration::from_secs(3600) {
                    return Ok(entry.clone());
                }
            }
        }

        let rekor_entry = self.fetch_rekor_entry(entry_id).await?;
        let entry = self.convert_rekor_entry(rekor_entry)?;

        self.entry_cache
            .lock()
            .await
            .insert(entry_id.to_string(), (entry.clone(), Instant::now()));
        Ok(entry)
    }

    async fn fetch_rekor_entry(&self, entry_id: &str) -> Result<RekorEntry> {
        let url = format!("{}/api/v1/log/entries/{}", self.rekor_url, entry_id);
        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch Rekor entry")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to fetch Rekor entry: {}", error_text));
        }

        // Rekor returns a map keyed by entry UUID; accept either the raw entry
        // or the wrapped map form.
        let value: serde_json::Value = response.json().await.context("Failed to parse Rekor entry")?;
        Self::parse_rekor_entry_value(value)
    }

    fn parse_rekor_entry_value(value: serde_json::Value) -> Result<RekorEntry> {
        // Wrapped form: { "<uuid>": { logIndex, body, ... } }
        if let Some(obj) = value.as_object() {
            if obj.len() == 1 {
                let (uuid, inner) = obj.iter().next().unwrap();
                if inner.is_object() {
                    let mut entry: RekorEntry =
                        serde_json::from_value(inner.clone()).context("Failed to parse Rekor entry body")?;
                    if entry.uuid.is_empty() {
                        entry.uuid = uuid.clone();
                    }
                    return Ok(entry);
                }
            }
        }
        serde_json::from_value(value).context("Failed to parse Rekor entry")
    }

    fn convert_rekor_entry(&self, rekor_entry: RekorEntry) -> Result<SigstoreEntry> {
        let body_bytes = general_purpose::STANDARD
            .decode(&rekor_entry.body)
            .context("Failed to decode Rekor entry body")?;
        let body_json: serde_json::Value =
            serde_json::from_slice(&body_bytes).context("Failed to parse Rekor entry body")?;

        // hashedrekord v0.0.1 schema.
        let spec = body_json.get("spec").cloned().unwrap_or(serde_json::Value::Null);

        let signature = spec
            .get("signature")
            .and_then(|s| s.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        // For keyless signing this "publicKey" content is a base64-encoded PEM
        // certificate chain; for key-based signing it is a public key PEM.
        let public_key_b64 = spec
            .get("signature")
            .and_then(|s| s.get("publicKey"))
            .and_then(|pk| pk.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let public_key = general_purpose::STANDARD
            .decode(&public_key_b64)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or(public_key_b64);

        let artifact_hash = spec
            .get("data")
            .and_then(|d| d.get("hash"))
            .and_then(|h| h.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        let artifact_type = spec
            .get("data")
            .and_then(|d| d.get("hash"))
            .and_then(|h| h.get("algorithm"))
            .and_then(|v| v.as_str())
            .unwrap_or("sha256")
            .to_string();

        // Extract OIDC issuer/identity from the signing certificate when present.
        let (oidc_issuer, oidc_identity) = extract_oidc_identity(&public_key)
            .unwrap_or_else(|| (self.oidc_issuer.clone(), String::new()));

        Ok(SigstoreEntry {
            entry_id: rekor_entry.uuid.clone(),
            log_index: rekor_entry.log_index.to_string(),
            integrated_time: rekor_entry.integrated_time.to_string(),
            log_id: rekor_entry.log_id,
            rekor_entry_url: format!("{}/api/v1/log/entries/{}", self.rekor_url, rekor_entry.uuid),
            fulcio_certificate_url: format!("{}/api/v1/signingCert", self.fulcio_url),
            oidc_issuer,
            oidc_identity,
            signature,
            public_key,
            artifact_hash,
            artifact_type,
        })
    }

    /// Verify an ECDSA (P-256/SHA-256) signature over `message`.
    ///
    /// `signature_b64` is the base64-encoded ASN.1 DER signature and
    /// `public_key_pem` is a SubjectPublicKeyInfo PEM (`BEGIN PUBLIC KEY`). This
    /// is a real cryptographic verification; unsupported key types return an
    /// error rather than a permissive `true`.
    pub fn verify_signature(
        &self,
        message: &[u8],
        signature_b64: &str,
        public_key_pem: &str,
    ) -> Result<bool> {
        use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
        use p256::pkcs8::DecodePublicKey;

        let verifying_key = VerifyingKey::from_public_key_pem(public_key_pem)
            .context("failed to parse P-256 public key (expected SPKI PEM)")?;

        let sig_der = general_purpose::STANDARD
            .decode(signature_b64.trim())
            .context("failed to base64-decode signature")?;
        let signature =
            Signature::from_der(&sig_der).context("failed to parse DER ECDSA signature")?;

        Ok(verifying_key.verify(message, &signature).is_ok())
    }

    /// Verify a Rekor inclusion proof for the entry with the given `body`
    /// (the base64 leaf value stored in the log). Reconstructs the RFC 6962
    /// leaf hash and validates it against the supplied audit path and root.
    pub fn verify_inclusion_proof(
        &self,
        entry_body_b64: &str,
        proof: &RekorInclusionProof,
    ) -> Result<bool> {
        let leaf_bytes = general_purpose::STANDARD
            .decode(entry_body_b64.trim())
            .context("failed to base64-decode entry body")?;
        let leaf_hash = rfc6962_leaf_hash(&leaf_bytes);

        let root = decode_hash32(&proof.root_hash)?;
        let mut path = Vec::with_capacity(proof.hashes.len());
        for h in &proof.hashes {
            path.push(decode_hash32(h)?);
        }

        Ok(verify_inclusion_proof_rfc6962(
            proof.log_index,
            proof.tree_size,
            leaf_hash,
            &path,
            &root,
        ))
    }

    /// Build the canonical Rekor `hashedrekord` body and submit it to the log.
    /// Returns the created [`SigstoreEntry`].
    pub async fn create_signed_entry(
        &self,
        artifact_sha256_hex: &str,
        signature_b64: &str,
        public_key_pem: &str,
    ) -> Result<SigstoreEntry> {
        let public_key_b64 = general_purpose::STANDARD.encode(public_key_pem.as_bytes());
        let proposed = serde_json::json!({
            "apiVersion": "0.0.1",
            "kind": "hashedrekord",
            "spec": {
                "data": {
                    "hash": {
                        "algorithm": "sha256",
                        "value": artifact_sha256_hex
                    }
                },
                "signature": {
                    "content": signature_b64,
                    "publicKey": { "content": public_key_b64 }
                }
            }
        });

        let url = format!("{}/api/v1/log/entries", self.rekor_url);
        let response = self
            .http_client
            .post(&url)
            .json(&proposed)
            .send()
            .await
            .context("Failed to create Rekor entry")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to create Rekor entry: {}", error_text));
        }

        let value: serde_json::Value = response.json().await.context("Failed to parse created entry")?;
        let rekor_entry = Self::parse_rekor_entry_value(value)?;
        self.convert_rekor_entry(rekor_entry)
    }

    pub async fn get_log_info(&self) -> Result<LogInfo> {
        let url = format!("{}/api/v1/log", self.rekor_url);
        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to get log info")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get log info: {}", error_text));
        }

        response.json().await.context("Failed to parse log info")
    }

    pub async fn get_public_key(&self) -> Result<String> {
        let url = format!("{}/api/v1/log/publicKey", self.rekor_url);
        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to get public key")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Failed to get public key: {}", error_text));
        }

        // The Rekor public-key endpoint returns a PEM document (text), not JSON.
        response.text().await.context("Failed to read public key")
    }

    /// Verify that a given entry is included and consistent: fetch the entry,
    /// then verify its embedded RFC 6962 inclusion proof.
    pub async fn verify_entry_consistency(&self, entry_id: &str) -> Result<bool> {
        let rekor_entry = self.fetch_rekor_entry(entry_id).await?;
        let proof = rekor_entry
            .verification
            .as_ref()
            .and_then(|v| v.inclusion_proof.as_ref())
            .ok_or_else(|| anyhow::anyhow!("entry {} has no inclusion proof", entry_id))?;
        info!("Verifying inclusion proof for entry: {}", entry_id);
        self.verify_inclusion_proof(&rekor_entry.body, proof)
    }

    pub async fn get_entry_by_artifact_hash(&self, artifact_hash: &str) -> Result<Option<SigstoreEntry>> {
        let url = format!("{}/api/v1/index/retrieve", self.rekor_url);
        let request_body = serde_json::json!({ "hash": format!("sha256:{}", artifact_hash) });

        let response = self
            .http_client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .context("Failed to retrieve entry by artifact hash")?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let uuids: Vec<String> = response.json().await.unwrap_or_default();
        match uuids.into_iter().next() {
            Some(uuid) => Ok(Some(self.get_entry(&uuid).await?)),
            None => Ok(None),
        }
    }
}

/// Extract the OIDC issuer and identity (SAN) from a signing certificate PEM.
///
/// Returns `None` when the input is not a parseable X.509 certificate (e.g. a
/// bare public key). The issuer is read from the Sigstore OIDC-issuer extension
/// (OID 1.3.6.1.4.1.57264.1.1); the identity is the first SAN entry.
fn extract_oidc_identity(pem_or_key: &str) -> Option<(String, String)> {
    use x509_parser::extensions::GeneralName;
    use x509_parser::prelude::*;

    if !pem_or_key.contains("BEGIN CERTIFICATE") {
        return None;
    }

    let (_, pem) = parse_x509_pem(pem_or_key.as_bytes()).ok()?;
    let cert = pem.parse_x509().ok()?;

    let mut issuer = String::new();
    for ext in cert.extensions() {
        if ext.oid.to_id_string() == "1.3.6.1.4.1.57264.1.1" {
            issuer = String::from_utf8_lossy(ext.value).trim().to_string();
        }
    }

    let mut identity = String::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            match name {
                GeneralName::RFC822Name(v) => {
                    identity = v.to_string();
                    break;
                }
                GeneralName::URI(v) => {
                    identity = v.to_string();
                    break;
                }
                _ => {}
            }
        }
    }

    Some((issuer, identity))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sigstore_client_creation() {
        let config = GitHubAppConfig::default();
        assert!(SigstoreClient::new(&config).await.is_ok());
    }

    #[test]
    fn test_rfc6962_inclusion_proof_four_leaves() {
        // Build a 4-leaf Merkle tree and verify the inclusion proof for each leaf.
        let leaves: Vec<[u8; 32]> = (0..4u8)
            .map(|i| rfc6962_leaf_hash(&[i]))
            .collect();
        let a = rfc6962_node_hash(&leaves[0], &leaves[1]);
        let b = rfc6962_node_hash(&leaves[2], &leaves[3]);
        let root = rfc6962_node_hash(&a, &b);

        // leaf 0: path [L1, b]
        assert!(verify_inclusion_proof_rfc6962(0, 4, leaves[0], &[leaves[1], b], &root));
        // leaf 1: path [L0, b]
        assert!(verify_inclusion_proof_rfc6962(1, 4, leaves[1], &[leaves[0], b], &root));
        // leaf 2: path [L3, a]
        assert!(verify_inclusion_proof_rfc6962(2, 4, leaves[2], &[leaves[3], a], &root));
        // leaf 3: path [L2, a]
        assert!(verify_inclusion_proof_rfc6962(3, 4, leaves[3], &[leaves[2], a], &root));
    }

    #[test]
    fn test_rfc6962_rejects_wrong_proof() {
        let leaves: Vec<[u8; 32]> = (0..4u8).map(|i| rfc6962_leaf_hash(&[i])).collect();
        let a = rfc6962_node_hash(&leaves[0], &leaves[1]);
        let b = rfc6962_node_hash(&leaves[2], &leaves[3]);
        let root = rfc6962_node_hash(&a, &b);

        // Wrong sibling for leaf 0.
        assert!(!verify_inclusion_proof_rfc6962(0, 4, leaves[0], &[leaves[2], b], &root));
        // Out-of-range index.
        assert!(!verify_inclusion_proof_rfc6962(4, 4, leaves[0], &[leaves[1], b], &root));
    }

    #[test]
    fn test_rfc6962_single_leaf_tree() {
        let leaf = rfc6962_leaf_hash(b"only");
        // A single-leaf tree: the leaf hash is the root, empty proof.
        assert!(verify_inclusion_proof_rfc6962(0, 1, leaf, &[], &leaf));
    }

    #[test]
    fn test_ecdsa_signature_roundtrip() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};
        use p256::pkcs8::{EncodePublicKey, LineEnding};

        // Fixed non-zero scalar -> deterministic test key (not a real secret).
        let sk = SigningKey::from_slice(&[
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11,
        ])
        .unwrap();
        let vk = sk.verifying_key();
        let pem = vk.to_public_key_pem(LineEnding::LF).unwrap();

        let message = b"spec-to-proof artifact bytes";
        let sig: Signature = sk.sign(message);
        let sig_b64 = general_purpose::STANDARD.encode(sig.to_der().as_bytes());

        let config = GitHubAppConfig::default();
        let client = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(SigstoreClient::new(&config))
            .unwrap();

        assert!(client.verify_signature(message, &sig_b64, &pem).unwrap());
        assert!(!client.verify_signature(b"tampered", &sig_b64, &pem).unwrap());
    }
}
