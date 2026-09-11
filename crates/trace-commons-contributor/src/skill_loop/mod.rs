//! INTEGRATION: turns an account-owned correction into a reviewed Agent Skill,
//! evaluates it on public held-out tasks, and installs only a passing digest.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::privacy::validate_outbound_text;
use trace_commons_protocol::trace_contribution::{TraceContributionEventType, UserFeedback};
use uuid::Uuid;

use crate::public_run::SessionDetail;

mod copy;
mod evaluation;
mod install;

pub use copy::*;
pub use evaluation::*;
pub use install::*;

/// Stable family identifier for corrections that repair generated files at their source.
pub const GENERATED_SOURCE_FAMILY: &str = "generated-source-repair";
/// Default Agent Skill name proposed for the generated-source repair family.
pub const DEFAULT_SKILL_NAME: &str = "repair-generated-sources";
/// Simple instruction used as the stronger-than-baseline evaluation control.
pub const MANUAL_CONTROL_INSTRUCTION: &str = "When a requested file is generated, edit its source and regenerate it instead of changing the generated file directly.";
/// Maximum number of Unicode scalar values accepted in an Agent Skill name.
pub const SKILL_NAME_MAX_CHARS: usize = 64;
/// Maximum number of Unicode scalar values accepted in a skill description.
pub const SKILL_DESCRIPTION_MAX_CHARS: usize = 1_024;
/// Maximum number of Unicode scalar values accepted in a skill procedure.
pub const SKILL_PROCEDURE_MAX_CHARS: usize = 12_000;

const SOURCE_TASK_MAX_TOKENS: usize = 256;
const SOURCE_TASK_SIMHASH_BITS: usize = 128;
// A single connective inserted into the representative task pinned below
// moves this 128-bit token/bigram/trigram SimHash by 32 bits. The four-bit
// margin admits punctuation and one short qualifier. Every pair in the fixed
// public fixture pool remains outside the cutoff; that invariant is tested in
// fixtures.rs and must be re-established when the pool changes.
const SOURCE_TASK_NEAR_DUPLICATE_DISTANCE: u32 = 36;

const DEFAULT_DESCRIPTION: &str = "Use when a debugging or maintenance task affects generated or derived files and the durable change belongs in a source schema, template, generator, dependency declaration, or other authoritative input.";

const DEFAULT_PROCEDURE: &str = r#"# Repair generated sources

1. Read the repository instructions and inspect the target for generated markers, sibling outputs, build scripts, and drift checks.
2. Identify the authoritative input: a schema, template, generator, dependency declaration, lock input, or source asset. State that mapping before editing.
3. Change the smallest authoritative input. Do not patch a generated output as the source of the fix.
4. Run the repository's existing generator. Do not copy its logic into a one-off command.
5. Inspect every output the generator changed, including sibling platforms and sizes. Revert unrelated generated churn.
6. Run an independent drift or correctness check plus the focused tests for the affected surface.
7. If the generator is unavailable, nondeterministic, or needs credentials you do not have, stop and report the blocker. Do not hand-edit around it."#;

/// Account-owned evidence retained for the owner to inspect while reviewing a candidate.
///
/// The excerpt contains session content. Callers must keep it on the authenticated
/// owner path; evaluation receives the approved package and public fixtures instead.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSourceEvidence {
    /// Stable source event identifier used to preserve contribution lineage.
    pub event_id: Uuid,
    /// Canonical event category retained through candidate review.
    pub kind: TraceContributionEventType,
    /// Bounded source excerpt shown during candidate review.
    pub excerpt: String,
}

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

/// A bounded, text-free fingerprint of the real session task. It persists
/// through review so held-out selection can reject source-task overlap without
/// placing the task or transcript in an evaluation prompt.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SkillSourceTaskFingerprint {
    pub(crate) sha256: String,
    pub(crate) similarity_hash: String,
    pub(crate) token_count: u16,
}

impl SkillSourceTaskFingerprint {
    pub(crate) fn is_valid(&self) -> bool {
        self.sha256.len() == 64
            && self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            && self.similarity_hash.len() == SOURCE_TASK_SIMHASH_BITS / 4
            && self
                .similarity_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && self.token_count >= 3
    }

    pub(crate) fn is_exact_or_near_duplicate(&self, other: &Self) -> bool {
        if !self.is_valid() || !other.is_valid() {
            return false;
        }
        if self.sha256 == other.sha256 {
            return true;
        }
        let smaller = usize::from(self.token_count.min(other.token_count));
        let larger = usize::from(self.token_count.max(other.token_count));
        // Permit a short qualifier while refusing a much longer task that only
        // embeds the source's generic words. Three tokens is the absolute
        // allowance for short tasks; longer tasks receive a 50% allowance.
        if larger > smaller.saturating_add((smaller / 2).max(3)) {
            return false;
        }
        similarity_distance(&self.similarity_hash, &other.similarity_hash)
            .is_some_and(|distance| distance <= SOURCE_TASK_NEAR_DUPLICATE_DISTANCE)
    }
}

/// Proposed skill plus the account-owned correction and evidence from which it arose.
///
/// `source_correction` and `source_evidence` contain session content for local review.
/// They are deliberately absent from the evaluation prompts and rendered package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillCandidate {
    /// Unique identifier for this local candidate proposal.
    pub candidate_id: Uuid,
    /// Supported correction-family identifier.
    pub family: &'static str,
    /// Account-owned contribution that supplies the correction and lineage.
    pub source_submission_id: Uuid,
    /// Decisive human correction copied from the source session for owner review.
    pub source_correction: String,
    /// Bounded source excerpts copied from the session for owner review.
    pub source_evidence: Vec<SkillSourceEvidence>,
    #[serde(default)]
    pub(crate) source_task_fingerprint: SkillSourceTaskFingerprint,
    /// Editable package fields initialized from the supported family template.
    pub draft: SkillDraft,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Earlier approved review superseded by this candidate, when revising a skill.
    pub replaces_review_id: Option<Uuid>,
    /// Fixed manually written instruction against which the candidate must improve.
    pub manual_control_instruction: &'static str,
    /// Disclosed held-out test controls required before installation.
    pub evaluation_contract: SkillEvaluationContract,
}

/// Owner-approved, immutable package snapshot bound to source lineage and a digest.
///
/// Plan evaluation receives the reviewed name and exact `skill_md` bytes. Raw
/// correction and evidence excerpts are replaced by event IDs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillReview {
    /// Unique identifier for this approved package snapshot.
    pub review_id: Uuid,
    /// Candidate proposal from which this review was approved.
    pub candidate_id: Uuid,
    /// Correction-family identifier preserved from the candidate.
    pub family: String,
    /// Account-owned contribution that supplies the review lineage.
    pub source_submission_id: Uuid,
    /// Source event identifiers retained without their excerpt text.
    pub source_evidence_ids: Vec<Uuid>,
    #[serde(default)]
    pub(crate) source_task_fingerprint: SkillSourceTaskFingerprint,
    /// Validated fields used to render the package.
    pub draft: SkillDraft,
    /// Exact UTF-8 Agent Skill package approved by the owner.
    pub skill_md: String,
    /// Lowercase SHA-256 digest of the exact `skill_md` bytes.
    pub skill_sha256: String,
}

/// Stable validation failures for owner-edited Agent Skill fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillDraftError {
    /// The name violates the lowercase, hyphenated Agent Skill name rules.
    InvalidName,
    /// The description is empty or has surrounding whitespace.
    DescriptionRequired,
    /// The description exceeds [`SKILL_DESCRIPTION_MAX_CHARS`].
    DescriptionTooLong,
    /// The procedure is empty or has surrounding whitespace.
    ProcedureRequired,
    /// The procedure exceeds [`SKILL_PROCEDURE_MAX_CHARS`].
    ProcedureTooLong,
    /// A field contains a disallowed control character.
    ControlCharacter,
    /// The description or procedure matches the public-text secret filter.
    SensitiveText,
}

impl SkillDraftError {
    #[must_use]
    /// Returns the stable daemon refusal label for this validation failure.
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

#[must_use]
/// Returns whether a human correction matches the first generated-source family.
pub fn correction_matches_generated_source_family(correction: &str) -> bool {
    let lower = correction.to_ascii_lowercase();
    let generated = [
        "generated",
        "auto-generated",
        "autogenerated",
        "derived file",
        "derived artifact",
        "generated artifact",
    ]
    .iter()
    .any(|signal| lower.contains(signal));
    let source = [
        "source schema",
        "source of truth",
        "generator",
        "template",
        "source file",
        "source asset",
        "dependency declaration",
        "cargo.toml",
    ]
    .iter()
    .any(|signal| lower.contains(signal));
    let correction_action = [
        "edit",
        "change",
        "fix",
        "update",
        "modify",
        "regenerate",
        "generate",
    ]
    .iter()
    .any(|signal| lower.contains(signal));
    generated && source && correction_action
}

/// Proposes a local candidate from an accepted, account-owned session projection.
///
/// The returned candidate exposes bounded correction and evidence text for explicit
/// owner review. It refuses non-corrections, unsupported families, and sessions whose
/// task cannot be fingerprinted for held-out overlap exclusion. This constructor does
/// not authenticate the account; callers must obtain both inputs through the same
/// authenticated owner lookup.
pub fn propose_candidate(
    source_submission_id: Uuid,
    detail: &SessionDetail,
) -> Result<SkillCandidate, &'static str> {
    if detail.contribution_status
        != Some(trace_commons_protocol::public_run::PublicRunContributionStatus::Accepted)
    {
        return Err("skill-session-not-accepted");
    }
    if detail.user_feedback != Some(UserFeedback::Correction) {
        return Err("skill-correction-required");
    }
    let correction = detail
        .human_correction
        .as_deref()
        .ok_or("skill-correction-required")?;
    if !correction_matches_generated_source_family(correction) {
        return Err("skill-family-not-supported");
    }
    let source_task_fingerprint = detail
        .task
        .as_deref()
        .and_then(source_task_fingerprint)
        .ok_or("skill-source-task-fingerprint-unavailable")?;
    let source_evidence = detail
        .evidence
        .iter()
        .take(6)
        .map(|item| SkillSourceEvidence {
            event_id: item.event_id,
            kind: item.kind,
            excerpt: item.excerpt.clone(),
        })
        .collect();
    Ok(SkillCandidate {
        candidate_id: Uuid::new_v4(),
        family: GENERATED_SOURCE_FAMILY,
        source_submission_id,
        source_correction: correction.to_string(),
        source_evidence,
        source_task_fingerprint,
        draft: SkillDraft {
            name: DEFAULT_SKILL_NAME.to_string(),
            description: DEFAULT_DESCRIPTION.to_string(),
            procedure: DEFAULT_PROCEDURE.to_string(),
        },
        replaces_review_id: None,
        manual_control_instruction: MANUAL_CONTROL_INSTRUCTION,
        evaluation_contract: evaluation_contract(),
    })
}

/// Validates owner-edited fields before rendering or approving an Agent Skill.
///
/// Validation enforces Agent Skill naming, bounded content, control-character rules,
/// and the shared outbound privacy filter used by public session projections.
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

#[must_use]
/// Returns whether `value` is a bounded lowercase Agent Skill name.
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

/// Freezes an approved draft into exact `SKILL.md` bytes and their SHA-256 digest.
///
/// The review keeps account-owned submission and evidence identifiers for lineage,
/// while omitting correction and excerpt text from the installable package.
pub fn review_candidate(
    candidate: &SkillCandidate,
    draft: SkillDraft,
) -> Result<SkillReview, SkillDraftError> {
    validate_draft(&draft)?;
    let skill_md = render_skill(&draft);
    let skill_sha256 = sha256(skill_md.as_bytes());
    Ok(SkillReview {
        review_id: Uuid::new_v4(),
        candidate_id: candidate.candidate_id,
        family: candidate.family.to_string(),
        source_submission_id: candidate.source_submission_id,
        source_evidence_ids: candidate
            .source_evidence
            .iter()
            .map(|evidence| evidence.event_id)
            .collect(),
        source_task_fingerprint: candidate.source_task_fingerprint.clone(),
        draft,
        skill_md,
        skill_sha256,
    })
}

#[must_use]
/// Renders validated draft fields as an Agent Skills specification package.
///
/// Call [`review_candidate`] when the rendered bytes must be validated and
/// digest-bound for evaluation or installation.
pub fn render_skill(draft: &SkillDraft) -> String {
    format!(
        "---\nname: {}\ndescription: {}\ncompatibility: Designed for coding agents with repository read, edit, and command tools.\nmetadata:\n  author: Trace Commons contributor\n  family: {}\n---\n\n{}\n",
        draft.name,
        yaml_double_quoted(&draft.description),
        GENERATED_SOURCE_FAMILY,
        draft.procedure
    )
}

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

#[must_use]
pub(crate) fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) fn source_task_fingerprint(value: &str) -> Option<SkillSourceTaskFingerprint> {
    let tokens = normalized_task_tokens(value);
    if tokens.len() < 3 {
        return None;
    }
    let normalized = tokens.join(" ");
    let mut weights = [0_i32; SOURCE_TASK_SIMHASH_BITS];
    for token in &tokens {
        add_similarity_feature(&mut weights, token, 1);
    }
    for window in tokens.windows(2) {
        add_similarity_feature(&mut weights, &format!("{}\0{}", window[0], window[1]), 2);
    }
    for window in tokens.windows(3) {
        add_similarity_feature(
            &mut weights,
            &format!("{}\0{}\0{}", window[0], window[1], window[2]),
            2,
        );
    }
    let mut similarity = [0_u8; SOURCE_TASK_SIMHASH_BITS / 8];
    for (bit, weight) in weights.into_iter().enumerate() {
        if weight >= 0 {
            similarity[bit / 8] |= 1 << (bit % 8);
        }
    }
    Some(SkillSourceTaskFingerprint {
        sha256: sha256(normalized.as_bytes()),
        similarity_hash: hex::encode(similarity),
        token_count: u16::try_from(tokens.len()).ok()?,
    })
}

fn normalized_task_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            if token.chars().count() < 64 {
                token.extend(character.to_lowercase());
            }
        } else if !token.is_empty() {
            tokens.push(std::mem::take(&mut token));
            if tokens.len() == SOURCE_TASK_MAX_TOKENS {
                return tokens;
            }
        }
    }
    if !token.is_empty() && tokens.len() < SOURCE_TASK_MAX_TOKENS {
        tokens.push(token);
    }
    tokens
}

fn add_similarity_feature(
    weights: &mut [i32; SOURCE_TASK_SIMHASH_BITS],
    feature: &str,
    feature_weight: i32,
) {
    let digest = Sha256::digest(feature.as_bytes());
    for bit in 0..SOURCE_TASK_SIMHASH_BITS {
        let set = digest[bit / 8] & (1 << (bit % 8)) != 0;
        weights[bit] += if set { feature_weight } else { -feature_weight };
    }
}

fn similarity_distance(left: &str, right: &str) -> Option<u32> {
    let left = hex::decode(left).ok()?;
    let right = hex::decode(right).ok()?;
    (left.len() == SOURCE_TASK_SIMHASH_BITS / 8 && right.len() == left.len()).then(|| {
        left.iter()
            .zip(right)
            .map(|(left, right)| (left ^ right).count_ones())
            .sum()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::public_run::SessionEvidenceCandidate;

    fn detail(correction: Option<&str>) -> SessionDetail {
        SessionDetail {
            content_unavailable: false,
            task: Some("Repair the generated package manifest from its source schema.".to_string()),
            contribution_status: Some(
                trace_commons_protocol::public_run::PublicRunContributionStatus::Accepted,
            ),
            permitted_uses: Vec::new(),
            task_success: Some(trace_commons_protocol::trace_contribution::TaskSuccess::Partial),
            user_feedback: Some(UserFeedback::Correction),
            human_correction: correction.map(str::to_string),
            evidence: vec![SessionEvidenceCandidate {
                event_id: Uuid::nil(),
                kind: trace_commons_protocol::trace_contribution::TraceContributionEventType::AssistantMessage,
                excerpt: "Changed the checked-in output directly.".to_string(),
            }],
            contributed_version: "2.0".to_string(),
            consent_policy_version: "2.0".to_string(),
            redaction_pipeline_version: "2.0".to_string(),
            publication: None,
            publication_version: 0,
            retained_source_slug: None,
        }
    }

    #[test]
    fn trigger_requires_generated_source_and_corrective_action_signals() {
        assert!(correction_matches_generated_source_family(
            "Do not edit the generated file. Change the source schema and regenerate it."
        ));
        assert!(!correction_matches_generated_source_family(
            "The generated file has the wrong value."
        ));
        assert!(!correction_matches_generated_source_family(
            "Change the source schema before the release."
        ));
        assert!(!correction_matches_generated_source_family(
            "The generator documentation is useful."
        ));
    }

    #[test]
    fn candidate_keeps_source_evidence_out_of_the_package() {
        let correction =
            "Do not edit the generated file. Change the source schema and regenerate it.";
        let candidate = propose_candidate(Uuid::nil(), &detail(Some(correction)))
            .expect("supported correction");
        assert_eq!(candidate.replaces_review_id, None);
        assert_eq!(
            candidate.source_evidence[0].kind,
            TraceContributionEventType::AssistantMessage
        );
        let review = review_candidate(&candidate, candidate.draft.clone()).expect("valid review");
        assert!(!review.skill_md.contains(correction));
        assert!(
            !review
                .skill_md
                .contains("Changed the checked-in output directly.")
        );
        assert_eq!(review.source_evidence_ids, vec![Uuid::nil()]);
        assert_eq!(review.skill_sha256, sha256(review.skill_md.as_bytes()));
    }

    #[test]
    fn unsupported_corrections_are_not_made_generic() {
        let result = propose_candidate(
            Uuid::nil(),
            &detail(Some(
                "Retry the network request after renewing the session.",
            )),
        );
        assert_eq!(result, Err("skill-family-not-supported"));
    }

    #[test]
    fn non_accepted_sessions_cannot_produce_a_skill() {
        let mut source = detail(Some(
            "Do not edit the generated file. Change the source schema and regenerate it.",
        ));
        source.contribution_status =
            Some(trace_commons_protocol::public_run::PublicRunContributionStatus::Quarantined);
        assert_eq!(
            propose_candidate(Uuid::nil(), &source),
            Err("skill-session-not-accepted")
        );
    }

    #[test]
    fn review_refuses_credentials_and_wallet_recovery_cues() {
        let mut draft = SkillDraft {
            name: DEFAULT_SKILL_NAME.to_string(),
            description: DEFAULT_DESCRIPTION.to_string(),
            procedure: DEFAULT_PROCEDURE.to_string(),
        };
        draft.procedure.push_str("\nUse recovery phrase here.");
        assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));
        draft.procedure = format!(
            "{}\napi_key = {}{}",
            DEFAULT_PROCEDURE,
            "sk-proj-",
            "x".repeat(16)
        );
        assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));

        let mnemonic = std::iter::repeat_n("abandon", 11)
            .chain(["about"])
            .collect::<Vec<_>>()
            .join(" ");
        draft.procedure = format!("{DEFAULT_PROCEDURE}\n{mnemonic}");
        assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));

        for prefix in ["ED25519:", "Ed25519:"] {
            draft.procedure = format!("{DEFAULT_PROCEDURE}\n{prefix}{}", "1".repeat(88));
            assert_eq!(
                validate_draft(&draft),
                Err(SkillDraftError::SensitiveText),
                "NEAR private-key prefixes must be rejected case-insensitively"
            );
        }
    }

    #[test]
    fn names_are_bounded_lowercase_kebab_case() {
        assert!(valid_skill_name("repair-generated-sources"));
        assert!(valid_skill_name(&"a".repeat(SKILL_NAME_MAX_CHARS)));
        assert!(!valid_skill_name(&"a".repeat(SKILL_NAME_MAX_CHARS + 1)));
        for value in [
            "",
            "Repair-files",
            "repair_files",
            "-repair",
            "repair-",
            "a--b",
        ] {
            assert!(!valid_skill_name(value), "{value}");
        }
    }

    #[test]
    fn draft_text_limits_accept_the_boundary_and_reject_one_character_more() {
        let boundary = SkillDraft {
            name: "boundary-skill".to_string(),
            description: "d".repeat(SKILL_DESCRIPTION_MAX_CHARS),
            procedure: "p".repeat(SKILL_PROCEDURE_MAX_CHARS),
        };
        assert_eq!(validate_draft(&boundary), Ok(()));

        let mut too_long = boundary.clone();
        too_long.description.push('d');
        assert_eq!(
            validate_draft(&too_long),
            Err(SkillDraftError::DescriptionTooLong)
        );
        too_long = boundary;
        too_long.procedure.push('p');
        assert_eq!(
            validate_draft(&too_long),
            Err(SkillDraftError::ProcedureTooLong)
        );
    }

    #[test]
    fn source_task_fingerprint_matches_only_exact_or_lexically_near_tasks() {
        let source = source_task_fingerprint(
            "Change the Windows package tile background and regenerate every display scale.",
        )
        .expect("source fingerprint");
        let near = source_task_fingerprint(
            "Change the Windows package tile background, then regenerate every display scale.",
        )
        .expect("near fingerprint");
        let unrelated = source_task_fingerprint(
            "Correct punctuation in a handwritten README without changing generated files.",
        )
        .expect("unrelated fingerprint");
        assert!(source.is_exact_or_near_duplicate(&source));
        assert_eq!(
            similarity_distance(&source.similarity_hash, &near.similarity_hash),
            Some(32)
        );
        assert!(source.is_exact_or_near_duplicate(&near));
        assert!(!source.is_exact_or_near_duplicate(&unrelated));

        let embedded = source_task_fingerprint(
            "Change the Windows package tile background and regenerate every display scale while also replacing the network protocol, rewriting account storage, migrating databases, and updating unrelated documentation.",
        )
        .expect("embedded fingerprint");
        assert!(!source.is_exact_or_near_duplicate(&embedded));
    }
}
