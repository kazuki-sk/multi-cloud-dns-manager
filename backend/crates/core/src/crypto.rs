use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const NONCE_LEN: usize = 12;
const KEY_LEN: usize = 32;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key: {0}")]
    InvalidKey(String),
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed")]
    DecryptionFailed,
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("environment variable error: {0}")]
    EnvVar(String),
}

pub type CryptoResult<T> = Result<T, CryptoError>;

/// Envelope-encrypted credential pair stored in the database.
///
/// `blob` = base64(nonce || AES-256-GCM ciphertext) of the plaintext, encrypted with DEK.
/// `dek`  = base64(nonce || AES-256-GCM ciphertext) of the 32-byte DEK, encrypted with KEK.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBlob {
    pub blob: String,
    pub dek: String,
}

/// Supplies the Key Encryption Key (KEK) used in envelope encryption.
/// Must be `Send + Sync` to be usable as `dyn KeyProvider`.
pub trait KeyProvider: Send + Sync {
    fn get_kek(&self) -> CryptoResult<[u8; KEY_LEN]>;
}

/// Reads the KEK from the `MASTER_KEY` environment variable.
/// The value must be a Base64-encoded 32-byte key.
pub struct EnvKeyProvider;

impl KeyProvider for EnvKeyProvider {
    fn get_kek(&self) -> CryptoResult<[u8; KEY_LEN]> {
        let val = std::env::var("MASTER_KEY")
            .map_err(|e| CryptoError::EnvVar(format!("MASTER_KEY: {e}")))?;
        let bytes = B64
            .decode(val.trim())
            .map_err(|_| CryptoError::InvalidKey("MASTER_KEY must be base64-encoded".into()))?;
        bytes.try_into().map_err(|_| {
            CryptoError::InvalidKey("MASTER_KEY must decode to exactly 32 bytes".into())
        })
    }
}

fn aes_gcm_encrypt(key_bytes: &[u8; KEY_LEN], plaintext: &[u8]) -> CryptoResult<Vec<u8>> {
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::EncryptionFailed)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn aes_gcm_decrypt(key_bytes: &[u8; KEY_LEN], data: &[u8]) -> CryptoResult<Vec<u8>> {
    if data.len() < NONCE_LEN {
        return Err(CryptoError::DecryptionFailed);
    }
    let (nonce_bytes, ciphertext) = data.split_at(NONCE_LEN);
    let key = Key::<Aes256Gcm>::from_slice(key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)
}

/// Encrypt `plaintext` using envelope encryption.
///
/// A random 32-byte DEK is generated per call. The DEK encrypts the plaintext
/// (AES-256-GCM), and the KEK from `provider` encrypts the DEK.
pub fn encrypt(provider: &dyn KeyProvider, plaintext: &[u8]) -> CryptoResult<EncryptedBlob> {
    let kek = provider.get_kek()?;
    let mut dek_bytes = [0u8; KEY_LEN];
    rand::thread_rng().fill_bytes(&mut dek_bytes);
    let blob_raw = aes_gcm_encrypt(&dek_bytes, plaintext)?;
    let dek_raw = aes_gcm_encrypt(&kek, &dek_bytes)?;
    Ok(EncryptedBlob {
        blob: B64.encode(blob_raw),
        dek: B64.encode(dek_raw),
    })
}

/// Decrypt an `EncryptedBlob` produced by [`encrypt`].
pub fn decrypt(provider: &dyn KeyProvider, blob: &EncryptedBlob) -> CryptoResult<Vec<u8>> {
    let kek = provider.get_kek()?;
    let dek_raw = B64.decode(&blob.dek)?;
    let dek_bytes: [u8; KEY_LEN] = aes_gcm_decrypt(&kek, &dek_raw)?
        .try_into()
        .map_err(|_| CryptoError::DecryptionFailed)?;
    let blob_raw = B64.decode(&blob.blob)?;
    aes_gcm_decrypt(&dek_bytes, &blob_raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedKeyProvider([u8; 32]);

    impl KeyProvider for FixedKeyProvider {
        fn get_kek(&self) -> CryptoResult<[u8; 32]> {
            Ok(self.0)
        }
    }

    #[test]
    fn round_trip() {
        let provider = FixedKeyProvider([0xABu8; 32]);
        let plaintext = b"secret-api-key-12345";
        let blob = encrypt(&provider, plaintext).unwrap();
        let decrypted = decrypt(&provider, &blob).unwrap();
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    #[test]
    fn different_dek_each_encrypt() {
        let provider = FixedKeyProvider([0x42u8; 32]);
        let plaintext = b"same plaintext";
        let blob1 = encrypt(&provider, plaintext).unwrap();
        let blob2 = encrypt(&provider, plaintext).unwrap();
        assert_ne!(blob1.blob, blob2.blob, "nonces must differ across calls");
        assert_ne!(blob1.dek, blob2.dek);
    }

    #[test]
    fn wrong_kek_fails_decryption() {
        let provider = FixedKeyProvider([0x01u8; 32]);
        let plaintext = b"sensitive data";
        let blob = encrypt(&provider, plaintext).unwrap();
        let wrong_provider = FixedKeyProvider([0x02u8; 32]);
        assert!(decrypt(&wrong_provider, &blob).is_err());
    }

    #[test]
    fn tampered_blob_fails_decryption() {
        let provider = FixedKeyProvider([0xFFu8; 32]);
        let plaintext = b"tamper test";
        let mut blob = encrypt(&provider, plaintext).unwrap();
        // Flip a byte in the middle of blob
        let mut raw = B64.decode(&blob.blob).unwrap();
        raw[NONCE_LEN] ^= 0xFF;
        blob.blob = B64.encode(raw);
        assert!(decrypt(&provider, &blob).is_err());
    }

    #[test]
    fn env_key_provider_missing_var() {
        if std::env::var("MASTER_KEY").is_err() {
            assert!(EnvKeyProvider.get_kek().is_err());
        }
    }
}
