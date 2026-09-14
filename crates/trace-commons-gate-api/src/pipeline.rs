//! Versioned contracts for the four-phase Trace Commons pipeline.
//!
//! The contracts in this module intentionally contain no persistence or
//! production policy implementation. Implementations consume these types
//! through trait objects so a policy backend can be replaced without changing
//! a runner.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PIPELINE_OUTCOME_SCHEMA_ID: &str = "trace_commons.pipeline_outcome";
pub const PIPELINE_OUTCOME_SCHEMA_VERSION: u32 = 1;
pub const BUNDLE_MANIFEST_FORMAT_VERSION: u32 = 1;
pub const MICROCREDITS_PER_CREDIT: u64 = 1_000_000;

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn is_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Admission,
    Review,
    Score,
    Settle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct ReasonCode(String);

impl ReasonCode {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(ContractError::InvalidReasonCode);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Microcredits(u64);

impl Microcredits {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    /// Parse a decimal credit amount without passing through floating point.
    pub fn from_credit_decimal(value: &str) -> Result<Self, MicrocreditConversionError> {
        if value.is_empty() || value.starts_with('-') || value.starts_with('+') {
            return Err(MicrocreditConversionError::Invalid);
        }
        let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
        if whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fractional.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(MicrocreditConversionError::Invalid);
        }
        if fractional.len() > 6 {
            return Err(MicrocreditConversionError::ExcessPrecision);
        }
        let whole = whole
            .parse::<u64>()
            .map_err(|_| MicrocreditConversionError::Overflow)?;
        let whole = whole
            .checked_mul(MICROCREDITS_PER_CREDIT)
            .ok_or(MicrocreditConversionError::Overflow)?;
        let mut fractional_micros = if fractional.is_empty() {
            0
        } else {
            fractional
                .parse::<u64>()
                .map_err(|_| MicrocreditConversionError::Overflow)?
        };
        for _ in fractional.len()..6 {
            fractional_micros = fractional_micros
                .checked_mul(10)
                .ok_or(MicrocreditConversionError::Overflow)?;
        }
        whole
            .checked_add(fractional_micros)
            .map(Self)
            .ok_or(MicrocreditConversionError::Overflow)
    }

    pub fn to_credit_decimal(self) -> String {
        let whole = self.0 / MICROCREDITS_PER_CREDIT;
        let fractional = self.0 % MICROCREDITS_PER_CREDIT;
        if fractional == 0 {
            return whole.to_string();
        }
        format!("{whole}.{fractional:06}")
            .trim_end_matches('0')
            .to_string()
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum MicrocreditConversionError {
    #[error("credit amount is not an unsigned decimal")]
    Invalid,
    #[error("credit amount has more than six decimal places")]
    ExcessPrecision,
    #[error("credit amount exceeds the microcredit range")]
    Overflow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchemaRef {
    pub id: String,
    pub version: u32,
}

impl SchemaRef {
    pub fn pipeline_v1() -> Self {
        Self {
            id: PIPELINE_OUTCOME_SCHEMA_ID.to_string(),
            version: PIPELINE_OUTCOME_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyRef {
    pub policy_id: String,
    pub implementation_id: String,
    pub code_artifact_hash: String,
    pub configuration_hash: String,
    pub data_artifact_hashes: Vec<String>,
    pub projection_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundleManifest {
    pub format_version: u32,
    pub admission: PolicyRef,
    pub review: PolicyRef,
    pub score: PolicyRef,
    pub settle: PolicyRef,
}

impl BundleManifest {
    /// Stable length-prefixed encoding. Lists are sorted before encoding.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        if self.format_version != BUNDLE_MANIFEST_FORMAT_VERSION {
            return Err(ContractError::UnsupportedManifestVersion);
        }
        let mut bytes = b"trace-commons-bundle-manifest\0".to_vec();
        bytes.extend_from_slice(&self.format_version.to_be_bytes());
        for policy in [&self.admission, &self.review, &self.score, &self.settle] {
            encode_policy(&mut bytes, policy)?;
        }
        Ok(bytes)
    }

    pub fn bundle_id(&self) -> Result<String, ContractError> {
        Ok(sha256_prefixed(&self.canonical_bytes()?))
    }

    fn referenced_artifacts(&self) -> Result<BTreeSet<String>, ContractError> {
        let mut hashes = BTreeSet::new();
        for policy in [&self.admission, &self.review, &self.score, &self.settle] {
            for hash in std::iter::once(&policy.code_artifact_hash)
                .chain(std::iter::once(&policy.configuration_hash))
                .chain(policy.data_artifact_hashes.iter())
            {
                if !is_sha256(hash) {
                    return Err(ContractError::InvalidArtifactHash);
                }
                hashes.insert(hash.clone());
            }
        }
        Ok(hashes)
    }
}

fn encode_string(output: &mut Vec<u8>, value: &str) -> Result<(), ContractError> {
    let len = u32::try_from(value.len()).map_err(|_| ContractError::ManifestFieldTooLarge)?;
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn encode_list(output: &mut Vec<u8>, values: &[String]) -> Result<(), ContractError> {
    let values = values.iter().collect::<BTreeSet<_>>();
    let len = u32::try_from(values.len()).map_err(|_| ContractError::ManifestFieldTooLarge)?;
    output.extend_from_slice(&len.to_be_bytes());
    for value in values {
        encode_string(output, value)?;
    }
    Ok(())
}

fn encode_policy(output: &mut Vec<u8>, policy: &PolicyRef) -> Result<(), ContractError> {
    if policy.policy_id.is_empty() || policy.implementation_id.is_empty() {
        return Err(ContractError::MissingPolicyIdentity);
    }
    encode_string(output, &policy.policy_id)?;
    encode_string(output, &policy.implementation_id)?;
    encode_string(output, &policy.code_artifact_hash)?;
    encode_string(output, &policy.configuration_hash)?;
    encode_list(output, &policy.data_artifact_hashes)?;
    encode_list(output, &policy.projection_ids)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundlePackage {
    pub bundle_id: String,
    pub manifest: BundleManifest,
    pub artifacts: BTreeMap<String, Vec<u8>>,
}

impl BundlePackage {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.bundle_id != self.manifest.bundle_id()? {
            return Err(ContractError::BundleIdMismatch);
        }
        let expected = self.manifest.referenced_artifacts()?;
        let actual = self.artifacts.keys().cloned().collect::<BTreeSet<_>>();
        if expected != actual {
            return Err(ContractError::ArtifactSetMismatch);
        }
        for (expected_hash, artifact) in &self.artifacts {
            if &sha256_prefixed(artifact) != expected_hash {
                return Err(ContractError::ArtifactHashMismatch);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum ContractError {
    #[error("unsupported bundle manifest version")]
    UnsupportedManifestVersion,
    #[error("bundle manifest field is too large")]
    ManifestFieldTooLarge,
    #[error("policy identity is missing")]
    MissingPolicyIdentity,
    #[error("artifact hash is malformed")]
    InvalidArtifactHash,
    #[error("bundle identifier does not match its manifest")]
    BundleIdMismatch,
    #[error("bundle artifact set does not match its manifest")]
    ArtifactSetMismatch,
    #[error("bundle artifact bytes do not match their hash")]
    ArtifactHashMismatch,
    #[error("reason code is not a safe label")]
    InvalidReasonCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AdmissionDecision {
    Admit,
    Quarantine { reason: ReasonCode },
    Reject { reason: ReasonCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved { registry_revision_id: Uuid },
    Rejected { reason: ReasonCode },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreDecision {
    pub credit_microcredits: Microcredits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IndexMembershipDecision {
    Exclude {
        reason: ReasonCode,
    },
    Include {
        command_hash: String,
        entry_count: u32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleDecision {
    pub index_membership: IndexMembershipDecision,
    pub credit_microcredits_finalized: Microcredits,
    pub settlement_batch_ref_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionEvidence {
    pub request_content_hash: String,
    pub schema_valid: bool,
    pub authority_valid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvidence {
    pub source_content_hash: String,
    pub result_content_hash: String,
    pub content_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreEvidence {
    pub fixed_credit_microcredits: Microcredits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreEvaluation {
    pub rule_id: String,
    pub credit_microcredits: Microcredits,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleEvidence {
    pub index_operation_required: bool,
    pub credit_operation_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PhaseResult<D, E, V> {
    pub decision: D,
    pub evidence: E,
    pub evaluation: V,
}

#[derive(Debug, Clone)]
pub struct AdmissionInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub request_content_hash: String,
    pub schema_version: String,
    pub authenticated: bool,
    pub authority_valid: bool,
}

#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub source_content_hash: String,
    pub source_artifact: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ScoreInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
}

#[derive(Debug, Clone)]
pub struct SettleInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
    pub score: ScoreDecision,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("policy failed: {label}")]
pub struct PolicyError {
    label: String,
}

impl PolicyError {
    pub fn new(label: impl Into<String>) -> Result<Self, ContractError> {
        Ok(Self {
            label: ReasonCode::new(label)?.0,
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }
}

#[async_trait]
pub trait AdmissionPolicy: Send + Sync {
    async fn execute(
        &self,
        input: &AdmissionInput,
    ) -> Result<PhaseResult<AdmissionDecision, AdmissionEvidence, AdmissionEvaluation>, PolicyError>;
}

#[async_trait]
pub trait ReviewPolicy: Send + Sync {
    async fn execute(
        &self,
        input: &ReviewInput,
    ) -> Result<PhaseResult<ReviewDecision, ReviewEvidence, ReviewEvaluation>, PolicyError>;
}

#[async_trait]
pub trait ScorePolicy: Send + Sync {
    async fn execute(
        &self,
        input: &ScoreInput,
    ) -> Result<PhaseResult<ScoreDecision, ScoreEvidence, ScoreEvaluation>, PolicyError>;
}

#[async_trait]
pub trait SettlePolicy: Send + Sync {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(bytes: &[u8]) -> String {
        sha256_prefixed(bytes)
    }

    fn policy(name: &str, code: &[u8], configuration: &[u8]) -> PolicyRef {
        PolicyRef {
            policy_id: name.to_string(),
            implementation_id: format!("{name}.v1"),
            code_artifact_hash: hash(code),
            configuration_hash: hash(configuration),
            data_artifact_hashes: Vec::new(),
            projection_ids: Vec::new(),
        }
    }

    #[test]
    fn checked_microcredit_conversion_is_exact() {
        assert_eq!(
            Microcredits::from_credit_decimal("1.000001").unwrap().get(),
            1_000_001
        );
        assert_eq!(
            Microcredits::from_credit_decimal("18446744073709.551615")
                .unwrap()
                .get(),
            u64::MAX
        );
        assert_eq!(
            Microcredits::from_credit_decimal("0.0000001"),
            Err(MicrocreditConversionError::ExcessPrecision)
        );
        assert_eq!(
            Microcredits::from_credit_decimal("-1"),
            Err(MicrocreditConversionError::Invalid)
        );
        assert_eq!(
            Microcredits::from_credit_decimal("18446744073709.551616"),
            Err(MicrocreditConversionError::Overflow)
        );
    }

    #[test]
    fn package_validation_binds_manifest_and_all_artifacts() {
        let policies = [
            (
                "admission",
                b"admission".as_slice(),
                b"admission-config".as_slice(),
            ),
            ("review", b"review".as_slice(), b"review-config".as_slice()),
            ("score", b"score".as_slice(), b"score-config".as_slice()),
            ("settle", b"settle".as_slice(), b"settle-config".as_slice()),
        ];
        let manifest = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy(policies[0].0, policies[0].1, policies[0].2),
            review: policy(policies[1].0, policies[1].1, policies[1].2),
            score: policy(policies[2].0, policies[2].1, policies[2].2),
            settle: policy(policies[3].0, policies[3].1, policies[3].2),
        };
        let artifacts = policies
            .iter()
            .flat_map(|(_, code, config)| {
                [(hash(code), code.to_vec()), (hash(config), config.to_vec())]
            })
            .collect();
        let package = BundlePackage {
            bundle_id: manifest.bundle_id().unwrap(),
            manifest,
            artifacts,
        };
        package.validate().unwrap();

        let mut altered = package.clone();
        altered.artifacts.values_mut().next().unwrap().push(0);
        assert_eq!(altered.validate(), Err(ContractError::ArtifactHashMismatch));

        let mut missing = package;
        missing.artifacts.pop_first();
        assert_eq!(missing.validate(), Err(ContractError::ArtifactSetMismatch));
    }

    #[test]
    fn every_immutable_policy_input_changes_bundle_identity() {
        let mut base = BundleManifest {
            format_version: BUNDLE_MANIFEST_FORMAT_VERSION,
            admission: policy("admission", b"admission", b"configuration"),
            review: policy("review", b"review", b"configuration"),
            score: policy("score", b"score", b"configuration"),
            settle: policy("settle", b"settle", b"configuration"),
        };
        let base_id = base.bundle_id().unwrap();

        let mut changed = base.clone();
        changed.admission.code_artifact_hash = hash(b"changed-code");
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed.review.configuration_hash = hash(b"changed-configuration");
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed
            .score
            .data_artifact_hashes
            .push(hash(b"changed-data"));
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed
            .settle
            .projection_ids
            .push("changed-projection".to_string());
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed.admission.policy_id = "changed-policy".to_string();
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        let mut changed = base.clone();
        changed.review.implementation_id = "changed-implementation".to_string();
        assert_ne!(changed.bundle_id().unwrap(), base_id);

        base.format_version += 1;
        assert_eq!(
            base.bundle_id(),
            Err(ContractError::UnsupportedManifestVersion)
        );
    }
}
