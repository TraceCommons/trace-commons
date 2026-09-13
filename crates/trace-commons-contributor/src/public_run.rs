//! Account-authenticated session detail and explicit public-run publication,
//! projected from the permanently redacted owned-content endpoint. Evidence
//! remains exact for server verification; tokens and raw errors never cross.

use reqwest::Method;
use serde::Serialize;
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER;
use trace_commons_protocol::public_run::{
    PublicRunContributionStatus, PublicRunPage, PublicRunPublishRequest, PublicRunSessionRecord,
    PublicRunUnpublishResult,
};
use trace_commons_protocol::trace_contribution::{
    TaskSuccess, TraceAllowedUse, TraceContributionEventType, UserFeedback,
};

use crate::config::allowlist_for;

mod copy;

pub use copy::*;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionEvidenceCandidate {
    pub event_id: Uuid,
    pub kind: TraceContributionEventType,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionDetail {
    pub content_unavailable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution_status: Option<PublicRunContributionStatus>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub permitted_uses: Vec<TraceAllowedUse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_success: Option<TaskSuccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_feedback: Option<UserFeedback>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_correction: Option<String>,
    pub evidence: Vec<SessionEvidenceCandidate>,
    pub contributed_version: String,
    pub consent_policy_version: String,
    pub redaction_pipeline_version: String,
    pub publication: Option<PublicRunPage>,
    /// Monotonic owner-state version to return with the next reviewed draft.
    pub publication_version: u32,
    pub retained_source_slug: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRunClientError {
    SessionInvalid,
    NotFound,
    SourceNotFound,
    Invalid,
    Conflict,
    Unavailable,
}

pub struct AccountCall<T> {
    pub result: Result<T, PublicRunClientError>,
    pub rotated_token: Option<String>,
}

fn client(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    account_session_token: &str,
) -> Result<Client, PublicRunClientError> {
    Client::builder(ingest_url, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
        .bearer_token(account_session_token)
        .host_allowlist(allowlist_for(allowed_hosts))
        .build()
        .map_err(|_| PublicRunClientError::Unavailable)
}

pub async fn call_session_detail(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    account_session_token: &str,
    submission_id: Uuid,
) -> AccountCall<SessionDetail> {
    let client = match client(ingest_url, allowed_hosts, account_session_token) {
        Ok(client) => client,
        Err(error) => {
            return AccountCall {
                result: Err(error),
                rotated_token: None,
            };
        }
    };
    let path = format!("/v1/account/traces/{submission_id}/session-detail");
    let response = client
        .call_json_with_response_header::<(), PublicRunSessionRecord>(
            Method::GET,
            &path,
            &[],
            None,
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    let result = response
        .result
        .map(project_session_detail)
        .map_err(|error| classify(&error));
    AccountCall {
        result,
        rotated_token: response.response_header,
    }
}

pub async fn call_publish(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    account_session_token: &str,
    submission_id: Uuid,
    request: &PublicRunPublishRequest,
) -> AccountCall<PublicRunPage> {
    if request.validate().is_err() {
        return AccountCall {
            result: Err(PublicRunClientError::Invalid),
            rotated_token: None,
        };
    }
    let client = match client(ingest_url, allowed_hosts, account_session_token) {
        Ok(client) => client,
        Err(error) => {
            return AccountCall {
                result: Err(error),
                rotated_token: None,
            };
        }
    };
    let path = format!("/v1/account/traces/{submission_id}/publication");
    let response = client
        .call_json_with_response_header::<PublicRunPublishRequest, PublicRunPage>(
            Method::PUT,
            &path,
            &[],
            Some(request),
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    AccountCall {
        result: response.result.map_err(|error| classify(&error)),
        rotated_token: response.response_header,
    }
}

pub async fn call_unpublish(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    account_session_token: &str,
    submission_id: Uuid,
) -> AccountCall<PublicRunUnpublishResult> {
    let client = match client(ingest_url, allowed_hosts, account_session_token) {
        Ok(client) => client,
        Err(error) => {
            return AccountCall {
                result: Err(error),
                rotated_token: None,
            };
        }
    };
    let path = format!("/v1/account/traces/{submission_id}/publication");
    let response = client
        .call_json_with_response_header::<(), PublicRunUnpublishResult>(
            Method::DELETE,
            &path,
            &[],
            None,
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    AccountCall {
        result: response.result.map_err(|error| classify(&error)),
        rotated_token: response.response_header,
    }
}

pub fn project_session_detail(record: PublicRunSessionRecord) -> SessionDetail {
    let evidence = record
        .evidence
        .into_iter()
        .map(|event| SessionEvidenceCandidate {
            event_id: event.event_id,
            kind: event.kind,
            excerpt: event.excerpt,
        })
        .collect();
    let owner_state = record.owner_state;
    SessionDetail {
        content_unavailable: record.content_unavailable,
        task: record.task,
        contribution_status: record.contribution_status,
        permitted_uses: record.permitted_uses,
        task_success: record.task_success,
        user_feedback: record.user_feedback,
        human_correction: record.human_correction,
        evidence,
        contributed_version: record.contributed_version,
        consent_policy_version: record.consent_policy_version,
        redaction_pipeline_version: record.redaction_pipeline_version,
        publication: owner_state.publication,
        publication_version: owner_state.expected_publication_version,
        retained_source_slug: owner_state.retained_source_slug,
    }
}

fn classify(error: &OcError) -> PublicRunClientError {
    match error {
        OcError::ServerLabel { status, label, .. } => match status.as_u16() {
            400 | 422 => PublicRunClientError::Invalid,
            401 | 403 => PublicRunClientError::SessionInvalid,
            404 if label == "source public run not found" => PublicRunClientError::SourceNotFound,
            404 => PublicRunClientError::NotFound,
            409 => PublicRunClientError::Conflict,
            _ => PublicRunClientError::Unavailable,
        },
        OcError::HttpFailure { status, .. } => match status.as_u16() {
            400 | 422 => PublicRunClientError::Invalid,
            401 | 403 => PublicRunClientError::SessionInvalid,
            404 => PublicRunClientError::NotFound,
            409 => PublicRunClientError::Conflict,
            _ => PublicRunClientError::Unavailable,
        },
        _ => PublicRunClientError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::Path,
        http::HeaderMap,
        routing::{get, put},
    };
    use chrono::Utc;
    use serde_json::json;
    use trace_commons_protocol::public_run::{
        PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS, PUBLIC_RUN_EVIDENCE_MAX_CHARS,
        PUBLIC_RUN_TASK_MAX_CHARS, PublicRunDraft, PublicRunEvidence, PublicRunEvidenceDraft,
        PublicRunOwnerState, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::{
        ConsentMetadata, ConsentScope, ContributorMetadata, IronclawTraceMetadata, OutcomeMetadata,
        PrivacyMetadata, ReplayMetadata, ResidualPiiRisk, SideEffectLevel, TraceAllowedUse,
        TraceCard, TraceChannel, TraceContributionEnvelope, TraceContributionEvent, TraceValueCard,
        ValueMetadata,
    };

    use super::*;

    fn envelope() -> TraceContributionEnvelope {
        TraceContributionEnvelope {
            schema_version: "trace.contribution.v1".to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: Utc::now(),
            ironclaw: IronclawTraceMetadata {
                version: "0.12.1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: "policy-v1".to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                correction_included: true,
                routing_metadata_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: "privacy-v2".to_string(),
                redaction_counts: BTreeMap::new(),
                redaction_distinct_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:test".to_string(),
                warnings: Vec::new(),
            },
            events: vec![TraceContributionEvent {
                event_id: Uuid::nil(),
                parent_event_id: None,
                event_type: TraceContributionEventType::ToolResult,
                timestamp: Utc::now(),
                redacted_content: Some("The build completed successfully.".to_string()),
                structured_payload: json!(null),
                tool_name: Some("build".to_string()),
                tool_category: None,
                tool_call_id: None,
                latency_ms: None,
                token_counts: None,
                cost_usd: None,
                success: Some(true),
                failure_modes: Vec::new(),
                side_effect: SideEffectLevel::ReadOnly,
            }],
            outcome: OutcomeMetadata {
                user_feedback: UserFeedback::Correction,
                task_success: TaskSuccess::Partial,
                error_taxonomy: Vec::new(),
                failure_modes: Vec::new(),
                human_correction: Some("Run the full build before publishing.".to_string()),
            },
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            conversation_id: None,
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    #[test]
    fn detail_uses_real_outcome_correction_evidence_and_versions() {
        let mut source = envelope();
        let template = source.events[0].clone();
        let mut empty_task = template.clone();
        empty_task.event_id = Uuid::new_v4();
        empty_task.event_type = TraceContributionEventType::UserMessage;
        empty_task.redacted_content = Some(" \n\t ".to_string());
        let mut bounded_task = template;
        bounded_task.event_id = Uuid::new_v4();
        bounded_task.event_type = TraceContributionEventType::UserMessage;
        bounded_task.redacted_content =
            Some(format!("  {}  ", "t".repeat(PUBLIC_RUN_TASK_MAX_CHARS + 1)));
        source.events.insert(0, bounded_task);
        source.events.insert(0, empty_task);
        source.trace_card.allowed_uses =
            vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation];
        let record = PublicRunSessionRecord::from_envelope(
            source,
            PublicRunOwnerState {
                publication: None,
                expected_publication_version: 0,
                retained_source_slug: Some("source-run".to_string()),
            },
        )
        .with_contribution_state(
            PublicRunContributionStatus::Accepted,
            vec![TraceAllowedUse::Evaluation],
        );
        let detail = project_session_detail(record);
        assert_eq!(
            detail.contribution_status,
            Some(PublicRunContributionStatus::Accepted)
        );
        assert!(!detail.content_unavailable);
        assert_eq!(
            detail.task.as_deref().map(|task| task.chars().count()),
            Some(PUBLIC_RUN_TASK_MAX_CHARS)
        );
        assert!(
            detail
                .task
                .as_deref()
                .is_some_and(|task| task.chars().all(|character| character == 't'))
        );
        assert_eq!(
            detail.contribution_status,
            Some(PublicRunContributionStatus::Accepted)
        );
        assert_eq!(detail.permitted_uses, vec![TraceAllowedUse::Evaluation]);
        assert_eq!(detail.task_success, Some(TaskSuccess::Partial));
        assert_eq!(detail.user_feedback, Some(UserFeedback::Correction));
        assert_eq!(
            detail.human_correction.as_deref(),
            Some("Run the full build before publishing.")
        );
        assert_eq!(
            detail.evidence[1].kind,
            TraceContributionEventType::ToolResult
        );
        assert_eq!(detail.evidence[1].event_id, Uuid::nil());
        assert_eq!(detail.contributed_version, "trace.contribution.v1");
        assert_eq!(detail.consent_policy_version, "policy-v1");
        assert_eq!(detail.redaction_pipeline_version, "privacy-v2");
        assert_eq!(detail.retained_source_slug.as_deref(), Some("source-run"));
        let wire = serde_json::to_value(&detail).expect("serialize typed session detail");
        assert_eq!(wire.get("task_success"), Some(&json!("partial")));
        assert_eq!(wire.get("user_feedback"), Some(&json!("correction")));
        assert_eq!(
            wire.pointer("/evidence/1/kind"),
            Some(&json!("tool_result"))
        );
        for presentation_field in ["task_outcome", "feedback_line"] {
            assert!(wire.get(presentation_field).is_none());
        }
        assert!(wire.pointer("/evidence/1/label").is_none());
    }

    #[test]
    fn detail_drops_empty_evidence_and_bounds_excerpt_count_and_length() {
        let mut source = envelope();
        let template = source.events[0].clone();
        let mut events = Vec::new();

        let mut empty = template.clone();
        empty.event_id = Uuid::new_v4();
        empty.redacted_content = Some(" \n\t ".to_string());
        events.push(empty);

        let mut oversized = template.clone();
        oversized.event_id = Uuid::new_v4();
        oversized.redacted_content = Some("x".repeat(PUBLIC_RUN_EVIDENCE_MAX_CHARS + 1));
        for index in 0..30 {
            let mut event = template.clone();
            event.event_id = Uuid::new_v4();
            event.redacted_content = Some(format!("Synthetic evidence {index}"));
            events.push(event);
        }
        events.push(oversized);
        source.events = events;

        let detail = project_session_detail(PublicRunSessionRecord::from_envelope(
            source,
            PublicRunOwnerState {
                publication: None,
                expected_publication_version: 0,
                retained_source_slug: None,
            },
        ));
        assert_eq!(detail.evidence.len(), PUBLIC_RUN_DETAIL_EVIDENCE_MAX_ITEMS);
        assert_eq!(
            detail.evidence.last().unwrap().excerpt.chars().count(),
            PUBLIC_RUN_EVIDENCE_MAX_CHARS
        );
        assert!(
            detail
                .evidence
                .iter()
                .all(|item| !item.excerpt.trim().is_empty())
        );
        assert!(
            detail
                .evidence
                .iter()
                .any(|item| item.excerpt == "Synthetic evidence 29")
        );
        assert!(
            !detail
                .evidence
                .iter()
                .any(|item| item.excerpt == "Synthetic evidence 0")
        );
    }

    #[test]
    fn status_only_detail_does_not_turn_missing_outcome_into_display_copy() {
        let record = PublicRunSessionRecord::status_only(
            PublicRunContributionStatus::Revoked,
            vec![TraceAllowedUse::Evaluation],
            "trace.contribution.v1".to_string(),
            "policy-v1".to_string(),
            "privacy-v2".to_string(),
            PublicRunOwnerState {
                publication: None,
                expected_publication_version: 2,
                retained_source_slug: None,
            },
        )
        .expect("revoked status-only record");

        let detail = project_session_detail(record);
        assert_ne!(
            detail.contribution_status,
            Some(PublicRunContributionStatus::Accepted)
        );
        assert!(detail.content_unavailable);
        assert!(detail.task.is_none());
        assert!(detail.task_success.is_none());
        assert!(detail.user_feedback.is_none());
        assert!(detail.human_correction.is_none());
        assert!(detail.evidence.is_empty());
        let json = serde_json::to_value(detail).expect("serialize status-only projection");
        for absent in ["task", "task_success", "user_feedback", "human_correction"] {
            assert!(json.get(absent).is_none(), "{absent} must stay absent");
        }
    }

    fn page() -> PublicRunPage {
        PublicRunPage {
            slug: "run-0123456789abcdef".to_string(),
            title: "Repair a stalled upload".to_string(),
            outcome_summary: "The upload completed.".to_string(),
            correction_excerpt: None,
            workflow: "Renew the session, then retry once.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidence {
                excerpt: "The retry succeeded.".to_string(),
            }],
            task_success: TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            version: 1,
            published_at: Utc::now(),
            public_url: Some("https://community.example/runs/run-0123456789abcdef".to_string()),
            source: None,
            source_unavailable: false,
            variations: Vec::new(),
        }
    }

    #[tokio::test]
    async fn publish_uses_the_account_bearer_and_exact_approval() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let captured = received.clone();
        let router = Router::new().route(
            "/v1/account/traces/{submission_id}/publication",
            put(
                move |headers: HeaderMap,
                      Path(_): Path<String>,
                      Json(body): Json<PublicRunPublishRequest>| {
                    let captured = captured.clone();
                    async move {
                        captured.lock().unwrap().push((
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("")
                                .to_string(),
                            body.approval_sha256,
                        ));
                        Json(page())
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let draft = PublicRunDraft {
            title: "Repair a stalled upload".to_string(),
            outcome_summary: "The upload completed.".to_string(),
            correction_excerpt: None,
            workflow: "Renew the session, then retry once.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::nil(),
                excerpt: "The retry succeeded.".to_string(),
            }],
            source_slug: None,
        };
        let approval_sha256 = draft
            .approval_sha256(TaskSuccess::Success, "trace.contribution.v1", 0)
            .unwrap();
        let request = PublicRunPublishRequest {
            draft,
            task_success: TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            expected_publication_version: 0,
            approval_sha256: approval_sha256.clone(),
        };
        let response = call_publish(
            &format!("http://{address}"),
            None,
            "session-token",
            Uuid::new_v4(),
            &request,
        )
        .await;
        assert_eq!(
            response.result.expect("publish response").slug,
            "run-0123456789abcdef"
        );
        assert_eq!(
            received.lock().unwrap()[0],
            ("Bearer session-token".to_string(), approval_sha256)
        );
    }

    #[tokio::test]
    async fn account_client_reads_detail_and_unpublishes_idempotently() {
        let router = Router::new()
            .route(
                "/v1/account/traces/{submission_id}/session-detail",
                get(|| async {
                    Json(PublicRunSessionRecord::from_envelope(
                        envelope(),
                        PublicRunOwnerState {
                            publication: Some(page()),
                            expected_publication_version: 1,
                            retained_source_slug: None,
                        },
                    ))
                }),
            )
            .route(
                "/v1/account/traces/{submission_id}/publication",
                get(|| async {
                    Json(PublicRunOwnerState {
                        publication: Some(page()),
                        expected_publication_version: 1,
                        retained_source_slug: None,
                    })
                })
                .delete(|| async {
                    Json(PublicRunUnpublishResult {
                        unpublished: true,
                        expected_publication_version: 2,
                    })
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let submission_id = Uuid::new_v4();
        let detail = call_session_detail(
            &format!("http://{address}"),
            None,
            "session-token",
            submission_id,
        )
        .await
        .result
        .expect("detail response");
        assert_eq!(detail.task_success, Some(TaskSuccess::Partial));
        assert_eq!(
            detail.publication.as_ref().map(|run| run.slug.as_str()),
            Some("run-0123456789abcdef")
        );
        let unpublish = call_unpublish(
            &format!("http://{address}"),
            None,
            "session-token",
            submission_id,
        )
        .await
        .result
        .expect("unpublish response");
        assert!(unpublish.unpublished);
        assert_eq!(unpublish.expected_publication_version, 2);
    }

    #[test]
    fn account_client_maps_typed_and_untyped_http_failures() {
        let labeled = |status, label: &str| OcError::ServerLabel {
            url: "https://ingest.example/v1/account/traces/id/publication".to_string(),
            status,
            label: label.to_string(),
            body: String::new(),
        };
        assert_eq!(
            classify(&labeled(
                reqwest::StatusCode::NOT_FOUND,
                "source public run not found"
            )),
            PublicRunClientError::SourceNotFound
        );
        assert_eq!(
            classify(&labeled(reqwest::StatusCode::NOT_FOUND, "trace not found")),
            PublicRunClientError::NotFound
        );
        for (status, expected) in [
            (
                reqwest::StatusCode::BAD_REQUEST,
                PublicRunClientError::Invalid,
            ),
            (
                reqwest::StatusCode::UNPROCESSABLE_ENTITY,
                PublicRunClientError::Invalid,
            ),
            (
                reqwest::StatusCode::UNAUTHORIZED,
                PublicRunClientError::SessionInvalid,
            ),
            (
                reqwest::StatusCode::FORBIDDEN,
                PublicRunClientError::SessionInvalid,
            ),
            (
                reqwest::StatusCode::CONFLICT,
                PublicRunClientError::Conflict,
            ),
            (
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                PublicRunClientError::Unavailable,
            ),
        ] {
            assert_eq!(classify(&labeled(status, "fixed")), expected);
        }
        let untyped = OcError::HttpFailure {
            url: "https://ingest.example/v1/account/traces/id/publication".to_string(),
            status: reqwest::StatusCode::NOT_FOUND,
            body: String::new(),
        };
        assert_eq!(classify(&untyped), PublicRunClientError::NotFound);
    }
}
