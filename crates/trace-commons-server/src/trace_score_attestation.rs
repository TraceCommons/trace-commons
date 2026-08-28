// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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

/// Schema version stamped into every signed attestation.
///
/// v2 (breaking): every submission entry now carries `coverage`, stating how
/// much of the trace the scores were actually computed over. A gate decision
/// on a trace whose chunk count exceeded the per-trace cap is a judgment on a
/// prefix, and v1 had no way to say so — the signed document read as if the
/// whole trace had been scored. Verifiers that pin the version string will
/// reject v2 until they are updated; that is intended, because the meaning of
/// the document changed. v1 attestations are no longer issued.
pub const SCORE_ATTESTATION_SCHEMA_VERSION: &str = "trace_commons.score_attestation.v2";

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
    /// How much of the trace these scores were computed over (schema v2).
    pub coverage: ScoreAttestationCoverage,
}

/// Wire shape of `ScoreAttestationCoverage`. A separate struct (rather than a
/// serde-tagged enum) so `chunks_total` is emitted for a fully scored trace —
/// where it equals `chunks_scored` — and is ABSENT, never a sentinel, when the
/// denominator is genuinely unknown.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoverageWire {
    coverage_state: String,
    chunks_scored: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chunks_total: Option<u32>,
}

const COVERAGE_STATE_COMPLETE: &str = "complete";
const COVERAGE_STATE_PARTIAL: &str = "partial";
const COVERAGE_STATE_PARTIAL_UNKNOWN_TOTAL: &str = "partial_unknown_total";

/// How much of a trace a gate decision actually scored.
///
/// The gate chunks a large trace and scores at most `chunk_cap` chunks; the
/// remainder is dropped. A signed statement that a trace passed or failed is
/// therefore sometimes a statement about a prefix, and a collector deciding
/// what to pay for needs to know which.
///
/// Three states, deliberately distinct:
///  * `Complete` — every chunk was scored.
///  * `Partial` — the cap dropped chunks and the pre-cap total is known
///    (migration V47 persists it).
///  * `PartialUnknownTotal` — the cap dropped chunks on a decision recorded
///    before V47. The denominator was never stored and cannot be recovered.
///    It is reported as unknown and is NEVER estimated (e.g. from envelope
///    byte size): an estimate inside a signed statement is worse than an
///    honest unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "CoverageWire", try_from = "CoverageWire")]
pub enum ScoreAttestationCoverage {
    Complete {
        chunks_scored: u32,
    },
    Partial {
        chunks_scored: u32,
        chunks_total: u32,
    },
    PartialUnknownTotal {
        chunks_scored: u32,
    },
}

impl ScoreAttestationCoverage {
    /// Derive coverage from the three `trace_gate_decisions` columns.
    ///
    /// NULL semantics match migration V37's: `chunk_count` NULL reads as one
    /// chunk and `chunks_capped` NULL reads as false, so pre-chunking rows are
    /// fully scored single-chunk traces. `total_chunk_count` (V47) is NULL on
    /// every decision recorded before that migration.
    ///
    /// A stored total that is not strictly greater than what was scored cannot
    /// describe a capped trace, so it is reported as unknown rather than
    /// signing a coverage claim the data does not support.
    pub fn from_decision_columns(
        chunk_count: Option<i32>,
        total_chunk_count: Option<i32>,
        chunks_capped: Option<bool>,
    ) -> Self {
        let chunks_scored = chunk_count.filter(|c| *c > 0).unwrap_or(1) as u32;
        if !chunks_capped.unwrap_or(false) {
            return Self::Complete { chunks_scored };
        }
        match total_chunk_count {
            Some(total) if total > 0 && total as u32 > chunks_scored => Self::Partial {
                chunks_scored,
                chunks_total: total as u32,
            },
            _ => Self::PartialUnknownTotal { chunks_scored },
        }
    }
}

impl From<ScoreAttestationCoverage> for CoverageWire {
    fn from(value: ScoreAttestationCoverage) -> Self {
        match value {
            ScoreAttestationCoverage::Complete { chunks_scored } => CoverageWire {
                coverage_state: COVERAGE_STATE_COMPLETE.to_string(),
                chunks_scored,
                chunks_total: Some(chunks_scored),
            },
            ScoreAttestationCoverage::Partial {
                chunks_scored,
                chunks_total,
            } => CoverageWire {
                coverage_state: COVERAGE_STATE_PARTIAL.to_string(),
                chunks_scored,
                chunks_total: Some(chunks_total),
            },
            ScoreAttestationCoverage::PartialUnknownTotal { chunks_scored } => CoverageWire {
                coverage_state: COVERAGE_STATE_PARTIAL_UNKNOWN_TOTAL.to_string(),
                chunks_scored,
                chunks_total: None,
            },
        }
    }
}

impl TryFrom<CoverageWire> for ScoreAttestationCoverage {
    type Error = String;

    fn try_from(wire: CoverageWire) -> Result<Self, Self::Error> {
        match wire.coverage_state.as_str() {
            COVERAGE_STATE_COMPLETE => Ok(Self::Complete {
                chunks_scored: wire.chunks_scored,
            }),
            COVERAGE_STATE_PARTIAL => {
                let chunks_total = wire
                    .chunks_total
                    .ok_or_else(|| "partial coverage requires chunks_total".to_string())?;
                Ok(Self::Partial {
                    chunks_scored: wire.chunks_scored,
                    chunks_total,
                })
            }
            COVERAGE_STATE_PARTIAL_UNKNOWN_TOTAL => Ok(Self::PartialUnknownTotal {
                chunks_scored: wire.chunks_scored,
            }),
            other => Err(format!("unknown coverage_state: {other}")),
        }
    }
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
    /// Asked-for submissions this principal owns that have no gate decision
    /// yet. Present ONLY in a scoped response; an unscoped attestation omits
    /// the key entirely (see `ScoreAttestationScope`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Vec<Uuid>>,
    /// Asked-for submissions this principal does not own. Deliberately
    /// collapses "belongs to someone else" and "does not exist" into one
    /// bucket so the route cannot be used to probe for submission ids.
    /// Present ONLY in a scoped response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unknown: Option<Vec<Uuid>>,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub nonce: String,
}

/// The two extra statements a SCOPED attestation makes, over and above the
/// scored `submissions` an unscoped one carries.
///
/// This is an `Option` at the call site rather than two empty vectors
/// because "no pending submissions" and "I was not asked a scoped question"
/// are different claims, and only the second may omit the keys. Emitting
/// `pending: []` on an unscoped document would be a schema change to every
/// verifier pinned to `trace_commons.score_attestation.v2`.
#[derive(Debug, Clone, Default)]
pub struct ScoreAttestationScope {
    pub pending: Vec<Uuid>,
    pub unknown: Vec<Uuid>,
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
    sign_scoped_score_attestation(state, tenant_id, auth_principal_ref, submissions, None, now)
}

/// As `sign_score_attestation`, but for a request that named a specific set
/// of submissions. `scope` carries what the caller asked about and did not
/// get a score for; `None` reproduces the unscoped document byte for byte,
/// including the ABSENCE of the `pending` and `unknown` keys.
///
/// The same non-negotiable applies: `tenant_id` and `auth_principal_ref`
/// MUST come from the authenticated request context. A scoped request adds
/// a submission-id list to the wire, and nothing else — the id list is a
/// filter over what the authenticated principal already owns, never a way
/// to name a principal.
pub fn sign_scoped_score_attestation(
    state: &AttestationSigningState,
    tenant_id: &str,
    auth_principal_ref: &str,
    submissions: Vec<ScoreAttestationSubmissionEntry>,
    scope: Option<ScoreAttestationScope>,
    now: DateTime<Utc>,
) -> anyhow::Result<String> {
    let expires_at = now
        .checked_add_signed(Duration::seconds(state.ttl_seconds))
        .context("attestation ttl_seconds overflow")?;
    let (pending, unknown) = match scope {
        Some(scope) => (Some(scope.pending), Some(scope.unknown)),
        None => (None, None),
    };
    let claims = ScoreAttestationClaims {
        schema_version: SCORE_ATTESTATION_SCHEMA_VERSION.to_string(),
        tenant_id: tenant_id.to_string(),
        auth_principal_ref: auth_principal_ref.to_string(),
        submissions,
        pending,
        unknown,
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
    fn schema_version_is_v2() {
        assert_eq!(
            SCORE_ATTESTATION_SCHEMA_VERSION,
            "trace_commons.score_attestation.v2"
        );
    }

    #[test]
    fn coverage_from_columns_distinguishes_the_three_states() {
        // Fully scored: not capped, chunk_count known.
        assert_eq!(
            ScoreAttestationCoverage::from_decision_columns(Some(4), Some(4), Some(false)),
            ScoreAttestationCoverage::Complete { chunks_scored: 4 }
        );
        // Pre-chunking legacy row: NULL chunk_count reads as one chunk, NULL
        // chunks_capped reads as false.
        assert_eq!(
            ScoreAttestationCoverage::from_decision_columns(None, None, None),
            ScoreAttestationCoverage::Complete { chunks_scored: 1 }
        );
        // Capped with a persisted denominator.
        assert_eq!(
            ScoreAttestationCoverage::from_decision_columns(Some(16), Some(61), Some(true)),
            ScoreAttestationCoverage::Partial {
                chunks_scored: 16,
                chunks_total: 61
            }
        );
        // Capped before V47: the denominator was never stored and must NOT be
        // fabricated or estimated.
        assert_eq!(
            ScoreAttestationCoverage::from_decision_columns(Some(16), None, Some(true)),
            ScoreAttestationCoverage::PartialUnknownTotal { chunks_scored: 16 }
        );
        // Inconsistent stored total (not greater than what was scored) is
        // reported as unknown rather than as a coverage claim we cannot back.
        assert_eq!(
            ScoreAttestationCoverage::from_decision_columns(Some(16), Some(16), Some(true)),
            ScoreAttestationCoverage::PartialUnknownTotal { chunks_scored: 16 }
        );
    }

    #[test]
    fn coverage_states_serialize_distinguishably() {
        let complete =
            serde_json::to_value(ScoreAttestationCoverage::Complete { chunks_scored: 3 })
                .expect("serializes");
        assert_eq!(complete["coverage_state"], "complete");
        assert_eq!(complete["chunks_scored"], 3);
        assert_eq!(complete["chunks_total"], 3);

        let partial = serde_json::to_value(ScoreAttestationCoverage::Partial {
            chunks_scored: 16,
            chunks_total: 61,
        })
        .expect("serializes");
        assert_eq!(partial["coverage_state"], "partial");
        assert_eq!(partial["chunks_scored"], 16);
        assert_eq!(partial["chunks_total"], 61);

        let unknown = serde_json::to_value(ScoreAttestationCoverage::PartialUnknownTotal {
            chunks_scored: 16,
        })
        .expect("serializes");
        assert_eq!(unknown["coverage_state"], "partial_unknown_total");
        assert_eq!(unknown["chunks_scored"], 16);
        assert!(
            unknown.get("chunks_total").is_none(),
            "an unknown denominator must be absent, never a sentinel"
        );
    }

    #[test]
    fn signed_v2_attestation_round_trips_with_coverage() {
        let keypair = generate_test_keypair();
        let config = AttestationConfig {
            signing_private_key_pem: keypair.private_key_pem.clone(),
            signing_public_key_pem: keypair.public_key_pem.clone(),
            signing_kid: "attestation-key-1".to_string(),
            ttl_seconds: 3600,
        };
        let state = AttestationSigningState::build(&config).expect("builds");
        let full_id = Uuid::new_v4();
        let capped_id = Uuid::new_v4();
        let legacy_capped_id = Uuid::new_v4();
        let submissions = vec![
            ScoreAttestationSubmissionEntry {
                submission_id: full_id,
                credit_quality_micros: Some(750_000),
                perplexity_micros: 1_200_000,
                novelty_score_micros: 900_000,
                gate_passed: true,
                coverage: ScoreAttestationCoverage::Complete { chunks_scored: 3 },
            },
            ScoreAttestationSubmissionEntry {
                submission_id: capped_id,
                credit_quality_micros: None,
                perplexity_micros: 1_000_000,
                novelty_score_micros: 100_000,
                gate_passed: false,
                coverage: ScoreAttestationCoverage::Partial {
                    chunks_scored: 16,
                    chunks_total: 61,
                },
            },
            ScoreAttestationSubmissionEntry {
                submission_id: legacy_capped_id,
                credit_quality_micros: None,
                perplexity_micros: 1_000_000,
                novelty_score_micros: 100_000,
                gate_passed: false,
                coverage: ScoreAttestationCoverage::PartialUnknownTotal { chunks_scored: 16 },
            },
        ];
        let token =
            sign_score_attestation(&state, "tenant-a", "principal:abc", submissions, Utc::now())
                .expect("signs");
        let decoding_key =
            DecodingKey::from_ed_pem(keypair.public_key_pem.as_bytes()).expect("decoding key");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        let decoded =
            jsonwebtoken::decode::<ScoreAttestationClaims>(&token, &decoding_key, &validation)
                .expect("verifies");
        assert_eq!(
            decoded.claims.schema_version,
            "trace_commons.score_attestation.v2"
        );
        assert_eq!(
            decoded.claims.submissions[0].coverage,
            ScoreAttestationCoverage::Complete { chunks_scored: 3 }
        );
        assert_eq!(
            decoded.claims.submissions[1].coverage,
            ScoreAttestationCoverage::Partial {
                chunks_scored: 16,
                chunks_total: 61
            }
        );
        assert_eq!(
            decoded.claims.submissions[2].coverage,
            ScoreAttestationCoverage::PartialUnknownTotal { chunks_scored: 16 }
        );
        assert_ne!(
            decoded.claims.submissions[0].coverage, decoded.claims.submissions[2].coverage,
            "a fully scored trace must not be confusable with an unknown denominator"
        );
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
            coverage: ScoreAttestationCoverage::Complete { chunks_scored: 1 },
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
