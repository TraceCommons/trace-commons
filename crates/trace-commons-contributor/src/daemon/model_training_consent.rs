//! IPC handlers for adding the `model_training` use to traces already sent
//! (owner decisions 2026-10-28).
//!
//! - `model_training_consent_status {}` answers how many of this
//!   contributor's traces were sent without "Train coding models" and could
//!   take it now: `{eligible_count, submission_ids}` (ids oldest first, at
//!   most 500).
//! - `model_training_consent_grant {all_eligible: true}` or
//!   `{submission_ids: [...]}` adds the use to those traces and re-runs
//!   only their withheld scoring award, on the server. The answer is the
//!   server's, per trace: `outcome` (`granted`, `already_granted`,
//!   `not_eligible`), `granted_at`, and `award` (`credited`, `pending`,
//!   `none`), plus `remaining_eligible`. "All eligible" asks the server
//!   again while traces remain (it grants 500 per call), up to 20 calls.
//!
//! Both relay `/v1/contributors/me/model-training-consent` on the device's
//! status claim (`crate::model_training_consent`). A grant asks the history
//! poller to read back statuses at once, so the traces' credit shows as it
//! lands. Refusals are fixed labels (`ConsentCallError::label`), never a
//! remote body.

use serde_json::Value;
use trace_commons_protocol::model_training_consent::{
    MODEL_TRAINING_CONSENT_MAX_IDS, ModelTrainingConsentGrantRequest, ModelTrainingConsentOutcome,
};
use uuid::Uuid;

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::model_training_consent::{ConsentCallError, fetch_eligibility, send_grant};

/// `model_training_consent_status`.
pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    match fetch_eligibility(&shared.store, &cfg).await {
        Ok(eligibility) => Response::ok(
            req.id,
            serde_json::to_value(eligibility).unwrap_or(Value::Null),
        ),
        Err(error) => refusal(req, error),
    }
}

/// `model_training_consent_grant`.
pub(super) async fn handle_grant(shared: &DaemonShared, req: &Request) -> Response {
    let request = match parse_grant(&req.params) {
        Ok(request) => request,
        Err(message) => return Response::err(req.id, ERR_BAD_PARAMS, message),
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let all_eligible = request == ModelTrainingConsentGrantRequest::AllEligible;
    let mut answer = match send_grant(&shared.store, &cfg, &request).await {
        Ok(answer) => answer,
        Err(error) => return refusal(req, error),
    };
    // "All eligible" means all of them: the server grants at most 500 per
    // call, so ask again while some remain and each call still grants.
    let mut rounds = 1;
    while all_eligible && answer.remaining_eligible > 0 && rounds < MAX_ALL_ELIGIBLE_ROUNDS {
        let Ok(next) = send_grant(&shared.store, &cfg, &request).await else {
            break;
        };
        rounds += 1;
        let progressed = next
            .results
            .iter()
            .any(|item| item.outcome == ModelTrainingConsentOutcome::Granted);
        answer.results.extend(next.results);
        answer.remaining_eligible = next.remaining_eligible;
        if !progressed {
            break;
        }
    }
    request_history_refresh(shared);
    Response::ok(req.id, serde_json::to_value(answer).unwrap_or(Value::Null))
}

/// At most this many server calls for one "all eligible" grant (500 traces
/// each). A contributor with more is told `remaining_eligible`.
const MAX_ALL_ELIGIBLE_ROUNDS: usize = 20;

fn refusal(req: &Request, error: ConsentCallError) -> Response {
    let code = match error {
        ConsentCallError::BadSelection => ERR_BAD_PARAMS,
        _ => ERR_UNAVAILABLE,
    };
    Response::err(req.id, code, error.label())
}

/// Exactly one of `all_eligible: true` or a non-empty `submission_ids` of at
/// most 500 ids. Nothing is chosen for the caller.
fn parse_grant(params: &Value) -> Result<ModelTrainingConsentGrantRequest, &'static str> {
    let all = params.get("all_eligible");
    let ids = params.get("submission_ids");
    match (all, ids) {
        (Some(Value::Bool(true)), None) => Ok(ModelTrainingConsentGrantRequest::AllEligible),
        (None, Some(Value::Array(ids))) => {
            if ids.is_empty() || ids.len() > MODEL_TRAINING_CONSENT_MAX_IDS {
                return Err("submission_ids-invalid");
            }
            let submission_ids = ids
                .iter()
                .map(|id| id.as_str().and_then(|id| id.parse::<Uuid>().ok()))
                .collect::<Option<Vec<_>>>()
                .ok_or("submission_ids-invalid")?;
            Ok(ModelTrainingConsentGrantRequest::SubmissionIds { submission_ids })
        }
        _ => Err("selection-required"),
    }
}

/// Ask the poller to read back statuses now, as `refresh_history` does. A
/// failed save only delays the read-back to the next interval.
fn request_history_refresh(shared: &DaemonShared) {
    let mut state = shared.state.lock().expect("state lock");
    let now = chrono::Utc::now();
    if state.history_refresh_due_at.is_none_or(|due| due > now) {
        state.history_refresh_due_at = Some(now);
        let _ = state.save(&shared.store);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_grant_names_one_selection() {
        assert_eq!(
            parse_grant(&json!({"all_eligible": true})),
            Ok(ModelTrainingConsentGrantRequest::AllEligible)
        );
        let id = Uuid::new_v4();
        assert_eq!(
            parse_grant(&json!({"submission_ids": [id.to_string()]})),
            Ok(ModelTrainingConsentGrantRequest::SubmissionIds {
                submission_ids: vec![id]
            })
        );
        for params in [
            json!({}),
            json!({"all_eligible": false}),
            json!({"all_eligible": true, "submission_ids": [id.to_string()]}),
        ] {
            assert_eq!(parse_grant(&params), Err("selection-required"), "{params}");
        }
        for params in [
            json!({"submission_ids": []}),
            json!({"submission_ids": ["not-a-uuid"]}),
            json!({"submission_ids": (0..=MODEL_TRAINING_CONSENT_MAX_IDS)
                .map(|_| Uuid::new_v4().to_string())
                .collect::<Vec<_>>()}),
        ] {
            assert_eq!(
                parse_grant(&params),
                Err("submission_ids-invalid"),
                "{params}"
            );
        }
    }

    #[test]
    fn a_refusal_is_a_fixed_label() {
        let req = Request {
            id: 7,
            method: "model_training_consent_grant".into(),
            params: json!({}),
        };
        let response = refusal(&req, ConsentCallError::NotPermitted);
        let error = response.error.unwrap();
        assert_eq!(error.code, ERR_UNAVAILABLE);
        assert_eq!(error.message, "model-training-not-permitted");
        let response = refusal(&req, ConsentCallError::BadSelection);
        assert_eq!(response.error.unwrap().code, ERR_BAD_PARAMS);
    }
}
