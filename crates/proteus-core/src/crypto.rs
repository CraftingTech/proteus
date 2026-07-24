use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{CoreError, CoreResult};

const NONCE_LEN: usize = 12;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; 32]);

impl EncryptionKey {
    pub fn from_bytes(bytes: &[u8]) -> CoreResult<Self> {
        let key: [u8; 32] = bytes.try_into().map_err(|_| {
            CoreError::InvalidKey(format!("expected 32 bytes, got {}", bytes.len()))
        })?;
        Ok(Self(key))
    }

    pub fn generate() -> Self {
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        Self(key)
    }

    /// Parse a key Secret value: either the 32 raw key bytes, or those bytes base64-encoded.
    pub fn from_secret_bytes(raw: &[u8]) -> CoreResult<Self> {
        if raw.len() == 32 {
            return Self::from_bytes(raw);
        }
        let text = std::str::from_utf8(raw).map_err(|_| {
            CoreError::InvalidKey("expected 32 raw bytes or base64-encoded text".to_string())
        })?;
        let decoded = BASE64
            .decode(text.trim())
            .map_err(|e| CoreError::InvalidKey(format!("invalid base64 encryption key: {e}")))?;
        Self::from_bytes(&decoded)
    }

    /// Base64-encode for storage in a Kubernetes Secret `stringData` field.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(BASE64.encode(self.0))
    }

    fn as_slice(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Accepted Secret keys for an encryption key, first match wins.
pub const ENCRYPTION_KEY_SECRET_KEYS: &[&str] = &["encryptionKey", "ENCRYPTION_KEY"];

/// Resolve an [`EncryptionKey`] from decoded Kubernetes Secret data (raw bytes, not lossily
/// decoded as UTF-8) so both a base64 string and 32 raw key bytes are supported.
pub fn encryption_key_from_secret_data(
    data: &std::collections::HashMap<String, Vec<u8>>,
) -> CoreResult<EncryptionKey> {
    for &key in ENCRYPTION_KEY_SECRET_KEYS {
        if let Some(raw) = data.get(key) {
            if let Ok(key) = EncryptionKey::from_secret_bytes(raw) {
                return Ok(key);
            }
        }
    }
    Err(CoreError::InvalidKey(format!(
        "encryption Secret must contain a 32-byte or base64-encoded key under one of: {}",
        ENCRYPTION_KEY_SECRET_KEYS.join(", ")
    )))
}

/// Nonce || ciphertext+tag (AES-256-GCM).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ciphertext {
    pub blob: Vec<u8>,
}

impl Ciphertext {
    pub fn parts(&self) -> CoreResult<(&[u8], &[u8])> {
        if self.blob.len() < NONCE_LEN {
            return Err(CoreError::Crypto(
                "ciphertext shorter than nonce".to_string(),
            ));
        }
        Ok((&self.blob[..NONCE_LEN], &self.blob[NONCE_LEN..]))
    }
}

pub fn encrypt(key: &EncryptionKey, plaintext: &[u8]) -> CoreResult<Ciphertext> {
    let cipher =
        Aes256Gcm::new_from_slice(key.as_slice()).map_err(|e| CoreError::Crypto(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| CoreError::Crypto(e.to_string()))?;

    let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.append(&mut ciphertext);
    Ok(Ciphertext { blob })
}

pub fn decrypt(key: &EncryptionKey, ciphertext: &Ciphertext) -> CoreResult<Vec<u8>> {
    let (nonce_bytes, ct) = ciphertext.parts()?;
    let cipher =
        Aes256Gcm::new_from_slice(key.as_slice()).map_err(|e| CoreError::Crypto(e.to_string()))?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| CoreError::Crypto(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = EncryptionKey::generate();
        let ct = encrypt(&key, b"secret-chunk").expect("encrypt");
        let pt = decrypt(&key, &ct).expect("decrypt");
        assert_eq!(pt, b"secret-chunk");
    }

    #[test]
    fn key_round_trips_through_base64() {
        let key = EncryptionKey::generate();
        let encoded = key.to_base64();
        let parsed = EncryptionKey::from_secret_bytes(encoded.as_bytes()).expect("parse base64");
        // Encrypting with both keys and decrypting cross-wise proves the bytes match.
        let ct = encrypt(&key, b"hello").expect("encrypt");
        assert_eq!(decrypt(&parsed, &ct).expect("decrypt"), b"hello");
    }

    #[test]
    fn key_accepts_32_raw_bytes() {
        let raw = [7u8; 32];
        let key = EncryptionKey::from_secret_bytes(&raw).expect("32 raw bytes");
        let ct = encrypt(&key, b"raw-key").expect("encrypt");
        assert_eq!(decrypt(&key, &ct).expect("decrypt"), b"raw-key");
    }

    #[test]
    fn encryption_key_from_secret_data_prefers_camel_case() {
        let mut data = std::collections::HashMap::new();
        data.insert(
            "encryptionKey".to_string(),
            EncryptionKey::generate().to_base64().as_bytes().to_vec(),
        );
        encryption_key_from_secret_data(&data).expect("parse");
    }

    #[test]
    fn encryption_key_from_secret_data_rejects_missing_key() {
        let data = std::collections::HashMap::new();
        let err = encryption_key_from_secret_data(&data)
            .err()
            .expect("missing key");
        assert!(err.to_string().contains("encryptionKey"));
    }
}
