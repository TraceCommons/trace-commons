//! Wire contract for adding the `model_training` use to traces a contributor
//! already sent (owner decisions 2026-10-28).
//!
//! The scoring award (`NoveltyUtility`) requires `model_training` in a
//! submission's stored allowed uses; a trace sent without it has its award
//! withheld as `policy_mismatch`. These routes let the contributor add that
//! one use afterwards:
//!
//! - `GET` [`MODEL_TRAINING_CONSENT_PATH`] lists the caller's eligible
//!   traces: [`ModelTrainingConsentEligibility`].
//! - `POST` [`MODEL_TRAINING_CONSENT_PATH`] with a
//!   [`ModelTrainingConsentGrantRequest`] adds the use to the listed traces
//!   (or every eligible one) and re-runs only their withheld award:
//!   [`ModelTrainingConsentGrantResponse`].
//!
//! The change is add-only: it adds the `model_training` consent scope and
//! the `model_training` allowed use and nothing else, and never removes
//! anything (removal is withdrawal). Each grant is stored with the time it
//! was given. A trace is eligible when it is the caller's own, `accepted`,
//! not withdrawn, revoked, purged or expired, and lacks `model_training`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The route, on the contributor's device credential (the same upload claim
/// the submission-status read uses).
pub const MODEL_TRAINING_CONSENT_PATH: &str = "/v1/contributors/me/model-training-consent";

/// Most ids one eligibility answer lists, and most one grant request may
/// name. "All eligible" grants at most this many per call; the answer's
/// `remaining_eligible` says whether another call is needed.
pub const MODEL_TRAINING_CONSENT_MAX_IDS: usize = 500;

/// Refusal label: the caller's access-grant ceiling, or the tenant's policy,
/// does not include model training.
pub const MODEL_TRAINING_NOT_PERMITTED_LABEL: &str = "model_training_not_permitted";

/// Refusal label: the deployment has no database to record the grant in.
pub const MODEL_TRAINING_CONSENT_UNAVAILABLE_LABEL: &str = "model_training_consent_unavailable";

/// Refusal label: a request named more than [`MODEL_TRAINING_CONSENT_MAX_IDS`]
/// ids, or none.
pub const MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL: &str = "model_training_consent_bad_selection";

/// The caller's traces that could take the use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTrainingConsentEligibility {
    /// Every eligible trace, not only the listed ones.
    pub eligible_count: u64,
    /// Oldest first, at most [`MODEL_TRAINING_CONSENT_MAX_IDS`].
    #[serde(default)]
    pub submission_ids: Vec<Uuid>,
}

/// Which traces to grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case")]
pub enum ModelTrainingConsentGrantRequest {
    /// Every eligible trace, oldest first, up to
    /// [`MODEL_TRAINING_CONSENT_MAX_IDS`].
    AllEligible,
    /// These traces. An id that is not eligible is answered `not_eligible`
    /// and nothing changes for it.
    SubmissionIds { submission_ids: Vec<Uuid> },
}

/// What the grant did for one trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTrainingConsentOutcome {
    /// The use was added now.
    Granted,
    /// The trace already carried it (a repeat call, or it was sent with it).
    AlreadyGranted,
    /// Not the caller's, not accepted, withdrawn, revoked, purged, expired,
    /// or unknown. Nothing changed.
    NotEligible,
}

/// The trace's scoring award after the grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTrainingAwardState {
    /// The withheld award was written now, or had been before.
    Credited,
    /// Not decided yet: the trace has not been scored, or the award could
    /// not be checked this time. Scoring, or a repeat grant, decides it.
    Pending,
    /// No award follows from the grant: the trace did not pass scoring, its
    /// award was withheld for another reason, or it is not eligible.
    None,
}

/// One trace's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTrainingConsentGrantItem {
    pub submission_id: Uuid,
    pub outcome: ModelTrainingConsentOutcome,
    /// When the use was added, for `granted` and `already_granted` traces
    /// whose grant is recorded. Absent for a trace sent with the use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<DateTime<Utc>>,
    pub award: ModelTrainingAwardState,
    /// A label-only reason when `award` is `none` and the award was withheld
    /// again (for example `central_issuer_denied`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub award_withheld_reason: Option<String>,
}

/// The grant's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTrainingConsentGrantResponse {
    pub results: Vec<ModelTrainingConsentGrantItem>,
    /// Eligible traces left after this call.
    pub remaining_eligible: u64,
}

impl ModelTrainingConsentGrantRequest {
    /// The named ids, or `None` for all eligible. Refuses an empty list and
    /// one over [`MODEL_TRAINING_CONSENT_MAX_IDS`] with
    /// [`MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL`]. Repeated ids are kept
    /// once, in first-seen order.
    pub fn selected_ids(&self) -> Result<Option<Vec<Uuid>>, &'static str> {
        match self {
            Self::AllEligible => Ok(None),
            Self::SubmissionIds { submission_ids } => {
                if submission_ids.is_empty()
                    || submission_ids.len() > MODEL_TRAINING_CONSENT_MAX_IDS
                {
                    return Err(MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL);
                }
                let mut seen = std::collections::BTreeSet::new();
                Ok(Some(
                    submission_ids
                        .iter()
                        .copied()
                        .filter(|id| seen.insert(*id))
                        .collect(),
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_request_wire_shapes() {
        let all: ModelTrainingConsentGrantRequest =
            serde_json::from_str(r#"{"selection":"all_eligible"}"#).unwrap();
        assert_eq!(all, ModelTrainingConsentGrantRequest::AllEligible);
        let id = Uuid::nil();
        let some: ModelTrainingConsentGrantRequest = serde_json::from_value(serde_json::json!({
            "selection": "submission_ids",
            "submission_ids": [id],
        }))
        .unwrap();
        assert_eq!(
            some,
            ModelTrainingConsentGrantRequest::SubmissionIds {
                submission_ids: vec![id]
            }
        );
        assert!(
            serde_json::from_str::<ModelTrainingConsentGrantRequest>(r#"{"selection":"none"}"#)
                .is_err()
        );
    }

    #[test]
    fn selection_is_bounded_and_deduplicated() {
        assert_eq!(
            ModelTrainingConsentGrantRequest::AllEligible.selected_ids(),
            Ok(None)
        );
        let empty = ModelTrainingConsentGrantRequest::SubmissionIds {
            submission_ids: Vec::new(),
        };
        assert_eq!(
            empty.selected_ids(),
            Err(MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL)
        );
        let too_many = ModelTrainingConsentGrantRequest::SubmissionIds {
            submission_ids: (0..=MODEL_TRAINING_CONSENT_MAX_IDS)
                .map(|_| Uuid::new_v4())
                .collect(),
        };
        assert_eq!(
            too_many.selected_ids(),
            Err(MODEL_TRAINING_CONSENT_BAD_SELECTION_LABEL)
        );
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let repeated = ModelTrainingConsentGrantRequest::SubmissionIds {
            submission_ids: vec![a, b, a],
        };
        assert_eq!(repeated.selected_ids(), Ok(Some(vec![a, b])));
    }

    #[test]
    fn grant_response_wire_shape() {
        let id = Uuid::nil();
        let response = ModelTrainingConsentGrantResponse {
            results: vec![ModelTrainingConsentGrantItem {
                submission_id: id,
                outcome: ModelTrainingConsentOutcome::AlreadyGranted,
                granted_at: None,
                award: ModelTrainingAwardState::Pending,
                award_withheld_reason: None,
            }],
            remaining_eligible: 3,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "results": [{
                    "submission_id": id,
                    "outcome": "already_granted",
                    "award": "pending",
                }],
                "remaining_eligible": 3,
            })
        );
        let back: ModelTrainingConsentGrantResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn eligibility_tolerates_a_missing_id_list() {
        let parsed: ModelTrainingConsentEligibility =
            serde_json::from_str(r#"{"eligible_count":4}"#).unwrap();
        assert_eq!(parsed.eligible_count, 4);
        assert!(parsed.submission_ids.is_empty());
    }
}
