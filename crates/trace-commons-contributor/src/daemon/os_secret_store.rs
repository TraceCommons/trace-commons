//! Per-entry OS storage for opaque, versioned Cloud credential references.

use std::fmt;
use std::sync::Arc;

use keyring_core::{Entry, Error as KeyringError, api::CredentialStore as NativeStore};

use crate::daemon::credential_store::{
    CredentialError, CredentialReference, MAX_SECRET_BYTES, SecretBackend,
};

// Stable namespace: neither account names nor config-directory paths belong in
// OS credential metadata. The validated reference supplies the unique entry ID.
const SERVICE: &str = "trace-commons.near-ai.credentials";

/// Synchronous OS operations may prompt the user or block on the session bus.
/// Async callers must use `spawn_blocking`, including for construction, and
/// serialize mutations through the lifecycle owner. Creating this handle does
/// not establish that the store is unlocked; each operation fails closed.
#[derive(Clone)]
pub(crate) struct OsSecretBackend {
    store: Arc<NativeStore>,
    service: &'static str,
}

impl OsSecretBackend {
    pub(crate) fn new() -> Result<Self, CredentialError> {
        #[cfg(target_os = "macos")]
        let store: Arc<NativeStore> =
            apple_native_keyring_store::keychain::Store::new().map_err(storage_error)?;
        #[cfg(target_os = "windows")]
        let store: Arc<NativeStore> =
            windows_native_keyring_store::Store::new().map_err(storage_error)?;
        #[cfg(target_os = "linux")]
        let store: Arc<NativeStore> =
            zbus_secret_service_keyring_store::Store::new().map_err(storage_error)?;

        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        {
            Ok(Self {
                store,
                service: SERVICE,
            })
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Err(CredentialError::Unavailable)
        }
    }

    pub(crate) fn commons() -> Result<Self, CredentialError> {
        let mut backend = Self::new()?;
        backend.service = "trace-commons.account.credentials";
        Ok(backend)
    }

    fn entry(&self, reference: &CredentialReference) -> Result<Entry, CredentialError> {
        let storage_key = reference.storage_key()?;
        // The Windows provider otherwise defaults to Enterprise persistence,
        // which permits roaming this device credential to other machines.
        #[cfg(target_os = "windows")]
        let modifiers = std::collections::HashMap::from([("persistence", "Local")]);
        #[cfg(target_os = "windows")]
        let modifiers = Some(&modifiers);
        #[cfg(not(target_os = "windows"))]
        let modifiers = None;
        self.store
            .build(self.service, &storage_key, modifiers)
            .map_err(storage_error)
    }
}

impl fmt::Debug for OsSecretBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OsSecretBackend").finish_non_exhaustive()
    }
}

impl SecretBackend for OsSecretBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        // The OS API returns a complete blob. Enforce the portable limit before
        // returning it to the bundle decoder; do not attempt a fallback read.
        let bytes = self.entry(reference)?.get_secret().map_err(storage_error)?;
        validate_bytes(&bytes)?;
        Ok(bytes)
    }

    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        validate_bytes(bytes)?;
        // Binary APIs preserve UTF-8 JSON verbatim; Windows password APIs would
        // convert it to UTF-16, changing the portable byte limit and read format.
        self.entry(reference)?
            .set_secret(bytes)
            .map_err(storage_error)
    }

    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.entry(reference)?
            .delete_credential()
            .map_err(storage_error)
    }
}

fn validate_bytes(bytes: &[u8]) -> Result<(), CredentialError> {
    if bytes.len() > MAX_SECRET_BYTES {
        return Err(CredentialError::TooLarge);
    }
    // KDE Wallet requires UTF-8. Schema validation stays in credential_store;
    // its serde JSON representation already satisfies this encoding contract.
    if bytes.is_empty() || std::str::from_utf8(bytes).is_err() {
        return Err(CredentialError::InvalidBundle);
    }
    Ok(())
}

fn storage_error(error: KeyringError) -> CredentialError {
    // Platform errors can contain secret bytes or identifying metadata. Never
    // format, log, or retain them in the daemon's error surface.
    match error {
        KeyringError::NoEntry => CredentialError::NoEntry,
        KeyringError::TooLong(_, _) => CredentialError::TooLarge,
        KeyringError::BadEncoding(_) | KeyringError::BadDataFormat(_, _) => {
            CredentialError::InvalidBundle
        }
        _ => CredentialError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use crate::daemon::credential_store::{
        CredentialError, CredentialReference, MAX_SECRET_BYTES, SecretBackend,
    };
    use crate::daemon::os_secret_store::{OsSecretBackend, storage_error, validate_bytes};
    use keyring_core::Error as KeyringError;

    #[test]
    fn portable_bounds_accept_utf8_json_and_reject_invalid_or_oversized_bytes() {
        let encoded = serde_json::to_vec(&serde_json::json!({"value": "é\\\""})).unwrap();
        assert_eq!(validate_bytes(&encoded), Ok(()));
        let exact = "é".repeat(MAX_SECRET_BYTES / 2).into_bytes();
        assert_eq!(validate_bytes(&exact), Ok(()));
        let mut oversized = exact;
        oversized.push(b'x');
        assert_eq!(validate_bytes(&oversized), Err(CredentialError::TooLarge));
        assert_eq!(validate_bytes(&[]), Err(CredentialError::InvalidBundle));
        assert_eq!(validate_bytes(&[0xff]), Err(CredentialError::InvalidBundle));
    }

    #[test]
    fn platform_details_are_replaced_with_fixed_error_labels() {
        let private = "synthetic-private-platform-detail";
        let cause = || Box::new(std::io::Error::other(private));
        for (error, expected) in [
            (KeyringError::NoEntry, CredentialError::NoEntry),
            (
                KeyringError::TooLong(private.into(), 1),
                CredentialError::TooLarge,
            ),
            (
                KeyringError::BadEncoding(private.as_bytes().to_vec()),
                CredentialError::InvalidBundle,
            ),
            (
                KeyringError::BadDataFormat(private.as_bytes().to_vec(), cause()),
                CredentialError::InvalidBundle,
            ),
            (
                KeyringError::PlatformFailure(cause()),
                CredentialError::Unavailable,
            ),
            (
                KeyringError::NoStorageAccess(cause()),
                CredentialError::Unavailable,
            ),
            (
                KeyringError::BadStoreFormat(private.into()),
                CredentialError::Unavailable,
            ),
            (
                KeyringError::Invalid(private.into(), private.into()),
                CredentialError::Unavailable,
            ),
            (
                KeyringError::Ambiguous(Vec::new()),
                CredentialError::Unavailable,
            ),
            (KeyringError::NoDefaultStore, CredentialError::Unavailable),
            (
                KeyringError::NotSupportedByStore(private.into()),
                CredentialError::Unavailable,
            ),
        ] {
            let safe = storage_error(error);
            assert_eq!(safe, expected);
            assert!(!safe.to_string().contains(private));
            assert!(!format!("{safe:?}").contains(private));
        }
    }

    #[test]
    #[ignore = "manual real Commons OS credential round trip; may prompt"]
    fn real_commons_os_round_trip_removes_only_its_synthetic_entry() {
        let backend = OsSecretBackend::commons().expect("OS store unavailable");
        let reference = CredentialReference::allocate();
        assert!(matches!(
            backend.read(&reference),
            Err(CredentialError::NoEntry)
        ));
        struct Cleanup(OsSecretBackend, CredentialReference);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.0.delete(&self.1);
            }
        }
        let _cleanup = Cleanup(backend.clone(), reference);
        let bytes = br#"{"version":1,"binding":"synthetic","payload":"c3ludGhldGlj"}"#;
        backend.write(&reference, bytes).unwrap();
        assert_eq!(
            OsSecretBackend::commons()
                .unwrap()
                .read(&reference)
                .unwrap(),
            bytes
        );
        assert!(matches!(
            OsSecretBackend::new().unwrap().read(&reference),
            Err(CredentialError::NoEntry)
        ));
        backend.delete(&reference).unwrap();
        assert!(matches!(
            backend.read(&reference),
            Err(CredentialError::NoEntry)
        ));
    }

    /// Run explicitly on each supported OS with its user's credential store
    /// available. This uses real OS storage and only its fresh synthetic UUID.
    #[test]
    #[ignore = "manual real OS credential-store round trip; may prompt for access"]
    fn real_os_round_trip_removes_only_its_synthetic_entry() {
        let backend = OsSecretBackend::new().expect("OS store unavailable");
        let reference: CredentialReference = serde_json::from_value(serde_json::json!({
            "version": 1,
            "id": uuid::Uuid::new_v4(),
        }))
        .unwrap();
        assert!(matches!(
            backend.read(&reference),
            Err(CredentialError::NoEntry)
        ));

        // Cleanup also runs when a later assertion fails. Do not arm it until
        // the UUID has been checked absent, so it cannot delete another entry.
        struct Cleanup<'a>(&'a OsSecretBackend, CredentialReference);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete(&self.1);
            }
        }
        let _cleanup = Cleanup(&backend, reference);
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "inference_key": "synthetic-os-store-round-trip",
            "binding_digest": format!("sha256:{}", "a".repeat(64)),
        }))
        .unwrap();
        backend.write(&reference, &bytes).expect("OS write failed");
        #[cfg(windows)]
        assert_eq!(
            backend
                .entry(&reference)
                .unwrap()
                .get_attributes()
                .unwrap()
                .get("persistence")
                .map(String::as_str),
            Some("Local"),
        );
        let restored = backend.read(&reference).expect("OS read failed");
        assert!(
            restored == bytes,
            "OS credential round trip changed the payload"
        );
        assert_eq!(
            OsSecretBackend::new().unwrap().read(&reference).unwrap(),
            bytes
        );
        backend.delete(&reference).expect("OS deletion failed");
        assert!(matches!(
            backend.read(&reference),
            Err(CredentialError::NoEntry)
        ));
    }
}
