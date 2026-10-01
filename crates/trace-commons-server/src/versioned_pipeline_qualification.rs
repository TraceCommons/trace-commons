// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Package trust and promotion qualification.
//!
//! Qualification evidence is deliberately separate from pipeline history.
//! The ingest database stores only the immutable fact that a package passed
//! qualification. Detailed reports and drill output stay in operator-owned
//! evidence storage.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::BundlePackage;
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{
    PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL, PgPipelineStore,
    PipelineBundleQualification, PipelineService, sha256_prefixed,
};
use crate::versioned_pipeline_bundle::package_configuration_is_qualifiable;
use crate::versioned_pipeline_compat::{
    COMPATIBILITY_ADMISSION_IMPLEMENTATION, COMPATIBILITY_REVIEW_IMPLEMENTATION,
    COMPATIBILITY_SCORE_IMPLEMENTATION, COMPATIBILITY_SETTLE_IMPLEMENTATION,
};

pub const PIPELINE_CHECK_RESULT_SCHEMA: &str = "trace_commons.pipeline_check_result.v1";
const PIPELINE_CHECK_RESULT_DIR_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR";
const PIPELINE_CHECK_RUN_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RUN_ID";
const PIPELINE_CHECK_CODE_REVISION_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH";

pub const PACKAGE_SIGNATURE_ALGORITHM: &str = "Ed25519";
pub const PACKAGE_SIGNATURE_INVALID_LABEL: &str = "bundle_package_signature_invalid";
pub const PACKAGE_SIGNER_UNTRUSTED_LABEL: &str = "bundle_package_signer_untrusted";
pub const PACKAGE_IMPLEMENTATION_UNKNOWN_LABEL: &str = "bundle_implementation_unknown";
pub const PACKAGE_DEVELOPMENT_DEPENDENCY_LABEL: &str = "bundle_development_dependency";
pub const QUALIFICATION_EVIDENCE_STALE_LABEL: &str = "qualification_evidence_stale";
pub const QUALIFICATION_EVIDENCE_MISSING_LABEL: &str = "qualification_evidence_missing";
pub const QUALIFICATION_EVIDENCE_FAILED_LABEL: &str = "qualification_evidence_failed";
pub const QUALIFICATION_EVIDENCE_INVALID_LABEL: &str = "qualification_evidence_invalid";
pub const QUALIFICATION_EVIDENCE_MIXED_REVISION_LABEL: &str =
    "qualification_evidence_mixed_revision";
pub const QUALIFICATION_EVIDENCE_MIXED_PACKAGE_LABEL: &str = "qualification_evidence_mixed_package";

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

/// The package a check result names: its three
/// [`PipelinePackageDigests`] values, each `None` when the result carries
/// none.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PromotionPackage {
    pub package_hash: Option<String>,
    pub configuration_digest: Option<String>,
    pub dependency_digest: Option<String>,
}

impl PromotionPackage {
    fn of(check: &PipelineCheckResult) -> Option<Self> {
        let package = Self {
            package_hash: check.package_hash.clone(),
            configuration_digest: check.configuration_digest.clone(),
            dependency_digest: check.dependency_digest.clone(),
        };
        (package.package_hash.is_some()
            || package.configuration_digest.is_some()
            || package.dependency_digest.is_some())
        .then_some(package)
    }

    /// Whether this is exactly `digests`, all three values present.
    pub fn is(&self, digests: &PipelinePackageDigests) -> bool {
        self.package_hash.as_deref() == Some(digests.package_hash.as_str())
            && self.configuration_digest.as_deref() == Some(digests.configuration_digest.as_str())
            && self.dependency_digest.as_deref() == Some(digests.dependency_digest.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionDecision {
    pub ready: bool,
    pub evaluated_at: DateTime<Utc>,
    pub evidence_hash: String,
    pub safe_blockers: Vec<String>,
    /// The one code revision every result carries; `None` when the
    /// evidence carries none or more than one.
    pub code_revision_hash: Option<String>,
    /// The one package the results that name a package name; `None` when
    /// none names one or they name more than one.
    pub package: Option<PromotionPackage>,
}

/// Whether `evidence` supports production promotion: exactly one current
/// result for each of [`PROMOTION_REQUIRED_CHECKS`], each passing and
/// carrying no safe blocker, all from one code revision and, among the
/// results that name a package, one package (wave 2; Zaki's review of
/// #1166, Major 1's note for PR 5). Every per-check blocker found is listed
/// as `<label>:<check_id>`: missing, failed (or blocked), stale evidence,
/// and each safe blocker a result carries, including a passing one. Evidence
/// from more than one revision adds `qualification_evidence_mixed_revision`,
/// and from more than one package `qualification_evidence_mixed_package`
/// (a package is its three digests together). A result that names no
/// package does not count: one result that names a package binds the
/// decision to it, and a decision can be ready with no package at all when
/// no result names one (only `qualify_bundle` refuses that). Every runtime
/// check names its own test bundle today, so PR 5 must have the mechanics
/// checks name no package, or run them against the candidate, before their
/// results can promote one.
///
/// `evidence_hash` covers, for each check id, the result's run id, code
/// revision, package digests and evidence hash, with the blockers, as
/// canonical JSON (`to_canonical_vec`). It leaves out the evaluation time
/// (`evaluated_at` is a field of the decision): a ready decision's hash then
/// names exactly the evidence it rests on, so a caller can record it in
/// [`BundleQualificationMetadata::evidence_hash`] and
/// [`PipelineQualificationStore::qualify_bundle`], which evaluates the same
/// evidence again at its own time, finds the same hash. The decision names
/// its one revision and package (`code_revision_hash`, `package`), which
/// `qualify_bundle` holds a qualification to.
///
/// Malformed input is an error, not a blocker: a check id outside
/// [`PROMOTION_REQUIRED_CHECKS`] is `qualification_evidence_invalid`, a
/// result that fails [`PipelineCheckResult::validate`] (schema, run id,
/// revision, evidence and digest hashes, safe-blocker labels) or carries no
/// maximum age is `qualification_evidence_invalid:<check_id>`, and a second
/// result for one check is `qualification_evidence_duplicate`.
pub fn evaluate_promotion(
    evidence: &[DrillEvidence],
    now: DateTime<Utc>,
) -> Result<PromotionDecision, String> {
    let mut by_id = BTreeMap::new();
    for item in evidence {
        let check_id = item.check.check_id.as_str();
        // An unknown id is not a label this function may echo.
        if !PROMOTION_REQUIRED_CHECKS.contains(&check_id) {
            return Err(QUALIFICATION_EVIDENCE_INVALID_LABEL.to_string());
        }
        if item.check.validate().is_err() || item.maximum_age_seconds == 0 {
            return Err(format!("{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{check_id}"));
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
        // A result's own safe blockers block promotion whatever its status:
        // a pass can carry one (the restore drill passes with
        // `filesystem_restore_local_only`), and it names what that pass does
        // not prove (final review I4, ruling FR-4).
        blockers.extend(
            item.check
                .safe_blockers
                .iter()
                .map(|label| format!("{label}:{check_id}")),
        );
        let maximum_age = i64::try_from(item.maximum_age_seconds)
            .map_err(|_| QUALIFICATION_EVIDENCE_INVALID_LABEL.to_string())?;
        if item.check.observed_at > now
            || now - item.check.observed_at > Duration::seconds(maximum_age)
        {
            blockers.push(format!("{QUALIFICATION_EVIDENCE_STALE_LABEL}:{check_id}"));
        }
    }
    let revisions = evidence
        .iter()
        .map(|item| item.check.code_revision_hash.as_str())
        .collect::<BTreeSet<_>>();
    if revisions.len() > 1 {
        blockers.push(QUALIFICATION_EVIDENCE_MIXED_REVISION_LABEL.to_string());
    }
    let packages = evidence
        .iter()
        .filter_map(|item| PromotionPackage::of(&item.check))
        .collect::<BTreeSet<_>>();
    if packages.len() > 1 {
        blockers.push(QUALIFICATION_EVIDENCE_MIXED_PACKAGE_LABEL.to_string());
    }
    blockers.sort();
    let checks = evidence
        .iter()
        .map(|item| {
            (
                item.check.check_id.clone(),
                serde_json::json!({
                    "run_id": item.check.run_id,
                    "code_revision_hash": item.check.code_revision_hash,
                    "package_hash": item.check.package_hash,
                    "configuration_digest": item.check.configuration_digest,
                    "dependency_digest": item.check.dependency_digest,
                    "evidence_hash": item.check.evidence_hash,
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let canonical = serde_json::json!({
        "schema": "trace_commons.pipeline_promotion.v2",
        "blockers": blockers.clone(),
        "checks": checks,
    });
    let evidence_digest =
        evidence_hash(&canonical).map_err(|_| QUALIFICATION_EVIDENCE_INVALID_LABEL.to_string())?;
    Ok(PromotionDecision {
        ready: blockers.is_empty(),
        evaluated_at: now,
        evidence_hash: evidence_digest,
        safe_blockers: blockers,
        code_revision_hash: (revisions.len() == 1)
            .then(|| revisions.first().map(|revision| revision.to_string()))
            .flatten(),
        package: (packages.len() == 1)
            .then(|| packages.first().cloned())
            .flatten(),
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

/// Which of the four kinds of production infrastructure a runtime holds for
/// one adapter seam: a genuine production implementation, a development
/// stand-in, a synthetic test double, or none at all.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductionAdapterKind {
    Production,
    Development,
    Synthetic,
    Missing,
}

/// The infrastructure a runtime holds outside the bundle's own scorer,
/// embedder, index, settlement, authority, and privacy dependencies
/// ([`PipelineBundleQualification`] already reports those). An operator
/// assembles this by hand from what it actually deployed; nothing in this
/// tree can derive it from a running service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductionInfrastructureProfile {
    pub authoritative_metadata: ProductionAdapterKind,
    pub artifact_store: ProductionAdapterKind,
    pub key_wrapper: ProductionAdapterKind,
    pub authentication: ProductionAdapterKind,
    pub plaintext_fallback: bool,
    pub best_effort_database_mirror: bool,
    pub static_bearer_authentication: bool,
    pub hs256_bridge_authentication: bool,
    pub unversioned_policy_dependencies: bool,
    pub live_external_payout_enabled: bool,
}

impl ProductionInfrastructureProfile {
    /// The infrastructure profile of a local test or CI run: real
    /// authoritative metadata, but development artifact storage, key
    /// wrapping, and authentication, plus the static bearer token every
    /// local run uses.
    pub fn local_test() -> Self {
        Self {
            authoritative_metadata: ProductionAdapterKind::Production,
            artifact_store: ProductionAdapterKind::Development,
            key_wrapper: ProductionAdapterKind::Development,
            authentication: ProductionAdapterKind::Development,
            plaintext_fallback: false,
            best_effort_database_mirror: false,
            static_bearer_authentication: true,
            hs256_bridge_authentication: false,
            unversioned_policy_dependencies: false,
            live_external_payout_enabled: false,
        }
    }
}

/// Whether one bundle is ready for production activation: the bundle's own
/// dependency qualification ([`PipelineBundleQualification`], scoped to what
/// the package actually names, decision P4-D7) plus the infrastructure an
/// operator holds around it. [`Self::blockers`] is the complete list a
/// caller needs; nothing here re-derives what
/// [`PipelineBundleQualification::blockers`] already reports.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProductionDependencyProfile {
    bundle: PipelineBundleQualification,
    infrastructure: ProductionInfrastructureProfile,
}

impl ProductionDependencyProfile {
    pub fn new(
        bundle: PipelineBundleQualification,
        infrastructure: ProductionInfrastructureProfile,
    ) -> Self {
        Self {
            bundle,
            infrastructure,
        }
    }

    /// [`PipelineService::bundle_qualification`] scoped to `package`, wrapped
    /// with `infrastructure`. Fails closed with the same safe label
    /// `bundle_qualification` itself reports.
    pub fn for_bundle(
        service: &PipelineService,
        package: &BundlePackage,
        infrastructure: ProductionInfrastructureProfile,
    ) -> Result<Self, String> {
        let bundle = service
            .bundle_qualification(package)
            .map_err(|label| label.to_string())?;
        Ok(Self::new(bundle, infrastructure))
    }

    /// A digest over the bundle qualification's dependency identities (the
    /// scorer, embedder, index reader, index writer, and every settlement
    /// adapter this bundle pins) and its own `dependency_digest`. A
    /// [`BundleQualificationMetadata::runtime_dependency_digest`] the
    /// qualification store records must equal this value at the moment of
    /// qualification -- see [`PipelineQualificationStore::qualify_bundle`].
    pub fn runtime_identity_digest(&self) -> Result<String, String> {
        let settlement_adapters = self
            .bundle
            .settlement_adapters
            .iter()
            .map(|(instrument, check)| (instrument.clone(), check.identity.clone()))
            .collect::<BTreeMap<_, _>>();
        let value = serde_json::json!({
            "scorer": self.bundle.scorer.identity,
            "embedder": self.bundle.embedder.identity,
            "index_reader": self.bundle.index_reader.identity,
            "index_writer": self.bundle.index_writer.identity,
            "settlement_adapters": settlement_adapters,
            "dependency_digest": self.bundle.dependency_digest,
        });
        evidence_hash(&value)
    }

    /// Every safe label blocking this bundle from production activation: the
    /// bundle's own blockers first, then the infrastructure this profile
    /// wraps around it. Empty exactly when every dependency and every piece
    /// of infrastructure is production-grade.
    pub fn blockers(&self) -> Vec<String> {
        let mut blockers: Vec<String> = self
            .bundle
            .blockers()
            .into_iter()
            .map(str::to_string)
            .collect();
        for (kind, name) in [
            (
                &self.infrastructure.authoritative_metadata,
                "authoritative_metadata",
            ),
            (&self.infrastructure.artifact_store, "artifact_store"),
            (&self.infrastructure.key_wrapper, "key_wrapper"),
            (&self.infrastructure.authentication, "authentication"),
        ] {
            if *kind != ProductionAdapterKind::Production {
                blockers.push(format!("{name}_not_production"));
            }
        }
        for (blocked, label) in [
            (
                self.infrastructure.plaintext_fallback,
                "plaintext_fallback_enabled",
            ),
            (
                self.infrastructure.best_effort_database_mirror,
                "best_effort_database_mirror_enabled",
            ),
            (
                self.infrastructure.static_bearer_authentication,
                "static_bearer_authentication_enabled",
            ),
            (
                self.infrastructure.hs256_bridge_authentication,
                "hs256_bridge_authentication_enabled",
            ),
            (
                self.infrastructure.unversioned_policy_dependencies,
                "unversioned_policy_dependency",
            ),
            (
                self.infrastructure.live_external_payout_enabled,
                "live_external_payout_enabled",
            ),
        ] {
            if blocked {
                blockers.push(label.to_string());
            }
        }
        blockers
    }
}

/// Refuses a package whose four phase implementations are not entirely the
/// compatibility family, or which carries any development or synthetic
/// dependency marker: a Score or Settle projection id containing `test` or
/// `reference`, or an artifact whose bytes contain `local_reference`,
/// `reference_`, `pipeline-test`, `mock_`, or `synthetic`.
pub fn validate_production_package(package: &BundlePackage) -> Result<(), String> {
    let known = package.manifest.admission.implementation_id
        == COMPATIBILITY_ADMISSION_IMPLEMENTATION
        && package.manifest.review.implementation_id == COMPATIBILITY_REVIEW_IMPLEMENTATION
        && package.manifest.score.implementation_id == COMPATIBILITY_SCORE_IMPLEMENTATION
        && package.manifest.settle.implementation_id == COMPATIBILITY_SETTLE_IMPLEMENTATION;
    if !known {
        return Err(PACKAGE_IMPLEMENTATION_UNKNOWN_LABEL.to_string());
    }
    let has_development_dependency = package
        .manifest
        .score
        .projection_ids
        .iter()
        .chain(package.manifest.settle.projection_ids.iter())
        .any(|identity| identity.contains("test") || identity.contains("reference"))
        || package.artifacts.values().any(|artifact| {
            let text = String::from_utf8_lossy(artifact).to_ascii_lowercase();
            [
                "local_reference",
                "reference_",
                "pipeline-test",
                "mock_",
                "synthetic",
            ]
            .iter()
            .any(|marker| text.contains(marker))
        });
    if has_development_dependency {
        return Err(PACKAGE_DEVELOPMENT_DEPENDENCY_LABEL.to_string());
    }
    Ok(())
}

/// The immutable digests a qualification records alongside a
/// [`BundleQualificationRecord`]'s package and signature identity. Every
/// field must be a `sha256:` digest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleQualificationMetadata {
    pub corpus_digest: String,
    pub input_digest: String,
    pub configuration_digest: String,
    pub code_revision_hash: String,
    pub runtime_dependency_digest: String,
    pub evidence_hash: String,
}

impl BundleQualificationMetadata {
    fn validate(&self) -> Result<(), String> {
        if [
            &self.corpus_digest,
            &self.input_digest,
            &self.configuration_digest,
            &self.code_revision_hash,
            &self.runtime_dependency_digest,
            &self.evidence_hash,
        ]
        .into_iter()
        .all(|value| is_sha256(value))
        {
            Ok(())
        } else {
            Err("bundle_qualification_metadata_invalid".to_string())
        }
    }
}

/// One bundle's recorded qualification: the row
/// [`PipelineQualificationStore::qualify_bundle`] inserts (or, on a repeat
/// call with identical inputs, the row it already inserted).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleQualificationRecord {
    pub bundle_id: String,
    pub package_hash: String,
    pub signing_key_id: String,
    pub signature_hash: String,
    pub metadata: BundleQualificationMetadata,
    pub qualified_at: DateTime<Utc>,
}

/// Records a production package's qualification once per `(tenant_id,
/// bundle_id)`. `pipeline_bundle_qualifications` is append-only in the
/// database (V107): a repeat call with the exact same inputs answers the
/// existing row; a repeat call with different metadata for the same bundle
/// is refused as a conflict, never silently overwritten.
pub struct PipelineQualificationStore {
    backend: Arc<PgBackend>,
    packages: PgPipelineStore,
}

impl PipelineQualificationStore {
    pub fn new(backend: Arc<PgBackend>) -> Self {
        Self {
            packages: PgPipelineStore::new(backend.clone()),
            backend,
        }
    }

    /// Verifies `signed`'s trust and production shape, checks `metadata` and
    /// `dependencies` against `signed.package` and each other, and records
    /// the qualification. Fails closed on: an untrusted or tampered package
    /// (`trust.verify`), a non-production or development package
    /// (`validate_production_package`), malformed metadata
    /// (`bundle_qualification_metadata_invalid`), a `dependencies` profile
    /// built for a different bundle -- a different `bundle_id` or a
    /// `dependency_digest` that does not match `signed.package`'s own
    /// (`bundle_qualification_profile_mismatch`; `dependencies` is
    /// bundle-scoped, so nothing else here ties it to `signed.package`), a
    /// `configuration_digest` that does not match `signed.package`'s own
    /// (`bundle_qualification_configuration_mismatch`), a metadata dependency
    /// digest that does not match `dependencies`
    /// (`runtime_dependency_identity_mismatch`), any blocked dependency or
    /// infrastructure control (`dependencies.blockers()`'s first label),
    /// evidence that does not back this qualification (below), or a second
    /// call for the same bundle with different metadata
    /// (`bundle_qualification_identity_conflict`). No row is left behind by a
    /// failed call.
    ///
    /// Two terms are derived here, never taken from the caller (wave 2,
    /// fix round 1; review I1):
    /// - The package's own configuration: `signed.package` must be
    ///   qualifiable (`package_configuration_is_qualifiable`,
    ///   `bundle_configuration_not_qualifiable`), whatever `dependencies`
    ///   says.
    /// - The promotion decision (Zaki's review of #1166, Z6): `evidence` is
    ///   the check results the metadata records, and this evaluates them
    ///   itself ([`evaluate_promotion`] at the current time, so stale
    ///   evidence blocks; a malformed result is refused with its own label).
    ///   The decision must be ready (`bundle_qualification_promotion_not_ready`),
    ///   its evidence hash must be the metadata's
    ///   (`bundle_qualification_evidence_mismatch`), its one code revision the
    ///   metadata's (`bundle_qualification_code_revision_mismatch`), and its
    ///   one package `signed.package` (`bundle_qualification_package_mismatch`).
    pub async fn qualify_bundle(
        &self,
        tenant_id: &str,
        signed: &SignedBundlePackage,
        trust: &BundlePackageTrustStore,
        metadata: &BundleQualificationMetadata,
        dependencies: &ProductionDependencyProfile,
        evidence: &[DrillEvidence],
    ) -> Result<BundleQualificationRecord, DatabaseError> {
        trust.verify(signed).map_err(DatabaseError::Constraint)?;
        validate_production_package(&signed.package).map_err(DatabaseError::Constraint)?;
        if !package_configuration_is_qualifiable(&signed.package) {
            return Err(DatabaseError::Constraint(
                PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL.to_string(),
            ));
        }
        metadata.validate().map_err(DatabaseError::Constraint)?;
        let digests = package_digests(&signed.package).map_err(DatabaseError::Constraint)?;
        if dependencies.bundle.bundle_id != signed.package.bundle_id
            || dependencies.bundle.dependency_digest != digests.dependency_digest
        {
            return Err(DatabaseError::Constraint(
                "bundle_qualification_profile_mismatch".to_string(),
            ));
        }
        if metadata.configuration_digest != digests.configuration_digest {
            return Err(DatabaseError::Constraint(
                "bundle_qualification_configuration_mismatch".to_string(),
            ));
        }
        if metadata.runtime_dependency_digest
            != dependencies
                .runtime_identity_digest()
                .map_err(DatabaseError::Constraint)?
        {
            return Err(DatabaseError::Constraint(
                "runtime_dependency_identity_mismatch".to_string(),
            ));
        }
        let blockers = dependencies.blockers();
        if !blockers.is_empty() {
            return Err(DatabaseError::Constraint(blockers[0].clone()));
        }
        let promotion =
            evaluate_promotion(evidence, Utc::now()).map_err(DatabaseError::Constraint)?;
        require_promotion_backs(&promotion, metadata, &digests)
            .map_err(|label| DatabaseError::Constraint(label.to_string()))?;
        self.packages
            .register_bundle(tenant_id, &signed.package)
            .await?;
        let package_hash = signed
            .package
            .package_hash()
            .map_err(|_| DatabaseError::Serialization(PACKAGE_SIGNATURE_INVALID_LABEL.into()))?;
        let signature_hash = sha256_prefixed(signed.signature.signature_base64url.as_bytes());
        let mut client = self.backend.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO pipeline_bundle_qualifications (
                tenant_id, bundle_id, package_hash, signing_key_id,
                signature_hash, corpus_digest, input_digest,
                configuration_digest, code_revision_hash,
                runtime_dependency_digest, evidence_hash
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (tenant_id, bundle_id) DO NOTHING",
            &[
                &tenant_id,
                &signed.package.bundle_id,
                &package_hash,
                &signed.signature.key_id,
                &signature_hash,
                &metadata.corpus_digest,
                &metadata.input_digest,
                &metadata.configuration_digest,
                &metadata.code_revision_hash,
                &metadata.runtime_dependency_digest,
                &metadata.evidence_hash,
            ],
        )
        .await?;
        let row = tx
            .query_one(
                "SELECT bundle_id, package_hash, signing_key_id, signature_hash,
                        corpus_digest, input_digest, configuration_digest,
                        code_revision_hash, runtime_dependency_digest,
                        evidence_hash, qualified_at
                   FROM pipeline_bundle_qualifications
                  WHERE tenant_id = $1 AND bundle_id = $2",
                &[&tenant_id, &signed.package.bundle_id],
            )
            .await?;
        let record = qualification_from_row(&row);
        if record.package_hash != package_hash
            || record.signing_key_id != signed.signature.key_id
            || record.signature_hash != signature_hash
            || record.metadata != *metadata
        {
            return Err(DatabaseError::Constraint(
                "bundle_qualification_identity_conflict".to_string(),
            ));
        }
        tx.commit().await?;
        Ok(record)
    }
}

/// Whether `promotion` backs a qualification recording `metadata` for the
/// package whose digests are `digests` (see
/// [`PipelineQualificationStore::qualify_bundle`]); `Err` names the first
/// condition that fails.
fn require_promotion_backs(
    promotion: &PromotionDecision,
    metadata: &BundleQualificationMetadata,
    digests: &PipelinePackageDigests,
) -> Result<(), &'static str> {
    if !promotion.ready {
        return Err("bundle_qualification_promotion_not_ready");
    }
    if promotion.evidence_hash != metadata.evidence_hash {
        return Err("bundle_qualification_evidence_mismatch");
    }
    if promotion.code_revision_hash.as_deref() != Some(metadata.code_revision_hash.as_str()) {
        return Err("bundle_qualification_code_revision_mismatch");
    }
    if !promotion
        .package
        .as_ref()
        .is_some_and(|package| package.is(digests))
    {
        return Err("bundle_qualification_package_mismatch");
    }
    Ok(())
}

fn qualification_from_row(row: &tokio_postgres::Row) -> BundleQualificationRecord {
    BundleQualificationRecord {
        bundle_id: row.get("bundle_id"),
        package_hash: row.get("package_hash"),
        signing_key_id: row.get("signing_key_id"),
        signature_hash: row.get("signature_hash"),
        metadata: BundleQualificationMetadata {
            corpus_digest: row.get("corpus_digest"),
            input_digest: row.get("input_digest"),
            configuration_digest: row.get("configuration_digest"),
            code_revision_hash: row.get("code_revision_hash"),
            runtime_dependency_digest: row.get("runtime_dependency_digest"),
            evidence_hash: row.get("evidence_hash"),
        },
        qualified_at: row.get("qualified_at"),
    }
}

#[cfg(test)]
mod tests {
    use ring::signature::Ed25519KeyPair;
    use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentDescriptor, InstrumentKind};
    use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};

    use super::*;
    use crate::versioned_pipeline::PipelineDependencyCheck;
    use crate::versioned_pipeline_bundle::{
        MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
    };
    use crate::versioned_pipeline_compat::CompatibilityBundleConfig;

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

        // Final review I4 (ruling FR-4): a passing result's safe blockers
        // are promotion blockers, each as `<label>:<check_id>`. The restore
        // drill passes carrying `filesystem_restore_local_only`.
        let mut restore_blocked = evidence.clone();
        let restore = restore_blocked
            .iter_mut()
            .find(|item| item.check.check_id == "pipeline_restore_drill")
            .expect("the restore drill is a promotion check");
        restore.check.safe_blockers = vec!["filesystem_restore_local_only".to_string()];
        let restore_decision = evaluate_promotion(&restore_blocked, now).unwrap();
        assert!(!restore_decision.ready);
        assert_eq!(
            restore_decision.safe_blockers,
            vec!["filesystem_restore_local_only:pipeline_restore_drill".to_string()]
        );
        assert_ne!(
            restore_decision.evidence_hash,
            evaluate_promotion(&evidence, now).unwrap().evidence_hash,
            "the blocker is part of the hashed decision"
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

    /// Zaki's review of #1166, Major 1: a probe with every required id, each
    /// from its own run and revision, one with the schema `bogus_schema`, one
    /// with the revision `not-a-hash`, and one with a run id that is not a
    /// label, returned `ready` with no blocker. Each malformed result is now
    /// refused by its check id, one per evaluation in input order, until none
    /// is left.
    #[test]
    fn promotion_refuses_each_result_that_fails_validation_by_check_id() {
        let now = Utc::now();
        let mut probe = PROMOTION_REQUIRED_CHECKS
            .iter()
            .enumerate()
            .map(|(index, check_id)| {
                let mut check = sample_check(check_id, PipelineCheckStatus::Pass, now);
                check.run_id = format!("q{index:08x}");
                check.code_revision_hash = sha256_prefixed(format!("revision-{index}").as_bytes());
                DrillEvidence {
                    check,
                    maximum_age_seconds: 3_600,
                }
            })
            .collect::<Vec<_>>();
        let well_formed = probe.clone();
        probe[2].check.schema = "bogus_schema".to_string();
        probe[7].check.code_revision_hash = "not-a-hash".to_string();
        probe[11].check.run_id = "Not A Label".to_string();

        let mut named = Vec::new();
        for index in [2, 7, 11] {
            let check_id = probe[index].check.check_id.clone();
            assert_eq!(
                evaluate_promotion(&probe, now),
                Err(format!("{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{check_id}")),
                "the first malformed result left is refused by its id"
            );
            named.push(check_id);
            probe[index] = well_formed[index].clone();
        }
        assert_eq!(
            named,
            [
                PROMOTION_REQUIRED_CHECKS[2],
                PROMOTION_REQUIRED_CHECKS[7],
                PROMOTION_REQUIRED_CHECKS[11]
            ]
        );

        // The control: with the three repaired, the same probe evaluates, so
        // each refusal above came from its malformed field. Its results come
        // from distinct revisions, so it is not ready, and that is its only
        // blocker (wave 2: promotion binds one revision).
        let control = evaluate_promotion(&probe, now).expect("a well-formed probe evaluates");
        assert!(!control.ready);
        assert_eq!(
            control.safe_blockers,
            vec![QUALIFICATION_EVIDENCE_MIXED_REVISION_LABEL.to_string()]
        );

        // The rest of `validate()` and the age bound are refused the same way.
        let mut bad_digest = probe.clone();
        bad_digest[4].check.package_hash = Some("sha256:short".to_string());
        let mut ageless = probe.clone();
        ageless[5].maximum_age_seconds = 0;
        for (refused, index) in [(bad_digest, 4), (ageless, 5)] {
            assert_eq!(
                evaluate_promotion(&refused, now),
                Err(format!(
                    "{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{}",
                    PROMOTION_REQUIRED_CHECKS[index]
                ))
            );
        }
    }

    fn passing_evidence(now: DateTime<Utc>) -> Vec<DrillEvidence> {
        PROMOTION_REQUIRED_CHECKS
            .iter()
            .map(|check_id| DrillEvidence {
                check: sample_check(check_id, PipelineCheckStatus::Pass, now),
                maximum_age_seconds: 3_600,
            })
            .collect()
    }

    fn name_package(check: &mut PipelineCheckResult, name: &str) {
        check.package_hash = Some(sha256_prefixed(format!("{name}-package").as_bytes()));
        check.configuration_digest =
            Some(sha256_prefixed(format!("{name}-configuration").as_bytes()));
        check.dependency_digest = Some(sha256_prefixed(format!("{name}-dependency").as_bytes()));
    }

    /// Wave 2 (Zaki's review of #1166, Major 1's note for PR 5): every
    /// result must come from one code revision. Evidence from two revisions
    /// is not ready, under `qualification_evidence_mixed_revision`, and the
    /// decision names no revision; from one, it names that revision.
    #[test]
    fn promotion_binds_one_code_revision() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        let decision = evaluate_promotion(&evidence, now).unwrap();
        assert!(decision.ready);
        assert_eq!(
            decision.code_revision_hash,
            Some(sha256_prefixed(b"code-revision"))
        );

        let mut mixed = evidence.clone();
        mixed[3].check.code_revision_hash = sha256_prefixed(b"another-revision");
        let decision = evaluate_promotion(&mixed, now).unwrap();
        assert!(!decision.ready);
        assert_eq!(
            decision.safe_blockers,
            vec![QUALIFICATION_EVIDENCE_MIXED_REVISION_LABEL.to_string()]
        );
        assert_eq!(decision.code_revision_hash, None);
    }

    /// Wave 2: among the results that name a package, every one must name
    /// the same package (all three digests). A result that names none does
    /// not count. Two packages, or one that differs only in its
    /// configuration digest, is not ready, under
    /// `qualification_evidence_mixed_package`, and the decision names no
    /// package; one package is named by the decision.
    #[test]
    fn promotion_binds_one_package() {
        let now = Utc::now();
        let mut evidence = passing_evidence(now);
        for item in evidence.iter_mut().step_by(2) {
            name_package(&mut item.check, "candidate");
        }
        let decision = evaluate_promotion(&evidence, now).unwrap();
        assert!(decision.ready, "{:?}", decision.safe_blockers);
        let package = decision.package.expect("the decision names its package");
        assert!(package.is(&PipelinePackageDigests {
            package_hash: sha256_prefixed(b"candidate-package"),
            configuration_digest: sha256_prefixed(b"candidate-configuration"),
            dependency_digest: sha256_prefixed(b"candidate-dependency"),
        }));

        let mut other_package = evidence.clone();
        name_package(&mut other_package[1].check, "other");
        let mut other_configuration = evidence.clone();
        other_configuration[2].check.configuration_digest =
            Some(sha256_prefixed(b"other-configuration"));
        for mixed in [other_package, other_configuration] {
            let decision = evaluate_promotion(&mixed, now).unwrap();
            assert!(!decision.ready);
            assert_eq!(
                decision.safe_blockers,
                vec![QUALIFICATION_EVIDENCE_MIXED_PACKAGE_LABEL.to_string()]
            );
            assert_eq!(decision.package, None);
        }

        let unnamed = passing_evidence(now);
        let decision = evaluate_promotion(&unnamed, now).unwrap();
        assert!(decision.ready);
        assert_eq!(decision.package, None, "no result names a package");
    }

    /// Wave 2: the decision's `evidence_hash` covers each result's run id,
    /// code revision, package digests and evidence hash, not only the
    /// evidence hash: changing any one of them changes it.
    #[test]
    fn promotion_evidence_hash_covers_run_revision_package_and_evidence() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        let base = evaluate_promotion(&evidence, now).unwrap().evidence_hash;

        let mut run = evidence.clone();
        run[0].check.run_id = "q9999abcd".to_string();
        let mut revision = evidence.clone();
        for item in &mut revision {
            item.check.code_revision_hash = sha256_prefixed(b"another-revision");
        }
        let mut package = evidence.clone();
        name_package(&mut package[0].check, "candidate");
        let mut dependency = package.clone();
        dependency[0].check.dependency_digest = Some(sha256_prefixed(b"other-dependency"));
        let mut observed = evidence.clone();
        observed[0].check.evidence_hash = sha256_prefixed(b"other-evidence");

        let mut hashes = vec![base];
        for changed in [run, revision, package, dependency, observed] {
            let decision = evaluate_promotion(&changed, now).unwrap();
            hashes.push(decision.evidence_hash);
        }
        let distinct = hashes.iter().collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), hashes.len(), "{hashes:?}");
    }

    /// Wave 2 (Zaki's review of #1166, Z6): a qualification records only
    /// what a ready promotion decision backs -- the decision's evidence hash
    /// and its one code revision are the metadata's, and its one package is
    /// the package being qualified. Each condition is refused by its own
    /// label.
    #[test]
    fn qualification_requires_a_ready_promotion_for_its_metadata_and_package() {
        let now = Utc::now();
        let mut evidence = passing_evidence(now);
        for item in evidence.iter_mut().step_by(3) {
            name_package(&mut item.check, "candidate");
        }
        let ready = evaluate_promotion(&evidence, now).unwrap();
        assert!(ready.ready);
        let candidate = PipelinePackageDigests {
            package_hash: sha256_prefixed(b"candidate-package"),
            configuration_digest: sha256_prefixed(b"candidate-configuration"),
            dependency_digest: sha256_prefixed(b"candidate-dependency"),
        };
        let metadata = BundleQualificationMetadata {
            corpus_digest: sha256_prefixed(b"corpus"),
            input_digest: sha256_prefixed(b"input"),
            configuration_digest: candidate.configuration_digest.clone(),
            code_revision_hash: sha256_prefixed(b"code-revision"),
            runtime_dependency_digest: sha256_prefixed(b"runtime"),
            evidence_hash: ready.evidence_hash.clone(),
        };
        assert_eq!(
            require_promotion_backs(&ready, &metadata, &candidate),
            Ok(())
        );

        let not_ready = evaluate_promotion(&evidence[1..], now).unwrap();
        let mut not_ready_metadata = metadata.clone();
        not_ready_metadata.evidence_hash = not_ready.evidence_hash.clone();
        let mut other_evidence = metadata.clone();
        other_evidence.evidence_hash = sha256_prefixed(b"other-evidence");
        let mut other_revision = metadata.clone();
        other_revision.code_revision_hash = sha256_prefixed(b"other-revision");
        let other_package = PipelinePackageDigests {
            package_hash: sha256_prefixed(b"other-package"),
            configuration_digest: candidate.configuration_digest.clone(),
            dependency_digest: candidate.dependency_digest.clone(),
        };
        let unnamed = evaluate_promotion(&passing_evidence(now), now).unwrap();
        let mut unnamed_metadata = metadata.clone();
        unnamed_metadata.evidence_hash = unnamed.evidence_hash.clone();
        for (promotion, metadata, digests, label) in [
            (
                &not_ready,
                &not_ready_metadata,
                &candidate,
                "bundle_qualification_promotion_not_ready",
            ),
            (
                &ready,
                &other_evidence,
                &candidate,
                "bundle_qualification_evidence_mismatch",
            ),
            (
                &ready,
                &other_revision,
                &candidate,
                "bundle_qualification_code_revision_mismatch",
            ),
            (
                &ready,
                &metadata,
                &other_package,
                "bundle_qualification_package_mismatch",
            ),
            (
                &unnamed,
                &unnamed_metadata,
                &candidate,
                "bundle_qualification_package_mismatch",
            ),
        ] {
            assert_eq!(
                require_promotion_backs(promotion, metadata, digests),
                Err(label)
            );
        }
    }

    #[test]
    fn production_package_validation_refuses_minimal_unknown_and_reference_packages() {
        let minimal = minimal_test_package();
        assert_eq!(
            validate_production_package(&minimal),
            Err(PACKAGE_IMPLEMENTATION_UNKNOWN_LABEL.to_string())
        );

        let scorer = ReferencePerplexityScorer::new();
        let embedder = ReferenceEmbedder::new();
        let reference_package = MinimalPolicyBundle::compatibility_package(
            &CompatibilityBundleConfig::local_reference(),
            &scorer,
            &embedder,
        )
        .expect("compatibility package builds under the local-reference config");
        assert_eq!(
            validate_production_package(&reference_package),
            Err(PACKAGE_DEVELOPMENT_DEPENDENCY_LABEL.to_string())
        );

        let mut unknown_score = reference_package;
        unknown_score.manifest.score.implementation_id =
            "trace_commons.score.unknown.v1".to_string();
        assert_eq!(
            validate_production_package(&unknown_score),
            Err(PACKAGE_IMPLEMENTATION_UNKNOWN_LABEL.to_string())
        );
    }

    #[test]
    fn production_profile_blocks_on_bundle_and_infrastructure() {
        fn qualified(identity: &str) -> PipelineDependencyCheck {
            PipelineDependencyCheck {
                identity: identity.to_string(),
                production_qualified: true,
            }
        }

        let bundle = PipelineBundleQualification {
            bundle_id: sha256_prefixed(b"bundle"),
            scorer: PipelineDependencyCheck {
                identity: "unqualified_scorer_test_only".to_string(),
                production_qualified: false,
            },
            embedder: qualified("qualified_embedder_test_only"),
            index_reader: qualified("qualified_index_reader_test_only"),
            index_writer: qualified("qualified_index_writer_test_only"),
            settlement_adapters: BTreeMap::new(),
            authority: true,
            privacy: true,
            payout: None,
            configuration_qualifiable: true,
            dependency_digest: sha256_prefixed(b"dependency-digest"),
        };
        let profile = ProductionDependencyProfile::new(
            bundle.clone(),
            ProductionInfrastructureProfile::local_test(),
        );
        let blockers = profile.blockers();
        assert!(blockers.contains(&"runtime_scorer_not_production".to_string()));
        assert!(blockers.contains(&"artifact_store_not_production".to_string()));
        assert!(blockers.contains(&"static_bearer_authentication_enabled".to_string()));

        let digest = profile
            .runtime_identity_digest()
            .expect("digest computes over a hand-built qualification");

        let mut changed_bundle = bundle;
        changed_bundle.scorer.identity = "different_scorer_test_only".to_string();
        let changed_profile = ProductionDependencyProfile::new(
            changed_bundle,
            ProductionInfrastructureProfile::local_test(),
        );
        assert_ne!(
            digest,
            changed_profile
                .runtime_identity_digest()
                .expect("digest computes over the changed qualification")
        );
    }
}
