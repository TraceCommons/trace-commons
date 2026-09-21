// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
pub const MAX_INSTRUMENT_ID_LEN: usize = 64;
pub const TRACE_CREDIT_INSTRUMENT_ID: &str = "trace_credit";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct InstrumentId(String);

impl InstrumentId {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_INSTRUMENT_ID_LEN
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            })
        {
            return Err(ContractError::InvalidInstrumentId);
        }
        Ok(Self(value))
    }

    pub fn trace_credit() -> Self {
        Self(TRACE_CREDIT_INSTRUMENT_ID.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct AtomicUnits(u64);

impl AtomicUnits {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    pub fn try_from_u128(value: u128) -> Result<Self, ContractError> {
        u64::try_from(value)
            .map(Self)
            .map_err(|_| ContractError::AtomicUnitOverflow)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub fn checked_add(self, other: Self) -> Result<Self, ContractError> {
        self.0
            .checked_add(other.0)
            .map(Self)
            .ok_or(ContractError::AtomicUnitOverflow)
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

    /// Convert Trace Credit microcredits to the generic settlement unit.
    ///
    /// This is the Trace Credit adapter boundary. Both representations use
    /// the same checked `u64`, so conversion is exact.
    pub const fn into_atomic_units(self) -> AtomicUnits {
        AtomicUnits::from_raw(self.0)
    }

    pub const fn from_atomic_units(value: AtomicUnits) -> Self {
        Self(value.get())
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
pub struct InstrumentAward {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
}

impl InstrumentAward {
    pub fn new(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
    ) -> Result<Self, ContractError> {
        if atomic_units == AtomicUnits::ZERO {
            return Err(ContractError::ZeroInstrumentAward);
        }
        Ok(Self {
            instrument_id,
            atomic_units,
        })
    }

    pub fn trace_credit(microcredits: Microcredits) -> Result<Self, ContractError> {
        Self::new(
            InstrumentId::trace_credit(),
            microcredits.into_atomic_units(),
        )
    }

    pub fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    pub const fn atomic_units(&self) -> AtomicUnits {
        self.atomic_units
    }

    pub fn trace_credit_microcredits(&self) -> Result<Microcredits, ContractError> {
        if self.instrument_id.as_str() != TRACE_CREDIT_INSTRUMENT_ID {
            return Err(ContractError::NotTraceCredit);
        }
        Ok(Microcredits::from_atomic_units(self.atomic_units))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct InstrumentAwards(Vec<InstrumentAward>);

impl InstrumentAwards {
    pub fn new(mut awards: Vec<InstrumentAward>) -> Result<Self, ContractError> {
        awards.sort_by(|left, right| left.instrument_id.cmp(&right.instrument_id));
        if awards
            .windows(2)
            .any(|pair| pair[0].instrument_id == pair[1].instrument_id)
        {
            return Err(ContractError::DuplicateInstrumentId);
        }
        Ok(Self(awards))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &InstrumentAward> {
        self.0.iter()
    }

    pub fn get(&self, instrument_id: &InstrumentId) -> Option<&InstrumentAward> {
        self.0
            .binary_search_by(|award| award.instrument_id.cmp(instrument_id))
            .ok()
            .map(|index| &self.0[index])
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Canonical identity for the complete ordered award set.
    pub fn canonical_id(&self) -> String {
        let mut bytes = b"trace-commons-instrument-awards\0".to_vec();
        bytes.extend_from_slice(&(self.0.len() as u64).to_be_bytes());
        for award in &self.0 {
            bytes.extend_from_slice(&(award.instrument_id.0.len() as u64).to_be_bytes());
            bytes.extend_from_slice(award.instrument_id.0.as_bytes());
            bytes.extend_from_slice(&award.atomic_units.0.to_be_bytes());
        }
        sha256_prefixed(&bytes)
    }
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
    /// Canonical package bytes used by an external package signature.
    ///
    /// The manifest already binds every policy input by content hash. The
    /// package encoding adds a domain separator, the bundle identifier, the
    /// canonical manifest, and the sorted artifact hashes. Artifact bytes are
    /// validated before encoding, so signing this value binds the complete
    /// package without depending on a JSON serializer.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ContractError> {
        self.validate()?;
        let mut bytes = b"trace-commons-bundle-package\0".to_vec();
        encode_string(&mut bytes, &self.bundle_id)?;
        let manifest = self.manifest.canonical_bytes()?;
        let manifest_len =
            u32::try_from(manifest.len()).map_err(|_| ContractError::ManifestFieldTooLarge)?;
        bytes.extend_from_slice(&manifest_len.to_be_bytes());
        bytes.extend_from_slice(&manifest);
        let artifact_count = u32::try_from(self.artifacts.len())
            .map_err(|_| ContractError::ManifestFieldTooLarge)?;
        bytes.extend_from_slice(&artifact_count.to_be_bytes());
        for hash in self.artifacts.keys() {
            encode_string(&mut bytes, hash)?;
        }
        Ok(bytes)
    }

    pub fn package_hash(&self) -> Result<String, ContractError> {
        Ok(sha256_prefixed(&self.canonical_bytes()?))
    }

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
    #[error("instrument identifier is not a bounded safe label")]
    InvalidInstrumentId,
    #[error("an instrument award must contain positive atomic units")]
    ZeroInstrumentAward,
    #[error("an instrument appears more than once")]
    DuplicateInstrumentId,
    #[error("atomic units exceed the supported integer range")]
    AtomicUnitOverflow,
    #[error("the award does not use the Trace Credit instrument")]
    NotTraceCredit,
    #[error("settlement operation references must be SHA-256 hashes")]
    InvalidSettlementReference,
    #[error("settlement operations do not exactly match the score awards")]
    SettlementOperationMismatch,
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
    pub awards: InstrumentAwards,
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
    settlement_operations: Vec<InstrumentSettlement>,
}

impl SettleDecision {
    pub fn new(
        index_membership: IndexMembershipDecision,
        awards: &InstrumentAwards,
        mut settlement_operations: Vec<InstrumentSettlement>,
    ) -> Result<Self, ContractError> {
        settlement_operations.sort_by(|left, right| {
            left.instrument_id
                .cmp(&right.instrument_id)
                .then_with(|| left.operation_ref_hash.cmp(&right.operation_ref_hash))
        });
        if settlement_operations
            .windows(2)
            .any(|pair| pair[0].instrument_id == pair[1].instrument_id)
        {
            return Err(ContractError::DuplicateInstrumentId);
        }
        let operations_match =
            awards
                .iter()
                .zip(&settlement_operations)
                .all(|(award, operation)| {
                    award.instrument_id == operation.instrument_id
                        && award.atomic_units == operation.atomic_units
                });
        if awards.iter().len() != settlement_operations.len() || !operations_match {
            return Err(ContractError::SettlementOperationMismatch);
        }
        Ok(Self {
            index_membership,
            settlement_operations,
        })
    }

    pub fn settlement_operations(&self) -> &[InstrumentSettlement] {
        &self.settlement_operations
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentSettlement {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
    operation_ref_hash: String,
    result_ref_hash: String,
}

impl InstrumentSettlement {
    pub fn new(
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
        operation_ref_hash: impl Into<String>,
        result_ref_hash: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let operation_ref_hash = operation_ref_hash.into();
        let result_ref_hash = result_ref_hash.into();
        if atomic_units == AtomicUnits::ZERO {
            return Err(ContractError::ZeroInstrumentAward);
        }
        if !is_sha256(&operation_ref_hash) || !is_sha256(&result_ref_hash) {
            return Err(ContractError::InvalidSettlementReference);
        }
        Ok(Self {
            instrument_id,
            atomic_units,
            operation_ref_hash,
            result_ref_hash,
        })
    }

    pub fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    pub const fn atomic_units(&self) -> AtomicUnits {
        self.atomic_units
    }

    pub fn operation_ref_hash(&self) -> &str {
        &self.operation_ref_hash
    }

    pub fn result_ref_hash(&self) -> &str {
        &self.result_ref_hash
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdmissionEvidence {
    pub request_content_hash: String,
    pub schema_valid: bool,
    pub authority_valid: bool,
    #[serde(default)]
    pub contribution_path_valid: bool,
    #[serde(default)]
    pub grant_valid: bool,
    #[serde(default)]
    pub consent_valid: bool,
    #[serde(default)]
    pub allowed_uses_valid: bool,
    #[serde(default)]
    pub quota_counted: bool,
    #[serde(default)]
    pub detector_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_risk: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transformed_artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_assessment_hash: Option<String>,
    #[serde(default)]
    pub resolved_quarantine_reasons: Vec<ReasonCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewEvaluation {
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScoreEvidence {
    pub fixed_awards: InstrumentAwards,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_object_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorer_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perplexity_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_fraction_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub novelty_score_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_perplexity_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_novelty_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub novelty_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nearest_neighbor_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_cardinality: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunk_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_capped: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_eligible: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_quality_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit_quality_version: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighbor_artifact_hash: Option<String>,
    #[serde(skip)]
    pub pending_embeddings: Vec<Vec<f32>>,
    #[serde(skip)]
    pub pending_neighbor_bytes: Option<Vec<u8>>,
}

impl ScoreEvidence {
    pub fn fixed(fixed_awards: InstrumentAwards) -> Self {
        Self {
            fixed_awards,
            embedding_artifact_hash: None,
            embedding_object_key: None,
            index_id: None,
            index_snapshot_id: None,
            index_snapshot_hash: None,
            scorer_model_id: None,
            embedder_model_id: None,
            projection_id: None,
            perplexity_micros: None,
            tail_fraction_micros: None,
            novelty_score_micros: None,
            peak_perplexity_micros: None,
            peak_novelty_micros: None,
            quality_passed: None,
            novelty_passed: None,
            nearest_neighbor_hash: None,
            index_cardinality: None,
            coverage_tokens: None,
            chunk_count: None,
            chunks_capped: None,
            include_eligible: None,
            credit_quality_micros: None,
            credit_quality_version: None,
            neighbor_artifact_hash: None,
            pending_embeddings: Vec::new(),
            pending_neighbor_bytes: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScoreEvaluation {
    pub rule_id: String,
    pub awards: InstrumentAwards,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SettleEvidence {
    pub index_operation_required: bool,
    pub settlement_operations_required: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_command_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settlement_progress: Vec<InstrumentSettlementProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_progress: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_operable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard_reason: Option<ReasonCode>,
}

impl SettleEvidence {
    pub fn operations(index_operation_required: bool, settlement_operations_required: u32) -> Self {
        Self {
            index_operation_required,
            settlement_operations_required,
            index_command_hash: None,
            settlement_progress: Vec::new(),
            index_progress: None,
            submission_operable: None,
            guard_reason: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentSettlementProgress {
    pub instrument_id: InstrumentId,
    pub operation_ref_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ref_hash: Option<String>,
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
    pub contribution_path_valid: bool,
    pub grant_valid: bool,
    pub consent_valid: bool,
    pub allowed_uses_valid: bool,
    pub tombstoned: bool,
    pub quota_available: bool,
    pub privacy_risk: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRecommendation {
    Approve,
    Reject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HumanReviewAssessment {
    pub assessment_id: Uuid,
    pub recommendation: ReviewRecommendation,
    pub reason: ReasonCode,
    pub resolved_quarantine_reasons: Vec<ReasonCode>,
    pub evidence_hash: String,
}

#[derive(Debug, Clone)]
pub struct ReviewInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub source_content_hash: String,
    pub source_artifact: Vec<u8>,
    pub admission: AdmissionDecision,
    pub human_assessment: Option<HumanReviewAssessment>,
}

#[derive(Debug, Clone)]
pub struct ScoreInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
    pub tenant_id: String,
    pub reviewed_artifact: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct SettleInput {
    pub run_id: Uuid,
    pub trace_id: Uuid,
    pub registry_revision_id: Uuid,
    pub source_content_hash: String,
    pub score: ScoreDecision,
    pub score_evidence: ScoreEvidence,
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
        let award =
            InstrumentAward::trace_credit(Microcredits::from_credit_decimal("2.000001").unwrap())
                .unwrap();
        assert_eq!(
            award.trace_credit_microcredits().unwrap(),
            Microcredits::from_raw(2_000_001)
        );
    }

    fn award(instrument_id: &str, atomic_units: u64) -> InstrumentAward {
        InstrumentAward::new(
            InstrumentId::new(instrument_id).unwrap(),
            AtomicUnits::from_raw(atomic_units),
        )
        .unwrap()
    }

    #[test]
    fn instrument_identity_is_bounded_and_canonical() {
        assert_eq!(
            InstrumentId::new("Trace_Credit"),
            Err(ContractError::InvalidInstrumentId)
        );
        assert_eq!(
            InstrumentId::new("x".repeat(MAX_INSTRUMENT_ID_LEN + 1)),
            Err(ContractError::InvalidInstrumentId)
        );

        let first =
            InstrumentAwards::new(vec![award("storage_rebate", 7), award("trace_credit", 3)])
                .unwrap();
        let second =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        assert_eq!(first.canonical_id(), second.canonical_id());
    }

    #[test]
    fn duplicate_instruments_are_rejected() {
        assert_eq!(
            InstrumentAwards::new(vec![award("trace_credit", 1), award("trace_credit", 2)]),
            Err(ContractError::DuplicateInstrumentId)
        );
    }

    #[test]
    fn instrument_awards_have_deterministic_ordering() {
        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let ids = awards
            .iter()
            .map(|award| award.instrument_id().as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["storage_rebate", "trace_credit"]);
    }

    #[test]
    fn atomic_unit_arithmetic_rejects_overflow() {
        assert_eq!(
            AtomicUnits::from_raw(u64::MAX).checked_add(AtomicUnits::from_raw(1)),
            Err(ContractError::AtomicUnitOverflow)
        );
        assert_eq!(
            AtomicUnits::try_from_u128(u128::from(u64::MAX) + 1),
            Err(ContractError::AtomicUnitOverflow)
        );
    }

    #[test]
    fn settle_decision_preserves_two_simultaneous_instruments() {
        let awards =
            InstrumentAwards::new(vec![award("trace_credit", 3), award("storage_rebate", 7)])
                .unwrap();
        let trace_credit = InstrumentSettlement::new(
            InstrumentId::new("trace_credit").unwrap(),
            AtomicUnits::from_raw(3),
            hash(b"trace-credit-operation"),
            hash(b"trace-credit-result"),
        )
        .unwrap();
        let storage_rebate = InstrumentSettlement::new(
            InstrumentId::new("storage_rebate").unwrap(),
            AtomicUnits::from_raw(7),
            hash(b"storage-rebate-operation"),
            hash(b"storage-rebate-result"),
        )
        .unwrap();

        let decision = SettleDecision::new(
            IndexMembershipDecision::Exclude {
                reason: ReasonCode::new("not_selected").unwrap(),
            },
            &awards,
            vec![trace_credit, storage_rebate],
        )
        .unwrap();

        assert_eq!(decision.settlement_operations().len(), 2);
        assert_eq!(
            decision.settlement_operations()[0].instrument_id().as_str(),
            "storage_rebate"
        );
        assert_eq!(
            decision.settlement_operations()[1].result_ref_hash(),
            hash(b"trace-credit-result")
        );

        let missing_operation = InstrumentSettlement::new(
            InstrumentId::new("trace_credit").unwrap(),
            AtomicUnits::from_raw(3),
            hash(b"trace-credit-operation"),
            hash(b"trace-credit-result"),
        )
        .unwrap();
        assert_eq!(
            SettleDecision::new(
                IndexMembershipDecision::Exclude {
                    reason: ReasonCode::new("not_selected").unwrap(),
                },
                &awards,
                vec![missing_operation],
            ),
            Err(ContractError::SettlementOperationMismatch)
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
        let canonical = package.canonical_bytes().unwrap();
        assert!(canonical.starts_with(b"trace-commons-bundle-package\0"));
        assert_eq!(package.package_hash().unwrap().len(), 71);

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
