//! Key encryption key (KEK) trait and supporting types for trace artifact envelope encryption.
//!
//! This module defines the pluggable `KmsKeyWrapper` trait used to wrap and unwrap
//! per-artifact data encryption keys (DEKs). Concrete implementations live in
//! submodules; this file contains only the shared types and the trait contract.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// Returns a deterministic hex-encoded SHA-256 hash of the canonical JSON
    /// representation of this context. Used to bind wrapped DEKs to their context.
    pub fn canonical_hash(&self) -> String {
        let canonical = serde_json::json!({
            "schema": "trace_commons_kek_context.v1",
            "tenant_storage_ref": self.tenant_storage_ref,
            "artifact_kind": self.artifact_kind.as_path_segment(),
        });
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string().as_bytes());
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
