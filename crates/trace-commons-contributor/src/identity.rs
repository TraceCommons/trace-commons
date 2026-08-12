//! Device identity, enrollment grants, and claim-request signing.
//!
//! The device keypair is generated once and persisted via `ConfigStore`.
//! Enrollment grants carry a signed `TraceInstanceEnrollAttestation` minted
//! by an instance's private key, base64-encoded for transport out-of-band
//! (e.g. QR code / paste). Claim requests are signed by the device key over
//! the exact JSON body bytes sent to the issuer.

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trace_commons_protocol::onboarding::{
    TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION, TraceInstanceEnrollAttestation,
    TraceInstanceEnrollRequest, TraceOnboardClientInfo, instance_enroll_attestation_signing_bytes,
    user_subject_hash,
};

use crate::config::{ConfigStore, ContributorConfig};

pub const ENROLLMENT_GRANT_SCHEMA_VERSION: &str = "trace_commons.enrollment_grant.v1";
pub const TRACE_DEVICE_KEY_ID_HEADER: &str = "x-trace-device-key-id";
pub const TRACE_DEVICE_SIGNATURE_HEADER: &str = "x-trace-device-signature";

const CLAIM_REQUEST_SCHEMA_VERSION: &str = "ironclaw.trace_upload_claim_request.v1";

/// This contributor's stable device keypair.
pub struct DeviceIdentity {
    pub device_key_id: String,
    pub public_key_b64: String,
    keypair: Ed25519KeyPair,
}

impl DeviceIdentity {
    /// Load and validate the persisted device key without creating one.
    pub fn load(store: &ConfigStore) -> Result<Option<Self>> {
        store
            .load_device_key()
            .context("loading device key")?
            .map(|pkcs8_der| Self::from_pkcs8(&pkcs8_der))
            .transpose()
    }

    /// Load the persisted device key, or generate and persist a new one.
    pub fn load_or_generate(store: &ConfigStore) -> Result<Self> {
        if let Some(identity) = Self::load(store)? {
            return Ok(identity);
        }
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .map_err(|_| anyhow!("generating device keypair"))?;
        let pkcs8_der = doc.as_ref().to_vec();
        store
            .save_device_key(&pkcs8_der)
            .context("saving device key")?;
        Self::from_pkcs8(&pkcs8_der)
    }

    fn from_pkcs8(pkcs8_der: &[u8]) -> Result<Self> {
        let keypair = Ed25519KeyPair::from_pkcs8(pkcs8_der)
            .map_err(|_| anyhow!("parsing stored device key"))?;
        let public_key_bytes = keypair.public_key().as_ref().to_vec();
        let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            &public_key_bytes,
        );
        let public_key_b64 = BASE64.encode(&public_key_bytes);
        Ok(Self {
            device_key_id,
            public_key_b64,
            keypair,
        })
    }

    /// Base64-STANDARD Ed25519 signature over `bytes`.
    pub fn sign_b64(&self, bytes: &[u8]) -> String {
        let sig = self.keypair.sign(bytes);
        BASE64.encode(sig.as_ref())
    }
}

/// A portable, instance-signed grant of enrollment for one device+user pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollmentGrant {
    pub schema_version: String,
    pub issuer_url: String,
    pub instance_public_key: String,
    pub attestation: TraceInstanceEnrollAttestation,
    pub attestation_sig: String,
}

impl EnrollmentGrant {
    /// Decode a base64-STANDARD JSON-encoded grant, rejecting unknown
    /// schema versions.
    pub fn decode(b64: &str) -> Result<Self> {
        let bytes = BASE64
            .decode(b64)
            .context("decoding enrollment grant base64")?;
        let grant: Self =
            serde_json::from_slice(&bytes).context("parsing enrollment grant JSON")?;
        if grant.schema_version != ENROLLMENT_GRANT_SCHEMA_VERSION {
            bail!(
                "unsupported enrollment grant schema_version: {}",
                grant.schema_version
            );
        }
        Ok(grant)
    }

    /// Base64-STANDARD of the JSON encoding of this grant.
    pub fn encode(&self) -> String {
        let json = serde_json::to_string(self).expect("EnrollmentGrant serializes");
        BASE64.encode(json.as_bytes())
    }
}

/// Mint a new enrollment grant, signing the attestation with the instance's
/// Ed25519 key (PKCS8 DER).
// Arity is fixed by the plan's interface contract for this function.
#[allow(clippy::too_many_arguments)]
pub fn mint_grant(
    instance_key_pkcs8_der: &[u8],
    issuer_url: &str,
    instance_id: &str,
    user_subject: &str,
    audience: &str,
    device_key_id: &str,
    ttl_seconds: i64,
    now: DateTime<Utc>,
) -> Result<EnrollmentGrant> {
    let keypair = Ed25519KeyPair::from_pkcs8(instance_key_pkcs8_der)
        .map_err(|_| anyhow!("parsing instance key"))?;
    let instance_public_key = BASE64.encode(keypair.public_key().as_ref());

    let attestation = TraceInstanceEnrollAttestation {
        device_key_id: device_key_id.to_string(),
        aud: audience.to_string(),
        instance_id: instance_id.to_string(),
        user_subject: user_subject.to_string(),
        nonce: Uuid::new_v4().to_string(),
        exp: now.timestamp() + ttl_seconds,
    };
    let signing_bytes = instance_enroll_attestation_signing_bytes(&attestation);
    let attestation_sig = BASE64.encode(keypair.sign(&signing_bytes).as_ref());

    Ok(EnrollmentGrant {
        schema_version: ENROLLMENT_GRANT_SCHEMA_VERSION.to_string(),
        issuer_url: issuer_url.to_string(),
        instance_public_key,
        attestation,
        attestation_sig,
    })
}

/// Strip PEM armor and base64-STANDARD decode a PKCS8 private key, matching
/// the shape emitted by the issuer's `generate_upload_claim_keypair`.
pub fn pem_to_pkcs8_der(pem: &str) -> Result<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|line| {
            !line.starts_with("-----BEGIN") && !line.starts_with("-----END") && !line.is_empty()
        })
        .collect();
    BASE64
        .decode(body.as_bytes())
        .context("decoding PEM body as base64")
}

/// Build the wire request the instance sends to the issuer's enroll
/// endpoint, verifying the grant was minted for this device.
pub fn build_enroll_request(
    grant: &EnrollmentGrant,
    device: &DeviceIdentity,
) -> Result<TraceInstanceEnrollRequest> {
    if grant.attestation.device_key_id != device.device_key_id {
        bail!("enrollment grant was minted for a different device");
    }
    Ok(TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
        instance_public_key: grant.instance_public_key.clone(),
        device_public_key: device.public_key_b64.clone(),
        attestation: grant.attestation.clone(),
        attestation_sig: grant.attestation_sig.clone(),
        client_info: TraceOnboardClientInfo {
            agent: "trace-commons-contributor".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    })
}

/// A device-signed upload-claim request body plus the header values needed
/// to submit it.
pub struct SignedClaimRequest {
    pub body: String,
    pub device_key_id: String,
    pub signature_b64: String,
}

/// Build and sign the upload-claim request body for `cfg`, using `device`'s
/// key. The body is serialized exactly once; the returned `body` string is
/// the same bytes the signature covers.
///
/// Uses `cfg.consent_scopes` (validated) and their implied allowed uses.
/// Callers that need a different scopes/uses request (e.g. an empty request
/// for a status read-back, which the server resolves to the full grant
/// ceiling) should use [`build_signed_claim_request_with_scopes`] directly.
pub fn build_signed_claim_request(
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<SignedClaimRequest> {
    let consent_scopes = crate::consent::validate_scopes(&cfg.consent_scopes)
        .context("invalid consent scopes in stored config (re-run login to fix)")?;
    let allowed_uses = crate::consent::scopes_to_allowed_uses(&consent_scopes);
    build_signed_claim_request_with_scopes(cfg, device, now, &consent_scopes, &allowed_uses)
}

/// Build and sign the upload-claim request body for `cfg`, using `device`'s
/// key, but with explicit `scopes`/`uses` instead of deriving them from
/// `cfg.consent_scopes`. An empty `scopes`/`uses` pair is a valid request:
/// the issuer resolves an empty request to the caller's full grant ceiling
/// (see `trace_upload_claim_issuer::resolve_granted_uses`), which is exactly
/// what a status read-back wants regardless of what scopes were narrowed for
/// submission.
pub fn build_signed_claim_request_with_scopes(
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
    scopes: &[String],
    uses: &[String],
) -> Result<SignedClaimRequest> {
    let subject = user_subject_hash(&cfg.user_subject);
    let payload = serde_json::json!({
        "schema_version": CLAIM_REQUEST_SCHEMA_VERSION,
        "tenant_id": cfg.tenant_id,
        "audience": cfg.audience,
        "consent_scopes": scopes,
        "allowed_uses": uses,
        "subject": subject,
        "requested_at": now.to_rfc3339(),
    });
    let body = serde_json::to_string(&payload).context("serializing claim request body")?;
    let signature_b64 = device.sign_b64(body.as_bytes());
    Ok(SignedClaimRequest {
        body,
        device_key_id: device.device_key_id.clone(),
        signature_b64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
    use trace_commons_protocol::onboarding::instance_enroll_attestation_signing_bytes;

    #[test]
    fn device_identity_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let a = DeviceIdentity::load_or_generate(&store).unwrap();
        let b = DeviceIdentity::load_or_generate(&store).unwrap();
        assert_eq!(a.device_key_id, b.device_key_id);
        assert!(a.device_key_id.starts_with("sha256:"));
    }

    #[test]
    fn minted_grant_signature_verifies_against_signing_bytes() {
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let grant = mint_grant(
            doc.as_ref(),
            "https://issuer.example",
            "instance-1",
            "alice@example.com",
            "trace-commons-upload",
            "sha256:ab",
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let pk = UnparsedPublicKey::new(&ED25519, kp.public_key().as_ref());
        use base64::Engine as _;
        let sig = base64::engine::general_purpose::STANDARD
            .decode(&grant.attestation_sig)
            .unwrap();
        pk.verify(
            &instance_enroll_attestation_signing_bytes(&grant.attestation),
            &sig,
        )
        .unwrap();
        assert_eq!(grant.attestation.user_subject, "alice@example.com");
        // Grant round-trips through base64.
        let decoded = EnrollmentGrant::decode(&grant.encode()).unwrap();
        assert_eq!(decoded.attestation.nonce, grant.attestation.nonce);
    }

    #[test]
    fn enroll_request_rejects_foreign_device_key_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let grant = mint_grant(
            doc.as_ref(),
            "https://issuer.example",
            "instance-1",
            "alice",
            "trace-commons-upload",
            "sha256:not-this-device",
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        assert!(build_enroll_request(&grant, &device).is_err());
    }

    #[test]
    fn claim_request_signature_covers_exact_body_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into(), "model_training".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        let signed = build_signed_claim_request(&cfg, &device, chrono::Utc::now()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&signed.body).unwrap();
        assert_eq!(
            parsed["schema_version"],
            "ironclaw.trace_upload_claim_request.v1"
        );
        assert_eq!(parsed["tenant_id"], "tenant-abc");
        assert_eq!(
            parsed["consent_scopes"],
            serde_json::json!(["debugging_evaluation", "model_training"])
        );
        let allowed_uses = parsed["allowed_uses"].as_array().unwrap();
        assert!(allowed_uses.contains(&serde_json::json!("model_training")));
        assert!(allowed_uses.contains(&serde_json::json!("aggregate_analytics")));
        // Subject is the sha256 form, never the raw user_subject.
        let subject = parsed["subject"].as_str().unwrap();
        assert!(subject.starts_with("sha256:"));
        assert_ne!(subject, "alice");
        // Signature verifies over the exact body string.
        let pk_bytes = base64::engine::general_purpose::STANDARD
            .decode(&device.public_key_b64)
            .unwrap();
        let pk = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pk_bytes);
        let sig = base64::engine::general_purpose::STANDARD
            .decode(&signed.signature_b64)
            .unwrap();
        pk.verify(signed.body.as_bytes(), &sig).unwrap();
    }
}
