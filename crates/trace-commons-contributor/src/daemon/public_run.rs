//! IPC bridge for owned session detail and reviewed public pages.
//!
//! Every network call uses the short-lived account session established by the
//! browser login flow. The device upload key has no read or publication
//! authority. Errors crossing IPC are fixed labels and never include trace
//! text, server bodies, URLs, or tokens.

use serde::{Deserialize, Serialize};
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

#[derive(Serialize)]
struct SessionDetailResponse {
    #[serde(flatten)]
    detail: crate::public_run::SessionDetail,
    owner_scope_sha256: String,
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

fn account_binding(
    config: &crate::config::ContributorConfig,
    session: &crate::account_auth::LoadedAccountSession,
) -> String {
    let mut subject = String::new();
    for value in [
        config.ingest_url.as_str(),
        config.tenant_id.as_str(),
        config.instance_id.as_str(),
        config.user_subject.as_str(),
        config.device_key_id.as_str(),
        session.session.account_id.as_str(),
    ] {
        subject.push_str(&value.len().to_string());
        subject.push(':');
        subject.push_str(value);
    }
    trace_commons_protocol::onboarding::user_subject_hash(&subject)
}

pub(super) fn current_account_binding(
    shared: &DaemonShared,
    request_id: u64,
) -> Result<String, Box<Response>> {
    let (config, session) = account_context(shared, request_id)?;
    Ok(account_binding(&config, &session))
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
    match load_detail_with_binding(shared, req.id, submission_id).await {
        Ok((detail, owner_scope_sha256)) => {
            let current_owner = match current_account_binding(shared, req.id) {
                Ok(owner) => owner,
                Err(response) => return *response,
            };
            if current_owner != owner_scope_sha256 {
                return Response::err(req.id, ERR_UNAVAILABLE, "session-owner-changed");
            }
            match serde_json::to_value(SessionDetailResponse {
                detail,
                owner_scope_sha256,
            }) {
                Ok(value) => Response::ok(req.id, value),
                Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "session-detail-unavailable"),
            }
        }
        Err(response) => *response,
    }
}

/// Re-read one account-owned record through the authenticated owner route.
///
/// Skill extraction uses this same seam so a local caller cannot manufacture
/// a correction and attribute it to an arbitrary submission id. The returned
/// response carries only fixed labels; the access and refresh tokens remain
/// inside this module.
pub(super) async fn load_detail_with_binding(
    shared: &DaemonShared,
    request_id: u64,
    submission_id: Uuid,
) -> Result<(crate::public_run::SessionDetail, String), Box<Response>> {
    let (config, session) = match account_context(shared, request_id) {
        Ok(context) => context,
        Err(response) => return Err(response),
    };
    let binding = account_binding(&config, &session);
    let call = crate::public_run::call_session_detail(
        &config.ingest_url,
        config.allowed_hosts.as_deref(),
        &session.session.access_token,
        submission_id,
    )
    .await;
    if persist_rotated_token(shared, &session, call.rotated_token).is_err() {
        return Err(Box::new(credential_storage_error(request_id)));
    }
    match call.result {
        Ok(detail) => Ok((detail, binding)),
        Err(error) => Err(Box::new(detail_error(request_id, error))),
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
#[path = "public_run/tests.rs"]
mod tests;
