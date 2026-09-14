//! User-reviewed comparison tasks built from frozen episode evidence.

use std::collections::BTreeSet;

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::TaskCategory;
use super::episodes::{EpisodeMember, LocalEpisode, members_digest, validate_episode_id};

pub const COMPARISON_TASK_SCHEMA_VERSION: u32 = 1;
pub const MAX_COMPARISON_TASKS: usize = 256;
pub const MAX_TASK_EPISODES: usize = 32;
pub const MAX_CONTEXT_LABEL_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum ComparisonTaskValidationError {
    #[error("insights_comparison_task_invalid")]
    Invalid,
    #[error("insights_comparison_task_episode_limit")]
    EpisodeLimit,
    #[error("insights_comparison_task_category_not_qualified")]
    CategoryNotQualified,
    #[error("insights_comparison_task_checkout_provenance_not_qualified")]
    CheckoutProvenanceNotQualified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrozenEpisodeBinding {
    pub episode_id: String,
    pub revision: u64,
    pub membership_revision: u64,
    pub members_digest: String,
    pub members: Vec<EpisodeMember>,
    /// Hashed producer session identities frozen with the task material so
    /// overlapping exports remain connected after source deletion.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_session_identity_sha256: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claude_root_session_identity_sha256: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claude_agent_branch_identity_sha256: Vec<String>,
}

impl FrozenEpisodeBinding {
    pub fn from_episode(episode: &LocalEpisode) -> Result<Self> {
        Ok(Self {
            episode_id: episode.id.clone(),
            revision: episode.revision,
            membership_revision: episode.membership_revision,
            members_digest: members_digest(&episode.members)?,
            members: episode.members.clone(),
            source_session_identity_sha256: Vec::new(),
            claude_root_session_identity_sha256: Vec::new(),
            claude_agent_branch_identity_sha256: Vec::new(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTaskContext {
    pub project_id: String,
    pub category: TaskCategory,
    pub task_date: NaiveDate,
    pub checkout_provenance: CheckoutProvenance,
    pub language: ContextString,
    pub configuration: ComparisonConfigurationV1,
    pub configuration_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTaskContextInput {
    pub project_id: String,
    pub category: TaskCategory,
    pub task_date: NaiveDate,
    pub checkout_provenance: CheckoutProvenance,
    pub language: ContextString,
    pub configuration: ComparisonConfigurationV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextString {
    Unknown,
    Known { value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContextDigest {
    Unknown,
    Known { digest: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    Unknown,
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum CheckoutProvenance {
    Unavailable,
    Known { scheme: String, digest: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonConfigurationV1 {
    pub harness_id: ContextString,
    pub harness_version: ContextString,
    pub reasoning_effort: ReasoningEffort,
    pub tool_policy_id: ContextString,
    pub tool_policy_version: ContextString,
    pub prompt_template_digest: ContextDigest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonTaskOutcome {
    Pending,
    Accepted,
    Partial,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MaterialBoundOutcome {
    pub value: ComparisonTaskOutcome,
    pub material_revision: u64,
    pub material_digest: String,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndependenceConfirmation {
    pub material_revision: u64,
    pub material_digest: String,
    pub confirmed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LocalComparisonTaskV1 {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    pub material_revision: u64,
    pub material_digest: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub material_recorded_at: Option<DateTime<Utc>>,
    pub episodes: Vec<FrozenEpisodeBinding>,
    pub context: Option<ComparisonTaskContext>,
    pub outcome: Option<MaterialBoundOutcome>,
    pub independence_confirmation: Option<IndependenceConfirmation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonTaskStaleReason {
    EpisodeMissing,
    EpisodeRevisionChanged,
    EpisodeMembershipChanged,
    SnapshotMissingOrReplaced,
    OutcomeMaterialChanged,
    IndependenceMaterialChanged,
    ContextIncomplete,
    AttributionPendingQualification,
    OverlappingTaskEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTaskSourceQualification {
    pub rule: super::comparison_specs::QualifiedSourceRule,
    pub declared_model_cohort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_configuration_sha256: Option<String>,
    pub material_revision: u64,
    pub material_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTaskDetail {
    pub task: LocalComparisonTaskV1,
    pub source_qualification: Option<ComparisonTaskSourceQualification>,
    pub stale_reasons: Vec<ComparisonTaskStaleReason>,
    pub overlapping_task_ids: Vec<String>,
    pub resolved_at: DateTime<Utc>,
}

pub fn validate_uuid(value: &str) -> Result<()> {
    let parsed =
        uuid::Uuid::parse_str(value).map_err(|_| ComparisonTaskValidationError::Invalid)?;
    if parsed.is_nil() || parsed.to_string() != value {
        return Err(ComparisonTaskValidationError::Invalid.into());
    }
    Ok(())
}

pub fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(ComparisonTaskValidationError::Invalid.into());
    }
    Ok(())
}

fn validate_label(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_CONTEXT_LABEL_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/+-".contains(&byte))
    {
        return Err(ComparisonTaskValidationError::Invalid.into());
    }
    Ok(())
}

impl ComparisonTaskContext {
    pub fn validate(&self) -> Result<()> {
        validate_uuid(&self.project_id)?;
        if self.category != TaskCategory::Refactor {
            return Err(ComparisonTaskValidationError::CategoryNotQualified.into());
        }
        if matches!(self.checkout_provenance, CheckoutProvenance::Known { .. }) {
            return Err(ComparisonTaskValidationError::CheckoutProvenanceNotQualified.into());
        }
        self.language.validate()?;
        self.configuration.validate()?;
        validate_digest(&self.configuration_fingerprint)?;
        if self.configuration_fingerprint != self.configuration.fingerprint()? {
            return Err(ComparisonTaskValidationError::Invalid.into());
        }
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.language, ContextString::Known { .. })
            && matches!(self.configuration.harness_id, ContextString::Known { .. })
            && matches!(
                self.configuration.harness_version,
                ContextString::Known { .. }
            )
            && self.configuration.reasoning_effort != ReasoningEffort::Unknown
            && matches!(
                self.configuration.tool_policy_id,
                ContextString::Known { .. }
            )
            && matches!(
                self.configuration.tool_policy_version,
                ContextString::Known { .. }
            )
            && matches!(
                self.configuration.prompt_template_digest,
                ContextDigest::Known { .. }
            )
    }
}

impl ComparisonTaskContextInput {
    pub fn finalize(self) -> Result<ComparisonTaskContext> {
        let fingerprint = self.configuration.fingerprint()?;
        let context = ComparisonTaskContext {
            project_id: self.project_id,
            category: self.category,
            task_date: self.task_date,
            checkout_provenance: self.checkout_provenance,
            language: self.language,
            configuration: self.configuration,
            configuration_fingerprint: fingerprint,
        };
        context.validate()?;
        Ok(context)
    }
}

impl ContextString {
    fn validate(&self) -> Result<()> {
        if let Self::Known { value } = self {
            validate_label(value)?;
        }
        Ok(())
    }
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Unknown => out.push(0),
            Self::Known { value } => {
                out.push(1);
                encode_bytes(out, value.as_bytes());
            }
        }
    }
}

impl ContextDigest {
    fn validate(&self) -> Result<()> {
        if let Self::Known { digest } = self {
            validate_digest(digest)?;
        }
        Ok(())
    }
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Unknown => out.push(0),
            Self::Known { digest } => {
                out.push(1);
                encode_bytes(out, digest.as_bytes());
            }
        }
    }
}

fn encode_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

impl ComparisonConfigurationV1 {
    pub fn validate(&self) -> Result<()> {
        self.harness_id.validate()?;
        self.harness_version.validate()?;
        self.tool_policy_id.validate()?;
        self.tool_policy_version.validate()?;
        self.prompt_template_digest.validate()
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = b"trace-commons-comparison-configuration-v1\0".to_vec();
        self.harness_id.encode(&mut out);
        self.harness_version.encode(&mut out);
        out.push(match self.reasoning_effort {
            ReasoningEffort::Unknown => 0,
            ReasoningEffort::None => 1,
            ReasoningEffort::Minimal => 2,
            ReasoningEffort::Low => 3,
            ReasoningEffort::Medium => 4,
            ReasoningEffort::High => 5,
            ReasoningEffort::Xhigh => 6,
        });
        self.tool_policy_id.encode(&mut out);
        self.tool_policy_version.encode(&mut out);
        self.prompt_template_digest.encode(&mut out);
        Ok(out)
    }

    pub fn fingerprint(&self) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(self.canonical_bytes()?)))
    }
}

pub fn validate_episode_bindings(bindings: &[FrozenEpisodeBinding]) -> Result<()> {
    if bindings.is_empty() || bindings.len() > MAX_TASK_EPISODES {
        return Err(ComparisonTaskValidationError::EpisodeLimit.into());
    }
    let mut ids = BTreeSet::new();
    let mut previous = None;
    for binding in bindings {
        validate_episode_id(&binding.episode_id)?;
        if binding.revision == 0
            || binding.membership_revision == 0
            || binding.membership_revision > binding.revision
            || !ids.insert(&binding.episode_id)
            || previous.is_some_and(|value: &str| value >= binding.episode_id.as_str())
            || binding.members_digest != members_digest(&binding.members)?
            || !binding
                .source_session_identity_sha256
                .iter()
                .all(|identity| validate_digest(identity).is_ok())
            || !binding
                .source_session_identity_sha256
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || !binding
                .claude_root_session_identity_sha256
                .iter()
                .all(|identity| validate_digest(identity).is_ok())
            || !binding
                .claude_root_session_identity_sha256
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || !binding
                .claude_agent_branch_identity_sha256
                .iter()
                .all(|identity| validate_digest(identity).is_ok())
            || !binding
                .claude_agent_branch_identity_sha256
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return Err(ComparisonTaskValidationError::Invalid.into());
        }
        previous = Some(binding.episode_id.as_str());
    }
    Ok(())
}

pub fn material_digest(
    bindings: &[FrozenEpisodeBinding],
    context: Option<&ComparisonTaskContext>,
) -> Result<String> {
    validate_episode_bindings(bindings)?;
    if let Some(context) = context {
        context.validate()?;
    }
    let bytes = serde_json::to_vec(&(bindings, context))?;
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-comparison-task-material-v1\0");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

impl LocalComparisonTaskV1 {
    pub fn validate(&self) -> Result<()> {
        validate_uuid(&self.id)?;
        validate_episode_bindings(&self.episodes)?;
        if self.schema_version != COMPARISON_TASK_SCHEMA_VERSION
            || self.revision == 0
            || self.material_revision == 0
            || self.material_revision > self.revision
            || self.updated_at < self.created_at
            || self.material_recorded_at.is_some_and(|recorded_at| {
                recorded_at < self.created_at || recorded_at > self.updated_at
            })
            || self.material_digest != material_digest(&self.episodes, self.context.as_ref())?
        {
            return Err(ComparisonTaskValidationError::Invalid.into());
        }
        if let Some(outcome) = &self.outcome {
            validate_digest(&outcome.material_digest)?;
            if outcome.material_revision == 0
                || outcome.material_revision > self.material_revision
                || (outcome.material_revision == self.material_revision
                    && outcome.material_digest != self.material_digest)
                || outcome.recorded_at < self.created_at
                || outcome.recorded_at > self.updated_at
            {
                return Err(ComparisonTaskValidationError::Invalid.into());
            }
        }
        if let Some(confirmation) = &self.independence_confirmation {
            validate_digest(&confirmation.material_digest)?;
            if confirmation.material_revision == 0
                || confirmation.material_revision > self.material_revision
                || (confirmation.material_revision == self.material_revision
                    && confirmation.material_digest != self.material_digest)
                || confirmation.confirmed_at < self.created_at
                || confirmation.confirmed_at > self.updated_at
            {
                return Err(ComparisonTaskValidationError::Invalid.into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(value: &str) -> ContextString {
        ContextString::Known {
            value: value.into(),
        }
    }

    #[test]
    fn configuration_v1_has_pinned_unambiguous_golden_bytes() {
        let value = ComparisonConfigurationV1 {
            harness_id: known("codex"),
            harness_version: known("0.12.2"),
            reasoning_effort: ReasoningEffort::Medium,
            tool_policy_id: known("direct-v1"),
            tool_policy_version: known("1"),
            prompt_template_digest: ContextDigest::Known {
                digest: "11".repeat(32),
            },
        };
        assert_eq!(
            hex::encode(value.canonical_bytes().unwrap()),
            "74726163652d636f6d6d6f6e732d636f6d70617269736f6e2d636f6e66696775726174696f6e2d7631000100000005636f6465780100000006302e31322e320401000000096469726563742d7631010000000131010000004031313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131313131"
        );
        assert_eq!(
            value.fingerprint().unwrap(),
            "8c24d6be0b933a0aab1876834b73ec46229aa7e4290895c06d8c4de5dda3b9aa"
        );
        let mut unknown = value.clone();
        unknown.reasoning_effort = ReasoningEffort::Unknown;
        assert_ne!(value.fingerprint().unwrap(), unknown.fingerprint().unwrap());
    }

    #[test]
    fn additive_claude_identities_preserve_legacy_codex_binding_bytes() {
        let legacy = r#"{"episode_id":"episode-1","revision":1,"membership_revision":1,"members_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","members":[],"source_session_identity_sha256":["bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"]}"#;
        let binding: FrozenEpisodeBinding = serde_json::from_str(legacy).unwrap();
        assert!(binding.claude_root_session_identity_sha256.is_empty());
        assert!(binding.claude_agent_branch_identity_sha256.is_empty());
        assert_eq!(serde_json::to_string(&binding).unwrap(), legacy);
    }
}
