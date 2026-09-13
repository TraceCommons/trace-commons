//! Exact, reviewable publication records for contributed sessions.
//!
//! Publication is an independent consent event. A trace may be accepted into
//! the commons without any public page, and a public profile does not grant
//! permission to publish trace content. The native client shows a
//! [`PublicRunDraft`] verbatim, records its digest, and the server accepts the
//! write only when the supplied digest still covers the exact request body.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::canonical_json::to_canonical_vec;
use crate::privacy::validate_outbound_text;

mod session_detail;

#[cfg(test)]
pub(crate) use session_detail::select_session_evidence;
pub use session_detail::{
    PublicRunContributionStatus, PublicRunSessionEvidence, PublicRunSessionRecord,
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
pub const PUBLIC_RUN_TASK_MAX_CHARS: usize = 1_000;

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
pub struct PublicRunUnpublishResult {
    pub unpublished: bool,
    pub expected_publication_version: u32,
}

fn is_false(value: &bool) -> bool {
    !*value
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
            validate_outbound_text(text).map_err(|_| PublicRunValidationError::SensitiveText)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_contribution::{
        SideEffectLevel, TaskSuccess, TraceAllowedUse, TraceContributionEvent,
        TraceContributionEventType, UserFeedback,
    };

    fn evidence_event(
        event_type: TraceContributionEventType,
        content: impl Into<String>,
    ) -> TraceContributionEvent {
        TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type,
            timestamp: Utc::now(),
            redacted_content: Some(content.into()),
            structured_payload: serde_json::Value::Null,
            tool_name: None,
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        }
    }

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
        for prefix in ["ed25519:", "ED25519:", "Ed25519:"] {
            for encoded_chars in [64, 88] {
                draft.workflow = format!("{prefix}{}", "A".repeat(encoded_chars));
                assert_eq!(
                    draft.validate(),
                    Err(PublicRunValidationError::SensitiveText)
                );
            }
        }

        for prefix in ["secp256k1:", "SECP256K1:", "Secp256K1:"] {
            for encoded_chars in [32, 44] {
                draft.workflow = format!("{prefix}{}", "A".repeat(encoded_chars));
                assert_eq!(
                    draft.validate(),
                    Err(PublicRunValidationError::SensitiveText)
                );
            }
        }

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
    fn legacy_session_detail_defaults_new_fields_without_granting_acceptance() {
        let legacy = serde_json::json!({
            "task_success": "success",
            "user_feedback": "none",
            "human_correction": null,
            "evidence": [],
            "contributed_version": "trace.contribution.v1",
            "consent_policy_version": "policy-v1",
            "redaction_pipeline_version": "privacy-v1",
            "owner_state": {
                "expected_publication_version": 0
            }
        });
        let record: PublicRunSessionRecord =
            serde_json::from_value(legacy.clone()).expect("legacy session detail");

        assert!(!record.is_accepted());
        assert!(!record.content_unavailable);
        assert_eq!(record.task_success, Some(TaskSuccess::Success));
        assert_eq!(record.user_feedback, Some(UserFeedback::None));
        assert!(record.contribution_status.is_none());
        assert!(record.task.is_none());
        assert!(record.permitted_uses.is_empty());

        let mut informational_status = legacy;
        informational_status["contribution_status"] = serde_json::json!("accepted");
        let record: PublicRunSessionRecord = serde_json::from_value(informational_status)
            .expect("session detail with informational status");
        assert_eq!(
            record.contribution_status,
            Some(PublicRunContributionStatus::Accepted)
        );
        assert!(record.is_accepted());
        assert!(!record.content_unavailable);
    }

    #[test]
    fn status_only_detail_omits_unobserved_content_and_outcome() {
        let owner_state = PublicRunOwnerState {
            publication: None,
            expected_publication_version: 4,
            retained_source_slug: None,
        };
        let record = PublicRunSessionRecord::status_only(
            PublicRunContributionStatus::Revoked,
            vec![TraceAllowedUse::Evaluation],
            "trace.contribution.v1".to_string(),
            "policy-v1".to_string(),
            "privacy-v2".to_string(),
            owner_state,
        )
        .expect("revoked records support status-only detail");

        assert!(!record.is_accepted());
        assert!(record.content_unavailable);
        assert!(record.task.is_none());
        assert!(record.task_success.is_none());
        assert!(record.user_feedback.is_none());
        assert!(record.human_correction.is_none());
        assert!(record.evidence.is_empty());
        assert_eq!(record.contributed_version, "trace.contribution.v1");
        assert_eq!(record.permitted_uses, vec![TraceAllowedUse::Evaluation]);
        let json = serde_json::to_value(&record).expect("serialize status-only detail");
        for absent in ["task", "task_success", "user_feedback", "human_correction"] {
            assert!(json.get(absent).is_none(), "{absent} must remain unclaimed");
        }
        assert_eq!(json["evidence"], serde_json::json!([]));
        assert_eq!(json["content_unavailable"], true);

        assert!(
            PublicRunSessionRecord::status_only(
                PublicRunContributionStatus::Accepted,
                Vec::new(),
                "trace.contribution.v1".to_string(),
                "policy-v1".to_string(),
                "privacy-v2".to_string(),
                PublicRunOwnerState {
                    publication: None,
                    expected_publication_version: 0,
                    retained_source_slug: None,
                },
            )
            .is_none(),
            "accepted detail must fail closed without its envelope"
        );
    }

    #[test]
    fn detail_prioritizes_correction_context_and_verification_before_bound() {
        let mut events = (0..30)
            .map(|index| {
                evidence_event(
                    TraceContributionEventType::AssistantMessage,
                    format!("Earlier transcript event {index}"),
                )
            })
            .collect::<Vec<_>>();
        let before_correction = evidence_event(
            TraceContributionEventType::AssistantMessage,
            "The agent proposed editing the generated output.",
        );
        let correction = evidence_event(
            TraceContributionEventType::UserMessage,
            "Change the source schema and regenerate the output.",
        );
        let after_correction = evidence_event(
            TraceContributionEventType::AssistantMessage,
            "The source schema was updated.",
        );
        let mut verification_call = evidence_event(
            TraceContributionEventType::ToolCall,
            "cargo test -p trace-commons-protocol",
        );
        verification_call.tool_name = Some("shell".to_string());
        let mut verification_result = evidence_event(
            TraceContributionEventType::ToolResult,
            "test result: ok. 42 passed; 0 failed",
        );
        verification_result.parent_event_id = Some(verification_call.event_id);
        verification_result.success = Some(true);
        let prioritized_ids = [
            before_correction.event_id,
            correction.event_id,
            after_correction.event_id,
            verification_call.event_id,
            verification_result.event_id,
        ];
        events.extend([
            before_correction,
            correction,
            after_correction,
            verification_call,
            verification_result,
        ]);

        let selected = select_session_evidence(
            &events,
            Some("Change the source schema and regenerate the output."),
        );
        assert_eq!(selected.len(), PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS);
        for id in prioritized_ids {
            assert!(
                selected.iter().any(|evidence| evidence.event_id == id),
                "prioritized evidence {id} was truncated"
            );
        }
        assert!(
            !selected
                .iter()
                .any(|evidence| evidence.excerpt == "Earlier transcript event 0"),
            "lower-priority head events must yield to decisive later evidence"
        );
        assert!(
            selected
                .windows(2)
                .all(|pair| pair[0].event_id != pair[1].event_id),
            "selection must not duplicate evidence"
        );
        let selected_positions = selected
            .iter()
            .map(|evidence| {
                events
                    .iter()
                    .position(|event| event.event_id == evidence.event_id)
                    .expect("selected evidence comes from the source trace")
            })
            .collect::<Vec<_>>();
        assert!(
            selected_positions.windows(2).all(|pair| pair[0] < pair[1]),
            "prioritization must preserve transcript order"
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
