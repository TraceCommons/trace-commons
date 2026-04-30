//! Local crypto helpers used by TraceDAO artifact storage.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng},
};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

pub use crate::error::{DecryptedSecret, SecretError};

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;
const SALT_SIZE: usize = 32;
const TAG_SIZE: usize = 16;

pub struct SecretsCrypto {
    master_key: SecretString,
}

impl SecretsCrypto {
    pub fn new(master_key: SecretString) -> Result<Self, SecretError> {
        if master_key.expose_secret().len() < KEY_SIZE {
            return Err(SecretError::InvalidMasterKey);
        }
        Ok(Self { master_key })
    }

    pub fn generate_salt() -> Vec<u8> {
        let mut salt = vec![0u8; SALT_SIZE];
        rand::RngCore::fill_bytes(&mut OsRng, &mut salt);
        salt
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
        let salt = Self::generate_salt();
        let derived_key = self.derive_key(&salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| SecretError::EncryptionFailed(format!("Failed to create cipher: {e}")))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| SecretError::EncryptionFailed(format!("Encryption failed: {e}")))?;

        let mut encrypted = Vec::with_capacity(NONCE_SIZE + ciphertext.len());
        encrypted.extend_from_slice(&nonce);
        encrypted.extend_from_slice(&ciphertext);
        Ok((encrypted, salt))
    }

    pub fn decrypt(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
    ) -> Result<DecryptedSecret, SecretError> {
        if encrypted_value.len() < NONCE_SIZE + TAG_SIZE {
            return Err(SecretError::DecryptionFailed(
                "Encrypted value too short".to_string(),
            ));
        }

        let derived_key = self.derive_key(salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| SecretError::DecryptionFailed(format!("Failed to create cipher: {e}")))?;
        let (nonce_bytes, ciphertext) = encrypted_value.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::DecryptionFailed(format!("Decryption failed: {e}")))?;
        DecryptedSecret::from_bytes(plaintext)
    }

    fn derive_key(&self, salt: &[u8]) -> Result<[u8; KEY_SIZE], SecretError> {
        let hk = Hkdf::<Sha256>::new(Some(salt), self.master_key.expose_secret().as_bytes());
        let mut derived = [0u8; KEY_SIZE];
        hk.expand(b"tracedao-server-secrets-v1", &mut derived)
            .map_err(|_| SecretError::EncryptionFailed("HKDF expansion failed".to_string()))?;
        Ok(derived)
    }
}

impl std::fmt::Debug for SecretsCrypto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretsCrypto")
            .field("master_key", &"[REDACTED]")
            .finish()
    }
}

pub mod keychain {
    pub fn generate_master_key_hex() -> String {
        use rand::RngCore;
        use rand::rngs::OsRng;

        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        key.iter().map(|b| format!("{:02x}", b)).collect()
    }
}
