//! INTEGRATION: turns an account-owned correction into a reviewed Agent Skill,
//! evaluates it on public held-out tasks, and installs only a passing digest.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::trace_contribution::{TraceContributionEventType, UserFeedback};
use uuid::Uuid;

use crate::public_run::SessionDetail;

pub use trace_commons_protocol::mission_evaluation::{
    GENERATED_SOURCE_FAMILY, SKILL_DESCRIPTION_MAX_CHARS, SKILL_NAME_MAX_CHARS,
    SKILL_PROCEDURE_MAX_CHARS, SkillDraft, SkillDraftError, render_skill, valid_skill_name,
    validate_draft,
};

mod copy;
mod evaluation;
mod install;

pub use copy::*;
pub use evaluation::*;
pub use install::*;

/// Default Agent Skill name proposed for the generated-source repair family.
pub const DEFAULT_SKILL_NAME: &str = "repair-generated-sources";
/// Simple instruction used as the stronger-than-baseline evaluation control.
pub const MANUAL_CONTROL_INSTRUCTION: &str = "When a requested file is generated, edit its source and regenerate it instead of changing the generated file directly.";

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
#[path = "mod_tests.rs"]
mod tests;
