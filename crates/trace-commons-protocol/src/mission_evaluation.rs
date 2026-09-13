//! INTEGRATION: Root must add `pub mod mission_evaluation;` to `lib.rs`.
//! Shared, bounded wire contract for skill-evaluation packages. It contains no
//! publication authorization and does not authorize execution or model spend.

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::privacy::validate_outbound_text;

/// Stable family identifier for corrections that repair generated files at their source.
pub const GENERATED_SOURCE_FAMILY: &str = "generated-source-repair";
/// Maximum number of Unicode scalar values accepted in an Agent Skill name.
pub const SKILL_NAME_MAX_CHARS: usize = 64;
/// Maximum number of Unicode scalar values accepted in a skill description.
pub const SKILL_DESCRIPTION_MAX_CHARS: usize = 1_024;
/// Maximum number of Unicode scalar values accepted in a skill procedure.
pub const SKILL_PROCEDURE_MAX_CHARS: usize = 12_000;

/// The only supported mission-evaluation package schema.
pub const MISSION_EVALUATION_SCHEMA_VERSION: u32 = 1;
/// The only evaluator runtime accepted by this package version.
pub const MISSION_EVALUATOR_ID: &str = "skill-evaluation-v1";
/// Maximum accepted encoded package size.
pub const MISSION_EVALUATION_PACKAGE_MAX_BYTES: usize = 64 * 1024;
/// Maximum UTF-8 byte length of a public starting task.
pub const MISSION_EVALUATION_TASK_MAX_BYTES: usize = 8_000;
/// Required model owner for the published evaluator policy.
pub const MISSION_EVALUATION_REQUIRED_MODEL_OWNER: &str = "nearai";
/// Fixed total evaluator request budget.
pub const MISSION_EVALUATION_TOTAL_REQUESTS: u32 = 24;
/// Fixed output-token limit per evaluator request.
pub const MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT: u32 = 900;
/// Fixed evaluator request timeout.
pub const MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS: u32 = 90;
/// Fixed evaluator request concurrency cap.
pub const MISSION_EVALUATION_MAX_CONCURRENCY: u32 = 2;

/// Owner-editable fields from which the exact installable `SKILL.md` is rendered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDraft {
    /// Lowercase, hyphenated Agent Skill directory and metadata name.
    pub name: String,
    /// Applicability guidance used for Agent Skill discovery.
    pub description: String,
    /// Markdown procedure inserted into the skill body.
    pub procedure: String,
}
/// Stable validation failures for owner-edited Agent Skill fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillDraftError {
    InvalidName,
    DescriptionRequired,
    DescriptionTooLong,
    ProcedureRequired,
    ProcedureTooLong,
    ControlCharacter,
    SensitiveText,
}

impl SkillDraftError {
    /// Returns the stable refusal label for this validation failure.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::InvalidName => "skill-name-invalid",
            Self::DescriptionRequired => "skill-description-required",
            Self::DescriptionTooLong => "skill-description-too-long",
            Self::ProcedureRequired => "skill-procedure-required",
            Self::ProcedureTooLong => "skill-procedure-too-long",
            Self::ControlCharacter => "skill-control-character",
            Self::SensitiveText => "skill-sensitive-text",
        }
    }
}

/// Validates owner-edited fields before rendering an Agent Skill.
pub fn validate_draft(draft: &SkillDraft) -> Result<(), SkillDraftError> {
    if !valid_skill_name(&draft.name) {
        return Err(SkillDraftError::InvalidName);
    }
    validate_field(
        &draft.description,
        SKILL_DESCRIPTION_MAX_CHARS,
        SkillDraftError::DescriptionRequired,
        SkillDraftError::DescriptionTooLong,
        false,
    )?;
    validate_field(
        &draft.procedure,
        SKILL_PROCEDURE_MAX_CHARS,
        SkillDraftError::ProcedureRequired,
        SkillDraftError::ProcedureTooLong,
        true,
    )?;
    for value in [&draft.description, &draft.procedure] {
        validate_outbound_text(value).map_err(|_| SkillDraftError::SensitiveText)?;
    }
    Ok(())
}

/// Validates a bounded skill text field.
fn validate_field(
    value: &str,
    max_chars: usize,
    empty: SkillDraftError,
    too_long: SkillDraftError,
    allow_newline: bool,
) -> Result<(), SkillDraftError> {
    if value.trim().is_empty() || value.trim() != value {
        return Err(empty);
    }
    if value.chars().count() > max_chars {
        return Err(too_long);
    }
    if value.chars().any(|character| {
        character.is_control() && !(allow_newline && matches!(character, '\n' | '\t'))
    }) {
        return Err(SkillDraftError::ControlCharacter);
    }
    if !allow_newline && (value.contains('\n') || value.contains('\r')) {
        return Err(SkillDraftError::ControlCharacter);
    }
    Ok(())
}

/// Returns whether `value` is a bounded lowercase Agent Skill name.
#[must_use]
pub fn valid_skill_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= SKILL_NAME_MAX_CHARS
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.contains("--")
}

/// Renders a draft as exact Agent Skills `SKILL.md` bytes.
#[must_use]
pub fn render_skill(draft: &SkillDraft) -> String {
    format!(
        "---\nname: {}\ndescription: {}\ncompatibility: Designed for coding agents with repository read, edit, and command tools.\nmetadata:\n  author: Trace Commons contributor\n  family: {}\n---\n\n{}\n",
        draft.name,
        yaml_double_quoted(&draft.description),
        GENERATED_SOURCE_FAMILY,
        draft.procedure
    )
}

/// Escapes a YAML double-quoted scalar without changing other Unicode text.
#[must_use]
fn yaml_double_quoted(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

/// Fixed resource policy for the published skill-evaluation runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionEvaluationPolicy {
    pub required_model_owner: String,
    pub total_requests: u32,
    pub output_token_limit: u32,
    pub request_timeout_seconds: u32,
    pub max_concurrency: u32,
}

/// A bounded structural package for evaluating skill applicability and repair plans.
///
/// Availability still requires a separately stored issuer-controlled publication;
/// model spend requires separate local user approval.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionEvaluationPackage {
    pub schema_version: u32,
    pub mission_id: Uuid,
    pub program_id: Uuid,
    pub offer_version_hash: String,
    pub task: String,
    #[serde(deserialize_with = "deserialize_strict_skill_draft")]
    pub skill: SkillDraft,
    pub skill_sha256: String,
    pub evaluator_id: String,
    pub evaluation_contract_hash: String,
    pub execution: MissionEvaluationPolicy,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictSkillDraft {
    name: String,
    description: String,
    procedure: String,
}

fn deserialize_strict_skill_draft<'de, D>(deserializer: D) -> Result<SkillDraft, D::Error>
where
    D: Deserializer<'de>,
{
    let draft = StrictSkillDraft::deserialize(deserializer)?;
    Ok(SkillDraft {
        name: draft.name,
        description: draft.description,
        procedure: draft.procedure,
    })
}

/// Stable mission-package parsing and validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionEvaluationPackageError {
    InputTooLarge,
    MalformedJson,
    UnsupportedSchemaVersion,
    NilMissionId,
    NilProgramId,
    InvalidOfferVersionHash,
    TaskRequired,
    TaskTooLong,
    TaskControlCharacter,
    SensitiveTask,
    InvalidSkill(SkillDraftError),
    InvalidSkillSha256,
    StaleSkillSha256,
    UnsupportedEvaluator,
    InvalidEvaluationContractHash,
    InvalidRequiredModelOwner,
    TotalRequestsMismatch,
    OutputTokenLimitMismatch,
    RequestTimeoutMismatch,
    MaxConcurrencyMismatch,
    CanonicalSerialization,
}

impl MissionEvaluationPackageError {
    /// Returns the stable refusal label for this package failure.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::InputTooLarge => "mission-package-input-too-large",
            Self::MalformedJson => "mission-package-malformed-json",
            Self::UnsupportedSchemaVersion => "mission-package-schema-version-unsupported",
            Self::NilMissionId => "mission-package-mission-id-invalid",
            Self::NilProgramId => "mission-package-program-id-invalid",
            Self::InvalidOfferVersionHash => "mission-package-offer-version-hash-invalid",
            Self::TaskRequired => "mission-package-task-required",
            Self::TaskTooLong => "mission-package-task-too-long",
            Self::TaskControlCharacter => "mission-package-task-control-character",
            Self::SensitiveTask => "mission-package-task-sensitive-text",
            Self::InvalidSkill(_) => "mission-package-skill-invalid",
            Self::InvalidSkillSha256 => "mission-package-skill-sha256-invalid",
            Self::StaleSkillSha256 => "mission-package-skill-sha256-stale",
            Self::UnsupportedEvaluator => "mission-package-evaluator-unsupported",
            Self::InvalidEvaluationContractHash => {
                "mission-package-evaluation-contract-hash-invalid"
            }
            Self::InvalidRequiredModelOwner => "mission-package-model-owner-invalid",
            Self::TotalRequestsMismatch => "mission-package-total-requests-mismatch",
            Self::OutputTokenLimitMismatch => "mission-package-output-token-limit-mismatch",
            Self::RequestTimeoutMismatch => "mission-package-request-timeout-mismatch",
            Self::MaxConcurrencyMismatch => "mission-package-max-concurrency-mismatch",
            Self::CanonicalSerialization => "mission-package-canonical-serialization-failed",
        }
    }
}

impl MissionEvaluationPackage {
    /// Decodes and validates a package, refusing oversized input before decoding JSON.
    pub fn parse(bytes: &[u8]) -> Result<Self, MissionEvaluationPackageError> {
        if bytes.len() > MISSION_EVALUATION_PACKAGE_MAX_BYTES {
            return Err(MissionEvaluationPackageError::InputTooLarge);
        }
        let package: Self = serde_json::from_slice(bytes)
            .map_err(|_| MissionEvaluationPackageError::MalformedJson)?;
        package.validate()?;
        Ok(package)
    }

    /// Validates every package boundary without normalizing caller-supplied values.
    pub fn validate(&self) -> Result<(), MissionEvaluationPackageError> {
        if self.schema_version != MISSION_EVALUATION_SCHEMA_VERSION {
            return Err(MissionEvaluationPackageError::UnsupportedSchemaVersion);
        }
        if self.mission_id.is_nil() {
            return Err(MissionEvaluationPackageError::NilMissionId);
        }
        if self.program_id.is_nil() {
            return Err(MissionEvaluationPackageError::NilProgramId);
        }
        if !is_r1_sha256(&self.offer_version_hash) {
            return Err(MissionEvaluationPackageError::InvalidOfferVersionHash);
        }
        validate_task(&self.task)?;
        validate_draft(&self.skill).map_err(MissionEvaluationPackageError::InvalidSkill)?;
        if !is_lowercase_sha256(&self.skill_sha256) {
            return Err(MissionEvaluationPackageError::InvalidSkillSha256);
        }
        if self.skill_sha256 != sha256(render_skill(&self.skill).as_bytes()) {
            return Err(MissionEvaluationPackageError::StaleSkillSha256);
        }
        if self.evaluator_id != MISSION_EVALUATOR_ID {
            return Err(MissionEvaluationPackageError::UnsupportedEvaluator);
        }
        if !is_lowercase_sha256(&self.evaluation_contract_hash) {
            return Err(MissionEvaluationPackageError::InvalidEvaluationContractHash);
        }
        validate_policy(&self.execution)
    }

    /// Validates, canonically serializes, then returns the package SHA-256 digest.
    pub fn package_sha256(&self) -> Result<String, MissionEvaluationPackageError> {
        self.validate()?;
        let canonical = serde_json::to_vec(self)
            .map_err(|_| MissionEvaluationPackageError::CanonicalSerialization)?;
        if canonical.len() > MISSION_EVALUATION_PACKAGE_MAX_BYTES {
            return Err(MissionEvaluationPackageError::InputTooLarge);
        }
        Ok(sha256(&canonical))
    }
}

fn validate_task(value: &str) -> Result<(), MissionEvaluationPackageError> {
    if value.is_empty() || value.trim() != value {
        return Err(MissionEvaluationPackageError::TaskRequired);
    }
    if value.len() > MISSION_EVALUATION_TASK_MAX_BYTES {
        return Err(MissionEvaluationPackageError::TaskTooLong);
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(MissionEvaluationPackageError::TaskControlCharacter);
    }
    validate_outbound_text(value).map_err(|_| MissionEvaluationPackageError::SensitiveTask)
}

fn validate_policy(policy: &MissionEvaluationPolicy) -> Result<(), MissionEvaluationPackageError> {
    if policy.required_model_owner != MISSION_EVALUATION_REQUIRED_MODEL_OWNER {
        return Err(MissionEvaluationPackageError::InvalidRequiredModelOwner);
    }
    if policy.total_requests != MISSION_EVALUATION_TOTAL_REQUESTS {
        return Err(MissionEvaluationPackageError::TotalRequestsMismatch);
    }
    if policy.output_token_limit != MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT {
        return Err(MissionEvaluationPackageError::OutputTokenLimitMismatch);
    }
    if policy.request_timeout_seconds != MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS {
        return Err(MissionEvaluationPackageError::RequestTimeoutMismatch);
    }
    if policy.max_concurrency != MISSION_EVALUATION_MAX_CONCURRENCY {
        return Err(MissionEvaluationPackageError::MaxConcurrencyMismatch);
    }
    Ok(())
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_r1_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(is_lowercase_sha256)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "mission_evaluation_tests.rs"]
mod tests;
