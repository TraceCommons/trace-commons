// INTEGRATION: allocate a CredentialReference, persist that pending reference in
// the lifecycle journal, then call prepare_at. Publish settings only after its
// readback succeeds; retain cleanup references until deletion succeeds. The
// lifecycle owner serializes operations on each reference and runs blocking OS
// calls off async workers. This module does not publish settings or own a wallet.
//! Immutable OS credential entries with bounded, privately serialized payloads.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::{Uuid, Variant, Version};

/// Windows Credential Manager's maximum binary credential size. Apply the same
/// bound everywhere, including JSON framing and escaping, for portable bundles.
pub(crate) const MAX_SECRET_BYTES: usize = 2560;
const VERSION: u8 = 1;

/// Fixed labels only: backend errors and secret parser details stay out of logs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialError {
    NoEntry,
    Unavailable,
    InvalidReference,
    UnsupportedVersion,
    InvalidBundle,
    TooLarge,
    BindingMismatch,
    AlreadyExists,
    VerificationFailed,
    CleanupFailed(CredentialReference),
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoEntry => "near_ai_credential_storage_missing",
            Self::Unavailable => "near_ai_credential_storage_unavailable",
            Self::InvalidReference => "near_ai_credential_storage_reference_invalid",
            Self::UnsupportedVersion => "near_ai_credential_storage_version_unsupported",
            Self::InvalidBundle => "near_ai_credential_storage_bundle_invalid",
            Self::TooLarge => "near_ai_credential_storage_bundle_too_large",
            Self::BindingMismatch => "near_ai_credential_storage_binding_mismatch",
            Self::AlreadyExists => "near_ai_credential_storage_entry_exists",
            Self::VerificationFailed => "near_ai_credential_storage_verification_failed",
            Self::CleanupFailed(_) => "near_ai_credential_storage_cleanup_failed",
        })
    }
}

impl std::error::Error for CredentialError {}

impl CredentialError {
    /// Preserve the address of an unpublished entry when cleanup itself fails.
    #[cfg(test)]
    pub(crate) fn cleanup_reference(&self) -> Option<&CredentialReference> {
        match self {
            Self::CleanupFailed(reference) => Some(reference),
            _ => None,
        }
    }
}

/// Safe to persist; contains no credential, account identity, or directory path.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CredentialReference {
    version: u8,
    id: Uuid,
}

impl CredentialReference {
    /// Allocation performs no backend access. Journal this reference before
    /// calling prepare_at so a crash cannot lose an unpublished entry's address.
    pub(crate) fn allocate() -> Self {
        Self {
            version: VERSION,
            id: Uuid::new_v4(),
        }
    }

    fn validate(&self) -> Result<(), CredentialError> {
        if self.version != VERSION {
            return Err(CredentialError::UnsupportedVersion);
        }
        if self.id.get_version() != Some(Version::Random)
            || self.id.get_variant() != Variant::RFC4122
        {
            return Err(CredentialError::InvalidReference);
        }
        Ok(())
    }

    /// The validated opaque name used by the platform-specific backend.
    pub(crate) fn storage_key(&self) -> Result<String, CredentialError> {
        self.validate()?;
        Ok(format!("cloud-v{}-{}", self.version, self.id.simple()))
    }
}

impl fmt::Debug for CredentialReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialReference")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// One or both Cloud credentials. Retaining the refresh token is normal v0.12
/// behavior. Serialization is private; settings and IPC carry no secret payload.
pub(crate) struct SecretBundle {
    inference_key: Option<String>,
    refresh_token: Option<String>,
    binding_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredBundle {
    version: u8,
    #[serde(default)]
    inference_key: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    binding_digest: String,
}

impl SecretBundle {
    /// The caller derives the canonical SHA-256 binding from trusted Cloud
    /// metadata and supplies the same expected binding when loading an entry.
    pub(crate) fn new(
        inference_key: Option<String>,
        refresh_token: Option<String>,
        binding_digest: String,
    ) -> Result<Self, CredentialError> {
        let bundle = Self {
            inference_key,
            refresh_token,
            binding_digest,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub(crate) fn inference_key(&self) -> Option<&str> {
        self.inference_key.as_deref()
    }

    pub(crate) fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    pub(crate) fn binding_digest(&self) -> &str {
        &self.binding_digest
    }

    fn validate(&self) -> Result<(), CredentialError> {
        if self.inference_key.is_none() && self.refresh_token.is_none() {
            return Err(CredentialError::InvalidBundle);
        }
        for secret in [self.inference_key(), self.refresh_token()]
            .into_iter()
            .flatten()
        {
            if secret.len() > MAX_SECRET_BYTES {
                return Err(CredentialError::TooLarge);
            }
            if secret.is_empty()
                || secret
                    .chars()
                    .any(|character| character.is_whitespace() || character.is_control())
            {
                return Err(CredentialError::InvalidBundle);
            }
        }
        validate_binding(&self.binding_digest)
    }

    fn encode(&self) -> Result<Vec<u8>, CredentialError> {
        self.validate()?;
        // Borrow secrets rather than cloning them into a serialization DTO.
        #[derive(Serialize)]
        struct BundleRef<'a> {
            version: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            inference_key: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            refresh_token: Option<&'a str>,
            binding_digest: &'a str,
        }
        let bytes = serde_json::to_vec(&BundleRef {
            version: VERSION,
            inference_key: self.inference_key(),
            refresh_token: self.refresh_token(),
            binding_digest: self.binding_digest(),
        })
        .map_err(|_| CredentialError::InvalidBundle)?;
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::TooLarge);
        }
        Ok(bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, CredentialError> {
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(CredentialError::TooLarge);
        }
        let stored: StoredBundle =
            serde_json::from_slice(bytes).map_err(|_| CredentialError::InvalidBundle)?;
        if stored.version != VERSION {
            return Err(CredentialError::UnsupportedVersion);
        }
        Self::new(
            stored.inference_key,
            stored.refresh_token,
            stored.binding_digest,
        )
    }
}

impl fmt::Debug for SecretBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretBundle").finish_non_exhaustive()
    }
}

fn validate_binding(binding: &str) -> Result<(), CredentialError> {
    let Some(digest) = binding.strip_prefix("sha256:") else {
        return Err(CredentialError::InvalidBundle);
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(CredentialError::InvalidBundle);
    }
    Ok(())
}

/// Implementations use the reference's storage_key, bound reads to
/// MAX_SECRET_BYTES, and sanitize platform errors. There is no default backend.
/// The lifecycle owner serializes operations on a reference, including the
/// absence check and write: the OS APIs do not provide compare-and-create.
pub(crate) trait SecretBackend: Send + Sync {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError>;
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError>;
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError>;
}

impl<B: SecretBackend + ?Sized> SecretBackend for std::sync::Arc<B> {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        (**self).read(reference)
    }
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        (**self).write(reference, bytes)
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        (**self).delete(reference)
    }
}

pub(crate) struct CredentialStore<B> {
    backend: B,
}

impl<B: SecretBackend> CredentialStore<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Write and verify an entry at an already-journaled, unpublished reference.
    /// Existing entries are neither overwritten nor deleted. On failure after a
    /// write attempt, cleanup is attempted; CleanupFailed preserves its address.
    /// A recovered pending reference must be inspected or deleted by the
    /// lifecycle owner before retrying; this method never adopts an old entry.
    pub(crate) fn prepare_at(
        &self,
        reference: &CredentialReference,
        bundle: &SecretBundle,
    ) -> Result<(), CredentialError> {
        reference.validate()?;
        let bytes = bundle.encode()?;
        match self.backend.read(reference) {
            Err(CredentialError::NoEntry) => {}
            Ok(_) => return Err(CredentialError::AlreadyExists),
            Err(error) => return Err(error),
        }
        let result = self.backend.write(reference, &bytes).and_then(|()| {
            let persisted = self.backend.read(reference)?;
            if persisted != bytes {
                return Err(CredentialError::VerificationFailed);
            }
            Ok(())
        });
        if let Err(error) = result {
            // An OS write can persist bytes and still return an error.
            self.delete(reference)
                .map_err(|_| CredentialError::CleanupFailed(*reference))?;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn load(
        &self,
        reference: &CredentialReference,
        expected_binding: &str,
    ) -> Result<SecretBundle, CredentialError> {
        reference.validate()?;
        validate_binding(expected_binding)?;
        let bundle = SecretBundle::decode(&self.backend.read(reference)?)?;
        if bundle.binding_digest != expected_binding {
            return Err(CredentialError::BindingMismatch);
        }
        Ok(bundle)
    }

    pub(crate) fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        reference.validate()?;
        match self.backend.delete(reference) {
            // A crash can leave a journal reference before its OS entry exists.
            // Treat absence as complete so recovery cannot wedge that journal.
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use crate::daemon::credential_store::*;

    #[derive(Clone, Copy, Default)]
    enum Fault {
        #[default]
        None,
        Read,
        WriteBefore,
        WriteAfter,
        Readback,
        CorruptReadback,
        MissingReadback,
    }

    #[derive(Default)]
    struct MemoryBackend {
        entries: Mutex<HashMap<String, Vec<u8>>>,
        fault: Mutex<Fault>,
        fail_delete: AtomicBool,
        reads: AtomicUsize,
        writes: AtomicUsize,
        deletes: AtomicUsize,
    }

    impl SecretBackend for MemoryBackend {
        fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            let fault = *self.fault.lock().unwrap();
            if matches!(fault, Fault::Read) {
                return Err(CredentialError::Unavailable);
            }
            let bytes = self
                .entries
                .lock()
                .unwrap()
                .get(&reference.storage_key()?)
                .cloned()
                .ok_or(CredentialError::NoEntry)?;
            match fault {
                Fault::Readback => Err(CredentialError::Unavailable),
                Fault::CorruptReadback => Ok(b"corrupt".to_vec()),
                Fault::MissingReadback => Err(CredentialError::NoEntry),
                _ => Ok(bytes),
            }
        }

        fn write(
            &self,
            reference: &CredentialReference,
            bytes: &[u8],
        ) -> Result<(), CredentialError> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            let fault = *self.fault.lock().unwrap();
            if matches!(fault, Fault::WriteBefore) {
                return Err(CredentialError::Unavailable);
            }
            self.entries
                .lock()
                .unwrap()
                .insert(reference.storage_key()?, bytes.to_vec());
            if matches!(fault, Fault::WriteAfter) {
                return Err(CredentialError::Unavailable);
            }
            Ok(())
        }

        fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
            self.deletes.fetch_add(1, Ordering::Relaxed);
            if self.fail_delete.load(Ordering::Relaxed) {
                return Err(CredentialError::Unavailable);
            }
            self.entries
                .lock()
                .unwrap()
                .remove(&reference.storage_key()?)
                .map(|_| ())
                .ok_or(CredentialError::NoEntry)
        }
    }

    fn binding() -> String {
        format!("sha256:{}", "a".repeat(64))
    }

    fn bundle() -> SecretBundle {
        SecretBundle::new(
            Some("synthetic-inference".into()),
            Some("synthetic-refresh".into()),
            binding(),
        )
        .unwrap()
    }

    fn counts(store: &CredentialStore<MemoryBackend>) -> (usize, usize, usize) {
        (
            store.backend.reads.load(Ordering::Relaxed),
            store.backend.writes.load(Ordering::Relaxed),
            store.backend.deletes.load(Ordering::Relaxed),
        )
    }

    #[test]
    fn reference_is_persistable_before_any_secret_write_and_recoverable_afterward() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        let encoded = serde_json::to_vec(&reference).unwrap();
        let recovered = serde_json::from_slice::<CredentialReference>(&encoded).unwrap();
        assert_eq!(reference, recovered);
        assert_ne!(reference, CredentialReference::allocate());
        assert_eq!(counts(&store), (0, 0, 0));
        store.prepare_at(&reference, &bundle()).unwrap();
        let loaded = store.load(&recovered, &binding()).unwrap();
        assert_eq!(loaded.inference_key(), Some("synthetic-inference"));
        assert_eq!(loaded.refresh_token(), Some("synthetic-refresh"));
        assert_eq!(loaded.binding_digest(), binding());
        assert_eq!(counts(&store), (3, 1, 0));
        let text = String::from_utf8(encoded).unwrap();
        assert!(!text.contains("synthetic"));
        assert!(!text.contains(&binding()));
        assert!(!format!("{reference:?}").contains(&reference.id.to_string()));
        assert!(!format!("{loaded:?}").contains("synthetic"));
    }

    #[test]
    fn inference_only_session_only_and_paired_bundles_round_trip() {
        for (key, refresh) in [(true, false), (false, true), (true, true)] {
            let store = CredentialStore::new(MemoryBackend::default());
            let value = SecretBundle::new(
                key.then(|| "synthetic-inference".into()),
                refresh.then(|| "synthetic-refresh".into()),
                binding(),
            )
            .unwrap();
            let reference = CredentialReference::allocate();
            store.prepare_at(&reference, &value).unwrap();
            let loaded = store.load(&reference, &binding()).unwrap();
            assert_eq!(loaded.inference_key(), value.inference_key());
            assert_eq!(loaded.refresh_token(), value.refresh_token());
            let wire: serde_json::Value = serde_json::from_slice(&value.encode().unwrap()).unwrap();
            assert_eq!(wire.get("inference_key").is_some(), key);
            assert_eq!(wire.get("refresh_token").is_some(), refresh);
            assert!(wire.get("legacy_refresh_token").is_none());
        }
    }

    #[test]
    fn existing_entries_are_never_overwritten_or_cleaned_up() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        store.prepare_at(&reference, &bundle()).unwrap();
        let other = SecretBundle::new(None, Some("replacement".into()), binding()).unwrap();
        assert_eq!(
            store.prepare_at(&reference, &other),
            Err(CredentialError::AlreadyExists)
        );
        let loaded = store.load(&reference, &binding()).unwrap();
        assert_eq!(loaded.refresh_token(), Some("synthetic-refresh"));
        assert_eq!(counts(&store), (4, 1, 0));
    }

    #[test]
    fn an_unreadable_or_malformed_existing_entry_cannot_be_replaced() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        store.backend.write(&reference, b"malformed").unwrap();
        assert_eq!(
            store.prepare_at(&reference, &bundle()),
            Err(CredentialError::AlreadyExists)
        );
        *store.backend.fault.lock().unwrap() = Fault::Read;
        assert_eq!(
            store.prepare_at(&reference, &bundle()),
            Err(CredentialError::Unavailable)
        );
        assert_eq!(counts(&store), (2, 1, 0));
        assert_eq!(store.backend.entries.lock().unwrap().len(), 1);
    }

    #[test]
    fn write_and_readback_failures_clean_up_only_the_requested_entry() {
        for (fault, expected) in [
            (Fault::WriteBefore, CredentialError::Unavailable),
            (Fault::WriteAfter, CredentialError::Unavailable),
            (Fault::Readback, CredentialError::Unavailable),
            (Fault::CorruptReadback, CredentialError::VerificationFailed),
            (Fault::MissingReadback, CredentialError::NoEntry),
        ] {
            let store = CredentialStore::new(MemoryBackend::default());
            let previous = CredentialReference::allocate();
            store.prepare_at(&previous, &bundle()).unwrap();
            *store.backend.fault.lock().unwrap() = fault;
            let pending = CredentialReference::allocate();
            assert_eq!(store.prepare_at(&pending, &bundle()), Err(expected));
            assert_eq!(store.backend.entries.lock().unwrap().len(), 1);
            *store.backend.fault.lock().unwrap() = Fault::None;
            assert!(store.load(&previous, &binding()).is_ok());
            assert_eq!(
                store.load(&pending, &binding()).unwrap_err(),
                CredentialError::NoEntry
            );
        }
    }

    #[test]
    fn unavailable_absence_check_never_writes_or_deletes() {
        let store = CredentialStore::new(MemoryBackend::default());
        *store.backend.fault.lock().unwrap() = Fault::Read;
        assert_eq!(
            store.prepare_at(&CredentialReference::allocate(), &bundle()),
            Err(CredentialError::Unavailable)
        );
        assert_eq!(counts(&store), (1, 0, 0));
    }

    #[test]
    fn failed_cleanup_retains_exact_reference_for_a_later_retry() {
        for fault in [Fault::WriteAfter, Fault::Readback] {
            let store = CredentialStore::new(MemoryBackend::default());
            *store.backend.fault.lock().unwrap() = fault;
            store.backend.fail_delete.store(true, Ordering::Relaxed);
            let pending = CredentialReference::allocate();
            let error = store.prepare_at(&pending, &bundle()).unwrap_err();
            assert_eq!(error.cleanup_reference(), Some(&pending));
            assert_eq!(store.backend.entries.lock().unwrap().len(), 1);
            assert!(!format!("{error:?}").contains(&pending.id.to_string()));
            assert!(!error.to_string().contains(&pending.id.to_string()));
            store.backend.fail_delete.store(false, Ordering::Relaxed);
            store.delete(error.cleanup_reference().unwrap()).unwrap();
            assert!(store.backend.entries.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn serialized_size_including_escaping_is_checked_before_backend_access() {
        let store = CredentialStore::new(MemoryBackend::default());
        let value = SecretBundle::new(Some("\\".repeat(1400)), None, binding()).unwrap();
        assert_eq!(
            store.prepare_at(&CredentialReference::allocate(), &value),
            Err(CredentialError::TooLarge)
        );
        assert_eq!(counts(&store), (0, 0, 0));
    }

    #[test]
    fn exact_serialized_bound_applies_to_each_credential_combination() {
        for (key, refresh) in [(true, false), (false, true), (true, true)] {
            let mut value = SecretBundle::new(
                key.then(|| "k".into()),
                refresh.then(|| "r".into()),
                binding(),
            )
            .unwrap();
            let remaining = MAX_SECRET_BYTES - value.encode().unwrap().len();
            let target = if key {
                value.inference_key.as_mut().unwrap()
            } else {
                value.refresh_token.as_mut().unwrap()
            };
            target.push_str(&"x".repeat(remaining));
            let store = CredentialStore::new(MemoryBackend::default());
            assert_eq!(value.encode().unwrap().len(), MAX_SECRET_BYTES);
            let reference = CredentialReference::allocate();
            store.prepare_at(&reference, &value).unwrap();
            assert!(store.load(&reference, &binding()).is_ok());
            if key {
                value.inference_key.as_mut().unwrap().push('x');
            } else {
                value.refresh_token.as_mut().unwrap().push('x');
            }
            let before = counts(&store);
            assert_eq!(
                store.prepare_at(&CredentialReference::allocate(), &value),
                Err(CredentialError::TooLarge)
            );
            assert_eq!(counts(&store), before);
        }
    }

    #[test]
    fn missing_load_is_an_error_and_only_delete_is_idempotent() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        assert_eq!(
            store.load(&reference, &binding()).unwrap_err(),
            CredentialError::NoEntry
        );
        assert_eq!(store.delete(&reference), Ok(()));
        store.backend.fail_delete.store(true, Ordering::Relaxed);
        assert_eq!(store.delete(&reference), Err(CredentialError::Unavailable));
    }

    #[test]
    fn invalid_references_are_refused_before_all_backend_operations() {
        let store = CredentialStore::new(MemoryBackend::default());
        let mut future = CredentialReference::allocate();
        future.version = VERSION + 1;
        let invalid = CredentialReference {
            version: VERSION,
            id: Uuid::nil(),
        };
        for (reference, expected) in [
            (future, CredentialError::UnsupportedVersion),
            (invalid, CredentialError::InvalidReference),
        ] {
            assert_eq!(store.prepare_at(&reference, &bundle()), Err(expected));
            assert_eq!(store.load(&reference, &binding()).unwrap_err(), expected);
            assert_eq!(store.delete(&reference), Err(expected));
            assert_eq!(reference.storage_key(), Err(expected));
        }
        assert_eq!(counts(&store), (0, 0, 0));
        let mut wire = serde_json::to_value(CredentialReference::allocate()).unwrap();
        wire["account"] = serde_json::json!("unexpected.near");
        assert!(serde_json::from_value::<CredentialReference>(wire).is_err());
    }

    #[test]
    fn malformed_oversized_future_or_unrecognized_payloads_fail_closed() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        let base: serde_json::Value = serde_json::from_slice(&bundle().encode().unwrap()).unwrap();
        let mut future = base.clone();
        future["version"] = serde_json::json!(VERSION + 1);
        let mut wallet = base.clone();
        wallet["wallet_private_key"] = serde_json::json!("synthetic-rejected");
        let mut old = base.clone();
        old["legacy_refresh_token"] = serde_json::json!("synthetic-rejected");
        let mut empty = base;
        empty["inference_key"] = serde_json::Value::Null;
        empty["refresh_token"] = serde_json::Value::Null;
        let duplicate = format!(
            "{{\"version\":1,\"inference_key\":\"a\",\"inference_key\":\"b\",\"binding_digest\":\"{}\"}}",
            binding()
        );
        for (bytes, expected) in [
            (b"broken".to_vec(), CredentialError::InvalidBundle),
            (vec![0xff], CredentialError::InvalidBundle),
            (vec![b'x'; MAX_SECRET_BYTES + 1], CredentialError::TooLarge),
            (
                serde_json::to_vec(&future).unwrap(),
                CredentialError::UnsupportedVersion,
            ),
            (
                serde_json::to_vec(&wallet).unwrap(),
                CredentialError::InvalidBundle,
            ),
            (
                serde_json::to_vec(&old).unwrap(),
                CredentialError::InvalidBundle,
            ),
            (
                serde_json::to_vec(&empty).unwrap(),
                CredentialError::InvalidBundle,
            ),
            (duplicate.into_bytes(), CredentialError::InvalidBundle),
        ] {
            store.backend.write(&reference, &bytes).unwrap();
            assert_eq!(store.load(&reference, &binding()).unwrap_err(), expected);
        }
    }

    #[test]
    fn metadata_binding_mismatch_never_releases_credentials() {
        let store = CredentialStore::new(MemoryBackend::default());
        let reference = CredentialReference::allocate();
        store.prepare_at(&reference, &bundle()).unwrap();
        let other = format!("sha256:{}", "b".repeat(64));
        assert_eq!(
            store.load(&reference, &other).unwrap_err(),
            CredentialError::BindingMismatch
        );
        let before = counts(&store);
        assert_eq!(
            store.load(&reference, "raw-account.near").unwrap_err(),
            CredentialError::InvalidBundle
        );
        assert_eq!(counts(&store), before);
    }

    #[test]
    fn no_credentials_empty_secrets_controls_and_bad_bindings_are_rejected() {
        assert_eq!(
            SecretBundle::new(None, None, binding()).unwrap_err(),
            CredentialError::InvalidBundle
        );
        for value in ["", " space", "line\n", "tab\t", "null\0", "unicode\u{2003}"] {
            assert_eq!(
                SecretBundle::new(Some(value.into()), None, binding()).unwrap_err(),
                CredentialError::InvalidBundle
            );
            assert_eq!(
                SecretBundle::new(Some("valid".into()), Some(value.into()), binding()).unwrap_err(),
                CredentialError::InvalidBundle
            );
        }
        for invalid in [
            "account.near".to_string(),
            format!("sha256:{}", "A".repeat(64)),
            format!("sha256:{}", "a".repeat(63)),
            format!("sha256:{}", "a".repeat(65)),
        ] {
            assert_eq!(
                SecretBundle::new(None, Some("valid".into()), invalid).unwrap_err(),
                CredentialError::InvalidBundle
            );
        }
        assert_eq!(
            SecretBundle::new(None, Some("x".repeat(MAX_SECRET_BYTES + 1)), binding()).unwrap_err(),
            CredentialError::TooLarge
        );
    }
}
