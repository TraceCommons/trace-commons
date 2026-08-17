use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The only manifest schema this build understands. An unknown value is a
/// refusal, not a warning: a client that guesses at a newer schema's meaning
/// is a client that can be talked into installing the wrong thing.
pub const UPDATE_MANIFEST_SCHEMA: &str = "trace_commons.update_manifest.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformArtifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub schema_version: String,
    pub version: String,
    pub published_at: String,
    /// Keyed by platform slug: `windows-x86_64`, `macos-universal`,
    /// `linux-x86_64`. A platform absent from this map has no release --
    /// which is the normal state when one build job failed and the others
    /// did not.
    pub platforms: BTreeMap<String, PlatformArtifact>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The detached signature was not valid base64.
    #[error("update_manifest_malformed_signature")]
    MalformedSignature,
    /// The signature did not verify against the pinned public key.
    #[error("update_manifest_bad_signature")]
    BadSignature,
    /// Signature verified, but the payload was not valid JSON.
    #[error("update_manifest_malformed_json")]
    MalformedJson,
    /// Signature verified and parsed, but the schema is not one we know.
    #[error("update_manifest_unknown_schema")]
    UnknownSchema,
}

/// Verify a detached Ed25519 signature over `bytes`, then parse.
///
/// The order matters and is the whole point of this function: nothing is
/// parsed until the signature over the exact bytes has verified. Callers
/// must not deserialize the manifest themselves.
pub fn verify_manifest(
    bytes: &[u8],
    signature_b64: &str,
    public_key: &[u8],
) -> Result<UpdateManifest, ManifestError> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| ManifestError::MalformedSignature)?;

    let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key);
    key.verify(bytes, &signature)
        .map_err(|_| ManifestError::BadSignature)?;

    let manifest: UpdateManifest =
        serde_json::from_slice(bytes).map_err(|_| ManifestError::MalformedJson)?;

    if manifest.schema_version != UPDATE_MANIFEST_SCHEMA {
        return Err(ManifestError::UnknownSchema);
    }

    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn test_keypair() -> Ed25519KeyPair {
        // A fixed seed keeps the test deterministic. This key is a test
        // fixture and signs nothing that is ever published.
        let seed = [7u8; 32];
        Ed25519KeyPair::from_seed_unchecked(&seed).expect("valid seed")
    }

    fn sign(kp: &Ed25519KeyPair, bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(kp.sign(bytes).as_ref())
    }

    fn good_manifest_bytes() -> Vec<u8> {
        br#"{
  "schema_version": "trace_commons.update_manifest.v1",
  "version": "0.2.0",
  "published_at": "2026-08-17T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "url": "https://example.invalid/tc-0.2.0.zip",
      "sha256": "a3f1b2c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f80",
      "size": 4096
    }
  }
}"#
        .to_vec()
    }

    #[test]
    fn accepts_a_correctly_signed_manifest() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let sig = sign(&kp, &bytes);
        let manifest =
            verify_manifest(&bytes, &sig, kp.public_key().as_ref()).expect("should verify");
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.platforms["windows-x86_64"].size, 4096);
    }

    #[test]
    fn rejects_a_manifest_whose_bytes_changed_after_signing() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let sig = sign(&kp, &bytes);
        let tampered = String::from_utf8(bytes)
            .unwrap()
            .replace("0.2.0", "9.9.9")
            .into_bytes();
        let err = verify_manifest(&tampered, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::BadSignature));
    }

    #[test]
    fn rejects_a_signature_from_the_wrong_key() {
        let kp = test_keypair();
        let other = Ed25519KeyPair::from_seed_unchecked(&[9u8; 32]).unwrap();
        let bytes = good_manifest_bytes();
        let sig = sign(&other, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::BadSignature));
    }

    #[test]
    fn rejects_an_unknown_schema_version() {
        let kp = test_keypair();
        let bytes = String::from_utf8(good_manifest_bytes())
            .unwrap()
            .replace(
                "trace_commons.update_manifest.v1",
                "trace_commons.update_manifest.v2",
            )
            .into_bytes();
        let sig = sign(&kp, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::UnknownSchema));
    }

    #[test]
    fn rejects_malformed_base64_without_panicking() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let err = verify_manifest(&bytes, "not base64!!", kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedSignature));
    }

    #[test]
    fn rejects_valid_signature_over_invalid_json() {
        let kp = test_keypair();
        let bytes = b"{ this is not json".to_vec();
        let sig = sign(&kp, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedJson));
    }
}
