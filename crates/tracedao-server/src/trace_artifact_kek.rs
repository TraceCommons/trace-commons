//! Key encryption key (KEK) trait and supporting types for trace artifact envelope encryption.
//!
//! Defines the pluggable `KmsKeyWrapper` trait used to wrap and unwrap per-artifact
//! data encryption keys (DEKs). The local-development implementation
//! (`LocalMasterKeyWrapper`) is co-located here; production implementations
//! (TEE-rooted, cloud KMS) will live in submodules.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

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
    ///
    /// The returned key bytes are wrapped in `Zeroizing` so callers receive a
    /// scrubbed-on-drop buffer by construction; callers MUST NOT copy the
    /// raw bytes out into a long-lived plain `[u8; 32]`.
    fn unwrap_dek(
        &self,
        wrapped: &WrappedDek,
        context: &KekContext,
    ) -> anyhow::Result<Zeroizing<[u8; 32]>>;

    /// Return observable status suitable for logging and health checks.
    fn safe_status(&self) -> KekWrapperStatus;

    /// Returns `true` if this wrapper enforces a production-grade trust boundary
    /// (e.g., a real KMS rather than a local test key).
    fn is_production_trust_boundary(&self) -> bool;
}

/// Blanket impl so a `Box<dyn KmsKeyWrapper>` (or any boxed wrapper) satisfies
/// the trait bound on the generic `K: KmsKeyWrapper` used by
/// `ServiceOwnedTraceArtifactStore`. This lets the bin select among multiple
/// concrete wrapper types at runtime via `TRACE_COMMONS_KEK_PROVIDER` without
/// duplicating the store construction path per wrapper type.
impl<K: KmsKeyWrapper + ?Sized> KmsKeyWrapper for Box<K> {
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek> {
        (**self).wrap_dek(dek, context)
    }

    fn unwrap_dek(
        &self,
        wrapped: &WrappedDek,
        context: &KekContext,
    ) -> anyhow::Result<Zeroizing<[u8; 32]>> {
        (**self).unwrap_dek(wrapped, context)
    }

    fn safe_status(&self) -> KekWrapperStatus {
        (**self).safe_status()
    }

    fn is_production_trust_boundary(&self) -> bool {
        (**self).is_production_trust_boundary()
    }
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
        // Pack: [salt_len_u8 || salt || ciphertext]. The u8 length prefix is
        // sound only while SecretsCrypto::generate_salt stays <= 255 bytes
        // (currently SALT_SIZE = 32 in secrets.rs); the assertion pins it.
        debug_assert!(
            salt.len() <= u8::MAX as usize,
            "salt too long for u8 length prefix"
        );
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

    fn unwrap_dek(
        &self,
        wrapped: &WrappedDek,
        context: &KekContext,
    ) -> anyhow::Result<Zeroizing<[u8; 32]>> {
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
        let mut dek = Zeroizing::new([0u8; 32]);
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

// ---------------------------------------------------------------------------
// DstackKekWrapper (stub)
// ---------------------------------------------------------------------------

/// Stub `KmsKeyWrapper` for the dstack-resident enclave KEK service.
///
/// This implementation claims `is_production_trust_boundary = true` because the
/// concrete dstack-rooted impl will satisfy that contract once the enclave
/// binary lands. Today the wrap/unwrap entry points fail closed with a stable
/// `DstackKekUnavailable` error class so an operator who configures the
/// production-trust-boundary requirement gets a clear runtime signal that the
/// dstack enclave has not been wired up yet, rather than a silent downgrade.
pub struct DstackKekWrapper {
    enclave_measurement_label: String,
}

impl DstackKekWrapper {
    /// Construct a stub wrapper bound to the operator-supplied enclave
    /// measurement label. The label is opaque to this stub — it exists only so
    /// the wrapper's `safe_status.key_ref_hash` is stable across restarts.
    pub fn new(enclave_measurement_label: impl Into<String>) -> Self {
        Self {
            enclave_measurement_label: enclave_measurement_label.into(),
        }
    }

    fn key_ref_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.enclave_measurement_label.as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

impl KmsKeyWrapper for DstackKekWrapper {
    fn wrap_dek(&self, _dek: &[u8; 32], _context: &KekContext) -> anyhow::Result<WrappedDek> {
        anyhow::bail!("DstackKekUnavailable: dstack enclave wrapping not yet implemented")
    }

    fn unwrap_dek(
        &self,
        _wrapped: &WrappedDek,
        _context: &KekContext,
    ) -> anyhow::Result<Zeroizing<[u8; 32]>> {
        anyhow::bail!("DstackKekUnavailable: dstack enclave wrapping not yet implemented")
    }

    fn safe_status(&self) -> KekWrapperStatus {
        KekWrapperStatus {
            kind: "dstack_kek".into(),
            key_ref_hash: self.key_ref_hash(),
            is_production_trust_boundary: true,
        }
    }

    fn is_production_trust_boundary(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// CloudKmsKeyWrapper
// ---------------------------------------------------------------------------

/// Pluggable cloud-vendor KMS client used by `CloudKmsKeyWrapper`.
///
/// The trait abstracts over GCP Cloud KMS, AWS KMS, Azure Key Vault, etc., so
/// the wrapper can be tested hermetically against `InMemoryCloudKmsClient` and
/// swapped to a real vendor adapter without touching the wrapper logic.
///
/// The AAD bytes are the canonical KEK-context hash; cloud KMS encrypt/decrypt
/// must bind them as Additional Authenticated Data so cross-context DEK
/// substitution fails closed at the vendor layer. For GCP Cloud KMS this maps
/// to the `additionalAuthenticatedData` field on `Encrypt` / `Decrypt`.
pub trait CloudKmsClient: Send + Sync {
    /// Encrypt `plaintext` under the configured cloud KMS key, binding `aad`
    /// as additional authenticated data.
    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> anyhow::Result<Vec<u8>>;

    /// Decrypt `ciphertext` under the configured cloud KMS key, requiring the
    /// same `aad` used at encrypt time. The returned plaintext is wrapped in
    /// `Zeroizing` so it is scrubbed on drop.
    fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>>;

    /// Stable opaque identifier for the configured KMS key. Returned verbatim
    /// from the vendor — never logged raw; the wrapper hashes it before
    /// exposing it through `safe_status`.
    fn key_ref(&self) -> &str;
}

/// `KmsKeyWrapper` impl backed by a cloud-vendor KMS service.
///
/// By convention, `is_production_trust_boundary()` returns `true` for this
/// wrapper. The cloud KMS is operator-trusted, not operator-constrained — the
/// operator and the cloud provider can read every wrapped DEK by calling the
/// KMS `Decrypt` API. The "production trust boundary" gate in this codebase
/// distinguishes "real KMS with key-access auditing" from "local master key in
/// process memory"; a TEE-attested KEK (Phase B) is a strictly stronger trust
/// boundary that will be modeled via a separate wrapper. See
/// `docs/superpowers/specs/2026-05-11-trace-kek-strategy-design.md`.
pub struct CloudKmsKeyWrapper<C> {
    client: C,
    key_ref_hash: String,
    wrapper_kind: String,
}

impl<C: CloudKmsClient> CloudKmsKeyWrapper<C> {
    /// Construct a wrapper around the supplied cloud KMS client. `wrapper_kind`
    /// is the stable string written into `WrappedDek::wrapper_kind` (e.g.
    /// `"gcp_cloud_kms"`), and is verified on unwrap.
    pub fn new(client: C, wrapper_kind: impl Into<String>) -> Self {
        let key_ref_hash = hash_label(client.key_ref());
        Self {
            client,
            key_ref_hash,
            wrapper_kind: wrapper_kind.into(),
        }
    }
}

fn hash_label(label: &str) -> String {
    let mut h = Sha256::new();
    h.update(label.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

impl<C: CloudKmsClient> KmsKeyWrapper for CloudKmsKeyWrapper<C> {
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek> {
        let context_hash = context.canonical_hash();
        let aad = context_hash.as_bytes();
        let ciphertext = self
            .client
            .encrypt(dek.as_slice(), aad)
            .map_err(|e| anyhow::anyhow!("KekWrapFailed: {e}"))?;
        Ok(WrappedDek {
            wrapper_kind: self.wrapper_kind.clone(),
            key_ref_hash: self.key_ref_hash.clone(),
            ciphertext_base64: STANDARD.encode(&ciphertext),
            context_hash,
        })
    }

    fn unwrap_dek(
        &self,
        wrapped: &WrappedDek,
        context: &KekContext,
    ) -> anyhow::Result<Zeroizing<[u8; 32]>> {
        anyhow::ensure!(
            wrapped.wrapper_kind == self.wrapper_kind,
            "KekUnwrapFailed: wrapper kind mismatch"
        );
        let expected_ctx = context.canonical_hash();
        anyhow::ensure!(
            wrapped.context_hash == expected_ctx,
            "KekContextMismatch: outer context_hash mismatch"
        );
        let ciphertext = STANDARD
            .decode(&wrapped.ciphertext_base64)
            .map_err(|_| anyhow::anyhow!("KekUnwrapFailed: base64 decode error"))?;
        let plaintext = self
            .client
            .decrypt(&ciphertext, expected_ctx.as_bytes())
            .map_err(|e| anyhow::anyhow!("KekUnwrapFailed: {e}"))?;
        anyhow::ensure!(
            plaintext.len() == 32,
            "KekUnwrapFailed: dek length mismatch"
        );
        let mut dek = Zeroizing::new([0u8; 32]);
        dek.copy_from_slice(plaintext.as_slice());
        Ok(dek)
    }

    fn safe_status(&self) -> KekWrapperStatus {
        KekWrapperStatus {
            kind: self.wrapper_kind.clone(),
            key_ref_hash: self.key_ref_hash.clone(),
            is_production_trust_boundary: true,
        }
    }

    fn is_production_trust_boundary(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// InMemoryCloudKmsClient
// ---------------------------------------------------------------------------

/// Hermetic `CloudKmsClient` for unit tests.
///
/// Encrypts the plaintext under a caller-supplied 256-bit master key using
/// AES-256-GCM with the caller-supplied AAD bound as Additional Authenticated
/// Data, matching the AAD semantics of real cloud KMS (e.g. GCP Cloud KMS's
/// `additionalAuthenticatedData` field). The wire format is
/// `nonce || ciphertext_with_tag`.
///
/// Not for production use — there is no key isolation. Intended only to
/// exercise `CloudKmsKeyWrapper` round-trip and AAD-binding semantics in tests.
pub struct InMemoryCloudKmsClient {
    master_key: [u8; 32],
    key_ref: String,
}

impl InMemoryCloudKmsClient {
    /// Construct an in-memory client with the supplied 32-byte master key and
    /// a stable `key_ref` label. The label is hashed by `CloudKmsKeyWrapper`
    /// before being exposed through `safe_status`.
    pub fn new_with_master(master_key: [u8; 32], key_ref: impl Into<String>) -> Self {
        Self {
            master_key,
            key_ref: key_ref.into(),
        }
    }
}

impl CloudKmsClient for InMemoryCloudKmsClient {
    fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> anyhow::Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|e| anyhow::anyhow!("InMemoryCloudKms: cipher init failed: {e}"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, Payload { msg: plaintext, aad })
            .map_err(|e| anyhow::anyhow!("InMemoryCloudKms: encrypt failed: {e}"))?;
        let mut out = Vec::with_capacity(nonce.len() + ciphertext.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    fn decrypt(&self, ciphertext: &[u8], aad: &[u8]) -> anyhow::Result<Zeroizing<Vec<u8>>> {
        const NONCE_SIZE: usize = 12;
        anyhow::ensure!(
            ciphertext.len() > NONCE_SIZE,
            "InMemoryCloudKms: ciphertext too short"
        );
        let (nonce_bytes, body) = ciphertext.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|e| anyhow::anyhow!("InMemoryCloudKms: cipher init failed: {e}"))?;
        let plaintext = cipher
            .decrypt(nonce, Payload { msg: body, aad })
            .map_err(|e| anyhow::anyhow!("InMemoryCloudKms: decrypt failed: {e}"))?;
        Ok(Zeroizing::new(plaintext))
    }

    fn key_ref(&self) -> &str {
        &self.key_ref
    }
}
