use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use zeroize::{Zeroize, ZeroizeOnDrop};

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

    fn as_slice(&self) -> &[u8; 32] {
        &self.0
    }
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
}
