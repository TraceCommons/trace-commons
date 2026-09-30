// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Package trust and promotion qualification.
//!
//! Qualification evidence is deliberately separate from pipeline history.
//! The ingest database stores only the immutable fact that a package passed
//! qualification. Detailed reports and drill output stay in operator-owned
//! evidence storage.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::BundlePackage;
use uuid::Uuid;

use crate::versioned_pipeline::sha256_prefixed;

pub const PIPELINE_CHECK_RESULT_SCHEMA: &str = "trace_commons.pipeline_check_result.v1";
const PIPELINE_CHECK_RESULT_DIR_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR";
const PIPELINE_CHECK_RUN_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID";
const PIPELINE_CHECK_CODE_REVISION_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH";

pub const PACKAGE_SIGNATURE_ALGORITHM: &str = "Ed25519";
pub const PACKAGE_SIGNATURE_INVALID_LABEL: &str = "bundle_package_signature_invalid";
pub const PACKAGE_SIGNER_UNTRUSTED_LABEL: &str = "bundle_package_signer_untrusted";
pub const QUALIFICATION_EVIDENCE_STALE_LABEL: &str = "qualification_evidence_stale";
pub const QUALIFICATION_EVIDENCE_MISSING_LABEL: &str = "qualification_evidence_missing";
pub const QUALIFICATION_EVIDENCE_FAILED_LABEL: &str = "qualification_evidence_failed";

pub const PROMOTION_REQUIRED_CHECKS: &[&str] = &[
    "pipeline_storage_upgrade_rls",
    "pipeline_http_corpus_minimal",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
    "pipeline_crash_matrix",
    "pipeline_http_restart_recovery",
    "pipeline_independent_instruments",
    "pipeline_stale_lease_fence",
    "pipeline_lease_renewal",
    "pipeline_receipt_replay_exact",
    "pipeline_http_receipt_ownership",
    "pipeline_payout_recovery",
    "pipeline_index_rebuild",
    "pipeline_orphan_sweep",
    "pipeline_bundle_qualification",
    "pipeline_restore_drill",
    // Promotion only: no local or CI run passes these.
    "pipeline_production_adapters",
    "pipeline_remote_restore",
    "pipeline_hf_network_canary",
];

pub(crate) fn is_safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A looser bound than [`is_safe_label`]: key identifiers accept the
/// characters a bundle signing key id or key rotation label commonly uses,
/// not only the label alphabet operational output is restricted to.
fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

pub fn evidence_hash(observed: &serde_json::Value) -> Result<String, String> {
    fn check(value: &serde_json::Value) -> Result<(), String> {
        match value {
            serde_json::Value::Number(n) if n.is_f64() => Err("evidence_value_invalid".into()),
            serde_json::Value::Array(items) => items.iter().try_for_each(check),
            serde_json::Value::Object(map) => map.values().try_for_each(check),
            _ => Ok(()),
        }
    }
    check(observed)?;
    // This build has serde_json/preserve_order on (dcap-qvl), so a plain
    // to_vec keeps insertion order. to_canonical_vec sorts every object's
    // keys: the same compact bytes as Python's canonical form.
    let bytes = trace_commons_protocol::canonical_json::to_canonical_vec(observed)
        .map_err(|_| "evidence_value_invalid".to_string())?;
    Ok(sha256_prefixed(&bytes))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipelineCheckStatus {
    Pass,
    Fail,
    Blocked,
}

/// A single qualification check's result, written to operator-owned evidence
/// storage by [`PipelineCheckEmitter`]. Hash-only: no field here may carry a
/// raw tenant id, URL, token, or trace body (repo convention).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineCheckResult {
    pub schema: String,
    pub run_id: String,
    pub check_id: String,
    pub status: PipelineCheckStatus,
    pub code_revision_hash: String,
    pub package_hash: Option<String>,
    pub configuration_digest: Option<String>,
    pub dependency_digest: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub evidence_hash: String,
    pub safe_blockers: Vec<String>,
}

impl PipelineCheckResult {
    pub fn validate(&self) -> Result<(), String> {
        let optional_digests = [
            &self.package_hash,
            &self.configuration_digest,
            &self.dependency_digest,
        ];
        if self.schema != PIPELINE_CHECK_RESULT_SCHEMA
            || !is_safe_label(&self.run_id)
            || !is_safe_label(&self.check_id)
            || !is_sha256(&self.code_revision_hash)
            || !is_sha256(&self.evidence_hash)
            || optional_digests
                .into_iter()
                .flatten()
                .any(|digest| !is_sha256(digest))
            || self.safe_blockers.iter().any(|label| !is_safe_label(label))
        {
            return Err("pipeline_check_result_invalid".to_string());
        }
        Ok(())
    }
}

/// The three digests [`package_digests`] derives from a [`BundlePackage`]'s
/// manifest: the package's own content hash, a hash over every phase's
/// configuration hash, and a hash over the Score phase's sorted data
/// artifact hashes.
pub struct PipelinePackageDigests {
    pub package_hash: String,
    pub configuration_digest: String,
    pub dependency_digest: String,
}

pub fn package_digests(package: &BundlePackage) -> Result<PipelinePackageDigests, String> {
    let package_hash = package
        .package_hash()
        .map_err(|_| "pipeline_package_invalid".to_string())?;
    let configuration_digest = evidence_hash(&serde_json::json!({
        "admission": package.manifest.admission.configuration_hash,
        "review": package.manifest.review.configuration_hash,
        "score": package.manifest.score.configuration_hash,
        "settle": package.manifest.settle.configuration_hash,
    }))?;
    let mut data_artifact_hashes = package.manifest.score.data_artifact_hashes.clone();
    data_artifact_hashes.sort();
    let dependency_digest = evidence_hash(&serde_json::json!(data_artifact_hashes))?;
    Ok(PipelinePackageDigests {
        package_hash,
        configuration_digest,
        dependency_digest,
    })
}

/// Writes [`PipelineCheckResult`] evidence to operator-owned storage.
/// `pipeline.py` points this at a run-scoped directory through the
/// `TRACE_COMMONS_PIPELINE_CHECK_*` environment variables; a test builds one
/// directly with [`PipelineCheckEmitter::new`] instead of touching process
/// environment (tests run in parallel and share the process environment).
#[doc(hidden)]
#[derive(Debug)]
pub struct PipelineCheckEmitter {
    dir: PathBuf,
    run_id: String,
    code_revision_hash: String,
}

impl PipelineCheckEmitter {
    pub fn new(dir: PathBuf, run_id: &str, code_revision_hash: &str) -> Result<Self, String> {
        if !is_safe_label(run_id) || !is_sha256(code_revision_hash) {
            return Err("pipeline_check_environment_invalid".to_string());
        }
        Ok(Self {
            dir,
            run_id: run_id.to_string(),
            code_revision_hash: code_revision_hash.to_string(),
        })
    }

    /// `Ok(None)` when none of the three values are set; `Err` when only
    /// some are set (a misconfigured environment) or a set value is
    /// malformed.
    pub fn from_vars(
        dir: Option<String>,
        run_id: Option<String>,
        code_revision_hash: Option<String>,
    ) -> Result<Option<Self>, String> {
        match (dir, run_id, code_revision_hash) {
            (None, None, None) => Ok(None),
            (Some(dir), Some(run_id), Some(code_revision_hash)) => {
                Self::new(PathBuf::from(dir), &run_id, &code_revision_hash).map(Some)
            }
            _ => Err("pipeline_check_environment_incomplete".to_string()),
        }
    }

    pub fn from_env() -> Result<Option<Self>, String> {
        Self::from_vars(
            std::env::var(PIPELINE_CHECK_RESULT_DIR_VAR).ok(),
            std::env::var(PIPELINE_CHECK_RUN_ID_VAR).ok(),
            std::env::var(PIPELINE_CHECK_CODE_REVISION_VAR).ok(),
        )
    }

    /// Writes `<check_id>.evidence.json` (the raw `observed` value) and
    /// `<check_id>.result.json` (the signed-off [`PipelineCheckResult`]),
    /// refusing a check id this emitter already wrote. The result file is
    /// the commit point: it is opened `create_new` on the final path first,
    /// so a repeat emit is refused before the evidence file is touched. The
    /// evidence file may already exist from an emit that crashed after
    /// writing it but before writing the result file; this call overwrites
    /// it in that case, since the run never committed to it.
    pub fn emit(
        &self,
        check_id: &str,
        status: PipelineCheckStatus,
        package: Option<&BundlePackage>,
        safe_blockers: &[&str],
        observed: serde_json::Value,
    ) -> Result<(), String> {
        if !is_safe_label(check_id) {
            return Err("pipeline_check_id_invalid".to_string());
        }
        let evidence_digest = evidence_hash(&observed)?;
        let digests = package.map(package_digests).transpose()?;
        let result = PipelineCheckResult {
            schema: PIPELINE_CHECK_RESULT_SCHEMA.to_string(),
            run_id: self.run_id.clone(),
            check_id: check_id.to_string(),
            status,
            code_revision_hash: self.code_revision_hash.clone(),
            package_hash: digests.as_ref().map(|d| d.package_hash.clone()),
            configuration_digest: digests.as_ref().map(|d| d.configuration_digest.clone()),
            dependency_digest: digests.as_ref().map(|d| d.dependency_digest.clone()),
            observed_at: Utc::now(),
            evidence_hash: evidence_digest,
            safe_blockers: safe_blockers
                .iter()
                .map(|label| (*label).to_string())
                .collect(),
        };
        result.validate()?;

        let result_path = self.dir.join(format!("{check_id}.result.json"));
        let evidence_path = self.dir.join(format!("{check_id}.evidence.json"));

        // Reserve (and refuse a repeat of) the result path before writing
        // anything: a second emit for the same check id must leave the
        // first emit's files exactly as they were.
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&result_path)
            .map_err(|_| "pipeline_check_already_emitted".to_string())?;

        let evidence_bytes = serde_json::to_vec(&observed)
            .map_err(|_| "pipeline_check_evidence_invalid".to_string())?;
        write_atomic(&self.dir, &evidence_path, &evidence_bytes)?;

        let result_bytes =
            serde_json::to_vec(&result).map_err(|_| "pipeline_check_result_invalid".to_string())?;
        write_atomic(&self.dir, &result_path, &result_bytes)?;
        Ok(())
    }

    /// No-op when the environment names no result directory. Panics on a
    /// partial or malformed environment (a misspelled variable must never
    /// pass silently) and on an `emit` failure.
    pub fn emit_from_env(
        check_id: &str,
        status: PipelineCheckStatus,
        package: Option<&BundlePackage>,
        safe_blockers: &[&str],
        observed: serde_json::Value,
    ) {
        match Self::from_env() {
            Ok(None) => {}
            Ok(Some(emitter)) => {
                if let Err(error) = emitter.emit(check_id, status, package, safe_blockers, observed)
                {
                    panic!("pipeline check emit failed: {error}");
                }
            }
            Err(error) => panic!("pipeline check environment invalid: {error}"),
        }
    }

    pub fn emit_pass_from_env(
        check_id: &str,
        package: Option<&BundlePackage>,
        observed: serde_json::Value,
    ) {
        Self::emit_from_env(check_id, PipelineCheckStatus::Pass, package, &[], observed)
    }
}

/// Writes `bytes` to a fresh temporary name in `dir`, then renames it onto
/// `path`, replacing any existing file there.
fn write_atomic(dir: &Path, path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp_path = dir.join(format!(".{}.tmp", Uuid::new_v4()));
    std::fs::write(&temp_path, bytes).map_err(|_| "pipeline_check_write_failed".to_string())?;
    std::fs::rename(&temp_path, path).map_err(|_| "pipeline_check_write_failed".to_string())
}

/// One promotion input: a check result plus how long its evidence stays
/// current.
#[derive(Debug, Clone)]
pub struct DrillEvidence {
    pub check: PipelineCheckResult,
    pub maximum_age_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionDecision {
    pub ready: bool,
    pub evaluated_at: DateTime<Utc>,
    pub evidence_hash: String,
    pub safe_blockers: Vec<String>,
}

pub fn evaluate_promotion(
    evidence: &[DrillEvidence],
    now: DateTime<Utc>,
) -> Result<PromotionDecision, String> {
    let mut by_id = BTreeMap::new();
    for item in evidence {
        let check_id = item.check.check_id.as_str();
        if !PROMOTION_REQUIRED_CHECKS.contains(&check_id)
            || !is_sha256(&item.check.evidence_hash)
            || item.maximum_age_seconds == 0
            || item
                .check
                .safe_blockers
                .iter()
                .any(|label| !is_safe_label(label))
        {
            return Err("qualification_evidence_invalid".to_string());
        }
        if by_id.insert(check_id, item).is_some() {
            return Err("qualification_evidence_duplicate".to_string());
        }
    }
    let mut blockers = Vec::new();
    for check_id in PROMOTION_REQUIRED_CHECKS {
        let Some(item) = by_id.get(check_id) else {
            blockers.push(format!("{QUALIFICATION_EVIDENCE_MISSING_LABEL}:{check_id}"));
            continue;
        };
        if item.check.status != PipelineCheckStatus::Pass {
            blockers.push(format!("{QUALIFICATION_EVIDENCE_FAILED_LABEL}:{check_id}"));
        }
        let maximum_age = i64::try_from(item.maximum_age_seconds)
            .map_err(|_| "qualification_evidence_invalid".to_string())?;
        if item.check.observed_at > now
            || now - item.check.observed_at > Duration::seconds(maximum_age)
        {
            blockers.push(format!("{QUALIFICATION_EVIDENCE_STALE_LABEL}:{check_id}"));
        }
    }
    blockers.sort();
    let checks = evidence
        .iter()
        .map(|item| {
            (
                item.check.check_id.clone(),
                item.check.evidence_hash.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let canonical = serde_json::json!({
        "schema": "trace_commons.pipeline_promotion.v1",
        "evaluated_at": now.timestamp(),
        "blockers": blockers.clone(),
        "checks": checks,
    });
    let evidence_digest =
        evidence_hash(&canonical).map_err(|_| "qualification_evidence_invalid".to_string())?;
    Ok(PromotionDecision {
        ready: blockers.is_empty(),
        evaluated_at: now,
        evidence_hash: evidence_digest,
        safe_blockers: blockers,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundlePackageSignature {
    pub algorithm: String,
    pub key_id: String,
    pub package_hash: String,
    pub signature_base64url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedBundlePackage {
    pub package: BundlePackage,
    pub signature: BundlePackageSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedBundleKey {
    pub key_id: String,
    pub public_key_base64url: String,
}

#[derive(Debug, Clone, Default)]
pub struct BundlePackageTrustStore {
    keys: BTreeMap<String, Vec<u8>>,
}

impl BundlePackageTrustStore {
    pub fn new(keys: impl IntoIterator<Item = TrustedBundleKey>) -> Result<Self, String> {
        let mut trusted = BTreeMap::new();
        for key in keys {
            if !is_safe_identifier(&key.key_id) {
                return Err(PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string());
            }
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(key.public_key_base64url)
                .map_err(|_| PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string())?;
            if bytes.len() != 32 || trusted.insert(key.key_id, bytes).is_some() {
                return Err(PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string());
            }
        }
        Ok(Self { keys: trusted })
    }

    pub fn verify(&self, signed: &SignedBundlePackage) -> Result<(), String> {
        signed
            .package
            .validate()
            .map_err(|_| PACKAGE_SIGNATURE_INVALID_LABEL.to_string())?;
        if signed.signature.algorithm != PACKAGE_SIGNATURE_ALGORITHM
            || !is_safe_identifier(&signed.signature.key_id)
        {
            return Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string());
        }
        let package_hash = signed
            .package
            .package_hash()
            .map_err(|_| PACKAGE_SIGNATURE_INVALID_LABEL.to_string())?;
        if signed.signature.package_hash != package_hash {
            return Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string());
        }
        let public_key = self
            .keys
            .get(&signed.signature.key_id)
            .ok_or_else(|| PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string())?;
        let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&signed.signature.signature_base64url)
            .map_err(|_| PACKAGE_SIGNATURE_INVALID_LABEL.to_string())?;
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(package_hash.as_bytes(), &signature)
            .map_err(|_| PACKAGE_SIGNATURE_INVALID_LABEL.to_string())
    }
}

/// Signs `package`'s canonical package hash with a PKCS#8-encoded Ed25519
/// private key, producing the signature a [`BundlePackageTrustStore`] built
/// from the matching [`trusted_key_for_pkcs8`] verifies.
pub fn sign_bundle_package(
    package: BundlePackage,
    key_id: &str,
    pkcs8: &[u8],
) -> anyhow::Result<SignedBundlePackage> {
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|_| anyhow::anyhow!("bundle_package_signing_key_invalid"))?;
    let package_hash = package.package_hash()?;
    let signature = key_pair.sign(package_hash.as_bytes());
    Ok(SignedBundlePackage {
        package,
        signature: BundlePackageSignature {
            algorithm: PACKAGE_SIGNATURE_ALGORITHM.to_string(),
            key_id: key_id.to_string(),
            package_hash,
            signature_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(signature.as_ref()),
        },
    })
}

/// The [`TrustedBundleKey`] a [`BundlePackageTrustStore`] needs to verify
/// packages signed by `pkcs8` via [`sign_bundle_package`].
pub fn trusted_key_for_pkcs8(key_id: &str, pkcs8: &[u8]) -> anyhow::Result<TrustedBundleKey> {
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|_| anyhow::anyhow!("bundle_package_signing_key_invalid"))?;
    Ok(TrustedBundleKey {
        key_id: key_id.to_string(),
        public_key_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(key_pair.public_key().as_ref()),
    })
}

#[cfg(test)]
mod tests {
    use ring::signature::Ed25519KeyPair;
    use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentDescriptor, InstrumentKind};
    use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};

    use super::*;
    use crate::versioned_pipeline_bundle::{
        MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
    };

    /// The same off-chain credit-account descriptor the PR 2 HTTP test
    /// assembler pins for its minimal bundle (`storage_rebate_descriptor` in
    /// `pipeline_http_pg_tests.rs`).
    fn storage_rebate_descriptor() -> InstrumentDescriptor {
        InstrumentDescriptor {
            kind: InstrumentKind::CreditAccount,
            network: "pipeline-test".to_string(),
            contract: "storage-rebate".to_string(),
            decimals: 0,
        }
    }

    fn minimal_test_package_with_variant(variant: Option<&str>) -> BundlePackage {
        MinimalPolicyBundle::minimal_package(
            &PipelineBundleConfig {
                instrument_awards: vec![PipelineInstrumentAwardConfig {
                    instrument_id: "storage_rebate".into(),
                    atomic_units: AtomicUnits::from_raw(5),
                    descriptor: storage_rebate_descriptor(),
                }],
                include_index: true,
                variant: variant.map(str::to_string),
            },
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        )
        .expect("minimal package builds")
    }

    fn minimal_test_package() -> BundlePackage {
        minimal_test_package_with_variant(None)
    }

    fn sample_check_result() -> PipelineCheckResult {
        PipelineCheckResult {
            schema: PIPELINE_CHECK_RESULT_SCHEMA.to_string(),
            run_id: "q0123abcd".to_string(),
            check_id: "pipeline_crash_matrix".to_string(),
            status: PipelineCheckStatus::Pass,
            code_revision_hash: sha256_prefixed(b"code-revision"),
            package_hash: Some(sha256_prefixed(b"package")),
            configuration_digest: Some(sha256_prefixed(b"configuration")),
            dependency_digest: Some(sha256_prefixed(b"dependency")),
            observed_at: Utc::now(),
            evidence_hash: sha256_prefixed(b"evidence"),
            safe_blockers: vec!["pipeline_check_blocked".to_string()],
        }
    }

    #[test]
    fn check_result_round_trips_and_refuses_unknown_fields() {
        let result = sample_check_result();
        result.validate().expect("sample result is valid");

        let json = serde_json::to_value(&result).expect("result serializes");
        let round_tripped: PipelineCheckResult =
            serde_json::from_value(json.clone()).expect("result deserializes");
        assert_eq!(result, round_tripped);

        let mut with_extra = json.clone();
        with_extra
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<PipelineCheckResult>(with_extra).is_err());

        let mut skipped = json.clone();
        skipped["status"] = serde_json::json!("skipped");
        assert!(serde_json::from_value::<PipelineCheckResult>(skipped).is_err());

        let mut bad_check_id = result.clone();
        bad_check_id.check_id = "pipeline.settle.recovery".to_string();
        assert!(bad_check_id.validate().is_err());

        let mut bad_run_id = result.clone();
        bad_run_id.run_id = "a".repeat(65);
        assert!(bad_run_id.validate().is_err());

        let mut bad_evidence_hash = result.clone();
        bad_evidence_hash.evidence_hash = "not-sha256".to_string();
        assert!(bad_evidence_hash.validate().is_err());

        let mut bad_package_hash = result.clone();
        bad_package_hash.package_hash = Some(format!("sha256:{}", "a".repeat(63)));
        assert!(bad_package_hash.validate().is_err());

        let mut bad_blocker = result.clone();
        bad_blocker.safe_blockers = vec!["Bad Label".to_string()];
        assert!(bad_blocker.validate().is_err());
    }

    #[test]
    fn package_digests_follow_the_manifest() {
        let package = minimal_test_package();
        let digests = package_digests(&package).expect("digests compute");
        assert_eq!(digests.package_hash, package.package_hash().unwrap());

        let expected_configuration_digest = evidence_hash(&serde_json::json!({
            "admission": package.manifest.admission.configuration_hash,
            "review": package.manifest.review.configuration_hash,
            "score": package.manifest.score.configuration_hash,
            "settle": package.manifest.settle.configuration_hash,
        }))
        .unwrap();
        assert_eq!(digests.configuration_digest, expected_configuration_digest);

        let mut sorted_hashes = package.manifest.score.data_artifact_hashes.clone();
        sorted_hashes.sort();
        let expected_dependency_digest = evidence_hash(&serde_json::json!(sorted_hashes)).unwrap();
        assert_eq!(digests.dependency_digest, expected_dependency_digest);

        let changed = minimal_test_package_with_variant(Some("variant-b"));
        let changed_digests = package_digests(&changed).expect("changed digests compute");
        assert_ne!(
            changed_digests.configuration_digest,
            digests.configuration_digest
        );
    }

    #[test]
    fn evidence_hash_is_canonical_json() {
        let value = serde_json::json!({
            "b": 1,
            "a": "x",
            "c": [true, null, format!("sha256:{}", "0".repeat(64))],
        });
        assert_eq!(
            evidence_hash(&value).unwrap(),
            "sha256:5c4967af7bf59c091e1b5d2c1a70e9d927f642ef996d1d3dc154221175a1ef6b"
        );

        let inserted_b_first = serde_json::json!({"b": 1, "a": "x"});
        let inserted_a_first = serde_json::json!({"a": "x", "b": 1});
        assert_eq!(
            evidence_hash(&inserted_b_first).unwrap(),
            evidence_hash(&inserted_a_first).unwrap()
        );

        let float_value = serde_json::json!({"x": 1.5});
        assert_eq!(
            evidence_hash(&float_value),
            Err("evidence_value_invalid".to_string())
        );
    }

    #[test]
    fn emitter_writes_result_and_evidence_once() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let code_hash = sha256_prefixed(b"code-revision");
        let emitter = PipelineCheckEmitter::new(dir.path().to_path_buf(), "q0123abcd", &code_hash)
            .expect("emitter builds");
        let package = minimal_test_package();

        emitter
            .emit(
                "pipeline_crash_matrix",
                PipelineCheckStatus::Pass,
                Some(&package),
                &[],
                serde_json::json!({"runs": 3}),
            )
            .expect("first emit succeeds");

        let result_path = dir.path().join("pipeline_crash_matrix.result.json");
        let evidence_path = dir.path().join("pipeline_crash_matrix.evidence.json");
        assert!(result_path.exists());
        assert!(evidence_path.exists());

        let result: PipelineCheckResult =
            serde_json::from_slice(&std::fs::read(&result_path).unwrap()).expect("result parses");
        result.validate().expect("emitted result is valid");
        assert_eq!(result.run_id, "q0123abcd");
        assert_eq!(result.code_revision_hash, code_hash);
        let expected_digests = package_digests(&package).unwrap();
        assert_eq!(result.package_hash, Some(expected_digests.package_hash));
        assert_eq!(
            result.configuration_digest,
            Some(expected_digests.configuration_digest)
        );
        assert_eq!(
            result.dependency_digest,
            Some(expected_digests.dependency_digest)
        );

        let evidence: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&evidence_path).unwrap())
                .expect("evidence parses");
        assert_eq!(result.evidence_hash, evidence_hash(&evidence).unwrap());

        let repeat = emitter.emit(
            "pipeline_crash_matrix",
            PipelineCheckStatus::Pass,
            Some(&package),
            &[],
            serde_json::json!({"runs": 4}),
        );
        assert_eq!(repeat, Err("pipeline_check_already_emitted".to_string()));

        let unchanged_result: PipelineCheckResult =
            serde_json::from_slice(&std::fs::read(&result_path).unwrap()).unwrap();
        assert_eq!(unchanged_result, result);
        let unchanged_evidence: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&evidence_path).unwrap()).unwrap();
        assert_eq!(unchanged_evidence, evidence);
    }

    #[test]
    fn emitter_from_vars_requires_all_three_values() {
        assert!(
            PipelineCheckEmitter::from_vars(None, None, None)
                .unwrap()
                .is_none()
        );

        let dir = tempfile::TempDir::new().expect("temp dir");
        let dir_str = dir.path().to_string_lossy().to_string();
        let code_hash = sha256_prefixed(b"code-revision");

        let all_set = PipelineCheckEmitter::from_vars(
            Some(dir_str.clone()),
            Some("q0123abcd".to_string()),
            Some(code_hash.clone()),
        );
        assert!(matches!(all_set, Ok(Some(_))));

        assert_eq!(
            PipelineCheckEmitter::from_vars(Some(dir_str.clone()), None, None).unwrap_err(),
            "pipeline_check_environment_incomplete"
        );
        assert_eq!(
            PipelineCheckEmitter::from_vars(
                Some(dir_str.clone()),
                Some("q0123abcd".to_string()),
                None
            )
            .unwrap_err(),
            "pipeline_check_environment_incomplete"
        );
        assert_eq!(
            PipelineCheckEmitter::from_vars(
                None,
                Some("q0123abcd".to_string()),
                Some(code_hash.clone())
            )
            .unwrap_err(),
            "pipeline_check_environment_incomplete"
        );

        assert_eq!(
            PipelineCheckEmitter::from_vars(
                Some(dir_str),
                Some("Not A Label".to_string()),
                Some(code_hash)
            )
            .unwrap_err(),
            "pipeline_check_environment_invalid"
        );
    }

    fn signed_test_package() -> (SignedBundlePackage, BundlePackageTrustStore) {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let package = minimal_test_package();
        let signed =
            sign_bundle_package(package, "release-2026-09", pkcs8.as_ref()).expect("package signs");
        let trust = BundlePackageTrustStore::new([trusted_key_for_pkcs8(
            "release-2026-09",
            pkcs8.as_ref(),
        )
        .expect("trusted key builds")])
        .unwrap();
        (signed, trust)
    }

    #[test]
    fn package_signature_binds_canonical_package_and_trusted_key() {
        let (mut signed, trust) = signed_test_package();
        trust.verify(&signed).unwrap();
        signed.signature.package_hash = sha256_prefixed(b"different");
        assert_eq!(
            trust.verify(&signed),
            Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string())
        );
    }

    #[test]
    fn package_trust_rejects_artifact_tampering_unknown_keys_and_algorithms() {
        let (signed, trust) = signed_test_package();

        let mut tampered = signed.clone();
        tampered
            .package
            .artifacts
            .values_mut()
            .next()
            .expect("package has policy artifacts")
            .push(0);
        assert_eq!(
            trust.verify(&tampered),
            Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string())
        );

        let empty_trust =
            BundlePackageTrustStore::new(std::iter::empty::<TrustedBundleKey>()).unwrap();
        assert_eq!(
            empty_trust.verify(&signed),
            Err(PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string())
        );

        let mut wrong_algorithm = signed;
        wrong_algorithm.signature.algorithm = "HS256".to_string();
        assert_eq!(
            trust.verify(&wrong_algorithm),
            Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string())
        );
    }

    #[test]
    fn sign_bundle_package_round_trips_through_the_trust_store() {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let package = minimal_test_package();
        let signed =
            sign_bundle_package(package, "release-2026-09", pkcs8.as_ref()).expect("package signs");
        let trust = BundlePackageTrustStore::new([trusted_key_for_pkcs8(
            "release-2026-09",
            pkcs8.as_ref(),
        )
        .expect("trusted key builds")])
        .unwrap();
        assert_eq!(trust.verify(&signed), Ok(()));

        let other_pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let other_trust = BundlePackageTrustStore::new([trusted_key_for_pkcs8(
            "other-key",
            other_pkcs8.as_ref(),
        )
        .expect("other trusted key builds")])
        .unwrap();
        assert_eq!(
            other_trust.verify(&signed),
            Err(PACKAGE_SIGNER_UNTRUSTED_LABEL.to_string())
        );
    }

    fn sample_check(
        check_id: &str,
        status: PipelineCheckStatus,
        observed_at: DateTime<Utc>,
    ) -> PipelineCheckResult {
        PipelineCheckResult {
            schema: PIPELINE_CHECK_RESULT_SCHEMA.to_string(),
            run_id: "q0123abcd".to_string(),
            check_id: check_id.to_string(),
            status,
            code_revision_hash: sha256_prefixed(b"code-revision"),
            package_hash: None,
            configuration_digest: None,
            dependency_digest: None,
            observed_at,
            evidence_hash: sha256_prefixed(check_id.as_bytes()),
            safe_blockers: Vec::new(),
        }
    }

    #[test]
    fn promotion_requires_current_passing_evidence_for_every_check() {
        let now = Utc::now();
        let evidence = PROMOTION_REQUIRED_CHECKS
            .iter()
            .map(|check_id| DrillEvidence {
                check: sample_check(check_id, PipelineCheckStatus::Pass, now),
                maximum_age_seconds: 3_600,
            })
            .collect::<Vec<_>>();
        assert!(evaluate_promotion(&evidence, now).unwrap().ready);

        let reversed = evidence.iter().rev().cloned().collect::<Vec<_>>();
        assert_eq!(
            evaluate_promotion(&evidence, now).unwrap().evidence_hash,
            evaluate_promotion(&reversed, now).unwrap().evidence_hash
        );

        let stale = evaluate_promotion(&evidence, now + Duration::hours(2)).unwrap();
        assert!(!stale.ready);
        assert!(
            stale
                .safe_blockers
                .iter()
                .all(|label| label.starts_with(QUALIFICATION_EVIDENCE_STALE_LABEL))
        );

        assert!(!evaluate_promotion(&evidence[..1], now).unwrap().ready);

        let mut failed = evidence.clone();
        failed[0].check.status = PipelineCheckStatus::Fail;
        let failed_decision = evaluate_promotion(&failed, now).unwrap();
        assert!(!failed_decision.ready);
        assert!(
            failed_decision
                .safe_blockers
                .iter()
                .any(|label| label.starts_with(QUALIFICATION_EVIDENCE_FAILED_LABEL))
        );

        let mut blocked = evidence.clone();
        blocked[0].check.status = PipelineCheckStatus::Blocked;
        let blocked_decision = evaluate_promotion(&blocked, now).unwrap();
        assert!(!blocked_decision.ready);
        let blocked_check_id = blocked[0].check.check_id.clone();
        assert!(
            blocked_decision.safe_blockers.iter().any(|label| *label
                == format!("{QUALIFICATION_EVIDENCE_FAILED_LABEL}:{blocked_check_id}"))
        );

        let mut duplicate = evidence.clone();
        duplicate.push(evidence[0].clone());
        assert_eq!(
            evaluate_promotion(&duplicate, now),
            Err("qualification_evidence_duplicate".to_string())
        );

        let mut unknown = evidence.clone();
        unknown[0].check.check_id = "not_a_required_check".to_string();
        assert_eq!(
            evaluate_promotion(&unknown, now),
            Err("qualification_evidence_invalid".to_string())
        );
    }
}
