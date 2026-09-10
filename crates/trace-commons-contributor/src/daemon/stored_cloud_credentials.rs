// INTEGRATION: persist this metadata only after the corresponding SecretBundle
// has been written and verified at reference(). Load that reference with
// binding_digest(), then resolve the returned bundle before adopting credentials.
// The lifecycle owns journaling, OS access, publication, and deletion. This
// module neither stores secrets in settings nor implements a plaintext fallback.
//! Nonsecret Cloud metadata bound to an immutable OS entry and its secret pair.
//! The persisted digest permits checking a guessed secret against the metadata;
//! it relies on Cloud tokens' entropy and is not a password-hardening function.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::daemon::credential_store::{
    CredentialError, CredentialReference, MAX_SECRET_BYTES, SecretBundle,
};
use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};

const BINDING_DOMAIN: &[u8] = b"trace-commons/stored-cloud-credentials/v1\0";
// Cloud IDs are opaque strings. Bound storage without imposing UUID, account,
// URL, or ASCII-only formats that the response types do not promise.
const MAX_ID_BYTES: usize = 1024;
const MAX_PREFIX_BYTES: usize = 128;

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredCloudCredentials {
    reference: CredentialReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    inference: Option<InferenceMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<SessionMetadata>,
    binding_digest: String,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InferenceMetadata {
    key_id: String,
    key_prefix: String,
    organization_id: String,
    workspace_id: String,
    minted_at: DateTime<Utc>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token_expires_at: Option<DateTime<Utc>>,
    stored_at: DateTime<Utc>,
}

impl StoredCloudCredentials {
    pub(crate) fn from_secrets(
        reference: CredentialReference,
        inference: Option<&NearAiInferenceCredential>,
        session: Option<&NearAiSession>,
    ) -> Result<(Self, SecretBundle), CredentialError> {
        let mut metadata = Self::metadata(reference, inference, session)?;
        let key = inference.map(|value| value.key.as_str());
        let refresh = session.map(|value| value.refresh_token.as_str());
        metadata.binding_digest = metadata.digest(key, refresh)?;
        let bundle = SecretBundle::new(
            key.map(str::to_owned),
            refresh.map(str::to_owned),
            metadata.binding_digest.clone(),
        )?;
        Ok((metadata, bundle))
    }

    pub(crate) fn reference(&self) -> CredentialReference {
        self.reference
    }

    pub(crate) fn binding_digest(&self) -> &str {
        &self.binding_digest
    }

    pub(crate) fn resolve(
        &self,
        bundle: SecretBundle,
    ) -> Result<(Option<NearAiInferenceCredential>, Option<NearAiSession>), CredentialError> {
        let expected = self.digest(bundle.inference_key(), bundle.refresh_token())?;
        if self.binding_digest != expected || bundle.binding_digest() != expected {
            return Err(CredentialError::BindingMismatch);
        }
        // digest checked presence on both sides before either zip can discard
        // a credential. No partial result is returned on a binding failure.
        let inference =
            self.inference
                .as_ref()
                .zip(bundle.inference_key())
                .map(|(metadata, key)| NearAiInferenceCredential {
                    key: key.to_owned(),
                    key_id: metadata.key_id.clone(),
                    key_prefix: metadata.key_prefix.clone(),
                    organization_id: metadata.organization_id.clone(),
                    workspace_id: metadata.workspace_id.clone(),
                    minted_at: metadata.minted_at,
                });
        let session =
            self.session
                .as_ref()
                .zip(bundle.refresh_token())
                .map(|(metadata, refresh_token)| NearAiSession {
                    refresh_token: refresh_token.to_owned(),
                    refresh_token_expires_at: metadata.refresh_token_expires_at,
                    stored_at: metadata.stored_at,
                });
        Ok((inference, session))
    }

    /// Match both secret values and every metadata field without cloning the
    /// secrets into another bundle. Rotation or account replacement differs.
    pub(crate) fn matches(
        &self,
        inference: Option<&NearAiInferenceCredential>,
        session: Option<&NearAiSession>,
    ) -> bool {
        let Ok(mut candidate) = Self::metadata(self.reference, inference, session) else {
            return false;
        };
        let Ok(digest) = candidate.digest(
            inference.map(|value| value.key.as_str()),
            session.map(|value| value.refresh_token.as_str()),
        ) else {
            return false;
        };
        candidate.binding_digest = digest;
        *self == candidate
    }

    fn metadata(
        reference: CredentialReference,
        inference: Option<&NearAiInferenceCredential>,
        session: Option<&NearAiSession>,
    ) -> Result<Self, CredentialError> {
        reference.storage_key()?;
        if inference.is_none() && session.is_none() {
            return Err(CredentialError::InvalidBundle);
        }
        let inference = inference
            .map(|value| {
                validate_fields(
                    &value.key_id,
                    &value.key_prefix,
                    &value.organization_id,
                    &value.workspace_id,
                )?;
                Ok(InferenceMetadata {
                    key_id: value.key_id.clone(),
                    key_prefix: value.key_prefix.clone(),
                    organization_id: value.organization_id.clone(),
                    workspace_id: value.workspace_id.clone(),
                    minted_at: value.minted_at,
                })
            })
            .transpose()?;
        let session = session.map(|value| SessionMetadata {
            refresh_token_expires_at: value.refresh_token_expires_at,
            stored_at: value.stored_at,
        });
        Ok(Self {
            reference,
            inference,
            session,
            binding_digest: String::new(),
        })
    }

    fn digest(&self, key: Option<&str>, refresh: Option<&str>) -> Result<String, CredentialError> {
        self.reference.storage_key()?;
        if self.inference.is_none() && self.session.is_none() {
            return Err(CredentialError::InvalidBundle);
        }
        if self.inference.is_some() != key.is_some() || self.session.is_some() != refresh.is_some()
        {
            return Err(CredentialError::BindingMismatch);
        }
        if let Some(metadata) = &self.inference {
            validate_fields(
                &metadata.key_id,
                &metadata.key_prefix,
                &metadata.organization_id,
                &metadata.workspace_id,
            )?;
        }
        for secret in [key, refresh].into_iter().flatten() {
            if secret.len() > MAX_SECRET_BYTES {
                return Err(CredentialError::TooLarge);
            }
        }
        // These fixed-order structs and tuple contain metadata only. JSON's
        // field and string framing prevents ambiguous concatenation. Secret
        // bytes enter the hash directly and never enter this serialized buffer.
        let metadata = serde_json::to_vec(&(&self.reference, &self.inference, &self.session))
            .map_err(|_| CredentialError::InvalidBundle)?;
        let mut hash = Sha256::new();
        hash.update(BINDING_DOMAIN);
        hash.update((metadata.len() as u64).to_be_bytes());
        hash.update(metadata);
        for secret in [key, refresh] {
            match secret {
                Some(secret) => {
                    hash.update([1]);
                    hash.update((secret.len() as u64).to_be_bytes());
                    hash.update(secret.as_bytes());
                }
                None => hash.update([0]),
            }
        }
        Ok(format!("sha256:{}", hex::encode(hash.finalize())))
    }
}

impl fmt::Debug for StoredCloudCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredCloudCredentials")
            .finish_non_exhaustive()
    }
}

fn validate_fields(
    key_id: &str,
    key_prefix: &str,
    organization_id: &str,
    workspace_id: &str,
) -> Result<(), CredentialError> {
    for id in [key_id, organization_id, workspace_id] {
        if id.len() > MAX_ID_BYTES {
            return Err(CredentialError::TooLarge);
        }
        if id.trim().is_empty() || id.chars().any(char::is_control) {
            return Err(CredentialError::InvalidBundle);
        }
    }
    // The display prefix is not an ID. Preserve an empty prefix if an older
    // credential has one; identity comes from the service's actual identifiers.
    if key_prefix.len() > MAX_PREFIX_BYTES {
        return Err(CredentialError::TooLarge);
    }
    if key_prefix.chars().any(char::is_control) {
        return Err(CredentialError::InvalidBundle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::daemon::stored_cloud_credentials::*;
    use serde_json::json;

    fn credentials() -> (NearAiInferenceCredential, NearAiSession) {
        let time = DateTime::from_timestamp(1_800_000_000, 123_456_789).unwrap();
        (
            NearAiInferenceCredential {
                key: "synthetic-inference-secret".into(),
                key_id: "opaque:key/東京".into(),
                key_prefix: "synthetic".into(),
                organization_id: "organization-1".into(),
                workspace_id: "workspace-1".into(),
                minted_at: time,
            },
            NearAiSession {
                refresh_token: "synthetic-refresh-secret".into(),
                refresh_token_expires_at: Some(time + chrono::Duration::hours(1)),
                stored_at: time,
            },
        )
    }

    #[test]
    fn key_only_session_only_and_paired_credentials_round_trip() {
        let (inference, session) = credentials();
        for (key, refresh) in [(true, false), (false, true), (true, true)] {
            let reference = CredentialReference::allocate();
            let (metadata, bundle) = StoredCloudCredentials::from_secrets(
                reference,
                key.then_some(&inference),
                refresh.then_some(&session),
            )
            .unwrap();
            let serialized = serde_json::to_vec(&metadata).unwrap();
            let restored: StoredCloudCredentials = serde_json::from_slice(&serialized).unwrap();
            assert_eq!(restored, metadata);
            assert_eq!(restored.reference(), reference);
            assert_eq!(restored.binding_digest(), bundle.binding_digest());
            assert!(restored.matches(key.then_some(&inference), refresh.then_some(&session)));
            assert_eq!(
                restored.resolve(bundle).unwrap(),
                (
                    key.then(|| inference.clone()),
                    refresh.then(|| session.clone())
                )
            );
            let serialized = String::from_utf8(serialized).unwrap();
            for secret in [&inference.key, &session.refresh_token] {
                assert!(!serialized.contains(secret));
                assert!(!format!("{restored:?}").contains(secret));
            }
            for field in [
                &inference.key_id,
                &inference.organization_id,
                &inference.workspace_id,
            ] {
                assert!(!format!("{restored:?}").contains(field));
            }
        }
    }

    #[test]
    fn every_metadata_field_and_reference_participates_in_the_binding() {
        let (inference, session) = credentials();
        for (section, field, value) in [
            ("inference", "key_id", json!("another-key")),
            ("inference", "key_prefix", json!("another-prefix")),
            (
                "inference",
                "organization_id",
                json!("another-organization"),
            ),
            ("inference", "workspace_id", json!("another-workspace")),
            ("inference", "minted_at", json!("2030-01-01T00:00:00Z")),
            ("session", "refresh_token_expires_at", json!(null)),
            ("session", "stored_at", json!("2030-01-01T00:00:00Z")),
        ] {
            let (metadata, bundle) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            let mut changed = serde_json::to_value(metadata).unwrap();
            changed[section][field] = value;
            let changed: StoredCloudCredentials = serde_json::from_value(changed).unwrap();
            assert!(
                !changed.matches(Some(&inference), Some(&session)),
                "{section}.{field}"
            );
            assert_eq!(
                changed.resolve(bundle).unwrap_err(),
                CredentialError::BindingMismatch
            );
        }
        for field in ["reference", "binding_digest"] {
            let (metadata, bundle) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            let mut changed = serde_json::to_value(metadata).unwrap();
            changed[field] = if field == "reference" {
                serde_json::to_value(CredentialReference::allocate()).unwrap()
            } else {
                json!(format!("sha256:{}", "0".repeat(64)))
            };
            let changed: StoredCloudCredentials = serde_json::from_value(changed).unwrap();
            assert!(!changed.matches(Some(&inference), Some(&session)));
            assert_eq!(
                changed.resolve(bundle).unwrap_err(),
                CredentialError::BindingMismatch
            );
        }
    }

    #[test]
    fn changed_secrets_cannot_reuse_the_original_bundle_binding() {
        let (inference, session) = credentials();
        for change_key in [true, false] {
            let (metadata, _) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            let mut changed_key = inference.clone();
            let mut changed_session = session.clone();
            if change_key {
                changed_key.key.push('x');
            } else {
                changed_session.refresh_token.push('x');
            }
            let bundle = SecretBundle::new(
                Some(changed_key.key.clone()),
                Some(changed_session.refresh_token.clone()),
                metadata.binding_digest().to_owned(),
            )
            .unwrap();
            assert!(!metadata.matches(Some(&changed_key), Some(&changed_session)));
            assert_eq!(
                metadata.resolve(bundle).unwrap_err(),
                CredentialError::BindingMismatch
            );
        }
    }

    #[test]
    fn both_sides_must_have_the_same_credential_presence() {
        let (inference, session) = credentials();
        assert_eq!(
            StoredCloudCredentials::from_secrets(CredentialReference::allocate(), None, None)
                .unwrap_err(),
            CredentialError::InvalidBundle
        );
        for (key, refresh) in [(true, false), (false, true)] {
            let (metadata, _) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            let bundle = SecretBundle::new(
                key.then(|| inference.key.clone()),
                refresh.then(|| session.refresh_token.clone()),
                metadata.binding_digest().to_owned(),
            )
            .unwrap();
            assert!(!metadata.matches(key.then_some(&inference), refresh.then_some(&session)));
            assert_eq!(
                metadata.resolve(bundle).unwrap_err(),
                CredentialError::BindingMismatch
            );
            let (mut metadata, bundle) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            if key {
                metadata.session = None;
            } else {
                metadata.inference = None;
            }
            assert_eq!(
                metadata.resolve(bundle).unwrap_err(),
                CredentialError::BindingMismatch
            );
        }
    }

    #[test]
    fn opaque_ids_are_bounded_without_requiring_a_specific_format() {
        let (mut inference, session) = credentials();
        inference.key_id = "x".repeat(MAX_ID_BYTES);
        inference.key_prefix.clear();
        assert!(
            StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session)
            )
            .is_ok()
        );
        inference.key_id.push('x');
        assert_eq!(
            StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session)
            )
            .unwrap_err(),
            CredentialError::TooLarge
        );
        for id in ["", "   ", "bad\0id", "bad\nid"] {
            inference.key_id = id.into();
            assert_eq!(
                StoredCloudCredentials::from_secrets(
                    CredentialReference::allocate(),
                    Some(&inference),
                    Some(&session)
                )
                .unwrap_err(),
                CredentialError::InvalidBundle
            );
        }
        inference.key_id = "valid".into();
        inference.key_prefix = "x".repeat(MAX_PREFIX_BYTES + 1);
        assert_eq!(
            StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                None
            )
            .unwrap_err(),
            CredentialError::TooLarge
        );
    }

    #[test]
    fn unknown_fields_and_invalid_references_fail_closed() {
        let (inference, mut session) = credentials();
        session.refresh_token_expires_at = None;
        let (metadata, bundle) = StoredCloudCredentials::from_secrets(
            CredentialReference::allocate(),
            None,
            Some(&session),
        )
        .unwrap();
        assert_eq!(
            metadata.resolve(bundle).unwrap(),
            (None, Some(session.clone()))
        );
        for section in [None, Some("inference"), Some("session")] {
            let (metadata, _) = StoredCloudCredentials::from_secrets(
                CredentialReference::allocate(),
                Some(&inference),
                Some(&session),
            )
            .unwrap();
            let mut value = serde_json::to_value(metadata).unwrap();
            let object = if let Some(section) = section {
                &mut value[section]
            } else {
                &mut value
            };
            object["wallet_private_key"] = json!("synthetic-rejected");
            assert!(serde_json::from_value::<StoredCloudCredentials>(value).is_err());
        }
        let (metadata, bundle) = StoredCloudCredentials::from_secrets(
            CredentialReference::allocate(),
            Some(&inference),
            None,
        )
        .unwrap();
        let mut value = serde_json::to_value(metadata).unwrap();
        value["reference"]["version"] = json!(255);
        let changed: StoredCloudCredentials = serde_json::from_value(value).unwrap();
        assert!(!changed.matches(Some(&inference), None));
        assert_eq!(
            changed.resolve(bundle).unwrap_err(),
            CredentialError::UnsupportedVersion
        );
    }
}
