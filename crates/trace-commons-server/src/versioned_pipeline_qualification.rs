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
use deadpool_postgres::Transaction;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::{BundlePackage, Phase};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::versioned_pipeline::{
    PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL, PIPELINE_BUNDLE_MISSING_LABEL,
    PIPELINE_POLICY_NOT_RUNNABLE_LABEL, PgPipelineStore, PipelineBundleQualification,
    PipelineService, load_bundle_from_transaction, lock_runnable_policy, sha256_prefixed,
};
use crate::versioned_pipeline_bundle::{
    PIPELINE_BUNDLE_INVALID_LABEL, package_configuration_is_qualifiable,
};
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
/// The results of the checks outside [`PROMOTION_ONLY_CHECKS`] carry more
/// than one run id (review round 1 of #1240, point 5).
pub const QUALIFICATION_EVIDENCE_MIXED_RUN_LABEL: &str = "qualification_evidence_mixed_run";
/// A check that tests the candidate ([`PROMOTION_PACKAGE_CHECKS`]) whose
/// result does not name all three package digests; blocks as
/// `<label>:<check_id>`.
pub const QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL: &str =
    "qualification_evidence_package_missing";
/// A mechanics check (one outside [`PROMOTION_PACKAGE_CHECKS`]) whose result
/// carries a package digest; blocks as `<label>:<check_id>`.
pub const QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL: &str =
    "qualification_evidence_package_unexpected";
/// The longest maximum age the server accepts for a check result: seven
/// days. A signed attestation carries its maximum age
/// ([`PipelineCheckAttestation::maximum_age_seconds`], under the signature)
/// and the signer chooses it, so the server bounds it whichever way a result
/// arrives: [`CheckResultTrustStore::verify_all`] refuses a larger age, so
/// every route that takes signed results (the qualification through
/// `qualify_bundle_attested`, the activation, and the rollback) refuses it,
/// and the bare `qualify_bundle` refuses it too. Both refuse with
/// [`QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL`].
pub const QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL: &str =
    "bundle_qualification_evidence_age_above_ceiling";
/// The maximum age `pipeline.py qualify --signing-key` gives a result it
/// signs when `--evidence-max-age-seconds` is not given: one day.
pub const QUALIFICATION_EVIDENCE_DEFAULT_MAX_AGE_SECONDS: u64 = 24 * 60 * 60;

/// The activation gate's refusals (`PipelineQualificationStore::activate_qualified_bundle_in`).
/// The tenant has no qualification row for the bundle.
pub const PACKAGE_QUALIFICATION_MISSING_LABEL: &str = "bundle_qualification_missing";
/// The deployed code revision is not the revision of the promotion, or the
/// bundle has no qualification on it.
pub const PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL: &str = "bundle_runtime_revision_mismatch";
/// The deployed code revision is not a `sha256:` digest (a binary built
/// without `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` has none).
pub const PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL: &str = "bundle_runtime_revision_unknown";
/// The promotion decision is not ready, or carries a blocker.
pub const ACTIVATION_PROMOTION_NOT_READY_LABEL: &str = "bundle_activation_promotion_not_ready";
/// `qualify_bundle`'s refusal of a promotion decision that is not ready.
pub const QUALIFICATION_PROMOTION_NOT_READY_LABEL: &str =
    "bundle_qualification_promotion_not_ready";
/// The promotion decision was evaluated more than
/// [`ACTIVATION_PROMOTION_MAX_AGE_SECONDS`] ago, or in the future.
pub const ACTIVATION_PROMOTION_STALE_LABEL: &str = "bundle_activation_promotion_stale";
/// The promotion decision does not name exactly the activated package.
pub const ACTIVATION_PACKAGE_MISMATCH_LABEL: &str = "bundle_activation_package_mismatch";
/// How long a promotion decision stays usable for an activation: 15 minutes.
pub const ACTIVATION_PROMOTION_MAX_AGE_SECONDS: i64 = 15 * 60;
/// The code revision this binary was built from (P5-D17): the value of
/// `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` at build time, which a release
/// build sets to `pipeline.py revision`'s output on the same checkout.
/// `None` for a binary built without it. Nothing in this module reads it:
/// the admin routes (Task 10) pass it to the qualification and the gate,
/// and refuse with `bundle_runtime_revision_unknown` when it is unset.
pub const DEPLOYED_CODE_REVISION_HASH: Option<&str> =
    option_env!("TRACE_COMMONS_BUILD_CODE_REVISION_HASH");

/// The schema of the envelope that signs a check result (P5-D13). The result
/// inside stays exactly [`PIPELINE_CHECK_RESULT_SCHEMA`].
pub const PIPELINE_CHECK_ATTESTATION_SCHEMA: &str = "trace_commons.pipeline_check_attestation.v1";
/// An attestation whose signature does not verify: an altered field, another
/// key behind a known key id, an algorithm other than Ed25519, or a signature
/// that is not one.
pub const CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL: &str = "check_attestation_signature_invalid";
/// An attestation signed under a key id the check trust store does not hold
/// (or a check key that the trust store refuses to hold).
pub const CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL: &str = "check_attestation_signer_untrusted";
/// An attestation that is not well formed whatever its signature says: a
/// schema other than [`PIPELINE_CHECK_ATTESTATION_SCHEMA`], a result that
/// fails [`PipelineCheckResult::validate`], a maximum age of zero, a digest
/// that is not a `sha256:` digest, or two attestations for one check.
pub const CHECK_ATTESTATION_INVALID_LABEL: &str = "check_attestation_invalid";

/// The checks that must pass for promotion: one current result for each
/// (see [`evaluate_promotion`]). Exactly the ids in
/// [`PROMOTION_PACKAGE_CHECKS`] test the candidate package and name it; every
/// other id is a mechanics check (or a promotion-only check) and names no
/// package.
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
    // PR 5's activation checks: mechanics checks, each emitted by one exact
    // test, that name no package.
    "pipeline_activation_containment",
    "pipeline_activation_rollback",
    "pipeline_legacy_drain",
    // Promotion only: no local or CI run passes these
    // (`PROMOTION_ONLY_CHECKS`).
    "pipeline_production_adapters",
    "pipeline_remote_restore",
    "pipeline_hf_network_canary",
];

/// The checks, among [`PROMOTION_REQUIRED_CHECKS`], that no local or CI run
/// passes: they need the production assembly, so their results come from
/// other runs than the `qualify` run. Every other required check is a check
/// that one `pipeline.py qualify` run produces, and [`evaluate_promotion`]
/// requires those results to share one run id.
/// `scripts/operator/test_pipeline_tooling.py` holds the same three as
/// `_PROMOTION_ONLY` (a test there requires the two lists to agree).
pub const PROMOTION_ONLY_CHECKS: &[&str] = &[
    "pipeline_production_adapters",
    "pipeline_remote_restore",
    "pipeline_hf_network_canary",
];

/// The checks, among [`PROMOTION_REQUIRED_CHECKS`], that test the candidate
/// package and so carry its three digests (P5-D15): the bundle qualification,
/// the compatibility and HF-local corpus runs, and the restore drill. Every
/// other required check is a mechanics check whose result names no package.
/// `scripts/operator/pipeline_tooling/checks.py` holds the same four as
/// `digests_required` (a self-test requires the two lists to agree).
pub const PROMOTION_PACKAGE_CHECKS: &[&str] = &[
    "pipeline_bundle_qualification",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
    "pipeline_restore_drill",
];

/// The local restore drill's check id. Its result always carries
/// [`LOCAL_RESTORE_BLOCKER_LABEL`]: the artifact restore it proves is a local
/// directory copy.
pub const RESTORE_DRILL_CHECK_ID: &str = "pipeline_restore_drill";
/// The safe blocker a local restore drill carries
/// (`scripts/operator/pipeline.py`'s `RESTORE_SAFE_BLOCKER`).
pub const LOCAL_RESTORE_BLOCKER_LABEL: &str = "filesystem_restore_local_only";
/// The operator-run remote restore drill (`pipeline.py promote
/// remote-restore`). A passing result of it, with no blocker of its own, from
/// the drill's code revision and naming the drill's package, discharges the
/// drill's [`LOCAL_RESTORE_BLOCKER_LABEL`] in [`evaluate_promotion`] (spec
/// 2026-10-08 B-D4), and nothing else.
pub const REMOTE_RESTORE_CHECK_ID: &str = "pipeline_remote_restore";

/// Whether `value` is a safe label, `^[a-z0-9_]{1,64}$`: the only text an
/// operational answer, log line, or stored refusal may carry. Public so that
/// ingest's admin routes answer a store's refusal only when it is one.
pub fn is_safe_label(value: &str) -> bool {
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
    /// none names one, they name more than one, or a result breaks the
    /// package rule of [`PROMOTION_PACKAGE_CHECKS`] (a candidate check
    /// without the package, a mechanics check with one): the decision then
    /// names no package, whatever else it says.
    ///
    /// A decision that is blocked only by a missing, failed, or stale result
    /// can still carry `Some(package)`: those blockers do not touch the
    /// package rule. So `package.is_some()` is not readiness. `ready` is.
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
/// (a package is its three digests together).
///
/// The results of one qualification are the output of one `qualify` run
/// (review round 1 of #1240, point 5): every result of a check outside
/// [`PROMOTION_ONLY_CHECKS`] must carry the same run id, else the decision is
/// blocked with `qualification_evidence_mixed_run`. So a set cannot take a
/// mechanics result from one run and a candidate result from another run of
/// the same revision. The three promotion-only results come from the
/// production assembly, not from `qualify`, and each may carry its own run
/// id. The run id is a field of the signed result, so the rule needs no
/// other field. It holds wherever this function runs: the qualification, the
/// activation, and the rollback.
///
/// A qualification run names exactly one package (P5-D15), and this
/// function enforces which results name it, in both directions, because the
/// decision gates `qualify_bundle` and so activation, for evidence that did
/// not pass through `pipeline.py`. Each check in [`PROMOTION_PACKAGE_CHECKS`]
/// (the four that test the candidate) must carry all three package digests:
/// one that carries fewer, none included, adds
/// `qualification_evidence_package_missing:<check_id>`. Every other check
/// (a mechanics check, or a promotion-only one) must carry none: one that
/// carries any digest adds `qualification_evidence_package_unexpected:<check_id>`.
/// With either blocker the decision is not ready and `package` is `None`: no
/// package is the one the candidate checks tested. A decision is therefore
/// ready only when the four candidate checks name one package and no other
/// result names any, and then `package` is that package.
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
        if item.check.validate().is_err()
            || item.maximum_age_seconds == 0
            || maximum_age(item).is_none()
        {
            return Err(format!("{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{check_id}"));
        }
        if by_id.insert(check_id, item).is_some() {
            return Err("qualification_evidence_duplicate".to_string());
        }
    }
    let mut blockers = Vec::new();
    let mut package_rule_broken = false;
    let local_restore_discharged = remote_restore_discharges_local_restore(&by_id, now);
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
        // The one exception (spec 2026-10-08 B-D4): a matching remote
        // restore discharges the restore drill's local-copy blocker.
        blockers.extend(
            item.check
                .safe_blockers
                .iter()
                .filter(|label| {
                    !(local_restore_discharged
                        && *check_id == RESTORE_DRILL_CHECK_ID
                        && label.as_str() == LOCAL_RESTORE_BLOCKER_LABEL)
                })
                .map(|label| format!("{label}:{check_id}")),
        );
        // Which results name the package (P5-D15): the candidate checks all
        // three digests, every other check none.
        let digests_named = [
            &item.check.package_hash,
            &item.check.configuration_digest,
            &item.check.dependency_digest,
        ]
        .map(|digest| digest.is_some());
        if PROMOTION_PACKAGE_CHECKS.contains(check_id) {
            if !digests_named.iter().all(|named| *named) {
                blockers.push(format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL}:{check_id}"
                ));
                package_rule_broken = true;
            }
        } else if digests_named.iter().any(|named| *named) {
            blockers.push(format!(
                "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:{check_id}"
            ));
            package_rule_broken = true;
        }
        let maximum_age = maximum_age(item)
            .ok_or_else(|| format!("{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{check_id}"))?;
        if item.check.observed_at > now || now - item.check.observed_at > maximum_age {
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
    // One run for every result that `qualify` produces; a promotion-only
    // result may come from another run.
    let runs = evidence
        .iter()
        .filter(|item| !PROMOTION_ONLY_CHECKS.contains(&item.check.check_id.as_str()))
        .map(|item| item.check.run_id.as_str())
        .collect::<BTreeSet<_>>();
    if runs.len() > 1 {
        blockers.push(QUALIFICATION_EVIDENCE_MIXED_RUN_LABEL.to_string());
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
        package: (packages.len() == 1 && !package_rule_broken)
            .then(|| packages.first().cloned())
            .flatten(),
    })
}

/// Whether the evidence holds a `pipeline_remote_restore` result that
/// discharges the restore drill's [`LOCAL_RESTORE_BLOCKER_LABEL`]: it passed,
/// carries no safe blocker of its own, is current at `now`, comes from the
/// drill's code revision, and names exactly the drill's package (all three
/// digests present and equal). Anything less, or either result missing, is
/// `false`, and the blocker stays.
fn remote_restore_discharges_local_restore(
    by_id: &BTreeMap<&str, &DrillEvidence>,
    now: DateTime<Utc>,
) -> bool {
    let (Some(remote), Some(drill)) = (
        by_id.get(REMOTE_RESTORE_CHECK_ID),
        by_id.get(RESTORE_DRILL_CHECK_ID),
    ) else {
        return false;
    };
    let current = maximum_age(remote).is_some_and(|maximum_age| {
        remote.check.observed_at <= now && now - remote.check.observed_at <= maximum_age
    });
    let names_every_digest = |check: &PipelineCheckResult| {
        check.package_hash.is_some()
            && check.configuration_digest.is_some()
            && check.dependency_digest.is_some()
    };
    remote.check.status == PipelineCheckStatus::Pass
        && remote.check.safe_blockers.is_empty()
        && current
        && remote.check.code_revision_hash == drill.check.code_revision_hash
        && names_every_digest(&remote.check)
        && names_every_digest(&drill.check)
        && PromotionPackage::of(&remote.check) == PromotionPackage::of(&drill.check)
}

/// `item`'s maximum age as a duration; `None` when it does not fit one
/// (above `i64::MAX` seconds, or above chrono's millisecond range, where
/// `Duration::seconds` would panic; fix round 2).
fn maximum_age(item: &DrillEvidence) -> Option<Duration> {
    i64::try_from(item.maximum_age_seconds)
        .ok()
        .and_then(Duration::try_seconds)
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

/// The public keys of `keys` by key id: a trust store holds no key whose id
/// is not a safe identifier, whose bytes are not base64url of a 32-byte
/// Ed25519 public key, or whose id repeats. `refused` is the label of the
/// store that asks (a package store and a check store each use their own).
fn trusted_public_keys(
    keys: impl IntoIterator<Item = TrustedBundleKey>,
    refused: &str,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut trusted = BTreeMap::new();
    for key in keys {
        if !is_safe_identifier(&key.key_id) {
            return Err(refused.to_string());
        }
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(key.public_key_base64url)
            .map_err(|_| refused.to_string())?;
        if bytes.len() != 32 || trusted.insert(key.key_id, bytes).is_some() {
            return Err(refused.to_string());
        }
    }
    Ok(trusted)
}

impl BundlePackageTrustStore {
    pub fn new(keys: impl IntoIterator<Item = TrustedBundleKey>) -> Result<Self, String> {
        Ok(Self {
            keys: trusted_public_keys(keys, PACKAGE_SIGNER_UNTRUSTED_LABEL)?,
        })
    }

    /// Whether this store holds a key under `key_id`: what a route asks of a
    /// stored package, which carries no signature to verify again, with the
    /// signing key id its qualification recorded (final fix wave G11). A key
    /// id that the store no longer holds is the unknown signer of `verify`
    /// (`bundle_package_signer_untrusted`).
    pub fn holds_key_id(&self, key_id: &str) -> bool {
        self.keys.contains_key(key_id)
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

/// The signature of a [`PipelineCheckAttestation`]: the algorithm
/// ([`PACKAGE_SIGNATURE_ALGORITHM`]), the id of the key that made it, the
/// hash that was signed ([`attestation_hash`], recorded so a reader sees
/// what the signature claims to cover; verification recomputes it and
/// requires the two to be equal), and the signature itself, base64url. Its
/// own type, not [`BundlePackageSignature`]: a package signature has a
/// different field for the hash it covers, and a reader that takes one for
/// the other fails to parse it. No other field is accepted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckAttestationSignature {
    pub algorithm: String,
    pub key_id: String,
    pub attestation_hash: String,
    pub signature_base64url: String,
}

/// A check result and the signature of the key that vouches for it (P5-D13).
/// The result stays exactly [`PIPELINE_CHECK_RESULT_SCHEMA`]; this envelope
/// adds the one thing a result file lacks, who made it. The signature covers
/// every other field ([`attestation_hash`]): the result, the maximum age
/// the signer gives its evidence, and the two digests of the corpus run that
/// produced it. Any change after signing breaks the signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PipelineCheckAttestation {
    pub schema: String,
    pub result: PipelineCheckResult,
    pub maximum_age_seconds: u64,
    pub corpus_digest: Option<String>,
    pub input_digest: Option<String>,
    pub signature: CheckAttestationSignature,
}

impl PipelineCheckAttestation {
    /// What an attestation must be whatever its signature says; each failure
    /// is [`CHECK_ATTESTATION_INVALID_LABEL`]. Both [`sign_check_result`]
    /// and [`CheckResultTrustStore::verify_all`] read it, so nothing is
    /// signed that verification would refuse for its shape.
    fn validate_shape(&self) -> Result<(), String> {
        if self.schema != PIPELINE_CHECK_ATTESTATION_SCHEMA
            || self.result.validate().is_err()
            || self.maximum_age_seconds == 0
            || [&self.corpus_digest, &self.input_digest]
                .into_iter()
                .flatten()
                .any(|digest| !is_sha256(digest))
        {
            return Err(CHECK_ATTESTATION_INVALID_LABEL.to_string());
        }
        Ok(())
    }
}

/// The hash an attestation's signature covers: every field but the
/// signature, as canonical JSON.
fn attestation_hash(attestation: &PipelineCheckAttestation) -> Result<String, String> {
    let result = serde_json::to_value(&attestation.result)
        .map_err(|_| CHECK_ATTESTATION_INVALID_LABEL.to_string())?;
    evidence_hash(&serde_json::json!({
        "schema": attestation.schema,
        "result": result,
        "maximum_age_seconds": attestation.maximum_age_seconds,
        "corpus_digest": attestation.corpus_digest,
        "input_digest": attestation.input_digest,
    }))
    .map_err(|_| CHECK_ATTESTATION_INVALID_LABEL.to_string())
}

/// Signs `result` with a PKCS#8-encoded Ed25519 private key, as
/// [`sign_bundle_package`] signs a package: the attestation is built with an
/// empty signature, [`attestation_hash`] is computed over it, and the ASCII
/// bytes of that hash are signed (the same algorithm label and base64url
/// encoding). `corpus_digest` and `input_digest` are the digests of the
/// corpus run behind the result, `None` for a check that has none. Refuses
/// what verification would refuse for its shape (`check_attestation_invalid`)
/// and a key that is not PKCS#8 Ed25519 (`check_attestation_signing_key_invalid`).
pub fn sign_check_result(
    result: PipelineCheckResult,
    maximum_age_seconds: u64,
    corpus_digest: Option<String>,
    input_digest: Option<String>,
    key_id: &str,
    pkcs8: &[u8],
) -> anyhow::Result<PipelineCheckAttestation> {
    let key_pair = Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|_| anyhow::anyhow!("check_attestation_signing_key_invalid"))?;
    if !is_safe_identifier(key_id) {
        anyhow::bail!(CHECK_ATTESTATION_INVALID_LABEL);
    }
    let mut attestation = PipelineCheckAttestation {
        schema: PIPELINE_CHECK_ATTESTATION_SCHEMA.to_string(),
        result,
        maximum_age_seconds,
        corpus_digest,
        input_digest,
        signature: CheckAttestationSignature {
            algorithm: PACKAGE_SIGNATURE_ALGORITHM.to_string(),
            key_id: key_id.to_string(),
            attestation_hash: String::new(),
            signature_base64url: String::new(),
        },
    };
    attestation
        .validate_shape()
        .map_err(|label| anyhow::anyhow!(label))?;
    let hash = attestation_hash(&attestation).map_err(|label| anyhow::anyhow!(label))?;
    let signature = key_pair.sign(hash.as_bytes());
    attestation.signature.attestation_hash = hash;
    attestation.signature.signature_base64url =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.as_ref());
    Ok(attestation)
}

/// The keys whose signatures make a check result count toward a
/// qualification. A different type from [`BundlePackageTrustStore`] on
/// purpose (P5-D13): a route holds both, and a key trusted to sign packages
/// is not thereby trusted to vouch for check results, nor the reverse.
#[derive(Debug, Clone, Default)]
pub struct CheckResultTrustStore {
    keys: BTreeMap<String, Vec<u8>>,
}

/// What [`CheckResultTrustStore::verify_all`] returns for a set of
/// attestations whose signatures all verified: the results (each with the
/// maximum age its signer gave it), and the two digests that name the
/// corpora and inputs behind them (P5-D14). `corpus_digest` is the canonical
/// hash of the object `{check_id: corpus_digest}` over every attestation
/// that carries one, `input_digest` the same for `input_digest`; with none
/// carrying one, each is the hash of the empty object.
#[derive(Debug, Clone)]
pub struct VerifiedEvidence {
    pub evidence: Vec<DrillEvidence>,
    pub corpus_digest: String,
    pub input_digest: String,
}

impl CheckResultTrustStore {
    /// Takes the keys [`BundlePackageTrustStore::new`] takes and refuses the
    /// ones it refuses, under [`CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL`].
    pub fn new(keys: impl IntoIterator<Item = TrustedBundleKey>) -> Result<Self, String> {
        Ok(Self {
            keys: trusted_public_keys(keys, CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL)?,
        })
    }

    /// Verifies every attestation, in order, and refuses the whole set at
    /// the first that fails. For each: its shape (`check_attestation_invalid`:
    /// the schema, the result, a nonzero maximum age, each digest it carries
    /// a `sha256:` digest); then its signature algorithm and key id shape
    /// (`check_attestation_signature_invalid`); then that the key id is one
    /// this store holds (`check_attestation_signer_untrusted`); then that
    /// the hash it records is [`attestation_hash`] and that the key's
    /// signature over that hash verifies
    /// (`check_attestation_signature_invalid`); then that its maximum age is
    /// at most [`QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS`]
    /// (`bundle_qualification_evidence_age_above_ceiling`, the label the
    /// bare [`PipelineQualificationStore::qualify_bundle`] uses for it). The
    /// signer chooses the age, so this bound holds on every path that
    /// verifies attestations, the activation and the rollback included; an
    /// age chrono cannot represent is above it. A second attestation for one
    /// check is `check_attestation_invalid`: it would otherwise hide a digest
    /// from the two this returns.
    ///
    /// Nothing here judges whether the results qualify a bundle
    /// ([`evaluate_promotion`] does, over `evidence`).
    pub fn verify_all(
        &self,
        attestations: &[PipelineCheckAttestation],
    ) -> Result<VerifiedEvidence, String> {
        let mut seen = BTreeSet::new();
        let mut corpus_digests = BTreeMap::new();
        let mut input_digests = BTreeMap::new();
        let mut evidence = Vec::with_capacity(attestations.len());
        for attestation in attestations {
            self.verify_one(attestation)?;
            let check_id = &attestation.result.check_id;
            if !seen.insert(check_id.clone()) {
                return Err(CHECK_ATTESTATION_INVALID_LABEL.to_string());
            }
            if let Some(digest) = &attestation.corpus_digest {
                corpus_digests.insert(check_id.clone(), digest.clone());
            }
            if let Some(digest) = &attestation.input_digest {
                input_digests.insert(check_id.clone(), digest.clone());
            }
            evidence.push(DrillEvidence {
                check: attestation.result.clone(),
                maximum_age_seconds: attestation.maximum_age_seconds,
            });
        }
        let digest_of = |digests: &BTreeMap<String, String>| {
            evidence_hash(&serde_json::json!(digests))
                .map_err(|_| CHECK_ATTESTATION_INVALID_LABEL.to_string())
        };
        Ok(VerifiedEvidence {
            corpus_digest: digest_of(&corpus_digests)?,
            input_digest: digest_of(&input_digests)?,
            evidence,
        })
    }

    fn verify_one(&self, attestation: &PipelineCheckAttestation) -> Result<(), String> {
        attestation.validate_shape()?;
        let signature = &attestation.signature;
        if signature.algorithm != PACKAGE_SIGNATURE_ALGORITHM
            || !is_safe_identifier(&signature.key_id)
        {
            return Err(CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL.to_string());
        }
        let public_key = self
            .keys
            .get(&signature.key_id)
            .ok_or_else(|| CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL.to_string())?;
        let hash = attestation_hash(attestation)?;
        if signature.attestation_hash != hash {
            return Err(CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL.to_string());
        }
        let signature_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&signature.signature_base64url)
            .map_err(|_| CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL.to_string())?;
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(hash.as_bytes(), &signature_bytes)
            .map_err(|_| CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL.to_string())?;
        // Final fix wave (K1): the signer's maximum age, bounded on every
        // path that verifies, not only in `qualify_bundle`.
        if attestation.maximum_age_seconds > QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS {
            return Err(QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL.to_string());
        }
        Ok(())
    }
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
/// ([`PipelineBundleQualification`] already reports those). Ingest's admin
/// routes derive it from the configuration the process started with
/// (`infrastructure_profile_from_state`, P5-D21); nothing here reads a
/// request.
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
/// field must be a `sha256:` digest. `qualify_bundle` binds
/// `configuration_digest`, `runtime_dependency_digest`, `evidence_hash` and
/// `code_revision_hash` to the package, the profile and the evidence.
/// `corpus_digest` and `input_digest` are the server's values when the
/// qualification comes through `qualify_bundle_attested` (P5-D14): it
/// computes both from the verified attestations
/// ([`VerifiedEvidence`]) and `qualify_bundle` then requires the metadata to
/// carry exactly those. Through the bare `qualify_bundle` with no verified
/// digests, they are the caller's input, checked for shape and recorded.
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
/// bundle_id, code_revision_hash)` (V112, P5-D10).
/// `pipeline_bundle_qualifications` is append-only in the database (V107): a
/// repeat call with the exact same inputs answers the existing row; a repeat
/// call with different metadata for the same bundle and revision is refused
/// as a conflict, never silently overwritten. The same bundle on another
/// revision records another row. The activation gate reads them
/// (`activate_qualified_bundle_in`).
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
    /// call for the same bundle and code revision with different metadata
    /// (`bundle_qualification_identity_conflict`).
    ///
    /// A refused call leaves no qualification row. It can leave the package:
    /// when every check above the store passed, `register_bundle` commits the
    /// package row and its four policy status rows (and the tenant's row in
    /// `trace_tenants`) in a transaction of its own, before the transaction
    /// that inserts the qualification. So a call that fails after that point
    /// (the identity conflict, or a database error in the qualification's
    /// transaction) leaves those rows, and a refusal before it leaves none. A
    /// registered package with no qualification row is not activated: the
    /// gate requires the qualification.
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
    ///   A result's maximum age above `QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS`
    ///   (seven days) is refused (`bundle_qualification_evidence_age_above_ceiling`):
    ///   a signer chooses the age its attestation carries, and the bare API
    ///   takes the results as its caller gives them, so the server bounds it
    ///   either way.
    ///   The decision must be ready (`bundle_qualification_promotion_not_ready`),
    ///   its evidence hash must be the metadata's
    ///   (`bundle_qualification_evidence_mismatch`), its one code revision the
    ///   metadata's (`bundle_qualification_code_revision_mismatch`), and its
    ///   one package `signed.package` (`bundle_qualification_package_mismatch`).
    ///
    /// The evidence is evaluated on every call, the repeat of a call that
    /// already recorded its row included: a repeat made after its evidence
    /// went stale is refused as `bundle_qualification_promotion_not_ready`,
    /// not answered with the existing row (fail closed; fix round 2).
    ///
    /// `verified` is what binds `metadata.corpus_digest` and
    /// `metadata.input_digest` (P5-D14; Zaki's re-review of #1166, Minor). A
    /// check result carries only the hash of its evidence, not the corpus
    /// report or the input it ran on, so nothing here can recompute either
    /// digest from `evidence`; signed attestations carry them, and
    /// [`CheckResultTrustStore::verify_all`] computes the two digests from
    /// the attestations it verified ([`VerifiedEvidence`]). With
    /// `Some((corpus_digest, input_digest))` the metadata's two values must
    /// equal those (`bundle_qualification_corpus_mismatch`,
    /// `bundle_qualification_input_mismatch`; the shape check comes first).
    /// With `None` they are the caller's input, checked for shape and
    /// recorded, as `evidence` is: a result file is a file anyone who can
    /// write one can make.
    ///
    /// The path a route takes is [`Self::qualify_bundle_attested`], which
    /// verifies signed results, derives the metadata's digests itself, and
    /// calls this with `Some`. This bare API with `None` is not reachable
    /// from a route: no ingest route calls it, and a caller that passes
    /// `None` vouches for its own evidence.
    ///
    /// A recorded qualification covers the signed package and its
    /// dependency profile, not the deployment's bindings (merge review M2).
    /// `main`'s gate configuration (`pipeline_runtime_main_gate_config_mismatch`)
    /// and the pipeline credit issuer (`pipeline_credit_issuer_principal_missing`)
    /// are startup checks of every bundle a routed or drained tenant may run,
    /// not terms of this record. The activation and rollback routes keep both
    /// for the bundle they select: they run `PipelineService::check_runnable_package`
    /// on it before the store's gate (`activate_tenant`, `rollback_bundle`).
    #[allow(clippy::too_many_arguments)]
    pub async fn qualify_bundle(
        &self,
        tenant_id: &str,
        signed: &SignedBundlePackage,
        trust: &BundlePackageTrustStore,
        metadata: &BundleQualificationMetadata,
        dependencies: &ProductionDependencyProfile,
        evidence: &[DrillEvidence],
        verified: Option<(&str, &str)>,
    ) -> Result<BundleQualificationRecord, DatabaseError> {
        trust.verify(signed).map_err(DatabaseError::Constraint)?;
        validate_production_package(&signed.package).map_err(DatabaseError::Constraint)?;
        if !package_configuration_is_qualifiable(&signed.package) {
            return Err(DatabaseError::Constraint(
                PIPELINE_BUNDLE_CONFIGURATION_NOT_QUALIFIABLE_LABEL.to_string(),
            ));
        }
        metadata.validate().map_err(DatabaseError::Constraint)?;
        if let Some((corpus_digest, input_digest)) = verified {
            if metadata.corpus_digest != corpus_digest {
                return Err(DatabaseError::Constraint(
                    "bundle_qualification_corpus_mismatch".to_string(),
                ));
            }
            if metadata.input_digest != input_digest {
                return Err(DatabaseError::Constraint(
                    "bundle_qualification_input_mismatch".to_string(),
                ));
            }
        }
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
        // A result's maximum age is its signer's or its caller's choice (fix
        // round 2), so a qualification accepts none above the server's
        // ceiling.
        if evidence
            .iter()
            .any(|item| item.maximum_age_seconds > QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS)
        {
            return Err(DatabaseError::Constraint(
                QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL.to_string(),
            ));
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
             ON CONFLICT (tenant_id, bundle_id, code_revision_hash) DO NOTHING",
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
                  WHERE tenant_id = $1 AND bundle_id = $2 AND code_revision_hash = $3",
                &[
                    &tenant_id,
                    &signed.package.bundle_id,
                    &metadata.code_revision_hash,
                ],
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

    /// Qualifies `signed` from signed check results only (P5-D13, P5-D14): the
    /// path a route takes. `check_trust` verifies every attestation
    /// ([`CheckResultTrustStore::verify_all`]: `check_attestation_invalid`,
    /// `check_attestation_signer_untrusted`,
    /// `check_attestation_signature_invalid`, and a maximum age above the
    /// ceiling, `bundle_qualification_evidence_age_above_ceiling`), so a
    /// result that no trusted key signed, or that changed after signing,
    /// qualifies nothing. The
    /// promotion is evaluated over the verified results at the current time
    /// ([`evaluate_promotion`]) for its evidence hash; the metadata is then
    /// built here, never taken from the caller: `corpus_digest` and
    /// `input_digest` are the two digests of the verified attestations,
    /// `configuration_digest` the signed package's own,
    /// `runtime_dependency_digest` the profile's, `evidence_hash` the
    /// promotion's, and `code_revision_hash` the argument (the revision the
    /// deployed code was built from; the promotion must name the same one,
    /// `bundle_qualification_code_revision_mismatch`). Everything else is
    /// [`Self::qualify_bundle`]'s, which this calls with the verified digests:
    /// the package trust and production shape, the dependency profile, a
    /// ready promotion for this package, the age ceiling, and the
    /// append-only record. A refused call leaves no qualification row; the
    /// package row and its four policy status rows that `register_bundle`
    /// committed stay (see [`Self::qualify_bundle`]). A refusal of this
    /// function's own checks comes before `register_bundle` and leaves none.
    #[allow(clippy::too_many_arguments)]
    pub async fn qualify_bundle_attested(
        &self,
        tenant_id: &str,
        signed: &SignedBundlePackage,
        package_trust: &BundlePackageTrustStore,
        check_trust: &CheckResultTrustStore,
        dependencies: &ProductionDependencyProfile,
        attestations: &[PipelineCheckAttestation],
        code_revision_hash: &str,
    ) -> Result<BundleQualificationRecord, DatabaseError> {
        let verified = check_trust
            .verify_all(attestations)
            .map_err(DatabaseError::Constraint)?;
        let digests = package_digests(&signed.package).map_err(DatabaseError::Constraint)?;
        let promotion = evaluate_promotion(&verified.evidence, Utc::now())
            .map_err(DatabaseError::Constraint)?;
        let metadata = BundleQualificationMetadata {
            corpus_digest: verified.corpus_digest.clone(),
            input_digest: verified.input_digest.clone(),
            configuration_digest: digests.configuration_digest,
            code_revision_hash: code_revision_hash.to_string(),
            runtime_dependency_digest: dependencies
                .runtime_identity_digest()
                .map_err(DatabaseError::Constraint)?,
            evidence_hash: promotion.evidence_hash,
        };
        self.qualify_bundle(
            tenant_id,
            signed,
            package_trust,
            &metadata,
            dependencies,
            &verified.evidence,
            Some((&verified.corpus_digest, &verified.input_digest)),
        )
        .await
    }

    /// The tenant's qualification of `bundle_id` on `code_revision_hash`
    /// (one row of the key `(tenant_id, bundle_id, code_revision_hash)`);
    /// `None` when there is none.
    pub async fn qualification(
        &self,
        tenant_id: &str,
        bundle_id: &str,
        code_revision_hash: &str,
    ) -> Result<Option<BundleQualificationRecord>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await?;
        let row = tx
            .query_opt(
                "SELECT bundle_id, package_hash, signing_key_id, signature_hash,
                        corpus_digest, input_digest, configuration_digest,
                        code_revision_hash, runtime_dependency_digest,
                        evidence_hash, qualified_at
                   FROM pipeline_bundle_qualifications
                  WHERE tenant_id = $1 AND bundle_id = $2 AND code_revision_hash = $3",
                &[&tenant_id, &bundle_id, &code_revision_hash],
            )
            .await?;
        tx.commit().await?;
        Ok(row.as_ref().map(qualification_from_row))
    }

    /// Selects `bundle_id` as the tenant's active bundle inside `tx`, when
    /// every term of the activation gate holds (P5-D9); returns the bundle
    /// that was active before. Fails closed with the first failing term's
    /// label and writes nothing. The caller holds the tenant's routing lock
    /// and writes the routing row and the event in the same transaction.
    ///
    /// The terms, in order, each with its refusal:
    ///
    /// 1. `promotion` is ready and carries no blocker
    ///    (`bundle_activation_promotion_not_ready`), and was evaluated at most
    ///    [`ACTIVATION_PROMOTION_MAX_AGE_SECONDS`] before `now` and not after
    ///    it (`bundle_activation_promotion_stale`).
    /// 2. `runtime_code_revision_hash` is a `sha256:` digest
    ///    (`bundle_runtime_revision_unknown`) and is the promotion's one code
    ///    revision (`bundle_runtime_revision_mismatch`).
    /// 3. The tenant has a qualification of the bundle
    ///    (`bundle_qualification_missing`), on that revision
    ///    (`bundle_runtime_revision_mismatch`).
    /// 4. The stored package loads and validates (`bundle_package_missing`,
    ///    which is also the label of a tampered package; a database error on
    ///    the read returns as it is: `stored_package_or_refusal`), and the
    ///    promotion names exactly its three digests
    ///    (`bundle_activation_package_mismatch`).
    /// 5. `dependencies` was built for this bundle
    ///    (`bundle_qualification_profile_mismatch`), has no blocker (its
    ///    first blocker's label), and its runtime identity is the one the
    ///    qualification recorded (`runtime_dependency_identity_mismatch`).
    /// 6. The bundle's four policies are runnable (`bundle_policy_not_runnable`),
    ///    by `lock_runnable_policy`, the definition every phase commit uses:
    ///    each status row is held `FOR SHARE` to the end of `tx`, so a
    ///    suspension waits for this activation or is seen by it.
    ///
    /// Then the active bundle row is locked `FOR UPDATE` and set to the
    /// bundle (V112's grant; the only statement of the ingest runtime that
    /// updates it), with `selected_at` the database clock at the write
    /// (`clock_timestamp()`, as the routing row's and the event's
    /// `recorded_at`; `write_routing_in`). Lock order inside `tx`, after the caller's exclusive
    /// routing lock: the four policy rows (`FOR SHARE`), then the active
    /// bundle row.
    ///
    /// The promotion is the caller's: an activation route evaluates it over
    /// verified check results just before the call. This function reads no
    /// other tenant and takes nothing from the promotion but the terms above.
    ///
    /// Crate-private, and reached only through
    /// `PipelineActivationStore::activate_tenant` and `rollback_bundle`, which
    /// take the routing lock first and write the routing row and the event in
    /// the same transaction: a switch through the gate alone would leave no
    /// event and wait for no receipt.
    pub(crate) async fn activate_qualified_bundle_in(
        tx: &Transaction<'_>,
        tenant_id: &str,
        bundle_id: &str,
        promotion: &PromotionDecision,
        runtime_code_revision_hash: &str,
        dependencies: &ProductionDependencyProfile,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, DatabaseError> {
        let refuse = |label: &str| Err(DatabaseError::Constraint(label.to_string()));
        if !promotion.ready || !promotion.safe_blockers.is_empty() {
            return refuse(ACTIVATION_PROMOTION_NOT_READY_LABEL);
        }
        if promotion.evaluated_at > now
            || now - promotion.evaluated_at
                > Duration::seconds(ACTIVATION_PROMOTION_MAX_AGE_SECONDS)
        {
            return refuse(ACTIVATION_PROMOTION_STALE_LABEL);
        }
        if !is_sha256(runtime_code_revision_hash) {
            return refuse(PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL);
        }
        if promotion.code_revision_hash.as_deref() != Some(runtime_code_revision_hash) {
            return refuse(PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL);
        }
        let qualified = tx
            .query(
                "SELECT code_revision_hash, runtime_dependency_digest
                   FROM pipeline_bundle_qualifications
                  WHERE tenant_id = $1 AND bundle_id = $2",
                &[&tenant_id, &bundle_id],
            )
            .await?;
        if qualified.is_empty() {
            return refuse(PACKAGE_QUALIFICATION_MISSING_LABEL);
        }
        let Some(row) = qualified
            .iter()
            .find(|row| row.get::<_, String>("code_revision_hash") == runtime_code_revision_hash)
        else {
            return refuse(PACKAGE_RUNTIME_REVISION_MISMATCH_LABEL);
        };
        let package = stored_package_or_refusal(
            load_bundle_from_transaction(tx, tenant_id, bundle_id).await,
        )?;
        let digests = package_digests(&package).map_err(DatabaseError::Constraint)?;
        if !promotion
            .package
            .as_ref()
            .is_some_and(|named| named.is(&digests))
        {
            return refuse(ACTIVATION_PACKAGE_MISMATCH_LABEL);
        }
        if dependencies.bundle.bundle_id != bundle_id
            || dependencies.bundle.dependency_digest != digests.dependency_digest
        {
            return refuse("bundle_qualification_profile_mismatch");
        }
        if let Some(blocker) = dependencies.blockers().into_iter().next() {
            return Err(DatabaseError::Constraint(blocker));
        }
        if row.get::<_, String>("runtime_dependency_digest")
            != dependencies
                .runtime_identity_digest()
                .map_err(DatabaseError::Constraint)?
        {
            return refuse("runtime_dependency_identity_mismatch");
        }
        // Four runnable policies, each row held FOR SHARE to the end of the
        // transaction, so a suspension waits for this activation or is seen
        // by it (Task 5's guard).
        for phase in [Phase::Admission, Phase::Review, Phase::Score, Phase::Settle] {
            if !lock_runnable_policy(tx, tenant_id, bundle_id, phase).await? {
                return refuse(PIPELINE_POLICY_NOT_RUNNABLE_LABEL);
            }
        }
        let previous: Option<String> = tx
            .query_opt(
                "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1 FOR UPDATE",
                &[&tenant_id],
            )
            .await?
            .map(|row| row.get(0));
        let selected = tx
            .execute(
                "INSERT INTO pipeline_active_bundles (tenant_id, bundle_id, selected_at)
                 SELECT $1, bundle_id, clock_timestamp() FROM pipeline_bundle_packages
                  WHERE tenant_id = $1 AND bundle_id = $2
                 ON CONFLICT (tenant_id) DO UPDATE
                    SET bundle_id = EXCLUDED.bundle_id, selected_at = EXCLUDED.selected_at",
                &[&tenant_id, &bundle_id],
            )
            .await?;
        // The package was loaded above in this transaction, and a package is
        // never deleted on its own; a count other than one fails closed.
        if selected != 1 {
            return refuse(PIPELINE_BUNDLE_MISSING_LABEL);
        }
        Ok(previous)
    }
}

/// The gate's reading of `load_bundle_from_transaction`'s result. A stored
/// package that is gone, or that no longer validates (a tampered package,
/// BND-002: `bundle_package_invalid` from the load), refuses with
/// `bundle_package_missing`. Any other error is a database error and returns
/// as it is, never as a refusal label, so a caller reports it as an internal
/// error rather than as a fact about the package.
fn stored_package_or_refusal(
    loaded: Result<Option<BundlePackage>, DatabaseError>,
) -> Result<BundlePackage, DatabaseError> {
    match loaded {
        Ok(Some(package)) => Ok(package),
        Ok(None) => Err(DatabaseError::Constraint(
            PIPELINE_BUNDLE_MISSING_LABEL.to_string(),
        )),
        Err(DatabaseError::Serialization(label)) if label == PIPELINE_BUNDLE_INVALID_LABEL => Err(
            DatabaseError::Constraint(PIPELINE_BUNDLE_MISSING_LABEL.to_string()),
        ),
        Err(error) => Err(error),
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
        return Err(QUALIFICATION_PROMOTION_NOT_READY_LABEL);
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

    /// The gate reads the package load as a refusal only for a package that
    /// is gone or that does not validate (a tampered package, BND-002); a
    /// database error returns as it is, never as `bundle_package_missing`.
    #[test]
    fn the_gate_refuses_a_missing_or_invalid_package_and_returns_a_database_error() {
        let refused_as_missing = |result: Result<BundlePackage, DatabaseError>| matches!(result, Err(DatabaseError::Constraint(label)) if label == PIPELINE_BUNDLE_MISSING_LABEL);
        assert!(refused_as_missing(stored_package_or_refusal(Ok(None))));
        assert!(refused_as_missing(stored_package_or_refusal(Err(
            DatabaseError::Serialization(PIPELINE_BUNDLE_INVALID_LABEL.to_string())
        ))));
        assert!(matches!(
            stored_package_or_refusal(Err(DatabaseError::Pool("pool_timeout".to_string()))),
            Err(DatabaseError::Pool(_))
        ));
        assert!(matches!(
            stored_package_or_refusal(Err(DatabaseError::Query("statement_failed".to_string()))),
            Err(DatabaseError::Query(_))
        ));
        assert!(matches!(
            stored_package_or_refusal(Err(DatabaseError::Serialization(
                "pipeline_routing_state_invalid".to_string()
            ))),
            Err(DatabaseError::Serialization(label)) if label == "pipeline_routing_state_invalid"
        ));
        let package = minimal_test_package();
        assert_eq!(
            stored_package_or_refusal(Ok(Some(package.clone()))).unwrap(),
            package
        );
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

    /// `scripts/operator/fixtures/pipeline-package-digests-vector.json` is
    /// the cross-language vector for [`package_digests`]: `pipeline.py
    /// promote` recomputes all three digests in Python from the same file
    /// (`test_package_digests_match_the_rust_vector`). This test is what holds
    /// that file to the Rust rule, so the Python side cannot pass against an
    /// oracle of its own making. Two instruments pin the instrument order.
    #[test]
    fn the_cross_language_package_digest_vector_follows_the_rust_rule() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scripts/operator/fixtures/pipeline-package-digests-vector.json");
        let vector: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("vector file reads"))
                .expect("vector file parses");
        let package: BundlePackage =
            serde_json::from_value(vector["package"].clone()).expect("vector package loads");
        assert_eq!(package.manifest.instruments.len(), 2);
        assert_eq!(package.manifest.bundle_id().unwrap(), package.bundle_id);
        let digests = package_digests(&package).expect("vector digests compute");
        assert_eq!(digests.package_hash, vector["package_hash"]);
        assert_eq!(digests.configuration_digest, vector["configuration_digest"]);
        assert_eq!(digests.dependency_digest, vector["dependency_digest"]);
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

    /// `sample_check` in the shape a real qualification run has (P5-D15): a
    /// check in [`PROMOTION_PACKAGE_CHECKS`] names the one candidate package,
    /// every other check names none.
    fn promotion_check(
        check_id: &str,
        status: PipelineCheckStatus,
        observed_at: DateTime<Utc>,
    ) -> PipelineCheckResult {
        let mut check = sample_check(check_id, status, observed_at);
        if PROMOTION_PACKAGE_CHECKS.contains(&check_id) {
            name_package(&mut check, "candidate");
        }
        check
    }

    #[test]
    fn promotion_requires_current_passing_evidence_for_every_check() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        assert!(evaluate_promotion(&evidence, now).unwrap().ready);

        let reversed = evidence.iter().rev().cloned().collect::<Vec<_>>();
        assert_eq!(
            evaluate_promotion(&evidence, now).unwrap().evidence_hash,
            evaluate_promotion(&reversed, now).unwrap().evidence_hash
        );

        // The hash names the evidence, not when it was evaluated (fix round
        // 2): the same results, still fresh, give the same hash half an hour
        // later, so `qualify_bundle`, evaluating at its own time, finds the
        // hash a caller recorded earlier.
        let later = evaluate_promotion(&evidence, now + Duration::minutes(30)).unwrap();
        assert!(later.ready);
        assert_eq!(
            later.evidence_hash,
            evaluate_promotion(&evidence, now).unwrap().evidence_hash
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
                let mut check = promotion_check(check_id, PipelineCheckStatus::Pass, now);
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
        // from distinct revisions and distinct runs, so it is not ready, and
        // those are its only blockers (wave 2: promotion binds one revision;
        // review round 1 of #1240, point 5: and one run).
        let control = evaluate_promotion(&probe, now).expect("a well-formed probe evaluates");
        assert!(!control.ready);
        assert_eq!(
            control.safe_blockers,
            vec![
                QUALIFICATION_EVIDENCE_MIXED_REVISION_LABEL.to_string(),
                QUALIFICATION_EVIDENCE_MIXED_RUN_LABEL.to_string(),
            ]
        );

        // A maximum age that does not fit a duration is refused by id, never
        // a panic (fix round 2): above `i64::MAX` seconds, and above
        // chrono's millisecond range (`Duration::seconds` panics there).
        for maximum_age_seconds in [u64::MAX, (i64::MAX / 1_000) as u64 + 1] {
            let mut overflowing = probe.clone();
            overflowing[6].maximum_age_seconds = maximum_age_seconds;
            assert_eq!(
                evaluate_promotion(&overflowing, now),
                Err(format!(
                    "{QUALIFICATION_EVIDENCE_INVALID_LABEL}:{}",
                    PROMOTION_REQUIRED_CHECKS[6]
                )),
                "{maximum_age_seconds}"
            );
        }

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

    /// One passing result for each promotion check, in the shape a real
    /// qualification run has: the four checks of [`PROMOTION_PACKAGE_CHECKS`]
    /// name the one candidate package, every other check names none.
    fn passing_evidence(now: DateTime<Utc>) -> Vec<DrillEvidence> {
        PROMOTION_REQUIRED_CHECKS
            .iter()
            .map(|check_id| DrillEvidence {
                check: promotion_check(check_id, PipelineCheckStatus::Pass, now),
                maximum_age_seconds: 3_600,
            })
            .collect()
    }

    /// `evidence` with every check of [`PROMOTION_PACKAGE_CHECKS`] naming
    /// the package `name` (and no other result changed).
    fn name_package_on_candidate_checks(evidence: &mut [DrillEvidence], name: &str) {
        for item in evidence.iter_mut() {
            if PROMOTION_PACKAGE_CHECKS.contains(&item.check.check_id.as_str()) {
                name_package(&mut item.check, name);
            }
        }
    }

    fn check_mut<'a>(
        evidence: &'a mut [DrillEvidence],
        check_id: &str,
    ) -> &'a mut PipelineCheckResult {
        &mut evidence
            .iter_mut()
            .find(|item| item.check.check_id == check_id)
            .unwrap_or_else(|| panic!("{check_id} is a promotion check"))
            .check
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

    /// Review round 1 of #1240, point 5: the results of the checks that one
    /// `qualify` run produces (every check outside [`PROMOTION_ONLY_CHECKS`])
    /// must share one run id. A set that takes one of them from another run
    /// is not ready, under `qualification_evidence_mixed_run`, whichever
    /// check it is; the three promotion-only results may each come from a
    /// run of their own.
    #[test]
    fn promotion_binds_one_run() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        assert!(evaluate_promotion(&evidence, now).unwrap().ready);

        // The constant names three of the required checks, and none of them
        // tests the candidate package.
        assert_eq!(PROMOTION_ONLY_CHECKS.len(), 3);
        for check_id in PROMOTION_ONLY_CHECKS {
            assert!(PROMOTION_REQUIRED_CHECKS.contains(check_id), "{check_id}");
            assert!(!PROMOTION_PACKAGE_CHECKS.contains(check_id), "{check_id}");
        }

        // One run for the other results, and each promotion-only result
        // from a run of its own: ready.
        let mut other_runs = evidence.clone();
        for (index, check_id) in PROMOTION_ONLY_CHECKS.iter().enumerate() {
            check_mut(&mut other_runs, check_id).run_id = format!("qpromotion{index}");
        }
        let decision = evaluate_promotion(&other_runs, now).unwrap();
        assert!(decision.ready, "{:?}", decision.safe_blockers);

        // Any one of the other results from a second run: blocked, with the
        // one label, on the set above and on the one-run set.
        for base in [&evidence, &other_runs] {
            for check_id in PROMOTION_REQUIRED_CHECKS
                .iter()
                .filter(|check_id| !PROMOTION_ONLY_CHECKS.contains(check_id))
            {
                let mut mixed = base.clone();
                check_mut(&mut mixed, check_id).run_id = "q9999abcd".to_string();
                let decision = evaluate_promotion(&mixed, now).unwrap();
                assert!(!decision.ready, "{check_id}");
                assert_eq!(
                    decision.safe_blockers,
                    vec![QUALIFICATION_EVIDENCE_MIXED_RUN_LABEL.to_string()],
                    "{check_id}"
                );
            }
        }
        assert!(is_safe_label(QUALIFICATION_EVIDENCE_MIXED_RUN_LABEL));
    }

    /// Spec 2026-10-08 B-D4: a passing `pipeline_remote_restore` with no
    /// safe blockers, from the same code revision and naming the same
    /// package as the restore drill, discharges exactly the drill's
    /// `filesystem_restore_local_only` blocker, and nothing else.
    ///
    /// The proof is the absence of that one label. Until Slice A moves
    /// `pipeline_remote_restore` into [`PROMOTION_PACKAGE_CHECKS`] (A-D12), a
    /// remote result that names a package also carries
    /// `qualification_evidence_package_unexpected:pipeline_remote_restore`,
    /// so the set is not ready yet; that coupling is asserted below.
    #[test]
    fn remote_restore_discharges_only_the_local_restore_blocker() {
        let now = Utc::now();
        let local = format!("{LOCAL_RESTORE_BLOCKER_LABEL}:{RESTORE_DRILL_CHECK_ID}");
        assert_eq!(LOCAL_RESTORE_BLOCKER_LABEL, "filesystem_restore_local_only");
        assert_eq!(REMOTE_RESTORE_CHECK_ID, "pipeline_remote_restore");
        assert!(PROMOTION_REQUIRED_CHECKS.contains(&REMOTE_RESTORE_CHECK_ID));
        assert!(PROMOTION_REQUIRED_CHECKS.contains(&RESTORE_DRILL_CHECK_ID));

        // The drill carries the local blocker, as every local drill does.
        let mut base = passing_evidence(now);
        check_mut(&mut base, RESTORE_DRILL_CHECK_ID).safe_blockers =
            vec![LOCAL_RESTORE_BLOCKER_LABEL.to_string()];

        // The remote result of `passing_evidence` names no package: it is not
        // the drill's package, so nothing is discharged.
        let decision = evaluate_promotion(&base, now).unwrap();
        assert!(!decision.ready);
        assert_eq!(decision.safe_blockers, vec![local.clone()]);

        // A matching remote result: the local blocker is gone. Only the
        // unexpected-package blocker of today's lists remains.
        let mut matching = base.clone();
        name_package(
            check_mut(&mut matching, REMOTE_RESTORE_CHECK_ID),
            "candidate",
        );
        let decision = evaluate_promotion(&matching, now).unwrap();
        assert!(!decision.safe_blockers.contains(&local), "{decision:?}");
        if PROMOTION_PACKAGE_CHECKS.contains(&REMOTE_RESTORE_CHECK_ID) {
            assert!(decision.ready, "{:?}", decision.safe_blockers);
        } else {
            assert_eq!(
                decision.safe_blockers,
                vec![format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:{REMOTE_RESTORE_CHECK_ID}"
                )]
            );
        }

        // Each way the remote result can fail to match keeps the blocker.
        let cases: Vec<(&str, Box<dyn Fn(&mut PipelineCheckResult)>)> = vec![
            (
                "failing",
                Box::new(|check| check.status = PipelineCheckStatus::Fail),
            ),
            (
                "blocked",
                Box::new(|check| check.status = PipelineCheckStatus::Blocked),
            ),
            (
                "own_blocker",
                Box::new(|check| {
                    check.safe_blockers = vec!["remote_versioning_unverified".to_string()]
                }),
            ),
            (
                "another_revision",
                Box::new(|check| check.code_revision_hash = sha256_prefixed(b"another-revision")),
            ),
            (
                "another_package",
                Box::new(|check| name_package(check, "other")),
            ),
            (
                "partial_package",
                Box::new(|check| check.dependency_digest = None),
            ),
            (
                "stale",
                Box::new(|check| check.observed_at = Utc::now() - Duration::hours(3)),
            ),
        ];
        for (name, change) in &cases {
            let mut unmatched = matching.clone();
            change(check_mut(&mut unmatched, REMOTE_RESTORE_CHECK_ID));
            let decision = evaluate_promotion(&unmatched, now).unwrap();
            assert!(!decision.ready, "{name}");
            assert!(
                decision.safe_blockers.contains(&local),
                "{name}: {decision:?}"
            );
        }
        // A missing remote result keeps it too.
        let without_remote = matching
            .iter()
            .filter(|item| item.check.check_id != REMOTE_RESTORE_CHECK_ID)
            .cloned()
            .collect::<Vec<_>>();
        let decision = evaluate_promotion(&without_remote, now).unwrap();
        assert!(decision.safe_blockers.contains(&local));

        // Any other blocker on the drill still blocks, beside a matching
        // remote result.
        let mut other = matching.clone();
        check_mut(&mut other, RESTORE_DRILL_CHECK_ID)
            .safe_blockers
            .push("restore_tenant_count_low".to_string());
        let decision = evaluate_promotion(&other, now).unwrap();
        assert!(!decision.safe_blockers.contains(&local));
        assert!(decision.safe_blockers.contains(&format!(
            "restore_tenant_count_low:{RESTORE_DRILL_CHECK_ID}"
        )));

        // The local blocker on another check is not discharged.
        let mut elsewhere = matching.clone();
        check_mut(&mut elsewhere, "pipeline_bundle_qualification").safe_blockers =
            vec![LOCAL_RESTORE_BLOCKER_LABEL.to_string()];
        let decision = evaluate_promotion(&elsewhere, now).unwrap();
        assert!(decision.safe_blockers.contains(&format!(
            "{LOCAL_RESTORE_BLOCKER_LABEL}:pipeline_bundle_qualification"
        )));
    }

    /// Wave 2: among the results that name a package, every one must name
    /// the same package (all three digests). Two packages, or one that
    /// differs only in its configuration digest, is not ready, under
    /// `qualification_evidence_mixed_package`, and the decision names no
    /// package; one package is named by the decision. (Since P5-D15 the
    /// results that name a package are exactly the four candidate checks,
    /// so these cases change a candidate check.)
    #[test]
    fn promotion_binds_one_package() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        let decision = evaluate_promotion(&evidence, now).unwrap();
        assert!(decision.ready, "{:?}", decision.safe_blockers);
        let package = decision.package.expect("the decision names its package");
        assert!(package.is(&PipelinePackageDigests {
            package_hash: sha256_prefixed(b"candidate-package"),
            configuration_digest: sha256_prefixed(b"candidate-configuration"),
            dependency_digest: sha256_prefixed(b"candidate-dependency"),
        }));

        let mut other_package = evidence.clone();
        name_package(
            check_mut(&mut other_package, "pipeline_http_corpus_compatibility"),
            "other",
        );
        let mut other_configuration = evidence.clone();
        check_mut(&mut other_configuration, "pipeline_restore_drill").configuration_digest =
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

        // The rule of P5-D15 reversed the old reading, "a decision can be
        // ready with no package at all when no result names one": the
        // candidate checks then name none, each blocks, and the decision
        // names no package.
        let mut unnamed = passing_evidence(now);
        for item in &mut unnamed {
            item.check.package_hash = None;
            item.check.configuration_digest = None;
            item.check.dependency_digest = None;
        }
        let decision = evaluate_promotion(&unnamed, now).unwrap();
        assert!(!decision.ready);
        assert_eq!(
            decision.safe_blockers,
            PROMOTION_PACKAGE_CHECKS
                .iter()
                .map(|check_id| format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL}:{check_id}"
                ))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(decision.package, None, "no result names a package");
    }

    /// P5-D15 (a) and (e): a qualification run names exactly one package.
    /// The four checks that test the candidate ([`PROMOTION_PACKAGE_CHECKS`])
    /// carry its three digests and every other check carries none: a result
    /// for each required check then promotes the one candidate. The crash
    /// matrix, a mechanics check, naming another package (a test bundle, the
    /// state before PR 5) is a mixed package, and is also a result that
    /// names a package it must not.
    #[test]
    fn mechanics_results_without_a_package_do_not_mix_a_promotion() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        assert_eq!(
            evidence
                .iter()
                .filter(|item| item.check.package_hash.is_some())
                .map(|item| item.check.check_id.as_str())
                .collect::<BTreeSet<_>>(),
            PROMOTION_PACKAGE_CHECKS
                .iter()
                .copied()
                .collect::<BTreeSet<_>>(),
            "exactly the candidate checks name a package"
        );
        assert_eq!(
            evidence.len(),
            PROMOTION_REQUIRED_CHECKS.len(),
            "one result for each promotion check"
        );
        let decision = evaluate_promotion(&evidence, now).unwrap();
        assert!(decision.ready, "{:?}", decision.safe_blockers);
        let candidate = PipelinePackageDigests {
            package_hash: sha256_prefixed(b"candidate-package"),
            configuration_digest: sha256_prefixed(b"candidate-configuration"),
            dependency_digest: sha256_prefixed(b"candidate-dependency"),
        };
        assert!(
            decision
                .package
                .as_ref()
                .expect("the decision names the candidate package")
                .is(&candidate)
        );

        let mut mixed = evidence.clone();
        name_package(
            check_mut(&mut mixed, "pipeline_crash_matrix"),
            "test-bundle",
        );
        let decision = evaluate_promotion(&mixed, now).unwrap();
        assert!(!decision.ready);
        assert_eq!(
            decision.safe_blockers,
            vec![
                QUALIFICATION_EVIDENCE_MIXED_PACKAGE_LABEL.to_string(),
                format!("{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:pipeline_crash_matrix"),
            ]
        );
        assert_eq!(decision.package, None);
    }

    /// P5-D15 (b): a check that tests the candidate must name it. Each of
    /// the four, left without its package in turn, blocks promotion under
    /// `qualification_evidence_package_missing:<check_id>`, and the decision
    /// names no package.
    #[test]
    fn a_candidate_check_that_names_no_package_blocks_promotion() {
        let now = Utc::now();
        for check_id in PROMOTION_PACKAGE_CHECKS {
            let mut evidence = passing_evidence(now);
            let check = check_mut(&mut evidence, check_id);
            check.package_hash = None;
            check.configuration_digest = None;
            check.dependency_digest = None;
            let decision = evaluate_promotion(&evidence, now).unwrap();
            assert!(!decision.ready, "{check_id}");
            assert_eq!(
                decision.safe_blockers,
                vec![format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL}:{check_id}"
                )],
                "{check_id}"
            );
            assert_eq!(decision.package, None, "{check_id}");
        }
    }

    /// P5-D15 (d): a mechanics check names no package, even the candidate.
    /// With the four candidate checks naming the candidate, a mechanics
    /// result that names the same package still blocks promotion under
    /// `qualification_evidence_package_unexpected:<check_id>` (the packages
    /// agree, so this is not a mixed package), and the decision names no
    /// package. A promotion-only check (its result comes from a production
    /// run, not a local one) is a mechanics check in this rule.
    #[test]
    fn a_mechanics_result_that_names_the_candidate_blocks_promotion() {
        let now = Utc::now();
        for check_id in PROMOTION_REQUIRED_CHECKS
            .iter()
            .filter(|check_id| !PROMOTION_PACKAGE_CHECKS.contains(check_id))
        {
            let mut evidence = passing_evidence(now);
            name_package(check_mut(&mut evidence, check_id), "candidate");
            let decision = evaluate_promotion(&evidence, now).unwrap();
            assert!(!decision.ready, "{check_id}");
            assert_eq!(
                decision.safe_blockers,
                vec![format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:{check_id}"
                )],
                "{check_id}: the same package is not a mixed package"
            );
            assert_eq!(decision.package, None, "{check_id}");
        }

        // One digest alone is a digest: a mechanics result that carries only
        // a dependency digest blocks too.
        let mut partial = passing_evidence(now);
        check_mut(&mut partial, "pipeline_lease_renewal").dependency_digest =
            Some(sha256_prefixed(b"candidate-dependency"));
        let decision = evaluate_promotion(&partial, now).unwrap();
        assert!(!decision.ready);
        assert!(
            decision.safe_blockers.contains(&format!(
                "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:pipeline_lease_renewal"
            )),
            "{:?}",
            decision.safe_blockers
        );
    }

    /// A package is its three digests together. A check that tests the
    /// candidate and carries only some of them is not ready, whichever
    /// digest it lacks, even when every candidate check carries the same
    /// partial set (one package among the results, so no mixed-package
    /// blocker), and the decision names no package.
    #[test]
    fn a_candidate_check_with_a_partial_digest_set_blocks_promotion() {
        let now = Utc::now();
        let digests: [fn(&mut PipelineCheckResult) -> &mut Option<String>; 3] = [
            |check| &mut check.package_hash,
            |check| &mut check.configuration_digest,
            |check| &mut check.dependency_digest,
        ];
        for lacking in digests {
            // Every candidate check lacks the same digest: one (partial)
            // package among the results.
            let mut evidence = passing_evidence(now);
            for item in &mut evidence {
                if PROMOTION_PACKAGE_CHECKS.contains(&item.check.check_id.as_str()) {
                    *lacking(&mut item.check) = None;
                }
            }
            let decision = evaluate_promotion(&evidence, now).unwrap();
            assert!(!decision.ready, "{:?}", decision.safe_blockers);
            assert_eq!(
                decision.safe_blockers,
                PROMOTION_PACKAGE_CHECKS
                    .iter()
                    .map(|check_id| format!(
                        "{QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL}:{check_id}"
                    ))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
            );
            assert_eq!(decision.package, None);
        }
    }

    /// The list of checks that name the package is a part of the list of
    /// checks promotion requires, with no repeat.
    #[test]
    fn package_checks_are_required_promotion_checks() {
        assert_eq!(PROMOTION_PACKAGE_CHECKS.len(), 4);
        assert_eq!(
            PROMOTION_PACKAGE_CHECKS
                .iter()
                .collect::<BTreeSet<_>>()
                .len(),
            PROMOTION_PACKAGE_CHECKS.len()
        );
        for check_id in PROMOTION_PACKAGE_CHECKS {
            assert!(
                PROMOTION_REQUIRED_CHECKS.contains(check_id),
                "{check_id} is not a required promotion check"
            );
        }
    }

    /// The hole (task 7 review): the evidence is ready when one mechanics
    /// result names the candidate and the four checks that test the
    /// candidate name none, so the decision names a package that no
    /// candidate check tested. `evaluate_promotion` gates `qualify_bundle`,
    /// so the rule must hold for evidence that did not pass through
    /// `pipeline.py`.
    #[test]
    fn a_package_named_only_by_a_mechanics_result_is_not_ready() {
        let now = Utc::now();
        let mut evidence = passing_evidence(now);
        for item in evidence.iter_mut() {
            item.check.package_hash = None;
            item.check.configuration_digest = None;
            item.check.dependency_digest = None;
        }
        let crash_matrix = evidence
            .iter_mut()
            .find(|item| item.check.check_id == "pipeline_crash_matrix")
            .expect("the crash matrix is a promotion check");
        name_package(&mut crash_matrix.check, "candidate");
        let decision = evaluate_promotion(&evidence, now).unwrap();
        assert!(!decision.ready, "{:?}", decision.safe_blockers);
        for check_id in PROMOTION_PACKAGE_CHECKS {
            assert!(
                decision.safe_blockers.contains(&format!(
                    "{QUALIFICATION_EVIDENCE_PACKAGE_MISSING_LABEL}:{check_id}"
                )),
                "{check_id}: {:?}",
                decision.safe_blockers
            );
        }
        assert!(
            decision.safe_blockers.contains(&format!(
                "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:pipeline_crash_matrix"
            )),
            "{:?}",
            decision.safe_blockers
        );
        assert_eq!(decision.package, None);
    }

    /// Wave 2: the decision's `evidence_hash` covers each result's run id,
    /// code revision, package digests and evidence hash, not only the
    /// evidence hash: changing any one of them changes it.
    #[test]
    fn promotion_evidence_hash_covers_run_revision_package_and_evidence() {
        let now = Utc::now();
        let evidence = passing_evidence(now);
        let base = evaluate_promotion(&evidence, now).unwrap().evidence_hash;

        // Another run, for every result that one `qualify` run produces:
        // still ready (review round 1 of #1240, point 5: one run), so the
        // hash differs by the run id itself, not by a blocker. And another
        // run for one promotion-only result alone, which may have its own.
        let mut run = evidence.clone();
        for item in &mut run {
            if !PROMOTION_ONLY_CHECKS.contains(&item.check.check_id.as_str()) {
                item.check.run_id = "q9999abcd".to_string();
            }
        }
        let mut promotion_only_run = evidence.clone();
        check_mut(&mut promotion_only_run, PROMOTION_ONLY_CHECKS[0]).run_id =
            "q9999abcd".to_string();
        for one_run in [&run, &promotion_only_run] {
            assert!(evaluate_promotion(one_run, now).unwrap().ready);
        }
        let mut revision = evidence.clone();
        for item in &mut revision {
            item.check.code_revision_hash = sha256_prefixed(b"another-revision");
        }
        // Another package, named by all four candidate checks: still ready,
        // so the hash differs by the digests themselves, not by a blocker.
        let mut package = evidence.clone();
        name_package_on_candidate_checks(&mut package, "other");
        // Each digest alone, against the same named package (fix round 1,
        // review M5), again on all four candidate checks so the evidence
        // stays ready: a hash that left out any one of them would repeat
        // the base evidence's.
        let change_on_candidate_checks = |change: fn(&mut PipelineCheckResult)| {
            let mut changed = evidence.clone();
            for item in &mut changed {
                if PROMOTION_PACKAGE_CHECKS.contains(&item.check.check_id.as_str()) {
                    change(&mut item.check);
                }
            }
            changed
        };
        let package_hash = change_on_candidate_checks(|check| {
            check.package_hash = Some(sha256_prefixed(b"other-package"));
        });
        let configuration = change_on_candidate_checks(|check| {
            check.configuration_digest = Some(sha256_prefixed(b"other-configuration"));
        });
        let dependency = change_on_candidate_checks(|check| {
            check.dependency_digest = Some(sha256_prefixed(b"other-dependency"));
        });
        for named in [&package, &package_hash, &configuration, &dependency] {
            assert!(evaluate_promotion(named, now).unwrap().ready);
        }
        let mut observed = evidence.clone();
        observed[0].check.evidence_hash = sha256_prefixed(b"other-evidence");

        let mut hashes = vec![base];
        for changed in [
            run,
            promotion_only_run,
            revision,
            package,
            package_hash,
            configuration,
            dependency,
            observed,
        ] {
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
        let evidence = passing_evidence(now);
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
        // `evaluate_promotion` no longer calls such a decision ready (the
        // candidate checks must name the package), but the function takes a
        // decision value, so it still refuses one that is ready and names no
        // package.
        let mut unnamed = ready.clone();
        unnamed.package = None;
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

    // -----------------------------------------------------------------------
    // Signed check results (P5-D13, P5-D14).
    // -----------------------------------------------------------------------

    const CHECK_KEY_ID: &str = "check-key";

    fn generated_pkcs8() -> ring::pkcs8::Document {
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap()
    }

    /// A fresh check key and a check trust store that trusts exactly it.
    fn check_signer() -> (ring::pkcs8::Document, CheckResultTrustStore) {
        let pkcs8 = generated_pkcs8();
        let store =
            CheckResultTrustStore::new([
                trusted_key_for_pkcs8(CHECK_KEY_ID, pkcs8.as_ref()).expect("trusted key builds")
            ])
            .expect("check trust store builds");
        (pkcs8, store)
    }

    fn sign_sample(
        pkcs8: &ring::pkcs8::Document,
        corpus_digest: Option<String>,
        input_digest: Option<String>,
    ) -> PipelineCheckAttestation {
        sign_check_result(
            sample_check_result(),
            3_600,
            corpus_digest,
            input_digest,
            CHECK_KEY_ID,
            pkcs8.as_ref(),
        )
        .expect("the sample result signs")
    }

    /// A passing result for `check_id` that names no package.
    fn attested_result(check_id: &str) -> PipelineCheckResult {
        sample_check(check_id, PipelineCheckStatus::Pass, Utc::now())
    }

    /// Final fix wave (K1): `verify_all` bounds the signer's maximum age, so
    /// every path that verifies attestations (the qualification, the
    /// activation, and the rollback) refuses one above the ceiling, with the
    /// label the bare `qualify_bundle` uses. An age that chrono cannot
    /// represent is above the ceiling too. An age at the ceiling verifies.
    #[test]
    fn verify_all_refuses_a_maximum_age_above_the_ceiling() {
        let (pkcs8, store) = check_signer();
        let signed_with = |maximum_age_seconds: u64| {
            sign_check_result(
                sample_check_result(),
                maximum_age_seconds,
                None,
                None,
                CHECK_KEY_ID,
                pkcs8.as_ref(),
            )
            .expect("the sample result signs")
        };
        for maximum_age_seconds in [QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS + 1, u64::MAX] {
            assert_eq!(
                store
                    .verify_all(&[signed_with(maximum_age_seconds)])
                    .unwrap_err(),
                QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL,
                "{maximum_age_seconds}"
            );
        }
        let at_ceiling = store
            .verify_all(&[signed_with(QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS)])
            .expect("an age at the ceiling verifies");
        assert_eq!(
            at_ceiling.evidence[0].maximum_age_seconds,
            QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS
        );
    }

    #[test]
    fn an_attestation_round_trips_through_the_check_trust_store() {
        let (pkcs8, store) = check_signer();
        let result = sample_check_result();
        let attestation = sign_check_result(
            result.clone(),
            3_600,
            None,
            None,
            CHECK_KEY_ID,
            pkcs8.as_ref(),
        )
        .expect("the sample result signs");
        assert_eq!(attestation.schema, PIPELINE_CHECK_ATTESTATION_SCHEMA);
        assert_eq!(attestation.result, result);
        assert_eq!(attestation.maximum_age_seconds, 3_600);
        assert_eq!(attestation.signature.algorithm, PACKAGE_SIGNATURE_ALGORITHM);
        assert_eq!(attestation.signature.key_id, CHECK_KEY_ID);

        let verified = store
            .verify_all(std::slice::from_ref(&attestation))
            .expect("a signed attestation verifies");
        assert_eq!(verified.evidence.len(), 1);
        assert_eq!(verified.evidence[0].check, result);
        assert_eq!(verified.evidence[0].maximum_age_seconds, 3_600);

        // As a file: written as JSON, read back, and verified again, with
        // both digests set.
        let with_digests = sign_sample(
            &pkcs8,
            Some(sha256_prefixed(b"corpus")),
            Some(sha256_prefixed(b"input")),
        );
        let bytes = serde_json::to_vec(&with_digests).expect("the attestation serializes");
        let read_back: PipelineCheckAttestation =
            serde_json::from_slice(&bytes).expect("the attestation deserializes");
        assert_eq!(read_back, with_digests);
        store
            .verify_all(&[read_back])
            .expect("the attestation verifies after a JSON round trip");

        // The key and its public half are not the same bytes: a signature
        // made with one key does not verify under another's public key.
        let other = generated_pkcs8();
        let other_store =
            CheckResultTrustStore::new([
                trusted_key_for_pkcs8(CHECK_KEY_ID, other.as_ref()).unwrap()
            ])
            .unwrap();
        assert_eq!(
            other_store.verify_all(&[attestation]).unwrap_err(),
            CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL
        );
    }

    /// Review Focus 3, the unit part: nothing that was changed after signing,
    /// signed by a key the store does not trust, or not signed at all
    /// verifies. A result that no key signed is no input to a qualification.
    #[test]
    fn an_unsigned_or_altered_result_cannot_qualify_a_bundle() {
        let (pkcs8, store) = check_signer();
        let corpus = Some(sha256_prefixed(b"corpus"));
        let input = Some(sha256_prefixed(b"input"));
        let signed = sign_sample(&pkcs8, corpus.clone(), input.clone());
        store
            .verify_all(std::slice::from_ref(&signed))
            .expect("the unaltered attestation verifies");
        let unsigned_digests = sign_sample(&pkcs8, None, None);

        // Altered after signing: each fails as an invalid signature.
        let mut altered: Vec<(&str, PipelineCheckAttestation)> = Vec::new();
        let mut change = |what: &'static str,
                          base: &PipelineCheckAttestation,
                          edit: &dyn Fn(&mut PipelineCheckAttestation)| {
            let mut attestation = base.clone();
            edit(&mut attestation);
            assert_ne!(&attestation, base, "{what} changes the attestation");
            altered.push((what, attestation));
        };
        change("observed_at", &signed, &|a| {
            a.result.observed_at += Duration::seconds(1)
        });
        change("status", &signed, &|a| {
            a.result.status = PipelineCheckStatus::Fail
        });
        change("maximum_age_seconds", &signed, &|a| {
            a.maximum_age_seconds += 1
        });
        change("a changed corpus_digest", &signed, &|a| {
            a.corpus_digest = Some(sha256_prefixed(b"another-corpus"))
        });
        change("a removed corpus_digest", &signed, &|a| {
            a.corpus_digest = None
        });
        change("an added corpus_digest", &unsigned_digests, &|a| {
            a.corpus_digest = Some(sha256_prefixed(b"corpus"))
        });
        change("a changed input_digest", &signed, &|a| {
            a.input_digest = Some(sha256_prefixed(b"another-input"))
        });
        change("an added input_digest", &unsigned_digests, &|a| {
            a.input_digest = Some(sha256_prefixed(b"input"))
        });
        change("check_id", &signed, &|a| {
            a.result.check_id = "pipeline_lease_renewal".to_string()
        });
        change("run_id", &signed, &|a| a.result.run_id = "q9999abcd".into());
        change("code_revision_hash", &signed, &|a| {
            a.result.code_revision_hash = sha256_prefixed(b"another-revision")
        });
        change("evidence_hash", &signed, &|a| {
            a.result.evidence_hash = sha256_prefixed(b"another-evidence")
        });
        change("package_hash", &signed, &|a| {
            a.result.package_hash = Some(sha256_prefixed(b"another-package"))
        });
        change("a removed package digest", &signed, &|a| {
            a.result.dependency_digest = None
        });
        change("safe_blockers", &signed, &|a| {
            a.result.safe_blockers = Vec::new()
        });
        change("the signature's own hash", &signed, &|a| {
            a.signature.attestation_hash = sha256_prefixed(b"another-hash")
        });
        change("a blank signature", &signed, &|a| {
            a.signature.signature_base64url = String::new()
        });
        change("a signature that is not base64", &signed, &|a| {
            a.signature.signature_base64url = "!!not base64!!".to_string()
        });
        change("an algorithm other than Ed25519", &signed, &|a| {
            a.signature.algorithm = "HS256".to_string()
        });
        for (what, attestation) in altered {
            assert_eq!(
                store.verify_all(&[attestation]).unwrap_err(),
                CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL,
                "{what}"
            );
        }

        // One altered attestation among good ones refuses the whole set.
        let mut tampered = signed.clone();
        tampered.result.status = PipelineCheckStatus::Fail;
        let other_check = sign_check_result(
            attested_result("pipeline_lease_renewal"),
            3_600,
            None,
            None,
            CHECK_KEY_ID,
            pkcs8.as_ref(),
        )
        .unwrap();
        assert!(
            store
                .verify_all(&[other_check.clone(), signed.clone()])
                .is_ok()
        );
        assert_eq!(
            store
                .verify_all(&[other_check.clone(), tampered])
                .unwrap_err(),
            CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL
        );

        // Signed by a key the store does not know: untrusted, whether the key
        // id is new or the same id names another key (the second is an
        // invalid signature: the id is known and its key does not verify).
        let other_pkcs8 = generated_pkcs8();
        let stranger = sign_check_result(
            sample_check_result(),
            3_600,
            None,
            None,
            "another-key",
            other_pkcs8.as_ref(),
        )
        .unwrap();
        assert_eq!(
            store.verify_all(&[stranger]).unwrap_err(),
            CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL
        );
        let impostor = sign_check_result(
            sample_check_result(),
            3_600,
            None,
            None,
            CHECK_KEY_ID,
            other_pkcs8.as_ref(),
        )
        .unwrap();
        assert_eq!(
            store.verify_all(&[impostor]).unwrap_err(),
            CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL
        );
        let empty_store = CheckResultTrustStore::new(std::iter::empty()).unwrap();
        assert_eq!(
            empty_store
                .verify_all(std::slice::from_ref(&signed))
                .unwrap_err(),
            CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL
        );

        // A schema other than the constant, a result that is not valid, a
        // maximum age of zero, and a digest that is not a `sha256:` digest
        // are refused before the signature is read: they are invalid
        // attestations, not bad signatures.
        let invalid = |edit: &dyn Fn(&mut PipelineCheckAttestation)| {
            let mut attestation = signed.clone();
            edit(&mut attestation);
            store.verify_all(&[attestation]).unwrap_err()
        };
        for (what, label) in [
            (
                "a schema string other than the constant",
                invalid(&|a| a.schema = "trace_commons.pipeline_check_attestation.v2".into()),
            ),
            (
                "a result that is not valid",
                invalid(&|a| a.result.run_id = "Not A Label".into()),
            ),
            (
                "the result schema",
                invalid(&|a| a.result.schema = "bogus_schema".into()),
            ),
            (
                "a maximum age of zero",
                invalid(&|a| a.maximum_age_seconds = 0),
            ),
            (
                "a corpus digest that is not a digest",
                invalid(&|a| a.corpus_digest = Some("not-a-digest".into())),
            ),
            (
                "an input digest that is not a digest",
                invalid(&|a| a.input_digest = Some("sha256:short".into())),
            ),
        ] {
            assert_eq!(label, CHECK_ATTESTATION_INVALID_LABEL, "{what}");
        }

        // An unknown JSON field does not deserialize, and a bare result (no
        // attestation around it) is not an attestation.
        let mut with_extra = serde_json::to_value(&signed).unwrap();
        with_extra
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<PipelineCheckAttestation>(with_extra).is_err());
        let bare = serde_json::to_value(sample_check_result()).unwrap();
        assert!(serde_json::from_value::<PipelineCheckAttestation>(bare).is_err());
        // The signature has its own fields and no others: an unknown field
        // inside it is refused, and so is a package signature's field name.
        let mut extra_in_signature = serde_json::to_value(&signed).unwrap();
        extra_in_signature["signature"]
            .as_object_mut()
            .unwrap()
            .insert("unexpected".to_string(), serde_json::json!(true));
        assert!(serde_json::from_value::<PipelineCheckAttestation>(extra_in_signature).is_err());
        let mut package_field_name = serde_json::to_value(&signed).unwrap();
        let hash = package_field_name["signature"]
            .as_object_mut()
            .unwrap()
            .remove("attestation_hash")
            .unwrap();
        package_field_name["signature"]
            .as_object_mut()
            .unwrap()
            .insert("package_hash".to_string(), hash);
        assert!(serde_json::from_value::<PipelineCheckAttestation>(package_field_name).is_err());

        // The same check id twice is refused, as is anything else that would
        // let one attestation hide another from the digests.
        assert_eq!(
            store
                .verify_all(&[other_check.clone(), other_check])
                .unwrap_err(),
            CHECK_ATTESTATION_INVALID_LABEL
        );
    }

    #[test]
    fn signing_refuses_a_result_or_a_key_that_verification_would_refuse() {
        let pkcs8 = generated_pkcs8();
        let mut bad_result = sample_check_result();
        bad_result.run_id = "Not A Label".to_string();
        assert!(
            sign_check_result(bad_result, 3_600, None, None, CHECK_KEY_ID, pkcs8.as_ref()).is_err()
        );
        assert!(
            sign_check_result(
                sample_check_result(),
                0,
                None,
                None,
                CHECK_KEY_ID,
                pkcs8.as_ref()
            )
            .is_err()
        );
        assert!(
            sign_check_result(
                sample_check_result(),
                3_600,
                Some("not-a-digest".to_string()),
                None,
                CHECK_KEY_ID,
                pkcs8.as_ref()
            )
            .is_err()
        );
        assert!(
            sign_check_result(
                sample_check_result(),
                3_600,
                None,
                None,
                CHECK_KEY_ID,
                b"not a pkcs8 document"
            )
            .is_err()
        );
        assert!(
            sign_check_result(
                sample_check_result(),
                3_600,
                None,
                None,
                "key id with spaces",
                pkcs8.as_ref()
            )
            .is_err()
        );
    }

    /// The check trust store takes the keys the package trust store takes
    /// and refuses the ones it refuses, under its own label.
    #[test]
    fn the_check_trust_store_refuses_the_keys_the_package_store_refuses() {
        let pkcs8 = generated_pkcs8();
        let good = trusted_key_for_pkcs8("check-key", pkcs8.as_ref()).unwrap();
        let refused = [
            TrustedBundleKey {
                key_id: "bad key id".to_string(),
                public_key_base64url: good.public_key_base64url.clone(),
            },
            TrustedBundleKey {
                key_id: "check-key".to_string(),
                public_key_base64url: "!!not base64!!".to_string(),
            },
            TrustedBundleKey {
                key_id: "check-key".to_string(),
                public_key_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode([7u8; 31]),
            },
        ];
        for key in refused {
            assert_eq!(
                BundlePackageTrustStore::new([key.clone()]).unwrap_err(),
                PACKAGE_SIGNER_UNTRUSTED_LABEL
            );
            assert_eq!(
                CheckResultTrustStore::new([key]).unwrap_err(),
                CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL
            );
        }
        assert_eq!(
            CheckResultTrustStore::new([good.clone(), good.clone()]).unwrap_err(),
            CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL
        );
        assert!(CheckResultTrustStore::new([good]).is_ok());
    }

    /// A key of the package trust store is not a check key: an attestation
    /// signed with a package-signing key does not verify in a check store
    /// built from other keys, and the two stores are different types, so the
    /// route holds both and a package key cannot stand in for a check key.
    #[test]
    fn a_package_key_is_not_a_check_key() {
        let package_pkcs8 = generated_pkcs8();
        let package_trust = BundlePackageTrustStore::new([trusted_key_for_pkcs8(
            "release-2026-09",
            package_pkcs8.as_ref(),
        )
        .unwrap()])
        .unwrap();
        let signed_package = sign_bundle_package(
            minimal_test_package(),
            "release-2026-09",
            package_pkcs8.as_ref(),
        )
        .unwrap();
        package_trust
            .verify(&signed_package)
            .expect("the package key signs packages");

        let attestation = sign_check_result(
            sample_check_result(),
            3_600,
            None,
            None,
            "release-2026-09",
            package_pkcs8.as_ref(),
        )
        .unwrap();
        let (_, check_store) = check_signer();
        assert_eq!(
            check_store
                .verify_all(std::slice::from_ref(&attestation))
                .unwrap_err(),
            CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL
        );
        // Under the package key's own id, with another key behind it.
        let other_pkcs8 = generated_pkcs8();
        let lookalike = CheckResultTrustStore::new([trusted_key_for_pkcs8(
            "release-2026-09",
            other_pkcs8.as_ref(),
        )
        .unwrap()])
        .unwrap();
        assert_eq!(
            lookalike.verify_all(&[attestation]).unwrap_err(),
            CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL
        );

        // The reverse, for real: a signature made over an attestation is not
        // a package signature, and a package signature is not an attestation
        // signature, even with the same key behind both and the same key id,
        // whichever hash is recorded beside it.
        let reverse = attestation_for_reverse(&package_pkcs8);
        let signature = &reverse.signature;
        let package = minimal_test_package();
        let package_hash = package.package_hash().unwrap();
        for recorded in [signature.attestation_hash.clone(), package_hash.clone()] {
            let moved = SignedBundlePackage {
                package: package.clone(),
                signature: BundlePackageSignature {
                    algorithm: signature.algorithm.clone(),
                    key_id: signature.key_id.clone(),
                    package_hash: recorded,
                    signature_base64url: signature.signature_base64url.clone(),
                },
            };
            assert_eq!(
                package_trust.verify(&moved),
                Err(PACKAGE_SIGNATURE_INVALID_LABEL.to_string()),
                "an attestation's signature does not sign a package"
            );
        }
        let own_check_store = CheckResultTrustStore::new([trusted_key_for_pkcs8(
            "release-2026-09",
            package_pkcs8.as_ref(),
        )
        .unwrap()])
        .unwrap();
        let mut moved = sign_check_result(
            sample_check_result(),
            3_600,
            None,
            None,
            "release-2026-09",
            other_pkcs8.as_ref(),
        )
        .unwrap();
        for recorded_correctly in [false, true] {
            moved.signature = CheckAttestationSignature {
                algorithm: signed_package.signature.algorithm.clone(),
                key_id: signed_package.signature.key_id.clone(),
                attestation_hash: signed_package.signature.package_hash.clone(),
                signature_base64url: signed_package.signature.signature_base64url.clone(),
            };
            if recorded_correctly {
                moved.signature.attestation_hash = attestation_hash(&moved).unwrap();
            }
            assert_eq!(
                own_check_store
                    .verify_all(std::slice::from_ref(&moved))
                    .unwrap_err(),
                CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL,
                "a package signature does not sign an attestation (hash recorded correctly: {recorded_correctly})"
            );
        }
    }

    /// An attestation signed with `pkcs8` under the key id of the package key.
    fn attestation_for_reverse(pkcs8: &ring::pkcs8::Document) -> PipelineCheckAttestation {
        sign_check_result(
            sample_check_result(),
            3_600,
            None,
            None,
            "release-2026-09",
            pkcs8.as_ref(),
        )
        .unwrap()
    }

    #[test]
    fn the_corpus_and_input_digests_cover_every_attestation_that_carries_one() {
        let (pkcs8, store) = check_signer();
        let digest = |label: &str| sha256_prefixed(label.as_bytes());
        let sign = |check_id: &str, corpus: Option<String>, input: Option<String>| {
            sign_check_result(
                attested_result(check_id),
                3_600,
                corpus,
                input,
                CHECK_KEY_ID,
                pkcs8.as_ref(),
            )
            .unwrap()
        };

        // Two of three carry a corpus digest, under the check ids `x` and
        // `y`; the third carries none.
        let attestations = [
            sign("x", Some(digest("a")), Some(digest("c"))),
            sign("y", Some(digest("b")), Some(digest("d"))),
            sign("z", None, None),
        ];
        let verified = store.verify_all(&attestations).unwrap();
        assert_eq!(verified.evidence.len(), 3);
        assert_eq!(
            verified.corpus_digest,
            evidence_hash(&serde_json::json!({"x": digest("a"), "y": digest("b")})).unwrap()
        );
        assert_eq!(
            verified.input_digest,
            evidence_hash(&serde_json::json!({"x": digest("c"), "y": digest("d")})).unwrap()
        );
        let reversed = [
            attestations[2].clone(),
            attestations[1].clone(),
            attestations[0].clone(),
        ];
        let reversed = store.verify_all(&reversed).unwrap();
        assert_eq!(reversed.corpus_digest, verified.corpus_digest);
        assert_eq!(reversed.input_digest, verified.input_digest);

        // The digest names each check: the same values under other check ids
        // are another digest, and a changed value is another digest.
        let renamed = [
            sign("x", Some(digest("b")), Some(digest("c"))),
            sign("y", Some(digest("a")), Some(digest("d"))),
        ];
        assert_ne!(
            store.verify_all(&renamed).unwrap().corpus_digest,
            verified.corpus_digest
        );

        // Each digest is covered on its own.
        let corpus_only = store
            .verify_all(&[sign("x", Some(digest("a")), None)])
            .unwrap();
        assert_eq!(
            corpus_only.corpus_digest,
            evidence_hash(&serde_json::json!({"x": digest("a")})).unwrap()
        );
        assert_eq!(
            corpus_only.input_digest,
            evidence_hash(&serde_json::json!({})).unwrap()
        );

        // None carries one: the empty object's hash.
        let none = store
            .verify_all(&[sign("x", None, None), sign("y", None, None)])
            .unwrap();
        let empty = evidence_hash(&serde_json::json!({})).unwrap();
        assert_eq!(none.corpus_digest, empty);
        assert_eq!(none.input_digest, empty);
        assert_eq!(store.verify_all(&[]).unwrap().corpus_digest, empty);
    }

    /// Task 7's review: a mechanics check that carries even one of the three
    /// package digests (and the four candidate checks correct) blocks
    /// promotion as `qualification_evidence_package_unexpected:<check_id>`,
    /// whichever digest it is. The digest it carries is only part of a
    /// package, so the results also name two packages.
    #[test]
    fn a_mechanics_check_with_any_one_package_digest_is_unexpected() {
        let now = Utc::now();
        let digests: [(&str, fn(&mut PipelineCheckResult) -> &mut Option<String>); 3] = [
            ("package_hash", |check| &mut check.package_hash),
            ("configuration_digest", |check| {
                &mut check.configuration_digest
            }),
            ("dependency_digest", |check| &mut check.dependency_digest),
        ];
        for (name, digest) in digests {
            let mut evidence = passing_evidence(now);
            *digest(check_mut(&mut evidence, "pipeline_lease_renewal")) =
                Some(sha256_prefixed(b"candidate-package"));
            let decision = evaluate_promotion(&evidence, now).unwrap();
            assert!(!decision.ready, "{name}");
            assert_eq!(
                decision.safe_blockers,
                vec![
                    QUALIFICATION_EVIDENCE_MIXED_PACKAGE_LABEL.to_string(),
                    format!(
                        "{QUALIFICATION_EVIDENCE_PACKAGE_UNEXPECTED_LABEL}:pipeline_lease_renewal"
                    ),
                ],
                "{name}"
            );
            assert_eq!(decision.package, None, "{name}");
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
