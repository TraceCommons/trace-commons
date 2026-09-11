//! Exact, reviewable publication records for contributed sessions.
//!
//! Publication is an independent consent event. A trace may be accepted into
//! the commons without any public page, and a public profile does not grant
//! permission to publish trace content. The native client shows a
//! [`PublicRunDraft`] verbatim, records its digest, and the server accepts the
//! write only when the supplied digest still covers the exact request body.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::canonical_json::to_canonical_vec;
use crate::trace_contribution::{
    DeterministicTraceRedactor, TaskSuccess, TraceContributionEnvelope, TraceContributionEventType,
    UserFeedback,
};

pub const PUBLIC_RUN_TITLE_MAX_CHARS: usize = 100;
pub const PUBLIC_RUN_OUTCOME_MAX_CHARS: usize = 600;
pub const PUBLIC_RUN_CORRECTION_MAX_CHARS: usize = 1_000;
pub const PUBLIC_RUN_WORKFLOW_MAX_CHARS: usize = 4_000;
pub const PUBLIC_RUN_EVIDENCE_MAX_ITEMS: usize = 4;
pub const PUBLIC_RUN_EVIDENCE_MAX_CHARS: usize = 700;
pub const PUBLIC_RUN_SLUG_MAX_CHARS: usize = 64;
pub const PUBLIC_RUN_CONTRIBUTED_VERSION_MAX_CHARS: usize = 128;
pub const PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS: usize = 24;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunReusePermission {
    #[serde(rename = "cc_by_4_0")]
    CcBy40,
    #[serde(rename = "cc0_1_0")]
    Cc0_10,
}

impl PublicRunReusePermission {
    pub fn label(self) -> &'static str {
        match self {
            Self::CcBy40 => "CC BY 4.0",
            Self::Cc0_10 => "CC0 1.0",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunEvidenceDraft {
    pub event_id: Uuid,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunDraft {
    pub title: String,
    pub outcome_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: PublicRunReusePermission,
    pub evidence: Vec<PublicRunEvidenceDraft>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunPublishRequest {
    pub draft: PublicRunDraft,
    pub task_success: crate::trace_contribution::TaskSuccess,
    pub contributed_version: String,
    pub expected_publication_version: u32,
    pub approval_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunLink {
    pub slug: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunEvidence {
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunPage {
    pub slug: String,
    pub title: String,
    pub outcome_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_excerpt: Option<String>,
    pub workflow: String,
    pub reuse_permission: PublicRunReusePermission,
    pub evidence: Vec<PublicRunEvidence>,
    pub task_success: crate::trace_contribution::TaskSuccess,
    pub contributed_version: String,
    pub version: u32,
    pub published_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PublicRunLink>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub source_unavailable: bool,
    #[serde(default)]
    pub variations: Vec<PublicRunLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunOwnerState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<PublicRunPage>,
    pub expected_publication_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_source_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunSessionEvidence {
    pub event_id: Uuid,
    pub kind: TraceContributionEventType,
    pub excerpt: String,
}

/// Bounded, account-owned projection used by the native session-detail view.
/// The stored envelope is reduced inside the server process so the client does
/// not download a multi-megabyte trace to render at most 24 excerpts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunSessionRecord {
    pub task_success: TaskSuccess,
    pub user_feedback: UserFeedback,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_correction: Option<String>,
    pub evidence: Vec<PublicRunSessionEvidence>,
    pub contributed_version: String,
    pub consent_policy_version: String,
    pub redaction_pipeline_version: String,
    pub owner_state: PublicRunOwnerState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunUnpublishResult {
    pub unpublished: bool,
    pub expected_publication_version: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl PublicRunSessionRecord {
    #[must_use]
    pub fn from_envelope(
        envelope: TraceContributionEnvelope,
        owner_state: PublicRunOwnerState,
    ) -> Self {
        let evidence = envelope
            .events
            .iter()
            .filter_map(|event| {
                let content = event.redacted_content.as_deref()?.trim();
                if content.is_empty() {
                    return None;
                }
                Some(PublicRunSessionEvidence {
                    event_id: event.event_id,
                    kind: event.event_type,
                    excerpt: content
                        .chars()
                        .take(PUBLIC_RUN_EVIDENCE_MAX_CHARS)
                        .collect(),
                })
            })
            .take(PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS)
            .collect();
        Self {
            task_success: envelope.outcome.task_success,
            user_feedback: envelope.outcome.user_feedback,
            human_correction: envelope.outcome.human_correction,
            evidence,
            contributed_version: envelope.schema_version,
            consent_policy_version: envelope.consent.policy_version,
            redaction_pipeline_version: envelope.privacy.redaction_pipeline_version,
            owner_state,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRunValidationError {
    EmptyField,
    FieldTooLong,
    SurroundingWhitespace,
    ControlCharacter,
    EvidenceCount,
    DuplicateEvidence,
    InvalidSourceSlug,
    SensitiveText,
    ApprovalDigest,
}

impl PublicRunDraft {
    pub fn validate(&self) -> Result<(), PublicRunValidationError> {
        validate_required(&self.title, PUBLIC_RUN_TITLE_MAX_CHARS)?;
        validate_required(&self.outcome_summary, PUBLIC_RUN_OUTCOME_MAX_CHARS)?;
        validate_required(&self.workflow, PUBLIC_RUN_WORKFLOW_MAX_CHARS)?;
        if let Some(correction) = self.correction_excerpt.as_deref() {
            validate_required(correction, PUBLIC_RUN_CORRECTION_MAX_CHARS)?;
        }
        if self.evidence.is_empty() || self.evidence.len() > PUBLIC_RUN_EVIDENCE_MAX_ITEMS {
            return Err(PublicRunValidationError::EvidenceCount);
        }
        let mut event_ids = BTreeSet::new();
        for item in &self.evidence {
            validate_required(&item.excerpt, PUBLIC_RUN_EVIDENCE_MAX_CHARS)?;
            if !event_ids.insert(item.event_id) {
                return Err(PublicRunValidationError::DuplicateEvidence);
            }
        }
        if let Some(slug) = self.source_slug.as_deref() {
            validate_slug(slug)?;
        }
        for text in self.public_text() {
            validate_public_text(text)?;
        }
        Ok(())
    }

    pub fn approval_sha256(
        &self,
        task_success: crate::trace_contribution::TaskSuccess,
        contributed_version: &str,
        expected_publication_version: u32,
    ) -> Result<String, serde_json::Error> {
        let value = serde_json::json!({
            "draft": self,
            "task_success": task_success,
            "contributed_version": contributed_version,
            "expected_publication_version": expected_publication_version,
        });
        let bytes = to_canonical_vec(&value)?;
        Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
    }

    pub fn public_text(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.title.as_str())
            .chain(std::iter::once(self.outcome_summary.as_str()))
            .chain(self.correction_excerpt.as_deref())
            .chain(std::iter::once(self.workflow.as_str()))
            .chain(self.evidence.iter().map(|item| item.excerpt.as_str()))
    }
}

impl PublicRunPublishRequest {
    pub fn validate(&self) -> Result<(), PublicRunValidationError> {
        self.draft.validate()?;
        validate_required(
            &self.contributed_version,
            PUBLIC_RUN_CONTRIBUTED_VERSION_MAX_CHARS,
        )?;
        let expected = self
            .draft
            .approval_sha256(
                self.task_success,
                &self.contributed_version,
                self.expected_publication_version,
            )
            .map_err(|_| PublicRunValidationError::ApprovalDigest)?;
        if self.approval_sha256 != expected {
            return Err(PublicRunValidationError::ApprovalDigest);
        }
        Ok(())
    }
}

pub fn validate_slug(slug: &str) -> Result<(), PublicRunValidationError> {
    if slug.is_empty()
        || slug.chars().count() > PUBLIC_RUN_SLUG_MAX_CHARS
        || slug.starts_with('-')
        || slug.ends_with('-')
        || !slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(PublicRunValidationError::InvalidSourceSlug);
    }
    Ok(())
}

fn validate_required(value: &str, max_chars: usize) -> Result<(), PublicRunValidationError> {
    if value.is_empty() {
        return Err(PublicRunValidationError::EmptyField);
    }
    if value.trim() != value {
        return Err(PublicRunValidationError::SurroundingWhitespace);
    }
    if value.chars().count() > max_chars {
        return Err(PublicRunValidationError::FieldTooLong);
    }
    if value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(PublicRunValidationError::ControlCharacter);
    }
    Ok(())
}

fn validate_public_text(value: &str) -> Result<(), PublicRunValidationError> {
    if contains_near_private_key(value) || resembles_wallet_recovery_phrase(value) {
        return Err(PublicRunValidationError::SensitiveText);
    }
    let redactor = DeterministicTraceRedactor::deterministic_only(Vec::new());
    let (redacted, report) = redactor.redact_text(value);
    if redacted != value || report.blocked_secret_detected {
        return Err(PublicRunValidationError::SensitiveText);
    }
    Ok(())
}

fn contains_near_private_key(value: &str) -> bool {
    value.match_indices("ed25519:").any(|(start, _)| {
        let encoded = value[start + "ed25519:".len()..]
            .chars()
            .take_while(|character| {
                character.is_ascii_alphanumeric() && !matches!(character, '0' | 'O' | 'I' | 'l')
            })
            .count();
        encoded >= 80
    })
}

fn resembles_wallet_recovery_phrase(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    if [
        "seed phrase",
        "recovery phrase",
        "wallet mnemonic",
        "secret phrase",
    ]
    .iter()
    .any(|cue| lowered.contains(cue))
    {
        return true;
    }
    let words = value
        .split(|character: char| !character.is_ascii_alphabetic())
        .map(str::to_ascii_lowercase)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    [12, 15, 18, 21, 24]
        .into_iter()
        .any(|word_count| words.windows(word_count).any(is_valid_bip39_mnemonic))
}

fn is_valid_bip39_mnemonic(words: &[String]) -> bool {
    // Official BIP-0039 English list, pinned from bitcoin/bips commit
    // 620871a7a442e276a058b487cd8743775fb499a4 (MIT licensed).
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    let wordlist = WORDS.get_or_init(|| include_str!("bip39_english.txt").lines().collect());
    let Some(indices) = words
        .iter()
        .map(|word| {
            wordlist
                .binary_search_by(|candidate| candidate.cmp(&word.as_str()))
                .ok()
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };

    let total_bits = indices.len() * 11;
    let entropy_bits = total_bits * 32 / 33;
    let checksum_bits = total_bits - entropy_bits;
    let mut entropy = vec![0u8; entropy_bits / 8];
    for bit_index in 0..entropy_bits {
        let word_index = bit_index / 11;
        let within_word = bit_index % 11;
        let bit = (indices[word_index] >> (10 - within_word)) & 1;
        entropy[bit_index / 8] |= (bit as u8) << (7 - (bit_index % 8));
    }
    let digest = Sha256::digest(&entropy);
    (0..checksum_bits).all(|offset| {
        let mnemonic_bit_index = entropy_bits + offset;
        let word_index = mnemonic_bit_index / 11;
        let within_word = mnemonic_bit_index % 11;
        let actual = (indices[word_index] >> (10 - within_word)) & 1;
        let expected = (digest[offset / 8] >> (7 - (offset % 8))) & 1;
        actual as u8 == expected
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_contribution::TaskSuccess;

    fn draft() -> PublicRunDraft {
        PublicRunDraft {
            title: "Repair a stalled upload".to_string(),
            outcome_summary: "The upload completed after the client renewed its session."
                .to_string(),
            correction_excerpt: Some(
                "Retry after the account session has been renewed.".to_string(),
            ),
            workflow: "Inspect the response status, renew the session, then retry once."
                .to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::nil(),
                excerpt: "The retry returned a successful status.".to_string(),
            }],
            source_slug: None,
        }
    }

    #[test]
    fn exact_reviewed_draft_validates() {
        let draft = draft();
        let request = PublicRunPublishRequest {
            approval_sha256: draft
                .approval_sha256(TaskSuccess::Success, "trace.contribution.v1", 0)
                .unwrap(),
            task_success: TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            expected_publication_version: 0,
            draft,
        };
        assert_eq!(request.validate(), Ok(()));
    }

    #[test]
    fn changing_text_after_approval_is_refused() {
        let mut draft = draft();
        let digest = draft
            .approval_sha256(TaskSuccess::Success, "trace.contribution.v1", 0)
            .unwrap();
        draft.title.push('!');
        let request = PublicRunPublishRequest {
            draft,
            task_success: TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            expected_publication_version: 0,
            approval_sha256: digest,
        };
        assert_eq!(
            request.validate(),
            Err(PublicRunValidationError::ApprovalDigest)
        );
    }

    #[test]
    fn changing_approval_metadata_after_review_is_refused() {
        const VERSION: &str = "trace.contribution.v1";
        let draft = draft();
        let request = PublicRunPublishRequest {
            approval_sha256: draft
                .approval_sha256(TaskSuccess::Success, VERSION, 0)
                .unwrap(),
            draft,
            task_success: TaskSuccess::Success,
            contributed_version: VERSION.to_string(),
            expected_publication_version: 0,
        };

        let mut changed_outcome = request.clone();
        changed_outcome.task_success = TaskSuccess::Failure;
        assert_eq!(
            changed_outcome.validate(),
            Err(PublicRunValidationError::ApprovalDigest)
        );
        let mut changed_version = request.clone();
        changed_version.contributed_version = "trace.contribution.v2".to_string();
        assert_eq!(
            changed_version.validate(),
            Err(PublicRunValidationError::ApprovalDigest)
        );

        let mut empty_version = request.clone();
        empty_version.contributed_version.clear();
        assert_eq!(
            empty_version.validate(),
            Err(PublicRunValidationError::EmptyField)
        );

        let exact_version = "v".repeat(PUBLIC_RUN_CONTRIBUTED_VERSION_MAX_CHARS);
        let exact_draft = request.draft.clone();
        let exact = PublicRunPublishRequest {
            approval_sha256: exact_draft
                .approval_sha256(TaskSuccess::Success, &exact_version, 0)
                .unwrap(),
            draft: exact_draft,
            task_success: TaskSuccess::Success,
            contributed_version: exact_version,
            expected_publication_version: 0,
        };
        assert_eq!(exact.validate(), Ok(()));

        let mut oversized = exact;
        oversized.contributed_version.push('v');
        assert_eq!(
            oversized.validate(),
            Err(PublicRunValidationError::FieldTooLong)
        );
    }

    #[test]
    fn public_text_with_a_credential_is_refused_without_echoing_it() {
        let mut draft = draft();
        draft.workflow = "Use token AKIAIOSFODNN7EXAMPLE to authenticate.".to_string();
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::SensitiveText)
        );
    }

    #[test]
    fn near_private_keys_and_wallet_recovery_phrases_are_refused() {
        let mut draft = draft();
        draft.workflow = format!("ed25519:{}", "A".repeat(88));
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::SensitiveText)
        );

        draft.workflow = "Share only the approved excerpt: abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about. Keep everything else private."
            .to_string();
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::SensitiveText)
        );

        draft.workflow = "Never paste a recovery phrase into a public page.".to_string();
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::SensitiveText)
        );
    }

    #[test]
    fn ordinary_twelve_word_sentences_are_not_treated_as_wallet_credentials() {
        let mut draft = draft();
        draft.workflow =
            "Review the output carefully and publish only the evidence that supports the result."
                .to_string();
        assert_eq!(draft.validate(), Ok(()));

        draft.workflow = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon"
            .to_string();
        assert_eq!(draft.validate(), Ok(()));
    }

    #[test]
    fn public_text_with_private_identity_is_refused() {
        let mut draft = draft();
        draft.outcome_summary = "Send the result to private.person@example.com".to_string();
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::SensitiveText)
        );
    }

    #[test]
    fn evidence_must_be_bounded_and_unique() {
        let mut draft = draft();
        draft.evidence.push(draft.evidence[0].clone());
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::DuplicateEvidence)
        );
    }

    #[test]
    fn source_slug_is_a_public_path_component() {
        let mut draft = draft();
        draft.source_slug = Some("Parent Run".to_string());
        assert_eq!(
            draft.validate(),
            Err(PublicRunValidationError::InvalidSourceSlug)
        );
    }

    #[test]
    fn draft_validation_covers_text_evidence_and_slug_boundaries() {
        let mut candidate = draft();
        candidate.title.clear();
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::EmptyField)
        );

        candidate = draft();
        candidate.title = " padded".to_string();
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::SurroundingWhitespace)
        );

        candidate = draft();
        candidate.title = "control\u{0007}".to_string();
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::ControlCharacter)
        );

        candidate = draft();
        candidate.title = "a".repeat(PUBLIC_RUN_TITLE_MAX_CHARS);
        assert_eq!(candidate.validate(), Ok(()));
        candidate.title.push('a');
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::FieldTooLong)
        );

        candidate = draft();
        candidate.evidence.clear();
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::EvidenceCount)
        );
        candidate.evidence = (0..=PUBLIC_RUN_EVIDENCE_MAX_ITEMS)
            .map(|_| PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "bounded evidence".to_string(),
            })
            .collect();
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::EvidenceCount)
        );

        candidate = draft();
        candidate.evidence[0].excerpt = "e".repeat(PUBLIC_RUN_EVIDENCE_MAX_CHARS);
        assert_eq!(candidate.validate(), Ok(()));
        candidate.evidence[0].excerpt.push('e');
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::FieldTooLong)
        );

        candidate = draft();
        candidate.source_slug = Some("a".repeat(PUBLIC_RUN_SLUG_MAX_CHARS));
        assert_eq!(candidate.validate(), Ok(()));
        candidate.source_slug = Some("a".repeat(PUBLIC_RUN_SLUG_MAX_CHARS + 1));
        assert_eq!(
            candidate.validate(),
            Err(PublicRunValidationError::InvalidSourceSlug)
        );
    }
}
