//! Error types for TraceCommons server-owned persistence and secrets.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

/// Database-related errors for TraceCommons server storage.
#[derive(Debug, thiserror::Error)]
pub enum DatabaseError {
    #[error("Connection pool error: {0}")]
    Pool(String),

    #[error("Query failed: {0}")]
    Query(String),

    #[error("Entity not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    #[error("Constraint violation: {0}")]
    Constraint(String),

    #[error("Migration failed: {0}")]
    Migration(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("PostgreSQL error: {0}")]
    Postgres(#[from] tokio_postgres::Error),

    #[error("Pool build error: {0}")]
    PoolBuild(#[from] deadpool_postgres::BuildError),

    #[error("Pool runtime error: {0}")]
    PoolRuntime(#[from] deadpool_postgres::PoolError),
}

/// A decrypted secret value, intentionally opaque in Debug output.
pub struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        let value = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(value),
        })
    }

    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    pub fn len(&self) -> usize {
        self.value.expose_secret().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A decrypted raw-byte secret value. Used for binary payloads that are not
/// valid UTF-8 (e.g., wrapped DEKs). Intentionally opaque in Debug output.
pub struct DecryptedBytes {
    bytes: Vec<u8>,
}

impl DecryptedBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// Expose the raw decrypted bytes.
    pub fn expose_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl fmt::Debug for DecryptedBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecryptedBytes([REDACTED, {} bytes])", self.len())
    }
}

impl fmt::Debug for DecryptedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecryptedSecret([REDACTED, {} bytes])", self.len())
    }
}

/// Errors that can occur during local artifact encryption.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretError {
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Invalid master key")]
    InvalidMasterKey,

    #[error("Secret value is not valid UTF-8")]
    InvalidUtf8,
}
