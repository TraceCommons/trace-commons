//! IPC bridge for owned session detail and reviewed public pages.
//!
//! Every network call uses the short-lived account session established by the
//! browser login flow. The device upload key has no read or publication
//! authority. Errors crossing IPC are fixed labels and never include trace
//! text, server bodies, URLs, or tokens.

use serde::Deserialize;
use uuid::Uuid;

use trace_commons_protocol::public_run::{PublicRunDraft, PublicRunPublishRequest};
use trace_commons_protocol::trace_contribution::TaskSuccess;

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};

pub const ERR_ACCOUNT_SESSION_REQUIRED: &str = "account-session-required";
const CREDENTIAL_STORAGE_UNAVAILABLE: &str = "commons_credential_storage_unavailable";

#[derive(Deserialize)]
struct PublishParams {
    submission_id: Uuid,
    draft: PublicRunDraft,
    task_success: TaskSuccess,
    contributed_version: String,
    /// `publication_version` from the reviewed `history_detail` response.
    /// The digest and database compare-and-swap both bind this value.
    expected_publication_version: u32,
}

fn parse_submission_id(params: &serde_json::Value) -> Result<Uuid, &'static str> {
    params
        .get("submission_id")
        .and_then(|value| value.as_str())
        .ok_or("submission_id-required")?
        .parse()
        .map_err(|_| "submission_id-invalid")
}

fn account_session(
    shared: &DaemonShared,
) -> anyhow::Result<Option<crate::account_auth::LoadedAccountSession>> {
    super::run_blocking(|| crate::account_auth::try_load_session_with_snapshot(&shared.store))
}

fn account_context(
    shared: &DaemonShared,
    request_id: u64,
) -> Result<
    (
        crate::config::ContributorConfig,
        crate::account_auth::LoadedAccountSession,
    ),
    Box<Response>,
> {
    let session = match account_session(shared) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Err(Box::new(Response::err(
                request_id,
                ERR_UNAVAILABLE,
                ERR_ACCOUNT_SESSION_REQUIRED,
            )));
        }
        Err(_) => {
            return Err(Box::new(Response::err(
                request_id,
                ERR_UNAVAILABLE,
                CREDENTIAL_STORAGE_UNAVAILABLE,
            )));
        }
    };
    let config = match shared.store.load_config() {
        Ok(Some(config)) => config,
        _ => {
            return Err(Box::new(Response::err(
                request_id,
                ERR_UNAVAILABLE,
                "not-logged-in",
            )));
        }
    };
    Ok((config, session))
}

fn persist_rotated_token(
    shared: &DaemonShared,
    session: &crate::account_auth::LoadedAccountSession,
    rotated_token: Option<String>,
) -> Result<(), ()> {
    let Some(rotated_token) = rotated_token else {
        return Ok(());
    };
    super::run_blocking(|| {
        crate::account_auth::store_rotated_token(&shared.store, session, rotated_token)
    })
    .map_err(|_| ())
}

fn credential_storage_error(request_id: u64) -> Response {
    Response::err(request_id, ERR_UNAVAILABLE, CREDENTIAL_STORAGE_UNAVAILABLE)
}

fn attach_credential_warning(value: &mut serde_json::Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "credential_warning".to_string(),
            serde_json::Value::String(CREDENTIAL_STORAGE_UNAVAILABLE.to_string()),
        );
    }
}

fn detail_error(request_id: u64, error: crate::public_run::PublicRunClientError) -> Response {
    match error {
        crate::public_run::PublicRunClientError::SessionInvalid => {
            Response::err(request_id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED)
        }
        crate::public_run::PublicRunClientError::NotFound => {
            Response::err(request_id, ERR_UNAVAILABLE, "session-detail-not-found")
        }
        _ => Response::err(request_id, ERR_UNAVAILABLE, "session-detail-unavailable"),
    }
}

fn publish_error(request_id: u64, error: crate::public_run::PublicRunClientError) -> Response {
    match error {
        crate::public_run::PublicRunClientError::SessionInvalid => {
            Response::err(request_id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED)
        }
        crate::public_run::PublicRunClientError::NotFound => {
            Response::err(request_id, ERR_UNAVAILABLE, "public-run-trace-not-found")
        }
        crate::public_run::PublicRunClientError::SourceNotFound => {
            Response::err(request_id, ERR_UNAVAILABLE, "public-run-source-not-found")
        }
        crate::public_run::PublicRunClientError::Conflict => {
            Response::err(request_id, ERR_BAD_PARAMS, "public-run-conflict")
        }
        crate::public_run::PublicRunClientError::Invalid => {
            Response::err(request_id, ERR_BAD_PARAMS, "public-run-invalid")
        }
        crate::public_run::PublicRunClientError::Unavailable => {
            Response::err(request_id, ERR_UNAVAILABLE, "public-run-publish-failed")
        }
    }
}

fn unpublish_error(request_id: u64, error: crate::public_run::PublicRunClientError) -> Response {
    match error {
        crate::public_run::PublicRunClientError::SessionInvalid => {
            Response::err(request_id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED)
        }
        _ => Response::err(request_id, ERR_UNAVAILABLE, "public-run-unpublish-failed"),
    }
}

pub(super) async fn handle_detail(shared: &DaemonShared, req: &Request) -> Response {
    let submission_id = match parse_submission_id(&req.params) {
        Ok(submission_id) => submission_id,
        Err(message) => return Response::err(req.id, ERR_BAD_PARAMS, message),
    };
    let (config, session) = match account_context(shared, req.id) {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let call = crate::public_run::call_session_detail(
        &config.ingest_url,
        config.allowed_hosts.as_deref(),
        &session.session.access_token,
        submission_id,
    )
    .await;
    if persist_rotated_token(shared, &session, call.rotated_token).is_err() {
        return credential_storage_error(req.id);
    }
    match call.result {
        Ok(detail) => match serde_json::to_value(detail) {
            Ok(value) => Response::ok(req.id, value),
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "session-detail-unavailable"),
        },
        Err(error) => detail_error(req.id, error),
    }
}

pub(super) async fn handle_publish(shared: &DaemonShared, req: &Request) -> Response {
    let params: PublishParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "public-run-invalid"),
    };
    let approval_sha256 = match params.draft.approval_sha256(
        params.task_success,
        &params.contributed_version,
        params.expected_publication_version,
    ) {
        Ok(digest) => digest,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "public-run-invalid"),
    };
    let request = PublicRunPublishRequest {
        draft: params.draft,
        task_success: params.task_success,
        contributed_version: params.contributed_version,
        expected_publication_version: params.expected_publication_version,
        approval_sha256,
    };
    if request.validate().is_err() {
        return Response::err(req.id, ERR_BAD_PARAMS, "public-run-invalid");
    }
    let (config, session) = match account_context(shared, req.id) {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let call = crate::public_run::call_publish(
        &config.ingest_url,
        config.allowed_hosts.as_deref(),
        &session.session.access_token,
        params.submission_id,
        &request,
    )
    .await;
    let credential_persisted = persist_rotated_token(shared, &session, call.rotated_token).is_ok();
    match call.result {
        Ok(page) => match serde_json::to_value(page) {
            Ok(mut value) => {
                if !credential_persisted {
                    attach_credential_warning(&mut value);
                }
                Response::ok(req.id, value)
            }
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "public-run-publish-failed"),
        },
        Err(error) if credential_persisted => publish_error(req.id, error),
        Err(_) => credential_storage_error(req.id),
    }
}

pub(super) async fn handle_unpublish(shared: &DaemonShared, req: &Request) -> Response {
    let submission_id = match parse_submission_id(&req.params) {
        Ok(submission_id) => submission_id,
        Err(message) => return Response::err(req.id, ERR_BAD_PARAMS, message),
    };
    let (config, session) = match account_context(shared, req.id) {
        Ok(context) => context,
        Err(response) => return *response,
    };
    let call = crate::public_run::call_unpublish(
        &config.ingest_url,
        config.allowed_hosts.as_deref(),
        &session.session.access_token,
        submission_id,
    )
    .await;
    let credential_persisted = persist_rotated_token(shared, &session, call.rotated_token).is_ok();
    match call.result {
        Ok(outcome) => match serde_json::to_value(outcome) {
            Ok(mut value) => {
                if !credential_persisted {
                    attach_credential_warning(&mut value);
                }
                Response::ok(req.id, value)
            }
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "public-run-unpublish-failed"),
        },
        Err(error) if credential_persisted => unpublish_error(req.id, error),
        Err(_) => credential_storage_error(req.id),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::{delete, get, put},
    };
    use chrono::Utc;
    use trace_commons_protocol::public_run::{
        PublicRunEvidence, PublicRunPage, PublicRunReusePermission,
    };
    use trace_commons_protocol::trace_contribution::{
        ConsentMetadata, ConsentScope, ContributorMetadata, IronclawTraceMetadata, OutcomeMetadata,
        PrivacyMetadata, ReplayMetadata, ResidualPiiRisk, SideEffectLevel, TraceCard, TraceChannel,
        TraceContributionEnvelope, TraceContributionEvent, TraceContributionEventType,
        TraceValueCard, UserFeedback, ValueMetadata,
    };

    use super::*;

    #[derive(Default)]
    struct ReadbackFaultBackend {
        values: Mutex<HashMap<String, Vec<u8>>>,
        fail_reads: AtomicBool,
    }

    impl SecretBackend for ReadbackFaultBackend {
        fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(CredentialError::Unavailable);
            }
            self.values
                .lock()
                .unwrap()
                .get(&reference.storage_key()?)
                .cloned()
                .ok_or(CredentialError::NoEntry)
        }

        fn write(
            &self,
            reference: &CredentialReference,
            bytes: &[u8],
        ) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(reference.storage_key()?, bytes.to_vec());
            Ok(())
        }

        fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .remove(&reference.storage_key()?);
            Ok(())
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (directory, store) = crate::config::tests_support::temp_store();
        (directory, DaemonShared::load(store).unwrap())
    }

    fn configured_shared(
        address: std::net::SocketAddr,
        backend: Option<Arc<dyn SecretBackend>>,
    ) -> (tempfile::TempDir, DaemonShared) {
        let (directory, store) = crate::config::tests_support::temp_store();
        if let Some(backend) = backend {
            crate::daemon::commons_credentials::install_test_backend(&store, backend);
        }
        store
            .save_config(&crate::config::ContributorConfig {
                schema_version: "contributor.v1".to_string(),
                issuer_url: format!("http://{address}"),
                ingest_url: format!("http://{address}"),
                audience: "synthetic-audience".to_string(),
                tenant_id: "synthetic-tenant".to_string(),
                instance_id: "synthetic-instance".to_string(),
                user_subject: "synthetic-subject".to_string(),
                device_key_id: "synthetic-device".to_string(),
                consent_scopes: Vec::new(),
                pii_filter: None,
                allowed_hosts: None,
                display_handle: None,
                public_bio: None,
                public_since: None,
                witness: None,
                inference_receipt_endpoint: None,
                inference_receipt_check_attestation: false,
            })
            .expect("save synthetic contributor config");
        let session = crate::account_auth::AccountSession {
            access_token: "synthetic-session-token".to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(1),
            account_id: "synthetic-account".to_string(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).expect("serialize synthetic session"),
            )
            .expect("store synthetic account session");
        let shared = DaemonShared::load(store).expect("load daemon state");
        (directory, shared)
    }

    fn request(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn detail_requires_a_structural_submission_id_before_auth() {
        let (_directory, shared) = shared();
        let response =
            handle_detail(&shared, &request("history_detail", serde_json::json!({}))).await;
        assert_eq!(response.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn detail_requires_an_account_session() {
        let (_directory, shared) = shared();
        let response = handle_detail(
            &shared,
            &request(
                "history_detail",
                serde_json::json!({"submission_id": Uuid::new_v4()}),
            ),
        )
        .await;
        let error = response.error.unwrap();
        assert_eq!(error.code, ERR_UNAVAILABLE);
        assert_eq!(error.message, ERR_ACCOUNT_SESSION_REQUIRED);
    }

    #[tokio::test]
    async fn publish_refuses_an_unreviewed_body_before_auth() {
        let (_directory, shared) = shared();
        let response = handle_publish(
            &shared,
            &request(
                "publish_public_run",
                serde_json::json!({"submission_id": Uuid::new_v4()}),
            ),
        )
        .await;
        assert_eq!(response.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn ipc_handlers_map_all_client_errors_without_leaking_bodies() {
        use crate::public_run::PublicRunClientError;

        let variants = [
            PublicRunClientError::SessionInvalid,
            PublicRunClientError::NotFound,
            PublicRunClientError::SourceNotFound,
            PublicRunClientError::Invalid,
            PublicRunClientError::Conflict,
            PublicRunClientError::Unavailable,
        ];
        let detail_messages = [
            ERR_ACCOUNT_SESSION_REQUIRED,
            "session-detail-not-found",
            "session-detail-unavailable",
            "session-detail-unavailable",
            "session-detail-unavailable",
            "session-detail-unavailable",
        ];
        let publish_messages = [
            ERR_ACCOUNT_SESSION_REQUIRED,
            "public-run-trace-not-found",
            "public-run-source-not-found",
            "public-run-invalid",
            "public-run-conflict",
            "public-run-publish-failed",
        ];
        let unpublish_messages = [
            ERR_ACCOUNT_SESSION_REQUIRED,
            "public-run-unpublish-failed",
            "public-run-unpublish-failed",
            "public-run-unpublish-failed",
            "public-run-unpublish-failed",
            "public-run-unpublish-failed",
        ];
        for (index, error) in variants.into_iter().enumerate() {
            let responses = [
                (detail_error(1, error), detail_messages[index]),
                (publish_error(2, error), publish_messages[index]),
                (unpublish_error(3, error), unpublish_messages[index]),
            ];
            for (response, expected) in responses {
                let ipc_error = response.error.expect("error response");
                assert_eq!(ipc_error.message, expected);
                assert!(!ipc_error.message.contains("private-upstream-body"));
                assert!(response.result.is_none());
            }
        }
    }

    fn envelope(submission_id: Uuid) -> TraceContributionEnvelope {
        TraceContributionEnvelope {
            schema_version: "trace.contribution.v1".to_string(),
            trace_id: Uuid::new_v4(),
            submission_id,
            created_at: Utc::now(),
            ironclaw: IronclawTraceMetadata {
                version: "0.12.1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: "consent.v1".to_string(),
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
                redaction_pipeline_version: "redaction.v1".to_string(),
                redaction_counts: BTreeMap::new(),
                redaction_distinct_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:synthetic".to_string(),
                warnings: Vec::new(),
            },
            events: vec![TraceContributionEvent {
                event_id: Uuid::nil(),
                parent_event_id: None,
                event_type: TraceContributionEventType::ToolResult,
                timestamp: Utc::now(),
                redacted_content: Some("The synthetic result was observed.".to_string()),
                structured_payload: serde_json::Value::Null,
                tool_name: Some("synthetic-tool".to_string()),
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
                task_success: TaskSuccess::Success,
                error_taxonomy: Vec::new(),
                failure_modes: Vec::new(),
                human_correction: Some("Use the bounded synthetic path.".to_string()),
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

    fn page() -> PublicRunPage {
        PublicRunPage {
            slug: "run-synthetic-ipc".to_string(),
            title: "Synthetic IPC workflow".to_string(),
            outcome_summary: "The synthetic IPC workflow completed.".to_string(),
            correction_excerpt: Some("Use the bounded synthetic path.".to_string()),
            workflow: "Apply the bounded synthetic IPC steps.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidence {
                excerpt: "The synthetic result was observed.".to_string(),
            }],
            task_success: TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            version: 1,
            published_at: Utc::now(),
            public_url: None,
            source: None,
            source_unavailable: false,
            variations: Vec::new(),
        }
    }

    fn reviewed_publish_request(submission_id: Uuid) -> Request {
        request(
            "publish_public_run",
            serde_json::json!({
                "submission_id": submission_id,
                "draft": {
                    "title": "Synthetic IPC workflow",
                    "outcome_summary": "The synthetic IPC workflow completed.",
                    "correction_excerpt": "Use the bounded synthetic path.",
                    "workflow": "Apply the bounded synthetic IPC steps.",
                    "reuse_permission": "cc_by_4_0",
                    "evidence": [{
                        "event_id": Uuid::nil(),
                        "excerpt": "The synthetic result was observed."
                    }],
                    "source_slug": null
                },
                "task_success": "success",
                "contributed_version": "trace.contribution.v1",
                "expected_publication_version": 0
            }),
        )
    }

    #[tokio::test]
    async fn publication_handlers_use_the_stored_session_for_real_http_round_trips() {
        let submission_id = Uuid::new_v4();
        let seen = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let published = Arc::new(Mutex::new(None::<PublicRunPublishRequest>));

        let detail_seen = seen.clone();
        let publication_put_seen = seen.clone();
        let publication_delete_seen = seen.clone();
        let captured_publish = published.clone();
        let router = Router::new()
            .route(
                "/v1/account/traces/{submission_id}/session-detail",
                get(move |headers: HeaderMap| {
                    let detail_seen = detail_seen.clone();
                    async move {
                        detail_seen.lock().unwrap().push((
                            "GET session detail".to_string(),
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                        ));
                        (
                            [(
                                "x-trace-commons-session-token",
                                "synthetic-session-token-rotated",
                            )],
                            Json(trace_commons_protocol::public_run::PublicRunSessionRecord::from_envelope(
                                envelope(submission_id),
                                trace_commons_protocol::public_run::PublicRunOwnerState {
                                    publication: None,
                                    expected_publication_version: 0,
                                    retained_source_slug: None,
                                },
                            )),
                        )
                    }
                }),
            )
            .route(
                "/v1/account/traces/{submission_id}/publication",
                put(
                    move |headers: HeaderMap, Json(body): Json<PublicRunPublishRequest>| {
                        let publication_put_seen = publication_put_seen.clone();
                        let captured_publish = captured_publish.clone();
                        async move {
                            publication_put_seen.lock().unwrap().push((
                                "PUT publication".to_string(),
                                headers
                                    .get("authorization")
                                    .and_then(|value| value.to_str().ok())
                                    .unwrap_or_default()
                                    .to_string(),
                            ));
                            *captured_publish.lock().unwrap() = Some(body);
                            Json(page())
                        }
                    },
                )
                .merge(delete(move |headers: HeaderMap| {
                    let publication_delete_seen = publication_delete_seen.clone();
                    async move {
                        publication_delete_seen.lock().unwrap().push((
                            "DELETE publication".to_string(),
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_string(),
                        ));
                        Json(trace_commons_protocol::public_run::PublicRunUnpublishResult {
                            unpublished: true,
                            expected_publication_version: 2,
                        })
                    }
                })),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind synthetic account API");
        let address = listener.local_addr().expect("read synthetic account API");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve synthetic account API");
        });

        let (_directory, shared) = configured_shared(address, None);

        let detail = handle_detail(
            &shared,
            &request(
                "history_detail",
                serde_json::json!({"submission_id": submission_id}),
            ),
        )
        .await;
        assert!(detail.error.is_none());
        assert_eq!(detail.result.unwrap()["task_success"], "success");
        assert_eq!(
            crate::account_auth::try_load_token(&shared.store)
                .expect("read rotated synthetic session")
                .as_deref(),
            Some("synthetic-session-token-rotated")
        );

        let publish = handle_publish(&shared, &reviewed_publish_request(submission_id)).await;
        assert!(publish.error.is_none());
        assert_eq!(publish.result.unwrap()["slug"], "run-synthetic-ipc");

        let unpublish = handle_unpublish(
            &shared,
            &request(
                "unpublish_public_run",
                serde_json::json!({"submission_id": submission_id}),
            ),
        )
        .await;
        assert!(unpublish.error.is_none());
        let unpublish = unpublish.result.unwrap();
        assert_eq!(unpublish["unpublished"], true);
        assert_eq!(unpublish["expected_publication_version"], 2);

        let calls = seen.lock().unwrap();
        assert_eq!(calls.len(), 3);
        for (operation, authorization) in calls.iter() {
            let expected = if operation.starts_with("GET") {
                "Bearer synthetic-session-token"
            } else {
                "Bearer synthetic-session-token-rotated"
            };
            assert_eq!(authorization, expected, "authorization for {operation}");
        }
        let request = published
            .lock()
            .unwrap()
            .clone()
            .expect("publish body seen");
        assert_eq!(request.task_success, TaskSuccess::Success);
        assert_eq!(request.contributed_version, "trace.contribution.v1");
        assert_eq!(request.expected_publication_version, 0);
        assert!(request.validate().is_ok());
        drop(calls);
        server.abort();
    }

    #[tokio::test]
    async fn error_response_rotation_is_persisted_before_error_mapping() {
        let submission_id = Uuid::new_v4();
        let delete_authorization = Arc::new(Mutex::new(None::<String>));
        let captured_delete = delete_authorization.clone();
        let router = Router::new().route(
            "/v1/account/traces/{submission_id}/publication",
            put(|| async {
                (
                    StatusCode::CONFLICT,
                    [(
                        "x-trace-commons-session-token",
                        "synthetic-session-token-after-conflict",
                    )],
                    Json(serde_json::json!({"error": "stale"})),
                )
            })
            .merge(delete(move |headers: HeaderMap| {
                let captured_delete = captured_delete.clone();
                async move {
                    *captured_delete.lock().unwrap() = headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_string);
                    Json(
                        trace_commons_protocol::public_run::PublicRunUnpublishResult {
                            unpublished: true,
                            expected_publication_version: 2,
                        },
                    )
                }
            })),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind conflict account API");
        let address = listener.local_addr().expect("read conflict account API");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve conflict account API");
        });
        let (_directory, shared) = configured_shared(address, None);

        let publish = handle_publish(&shared, &reviewed_publish_request(submission_id)).await;
        assert_eq!(publish.error.unwrap().message, "public-run-conflict");
        assert_eq!(
            crate::account_auth::try_load_token(&shared.store)
                .expect("read session after conflict")
                .as_deref(),
            Some("synthetic-session-token-after-conflict")
        );

        let unpublish = handle_unpublish(
            &shared,
            &request(
                "unpublish_public_run",
                serde_json::json!({"submission_id": submission_id}),
            ),
        )
        .await;
        assert!(unpublish.error.is_none());
        assert_eq!(
            delete_authorization.lock().unwrap().as_deref(),
            Some("Bearer synthetic-session-token-after-conflict")
        );
        server.abort();
    }

    #[tokio::test]
    async fn committed_mutations_keep_state_when_rotation_persistence_fails() {
        let submission_id = Uuid::new_v4();
        let backend = Arc::new(ReadbackFaultBackend::default());
        let publish_fault = backend.clone();
        let unpublish_fault = backend.clone();
        let router = Router::new().route(
            "/v1/account/traces/{submission_id}/publication",
            put(move || {
                let publish_fault = publish_fault.clone();
                async move {
                    publish_fault.fail_reads.store(true, Ordering::SeqCst);
                    (
                        [(
                            "x-trace-commons-session-token",
                            "synthetic-session-token-after-publish",
                        )],
                        Json(page()),
                    )
                }
            })
            .merge(delete(move || {
                let unpublish_fault = unpublish_fault.clone();
                async move {
                    unpublish_fault.fail_reads.store(true, Ordering::SeqCst);
                    (
                        [(
                            "x-trace-commons-session-token",
                            "synthetic-session-token-after-unpublish",
                        )],
                        Json(
                            trace_commons_protocol::public_run::PublicRunUnpublishResult {
                                unpublished: true,
                                expected_publication_version: 2,
                            },
                        ),
                    )
                }
            })),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind credential-failure account API");
        let address = listener
            .local_addr()
            .expect("read credential-failure account API");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve credential-failure account API");
        });
        let backend_for_store: Arc<dyn SecretBackend> = backend.clone();
        let (_directory, shared) = configured_shared(address, Some(backend_for_store));

        let publish = handle_publish(&shared, &reviewed_publish_request(submission_id)).await;
        assert!(publish.error.is_none());
        let publish = publish.result.expect("committed publication result");
        assert_eq!(publish["slug"], "run-synthetic-ipc");
        assert_eq!(
            publish["credential_warning"],
            CREDENTIAL_STORAGE_UNAVAILABLE
        );

        backend.fail_reads.store(false, Ordering::SeqCst);
        let unpublish = handle_unpublish(
            &shared,
            &request(
                "unpublish_public_run",
                serde_json::json!({"submission_id": submission_id}),
            ),
        )
        .await;
        assert!(unpublish.error.is_none());
        let unpublish = unpublish.result.expect("committed unpublish result");
        assert_eq!(unpublish["unpublished"], true);
        assert_eq!(unpublish["expected_publication_version"], 2);
        assert_eq!(
            unpublish["credential_warning"],
            CREDENTIAL_STORAGE_UNAVAILABLE
        );
        server.abort();
    }
}
