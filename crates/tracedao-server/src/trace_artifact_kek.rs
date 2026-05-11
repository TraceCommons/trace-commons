//! Key encryption key (KEK) trait and supporting types for trace artifact envelope encryption.
//!
//! This module defines the pluggable `KmsKeyWrapper` trait used to wrap and unwrap
//! per-artifact data encryption keys (DEKs). Concrete implementations live in
//! submodules; this file contains only the shared types and the trait contract.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::secrets::SecretsCrypto;
use crate::trace_artifact_store::TraceArtifactKind;

/// Stable context used to bind a DEK wrap/unwrap operation to a specific
/// tenant and artifact kind. The canonical hash is included in `WrappedDek`
/// so that unwrap can verify the ciphertext was produced under the same context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KekContext {
    pub tenant_storage_ref: String,
    pub artifact_kind: TraceArtifactKind,
}

impl KekContext {
    /// Returns a `sha256:`-prefixed SHA-256 hash of the canonical byte
    /// representation of this context. Used to bind wrapped DEKs to their context.
    ///
    /// The canonical form is a fixed newline-delimited key=value sequence built
    /// with `format!` so the hash is stable regardless of `serde_json` map
    /// ordering, feature flags, or future struct changes. Do not refactor to
    /// dynamic map construction — stored `context_hash` values depend on this
    /// exact serialization.
    pub fn canonical_hash(&self) -> String {
        let canonical = format!(
            "schema=trace_commons_kek_context.v1\ntenant_storage_ref={}\nartifact_kind={}",
            self.tenant_storage_ref,
            self.artifact_kind.as_path_segment(),
        );
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

/// A DEK that has been wrapped (encrypted) by a KMS key wrapper. The fields
/// are intentionally opaque to consumers — only the originating `KmsKeyWrapper`
/// implementation is expected to interpret `ciphertext_base64`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedDek {
    /// Identifies the wrapper implementation that produced this ciphertext.
    pub wrapper_kind: String,
    /// Hash of the key reference used during wrapping (safe to log/store).
    pub key_ref_hash: String,
    /// Base64-encoded wrapped key ciphertext.
    pub ciphertext_base64: String,
    /// Canonical hash of the `KekContext` at wrap time; verified on unwrap.
    pub context_hash: String,
}

/// Observable status of a `KmsKeyWrapper` instance, safe to surface in logs
/// and health checks. Contains no key material.
#[derive(Debug, Clone, Serialize)]
pub struct KekWrapperStatus {
    /// Identifies the wrapper implementation (matches `WrappedDek::wrapper_kind`).
    pub kind: String,
    /// Hash of the key reference in use (safe to log).
    pub key_ref_hash: String,
    /// Whether this wrapper enforces a production-grade trust boundary.
    pub is_production_trust_boundary: bool,
}

/// Pluggable key encryption key wrapper.
///
/// Implementations are responsible for wrapping a 256-bit DEK under a KMS-managed
/// key and unwrapping it on retrieval. The `context` parameter binds each operation
/// to a specific tenant and artifact kind so that wrapped keys cannot be replayed
/// across tenants or artifact kinds.
pub trait KmsKeyWrapper: Send + Sync {
    /// Wrap `dek` under the configured KMS key, binding the result to `context`.
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek>;

    /// Unwrap a previously wrapped DEK, verifying that `context` matches the
    /// hash recorded at wrap time.
    fn unwrap_dek(&self, wrapped: &WrappedDek, context: &KekContext) -> anyhow::Result<[u8; 32]>;

    /// Return observable status suitable for logging and health checks.
    fn safe_status(&self) -> KekWrapperStatus;

    /// Returns `true` if this wrapper enforces a production-grade trust boundary
    /// (e.g., a real KMS rather than a local test key).
    fn is_production_trust_boundary(&self) -> bool;
}

// ---------------------------------------------------------------------------
// LocalMasterKeyWrapper
// ---------------------------------------------------------------------------

/// A `KmsKeyWrapper` implementation backed by a local `SecretsCrypto` master key.
///
/// This implementation is intended for local development and testing only. It
/// does NOT provide a production-grade trust boundary — use a real KMS
/// (e.g., GCP KMS) for production deployments.
///
/// Wrap format: `packed = [salt_len_u8 || salt || aes_gcm_ciphertext]`
/// The AES-GCM plaintext is `context_hash_bytes || dek_bytes`, giving inner
/// integrity binding between the context and the DEK.
pub struct LocalMasterKeyWrapper {
    crypto: SecretsCrypto,
    key_ref_label: String,
}

impl LocalMasterKeyWrapper {
    pub fn new(crypto: SecretsCrypto, key_ref_label: impl Into<String>) -> Self {
        Self {
            crypto,
            key_ref_label: key_ref_label.into(),
        }
    }

    fn key_ref_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.key_ref_label.as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

impl KmsKeyWrapper for LocalMasterKeyWrapper {
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek> {
        let context_hash = context.canonical_hash();
        // Inner plaintext: context_hash_bytes || dek, providing integrity binding.
        let mut plaintext = Vec::with_capacity(context_hash.len() + 32);
        plaintext.extend_from_slice(context_hash.as_bytes());
        plaintext.extend_from_slice(dek);
        let (encrypted, salt) = self
            .crypto
            .encrypt(&plaintext)
            .map_err(|e| anyhow::anyhow!("KekWrapFailed: {e}"))?;
        // Pack: [salt_len_u8 || salt || ciphertext]
        let mut packed = Vec::with_capacity(1 + salt.len() + encrypted.len());
        packed.push(salt.len() as u8);
        packed.extend_from_slice(&salt);
        packed.extend_from_slice(&encrypted);
        Ok(WrappedDek {
            wrapper_kind: "local_master_key".into(),
            key_ref_hash: self.key_ref_hash(),
            ciphertext_base64: STANDARD.encode(&packed),
            context_hash,
        })
    }

    fn unwrap_dek(&self, wrapped: &WrappedDek, context: &KekContext) -> anyhow::Result<[u8; 32]> {
        anyhow::ensure!(
            wrapped.wrapper_kind == "local_master_key",
            "KekUnwrapFailed: wrapper kind mismatch"
        );
        let expected_ctx = context.canonical_hash();
        anyhow::ensure!(
            wrapped.context_hash == expected_ctx,
            "KekContextMismatch: outer context_hash mismatch"
        );
        let packed = STANDARD
            .decode(&wrapped.ciphertext_base64)
            .map_err(|_| anyhow::anyhow!("KekUnwrapFailed: base64 decode error"))?;
        anyhow::ensure!(packed.len() > 1, "KekUnwrapFailed: ciphertext too short");
        let salt_len = packed[0] as usize;
        anyhow::ensure!(
            packed.len() > 1 + salt_len,
            "KekUnwrapFailed: encoded salt length exceeds buffer"
        );
        let salt = &packed[1..1 + salt_len];
        let encrypted = &packed[1 + salt_len..];
        let decrypted = self
            .crypto
            .decrypt_bytes(encrypted, salt)
            .map_err(|e| anyhow::anyhow!("KekUnwrapFailed: {e}"))?;
        let bytes = decrypted.expose_bytes();
        let ctx_len = expected_ctx.len();
        anyhow::ensure!(
            bytes.len() == ctx_len + 32,
            "KekUnwrapFailed: decrypted length mismatch"
        );
        anyhow::ensure!(
            &bytes[..ctx_len] == expected_ctx.as_bytes(),
            "KekContextMismatch: inner context tag mismatch"
        );
        let mut dek = [0u8; 32];
        dek.copy_from_slice(&bytes[ctx_len..]);
        Ok(dek)
    }

    fn safe_status(&self) -> KekWrapperStatus {
        KekWrapperStatus {
            kind: "local_master_key".into(),
            key_ref_hash: self.key_ref_hash(),
            is_production_trust_boundary: false,
        }
    }

    fn is_production_trust_boundary(&self) -> bool {
        false
    }
}
