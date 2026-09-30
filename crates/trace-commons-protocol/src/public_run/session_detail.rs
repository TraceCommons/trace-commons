//! Bounded, account-owned session detail for review and skill extraction.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::trace_contribution::{
    TaskSuccess, TraceAllowedUse, TraceContributionEnvelope, TraceContributionEvent,
    TraceContributionEventType, UserFeedback,
};

use crate::public_run::{
    PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS, PUBLIC_RUN_EVIDENCE_MAX_CHARS, PUBLIC_RUN_TASK_MAX_CHARS,
    PublicRunOwnerState,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunSessionEvidence {
    pub event_id: Uuid,
    pub kind: TraceContributionEventType,
    pub excerpt: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PublicRunContributionStatus {
    Received,
    Accepted,
    Quarantined,
    AwaitingPiiBackstop,
    Rejected,
    Revoked,
    Expired,
    Purged,
}

impl PublicRunContributionStatus {
    /// States whose owner detail remains useful after the redacted envelope is
    /// either not written yet or has been removed by a terminal lifecycle step.
    #[must_use]
    pub const fn supports_status_only_detail(self) -> bool {
        matches!(
            self,
            Self::Received | Self::Revoked | Self::Expired | Self::Purged
        )
    }
}

/// Bounded, account-owned projection used by the native session-detail view.
/// The stored envelope is reduced inside the server process so the client does
/// not download a multi-megabyte trace to render at most 24 excerpts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRunSessionRecord {
    /// True when the lifecycle metadata is available but the redacted envelope
    /// has not been written yet or has been removed by retention/withdrawal.
    #[serde(default, skip_serializing_if = "is_false")]
    pub content_unavailable: bool,
    /// The first nonempty redacted user message. Older servers omit it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Canonical corpus state. Missing status from an older server fails closed
    /// for acceptance-gated behavior while its remaining detail stays readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contribution_status: Option<PublicRunContributionStatus>,
    /// Current stored permissions, expressed with the canonical protocol enum.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_uses: Vec<TraceAllowedUse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_success: Option<TaskSuccess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<UserFeedback>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_correction: Option<String>,
    pub evidence: Vec<PublicRunSessionEvidence>,
    pub contributed_version: String,
    pub consent_policy_version: String,
    pub redaction_pipeline_version: String,
    pub owner_state: PublicRunOwnerState,
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
        let task = envelope.events.iter().find_map(|event| {
            if event.event_type != TraceContributionEventType::UserMessage {
                return None;
            }
            let content = event.redacted_content.as_deref()?.trim();
            if content.is_empty() {
                return None;
            }
            Some(content.chars().take(PUBLIC_RUN_TASK_MAX_CHARS).collect())
        });
        let evidence = select_session_evidence(
            &envelope.events,
            envelope.outcome.human_correction.as_deref(),
        );
        Self {
            content_unavailable: false,
            task,
            contribution_status: None,
            permitted_uses: envelope.trace_card.allowed_uses,
            task_success: Some(envelope.outcome.task_success),
            user_feedback: Some(envelope.outcome.user_feedback),
            human_correction: envelope.outcome.human_correction,
            evidence,
            contributed_version: envelope.schema_version,
            consent_policy_version: envelope.consent.policy_version,
            redaction_pipeline_version: envelope.privacy.redaction_pipeline_version,
            owner_state,
        }
    }

    /// Build an owner-visible lifecycle record without claiming that envelope
    /// content or outcome metadata was observed. The three version values and
    /// permitted uses must come from the authoritative submission row.
    #[must_use]
    pub fn status_only(
        contribution_status: PublicRunContributionStatus,
        permitted_uses: Vec<TraceAllowedUse>,
        contributed_version: String,
        consent_policy_version: String,
        redaction_pipeline_version: String,
        owner_state: PublicRunOwnerState,
    ) -> Option<Self> {
        if !contribution_status.supports_status_only_detail() {
            return None;
        }
        Some(Self {
            content_unavailable: true,
            task: None,
            contribution_status: Some(contribution_status),
            permitted_uses,
            task_success: None,
            user_feedback: None,
            human_correction: None,
            evidence: Vec::new(),
            contributed_version,
            consent_policy_version,
            redaction_pipeline_version,
            owner_state,
        })
    }

    /// Attach the current database state to the bounded envelope projection.
    #[must_use]
    pub fn with_contribution_state(
        mut self,
        contribution_status: PublicRunContributionStatus,
        permitted_uses: Vec<TraceAllowedUse>,
    ) -> Self {
        self.contribution_status = Some(contribution_status);
        self.permitted_uses = permitted_uses;
        self
    }

    #[must_use]
    pub fn is_accepted(&self) -> bool {
        self.contribution_status == Some(PublicRunContributionStatus::Accepted)
    }
}

pub(crate) fn select_session_evidence(
    events: &[TraceContributionEvent],
    human_correction: Option<&str>,
) -> Vec<PublicRunSessionEvidence> {
    let correction_anchor = human_correction
        .filter(|correction| !correction.trim().is_empty())
        .and_then(|_| {
            events
                .iter()
                .rposition(|event| event.event_type == TraceContributionEventType::Feedback)
                .or_else(|| {
                    events.iter().rposition(|event| {
                        event.event_type == TraceContributionEventType::UserMessage
                    })
                })
        });
    let verification_event_ids = events
        .iter()
        .filter(|event| is_verification_evidence(event))
        .map(|event| event.event_id)
        .collect::<BTreeSet<_>>();
    let mut ranked = Vec::with_capacity(PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS + 1);

    for (index, event) in events.iter().enumerate() {
        let Some(content) = event.redacted_content.as_deref().map(str::trim) else {
            continue;
        };
        if content.is_empty() {
            continue;
        }
        let correction_adjacent =
            correction_anchor.is_some_and(|anchor| index.abs_diff(anchor) <= 2);
        let verification = verification_event_ids.contains(&event.event_id)
            || event
                .parent_event_id
                .is_some_and(|parent| verification_event_ids.contains(&parent));
        let priority = if correction_adjacent {
            0
        } else if verification {
            1
        } else if event.event_type == TraceContributionEventType::ToolResult
            && (event.success.is_some() || !event.failure_modes.is_empty())
        {
            2
        } else {
            3
        };
        ranked.push((priority, index));
        ranked.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1))
        });
        if ranked.len() > PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS {
            ranked.pop();
        }
    }

    ranked.sort_unstable_by_key(|(_, index)| *index);
    ranked
        .into_iter()
        .filter_map(|(_, index)| events.get(index))
        .filter_map(|event| {
            let content = event.redacted_content.as_deref()?.trim();
            Some(PublicRunSessionEvidence {
                event_id: event.event_id,
                kind: event.event_type,
                excerpt: content
                    .chars()
                    .take(PUBLIC_RUN_EVIDENCE_MAX_CHARS)
                    .collect(),
            })
        })
        .collect()
}

fn is_verification_evidence(event: &TraceContributionEvent) -> bool {
    if !matches!(
        event.event_type,
        TraceContributionEventType::ToolCall
            | TraceContributionEventType::ToolResult
            | TraceContributionEventType::HttpExchange
    ) {
        return false;
    }
    event
        .tool_name
        .as_deref()
        .into_iter()
        .chain(event.redacted_content.as_deref())
        .any(has_verification_cue)
}

fn has_verification_cue(value: &str) -> bool {
    const WORD_CUES: [&str; 8] = [
        "build",
        "check",
        "clippy",
        "lint",
        "test",
        "tests",
        "verification",
        "verify",
    ];
    const COMMAND_CUES: [&str; 5] = ["pytest", "xcodebuild", "xctest", "unittest", "vitest"];
    WORD_CUES.iter().any(|cue| contains_ascii_word(value, cue))
        || COMMAND_CUES
            .iter()
            .any(|cue| contains_ascii_case_insensitive(value, cue))
}

fn contains_ascii_word(value: &str, word: &str) -> bool {
    value
        .as_bytes()
        .windows(word.len())
        .enumerate()
        .any(|(start, candidate)| {
            candidate.eq_ignore_ascii_case(word.as_bytes())
                && (start == 0 || !is_ascii_word_byte(value.as_bytes()[start - 1]))
                && value
                    .as_bytes()
                    .get(start + word.len())
                    .is_none_or(|byte| !is_ascii_word_byte(*byte))
        })
}

fn contains_ascii_case_insensitive(value: &str, needle: &str) -> bool {
    value
        .as_bytes()
        .windows(needle.len())
        .any(|candidate| candidate.eq_ignore_ascii_case(needle.as_bytes()))
}

const fn is_ascii_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}
