//! Read-only trace-activity catalogue and account progress, with rewards off.
//!
//! Catalogue requests are anonymous and independent of local matching. Status
//! uses only the native account session; every rotation is retained, including
//! on refusals. Neither read changes contribution consent or upload state.

use std::{collections::BTreeSet, time::Duration};

use chrono::Datelike;
use serde::de::DeserializeOwned;
use trace_commons_protocol::{
    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
    activity_missions::{ActivityCatalogue, ActivityProgress},
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, account_session};
use crate::config::{ContributorConfig, allowlist_for, ingest_origin_url};

const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const UNAVAILABLE: &str = "activity-missions-unavailable";
const INVALID: &str = "activity-missions-params-invalid";
const CREDENTIAL_UNAVAILABLE: &str = "commons_credential_storage_unavailable";

struct Call<T> {
    result: Result<T, ()>,
    rotated_token: Option<String>,
}

fn empty_params(req: &Request) -> bool {
    req.params
        .as_object()
        .is_some_and(|object| object.is_empty())
}

pub(super) async fn handle_catalogue(shared: &DaemonShared, req: &Request) -> Response {
    if !empty_params(req) {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    }
    let Ok(Some(config)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    let call = fetch::<ActivityCatalogue>(&config, "/v1/activity-missions", None).await;
    match call.result {
        Ok(catalogue) if valid_catalogue(&catalogue) => Response::ok(
            req.id,
            serde_json::json!({
                "catalogue":catalogue,
                "disclosure":crate::consent_copy::ACTIVITY_MISSIONS_DISCLOSURE,
            }),
        ),
        _ => Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE),
    }
}

pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    if !empty_params(req) {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    }
    let Ok(Some(config)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    let session = match account_session(shared) {
        Ok(Some(session)) => session,
        Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED),
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, CREDENTIAL_UNAVAILABLE),
    };
    // Bind the credential to the exact configured authority before sending it.
    // Config and account-session reads can race with identity changes.
    if !same_account_snapshot(shared, &config, &session) {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    }
    let call = fetch::<ActivityProgress>(
        &config,
        "/v1/account/activity-missions/status",
        Some(&session.session.access_token),
    )
    .await;
    // The account middleware rotates on an error response too. Retain it
    // before classifying HTTP, parsing or domain-validation outcomes.
    let rotated = call.rotated_token.is_some();
    if super::public_run::persist_rotated_token(shared, &session, call.rotated_token).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, CREDENTIAL_UNAVAILABLE);
    }
    // A read without a rotation still needs to notice sign-out or identity
    // replacement while the HTTP request was in flight.
    if !original_account_still_current(shared, &config, &session, rotated) {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    }
    match call.result {
        Ok(status) if valid_progress(&status) => Response::ok(
            req.id,
            serde_json::json!({
                "status":status,
                "disclosure":crate::consent_copy::ACTIVITY_MISSIONS_DISCLOSURE,
            }),
        ),
        _ => Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE),
    }
}

fn same_account_snapshot(
    shared: &DaemonShared,
    config: &ContributorConfig,
    session: &crate::account_auth::LoadedAccountSession,
) -> bool {
    super::commons_credentials::account_snapshot(&shared.store, config)
        .is_ok_and(|snapshot| snapshot == session.snapshot)
}

fn original_account_still_current(
    shared: &DaemonShared,
    config: &ContributorConfig,
    original: &crate::account_auth::LoadedAccountSession,
    rotated: bool,
) -> bool {
    let Ok(Some(current)) = account_session(shared) else {
        return false;
    };
    // Keep the original context; a reload is validation, never adoption. A
    // browser sign-in can replace the account without first clearing it.
    current.session.account_id == original.session.account_id
        && current.session.expires_at == original.session.expires_at
        && same_account_snapshot(shared, config, &current)
        && if rotated {
            current.snapshot.same_lifecycle(&original.snapshot)
        } else {
            current.snapshot == original.snapshot
        }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

// Validate the known semantic contract independently of additive response
// metadata, which deserialization discards. The policy itself stays strict.
fn valid_catalogue(catalogue: &ActivityCatalogue) -> bool {
    if catalogue.schema_version != 1
        || catalogue.kind != "trace_activity"
        || catalogue.rewards_enabled
        || catalogue.credit_points_pending.is_some()
        || catalogue.credit_condition != "mission_credit_ledger_unavailable"
    {
        return false;
    }
    match &catalogue.policy {
        Some(policy) => {
            catalogue.state == "configured"
                && policy
                    .digest()
                    .is_ok_and(|digest| catalogue.policy_sha256.as_deref() == Some(digest.as_str()))
        }
        None => catalogue.state == "unconfigured" && catalogue.policy_sha256.is_none(),
    }
}

fn valid_progress(status: &ActivityProgress) -> bool {
    let today = status.observed_at.date_naive();
    let elapsed_days = (today - status.coverage_starts_on).num_days();
    if status.schema_version != 1
        || status.kind != "trace_activity"
        || status.source != "account_contributed_submissions"
        || status.rewards_enabled
        || status.credit_points_pending.is_some()
        || status.credit_condition != "mission_credit_ledger_unavailable"
        || status.policy_sha256.len() != 64
        || !status
            .policy_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !(0..366).contains(&elapsed_days)
        || today.with_day(1) != Some(status.month_starts_on)
        || status
            .level
            .as_deref()
            .is_some_and(|level| !valid_label(level))
        || (!status.levels_configured && status.level.is_some())
    {
        return false;
    }
    match (&status.daily, status.completed_days, status.current_streak) {
        (None, None, None) => {}
        (Some(daily), Some(completed), Some(streak))
            if daily.day == today
                && valid_label(&daily.mission_id)
                && daily.required_contributions > 0
                && daily.complete
                    == (daily.contributions >= u64::from(daily.required_contributions))
                && status.monthly_contributions >= daily.contributions
                && u64::from(completed) <= (elapsed_days + 1) as u64
                && streak <= completed
                && (!daily.complete || (completed > 0 && streak > 0)) => {}
        _ => return false,
    }
    if let Some(badges) = &status.badges {
        let mut ids = BTreeSet::new();
        if badges.is_empty()
            || badges.len() > 128
            || badges
                .iter()
                .any(|badge| !valid_label(&badge.id) || !ids.insert(&badge.id))
        {
            return false;
        }
    }
    true
}

async fn fetch<T: DeserializeOwned>(
    config: &ContributorConfig,
    path: &str,
    token: Option<&str>,
) -> Call<T> {
    let failed = || Call {
        result: Err(()),
        rotated_token: None,
    };
    let Ok(url) = ingest_origin_url(&config.ingest_url, path) else {
        return failed();
    };
    let literal_loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && literal_loopback))
        || allowlist_for(config.allowed_hosts.as_deref())
            .check(&url)
            .is_err()
    {
        return failed();
    }
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
    else {
        return failed();
    };
    let request = client.get(url);
    let request = match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    let Ok(mut response) = request.send().await else {
        return failed();
    };
    let rotated_token = token.and_then(|_| {
        response
            .headers()
            .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    if rotated_token
        .as_ref()
        .is_some_and(|token| token.len() > 4096)
    {
        return failed();
    }
    let refused = || Call {
        result: Err(()),
        rotated_token: rotated_token.clone(),
    };
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return refused();
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            _ => return refused(),
        }
    }
    Call {
        result: serde_json::from_slice(&bytes).map_err(|_| ()),
        rotated_token,
    }
}
