//! Server-signed score attestations.
//!
//! See `docs/superpowers/specs/2026-07-29-score-attestation-design.md` for
//! the design rationale. Mirrors `trace_upload_claim_issuer`'s key/config
//! conventions rather than inventing a second one: the same PKCS#8 v2 /
//! SPKI PEM key material, the same `{ "keys": [...] }` keyset shape, and the
//! same `EncodingKey`/`DecodingKey` validation helpers (reused directly from
//! that module).
//!
//! Ingest gains a signing key here so it can attest, over its own database
//! read, that a specific `auth_principal_ref` (resolved ONLY from the
//! caller's authenticated upload claim — never from a request parameter)
//! owns a set of scored submissions. See
//! `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`'s
//! `score_attestation_handler` for the endpoint that uses this module; it is
//! the only caller of `sign_score_attestation` and it must never accept a
//! principal from the request.

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::trace_upload_claim_issuer::{
    optional_env, validate_eddsa_private_key_pem, validate_eddsa_public_key_pem,
};

pub const SCORE_ATTESTATION_SCHEMA_VERSION: &str = "trace_commons.score_attestation.v1";

pub const TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM_ENV: &str =
    "TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM";
pub const TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM_ENV: &str =
    "TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM";
pub const TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID_ENV: &str =
    "TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID";
/// Operator-configurable attestation TTL in seconds. Optional; defaults to
/// `DEFAULT_ATTESTATION_TTL_SECONDS` (24h) per spec.
pub const TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS_ENV: &str =
    "TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS";

const DEFAULT_ATTESTATION_TTL_SECONDS: i64 = 24 * 60 * 60;

/// Missing-control label returned (503) when the attestation signing key is
/// not configured. Per repo convention this is fail-closed: the endpoint
/// NEVER returns an unsigned document.
pub const ATTESTATION_SIGNING_KEY_UNCONFIGURED: &str = "attestation_signing_key_unconfigured";

/// Raw env-sourced attestation signing config. `from_env` returns `Ok(None)`
/// when none of the three required env vars are set (attestations disabled,
/// the endpoint fails closed at request time) and `Err` when the
/// configuration is present but malformed or partial — a partially
/// configured signer is an operator error, not a silent disable.
#[derive(Clone)]
pub struct AttestationConfig {
    pub signing_private_key_pem: String,
    pub signing_public_key_pem: String,
    pub signing_kid: String,
    pub ttl_seconds: i64,
}

impl AttestationConfig {
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let private_key_pem = optional_env(TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM_ENV)?;
        let public_key_pem = optional_env(TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM_ENV)?;
        let kid = optional_env(TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID_ENV)?;

        if private_key_pem.is_none() && public_key_pem.is_none() && kid.is_none() {
            return Ok(None);
        }

        let signing_private_key_pem = private_key_pem.ok_or_else(|| {
            anyhow::anyhow!(
                "{TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM_ENV} is required when attestation signing is configured"
            )
        })?;
        let signing_public_key_pem = public_key_pem.ok_or_else(|| {
            anyhow::anyhow!(
                "{TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM_ENV} is required when attestation signing is configured"
            )
        })?;
        let signing_kid = kid.ok_or_else(|| {
            anyhow::anyhow!(
                "{TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID_ENV} is required when attestation signing is configured"
            )
        })?;
        anyhow::ensure!(
            !signing_kid.trim().is_empty(),
            "{TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID_ENV} must not be blank"
        );

        let ttl_seconds = optional_env(TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS_ENV)?
            .map(|value| {
                value
                    .parse::<i64>()
                    .context("invalid TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS")
            })
            .transpose()?
            .unwrap_or(DEFAULT_ATTESTATION_TTL_SECONDS);
        anyhow::ensure!(
            ttl_seconds > 0,
            "TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS must be positive"
        );

        Ok(Some(Self {
            signing_private_key_pem,
            signing_public_key_pem,
            signing_kid: signing_kid.trim().to_string(),
            ttl_seconds,
        }))
    }
}

/// Validated, ready-to-sign attestation key state. Built once at startup via
/// `build`; a construction failure (malformed key material) fails ingest
/// startup rather than deferring the failure to the first request.
pub struct AttestationSigningState {
    signing_key: EncodingKey,
    kid: String,
    public_key_pem: String,
    ttl_seconds: i64,
}

impl AttestationSigningState {
    pub fn build(config: &AttestationConfig) -> anyhow::Result<Self> {
        let private_key_pem = validate_eddsa_private_key_pem(&config.signing_private_key_pem)?;
        let public_key_pem = validate_eddsa_public_key_pem(&config.signing_public_key_pem)?;
        let signing_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes())
            .context("invalid attestation EdDSA signing private key")?;
        Ok(Self {
            signing_key,
            kid: config.signing_kid.clone(),
            public_key_pem,
            ttl_seconds: config.ttl_seconds,
        })
    }

    /// Keyset publication shape, matching
    /// `trace_upload_claim_issuer::keyset_handler` exactly: `{ "keys": [{
    /// "kid", "public_key_pem" }] }`. An array (not a bare object) so
    /// rotation can publish the old and new key together; see the design
    /// spec's rotation procedure.
    pub fn keyset_json(&self) -> serde_json::Value {
        serde_json::json!({
            "keys": [{
                "kid": self.kid,
                "public_key_pem": self.public_key_pem,
            }]
        })
    }
}

/// One attested submission score, mirroring the wire shape of the existing
/// `/v1/admin/scores-by-submission` bundle (`SubmissionScoreBundle` in the
/// ingest binary) so a collector that already parses that shape needs no new
/// field mapping — just a signature to check first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreAttestationSubmissionEntry {
    pub submission_id: Uuid,
    pub credit_quality_micros: Option<i64>,
    pub perplexity_micros: i64,
    pub novelty_score_micros: i64,
    pub gate_passed: bool,
}

/// The signed statement's claims, in the field order the design spec's JSON
/// example uses. `auth_principal_ref` is a reference, not raw contributor
/// identity (see spec "What is attested"); it lets a collector detect two
/// participants relaying attestations for the same contributor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreAttestationClaims {
    pub schema_version: String,
    pub tenant_id: String,
    pub auth_principal_ref: String,
    pub submissions: Vec<ScoreAttestationSubmissionEntry>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub nonce: String,
}

/// Sign a score attestation for `(tenant_id, auth_principal_ref)` as of
/// `now`. Both identity fields MUST be resolved by the caller from the
/// authenticated request context only — this function has no way to enforce
/// that itself, so the non-negotiable is enforced at the call site (the
/// ingest handler never deserializes a principal from the request body or
/// query string; see that handler's doc comment).
///
/// `expires_at` is always `now + ttl_seconds` — mandatory on every
/// attestation per the design spec; there is no code path that omits it.
pub fn sign_score_attestation(
    state: &AttestationSigningState,
    tenant_id: &str,
    auth_principal_ref: &str,
    submissions: Vec<ScoreAttestationSubmissionEntry>,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    let expires_at = now
        .checked_add_signed(Duration::seconds(state.ttl_seconds))
        .context("attestation ttl_seconds overflow")?;
    let claims = ScoreAttestationClaims {
        schema_version: SCORE_ATTESTATION_SCHEMA_VERSION.to_string(),
        tenant_id: tenant_id.to_string(),
        auth_principal_ref: auth_principal_ref.to_string(),
        submissions,
        issued_at: now,
        expires_at,
        nonce: Uuid::new_v4().to_string(),
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(state.kid.clone());
    jsonwebtoken::encode(&header, &claims, &state.signing_key)
        .context("failed to sign score attestation")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation};
    use std::sync::Mutex;

    // Serialize env-mutating tests: `AttestationConfig::from_env` reads
    // process-global env vars, so concurrent tests stepping on the same
    // names would flake.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn clear_env() {
        for name in [
            TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM_ENV,
            TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM_ENV,
            TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID_ENV,
            TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS_ENV,
        ] {
            unsafe {
                std::env::remove_var(name);
            }
        }
    }

    fn generate_test_keypair() -> crate::trace_upload_claim_issuer::GeneratedUploadClaimKeypair {
        crate::trace_upload_claim_issuer::generate_upload_claim_keypair()
            .expect("keypair generation")
    }

    #[test]
    fn from_env_returns_none_when_fully_unconfigured() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let config = AttestationConfig::from_env().expect("from_env does not error");
        assert!(config.is_none());
        clear_env();
    }

    #[test]
    fn from_env_errors_on_partial_configuration() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let keypair = generate_test_keypair();
        unsafe {
            std::env::set_var(
                TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM_ENV,
                &keypair.private_key_pem,
            );
        }
        // public key + kid deliberately left unset.
        let result = AttestationConfig::from_env();
        assert!(
            result.is_err(),
            "a signing key with no public key/kid must fail loudly, not silently disable"
        );
        clear_env();
    }

    #[test]
    fn build_and_sign_round_trips_and_verifies() {
        let _guard = ENV_LOCK.lock().unwrap();
        clear_env();
        let keypair = generate_test_keypair();
        let config = AttestationConfig {
            signing_private_key_pem: keypair.private_key_pem.clone(),
            signing_public_key_pem: keypair.public_key_pem.clone(),
            signing_kid: "attestation-key-1".to_string(),
            ttl_seconds: 3600,
        };
        let state = AttestationSigningState::build(&config).expect("builds");

        let submission_id = Uuid::new_v4();
        let submissions = vec![ScoreAttestationSubmissionEntry {
            submission_id,
            credit_quality_micros: Some(750_000),
            perplexity_micros: 1_200_000,
            novelty_score_micros: 900_000,
            gate_passed: true,
        }];
        let now = Utc::now();
        let token = sign_score_attestation(&state, "tenant-a", "principal:abc", submissions, now)
            .expect("signs");

        // The header carries the configured kid so a collector can select
        // the right verifying key during rotation.
        let header = jsonwebtoken::decode_header(&token).expect("decodes header");
        assert_eq!(header.kid.as_deref(), Some("attestation-key-1"));
        assert_eq!(header.alg, Algorithm::EdDSA);

        let decoding_key =
            DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes()).expect("decoding key");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let decoded =
            jsonwebtoken::decode::<ScoreAttestationClaims>(&token, &decoding_key, &validation)
                .expect("verifies against the published public key");

        assert_eq!(
            decoded.claims.schema_version,
            SCORE_ATTESTATION_SCHEMA_VERSION
        );
        assert_eq!(decoded.claims.tenant_id, "tenant-a");
        assert_eq!(decoded.claims.auth_principal_ref, "principal:abc");
        assert_eq!(decoded.claims.submissions.len(), 1);
        assert_eq!(decoded.claims.submissions[0].submission_id, submission_id);
        assert_eq!(decoded.claims.issued_at, now);
        assert_eq!(
            decoded.claims.expires_at,
            now + Duration::seconds(3600),
            "expires_at must always be present and derived from ttl_seconds"
        );
        assert!(!decoded.claims.nonce.is_empty());

        let keyset = state.keyset_json();
        assert_eq!(keyset["keys"][0]["kid"], "attestation-key-1");
        assert_eq!(keyset["keys"][0]["public_key_pem"], keypair.public_key_pem);
    }

    #[test]
    fn signature_does_not_verify_against_a_different_key() {
        let other = generate_test_keypair();
        let signing = generate_test_keypair();
        let config = AttestationConfig {
            signing_private_key_pem: signing.private_key_pem,
            signing_public_key_pem: signing.public_key_pem,
            signing_kid: "k1".to_string(),
            ttl_seconds: 60,
        };
        let state = AttestationSigningState::build(&config).expect("builds");
        let token =
            sign_score_attestation(&state, "tenant-a", "principal:abc", Vec::new(), Utc::now())
                .expect("signs");

        let wrong_decoding_key =
            DecodingKey::from_ed_pem(other.public_key_pem.as_bytes()).expect("decoding key");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let result = jsonwebtoken::decode::<ScoreAttestationClaims>(
            &token,
            &wrong_decoding_key,
            &validation,
        );
        assert!(
            result.is_err(),
            "an attestation must not verify against a keyset entry it was not signed with"
        );
    }
}
