//! IPC handlers for referral codes
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 5 and 6; ingest client in `crate::referral`).
//!
//! Five methods, each a read or an explicit contributor action, on the
//! signed-in account session as the payout-link methods are
//! (`super::payout_link`):
//!
//! - `referral_codes_list` -- the account's codes (text, state, people who
//!   joined with each), the people joined in all, and the referral bonus
//!   owed and sent;
//! - `referral_codes_create` -- make a code; its text is answered;
//! - `referral_codes_disable` -- turn one off (`code_id`). People who
//!   already joined with it keep earning the owner a bonus;
//! - `referral_apply` -- apply a code someone shared (`code`) to this
//!   account. Refused with the one outward label `referral_not_applied`
//!   whatever the reason, so codes cannot be probed;
//! - `referral_status` -- whether a code is applied, and until when one may
//!   be (`apply_until`, `can_apply`).
//!
//! The spec names them `referral.codes.list` and so on; every daemon method
//! is spelled with underscores, so these are too.
//!
//! # What never leaves this device, and what does
//!
//! A typed code is checked here first ([`normalize_referral_code`]): input
//! that is not a code is refused (`referral-code-invalid`) and never sent,
//! and a code is sent only in its canonical form. Code text crosses IPC
//! because the shell shows it with Copy; it is never logged and never
//! written to disk here. Audit rows (`referral-code-created`,
//! `referral-code-disabled`, `referral-applied`) are label-only: no code, no
//! id.

use chrono::Utc;
use uuid::Uuid;

use trace_commons_protocol::referral::{ReferralCodeState, normalize_referral_code};

use super::account_session_renewal::{RenewVia, account_session};
use super::audit::{self, AuditEntry};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::nearai_credential::api::CloudApi;
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, keep_rotated_token};
use crate::account_auth::LoadedAccountSession;
use crate::config::ContributorConfig;
use crate::referral::{self as client, Endpoint, ReferralError};

pub const AUDIT_CODE_CREATED: &str = "referral-code-created";
pub const AUDIT_CODE_DISABLED: &str = "referral-code-disabled";
pub const AUDIT_APPLIED: &str = "referral-applied";

/// `referral_apply` was given text that is not a referral code. Nothing was
/// sent.
pub const ERR_CODE_INVALID: &str = "referral-code-invalid";

fn audit_row(shared: &DaemonShared, action: &str) {
    // Best effort, like every other label-only row: a full disk must not
    // turn a change the commons already made into a reported failure.
    let _ = audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: action.to_string(),
            project_label: None,
            detail: None,
        },
    );
}

/// The account session and enrolled config every method needs, or the
/// refusal; `super::payout_link`'s shape. The session is checked first, so
/// a shell that has not signed in is routed to sign-in whatever else is
/// missing.
async fn session_and_config(
    shared: &DaemonShared,
    req: &Request,
    via: Option<&RenewVia<'_>>,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let session = match account_session(shared, via).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Err(Box::new(Response::err(
                req.id,
                ERR_UNAVAILABLE,
                ERR_ACCOUNT_SESSION_REQUIRED,
            )));
        }
        Err(_) => {
            return Err(Box::new(Response::err(
                req.id,
                ERR_UNAVAILABLE,
                "commons_credential_storage_unavailable",
            )));
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Err(Box::new(Response::err(
            req.id,
            ERR_UNAVAILABLE,
            "not-logged-in",
        )));
    };
    Ok((session, cfg))
}

/// For a read: an expired session is renewed in the background and waited
/// for briefly, never for the whole ceremony
/// (`account_session_renewal`, "Never waited for by a read").
async fn read_session(
    shared: &DaemonShared,
    req: &Request,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let _ = super::account_session_renewal::account_session_for_read(shared).await;
    session_and_config(shared, req, None).await
}

/// For an action: an expired session on a NEAR AI-provisioned account is
/// renewed first, when the live NEAR AI service is reachable.
async fn action_session(
    shared: &DaemonShared,
    req: &Request,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let api = CloudApi::live().ok();
    let via = api.as_ref().map(RenewVia::live);
    session_and_config(shared, req, via.as_ref()).await
}

fn endpoint<'a>(cfg: &'a ContributorConfig, session: &'a LoadedAccountSession) -> Endpoint<'a> {
    Endpoint {
        ingest_url: &cfg.ingest_url,
        allowed_hosts: cfg.allowed_hosts.as_deref(),
        account_session_token: &session.session.access_token,
    }
}

fn refused(req: &Request, error: ReferralError) -> Response {
    Response::err(req.id, ERR_UNAVAILABLE, error.label())
}

fn finish(req: &Request, mut value: serde_json::Value, credential_persisted: bool) -> Response {
    if !credential_persisted {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

fn bad(req: &Request, name: &'static str) -> Response {
    Response::err(req.id, ERR_BAD_PARAMS, name)
}

/// `referral_codes_list`.
pub(super) async fn handle_list(shared: &DaemonShared, req: &Request) -> Response {
    let (mut session, cfg) = match read_session(shared, req).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::list_codes(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let listed = match call.result {
        Ok(listed) => listed,
        Err(error) => return refused(req, error),
    };
    let live = listed
        .codes
        .iter()
        .filter(|code| code.state == ReferralCodeState::Live)
        .count() as u64;
    let people_joined: u64 = listed
        .codes
        .iter()
        .map(|code| code.referee_count)
        .fold(0, u64::saturating_add);
    let codes: Vec<serde_json::Value> = listed
        .codes
        .iter()
        .map(|code| {
            serde_json::json!({
                "code_id": code.code_id,
                "code": code.code,
                "state": code.state.as_str(),
                "created_at": code.created_at,
                "referee_count": code.referee_count,
                "bonus_owed_nano_usd": code.bonus_owed_nano_usd,
            })
        })
        .collect();
    finish(
        req,
        serde_json::json!({
            "codes": codes,
            "max_live_codes": listed.max_live_codes,
            "live_codes": live,
            "can_create": live < u64::from(listed.max_live_codes),
            "people_joined": people_joined,
            "bonus_owed_nano_usd": listed.bonus_owed_nano_usd,
            "bonus_sent_nano_usd": listed.bonus_sent_nano_usd,
            "bonus_waiting_nano_usd": listed
                .bonus_owed_nano_usd
                .saturating_sub(listed.bonus_sent_nano_usd),
        }),
        persisted,
    )
}

/// `referral_codes_create`.
pub(super) async fn handle_create(shared: &DaemonShared, req: &Request) -> Response {
    let (mut session, cfg) = match action_session(shared, req).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::create_code(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let created = match call.result {
        Ok(created) => created,
        Err(error) => return refused(req, error),
    };
    audit_row(shared, AUDIT_CODE_CREATED);
    finish(
        req,
        serde_json::json!({
            "code_id": created.code_id,
            "code": created.code,
            "created_at": created.created_at,
        }),
        persisted,
    )
}

/// `referral_codes_disable`: requires `code_id`, a code this account
/// listed.
pub(super) async fn handle_disable(shared: &DaemonShared, req: &Request) -> Response {
    let Some(code_id) = req
        .params
        .get("code_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<Uuid>().ok())
        .filter(|id| !id.is_nil())
    else {
        return bad(req, "code_id");
    };
    let (mut session, cfg) = match action_session(shared, req).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::disable_code(&endpoint(&cfg, &session), code_id).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let disabled = match call.result {
        Ok(disabled) => disabled,
        Err(error) => return refused(req, error),
    };
    audit_row(shared, AUDIT_CODE_DISABLED);
    finish(
        req,
        serde_json::json!({
            "code_id": disabled.code_id,
            "state": disabled.state.as_str(),
        }),
        persisted,
    )
}

/// `referral_apply`: requires `code`, as the person typed it.
pub(super) async fn handle_apply(shared: &DaemonShared, req: &Request) -> Response {
    let Some(typed) = req.params.get("code").and_then(serde_json::Value::as_str) else {
        return bad(req, "code");
    };
    // Checked before the session, so a typo never costs a renewal.
    if normalize_referral_code(typed).is_none() {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_CODE_INVALID);
    }
    let (mut session, cfg) = match action_session(shared, req).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::apply(&endpoint(&cfg, &session), typed).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let applied = match call.result {
        Ok(applied) => applied,
        Err(error) => return refused(req, error),
    };
    audit_row(shared, AUDIT_APPLIED);
    finish(
        req,
        serde_json::json!({
            "outcome": applied.outcome.as_str(),
            "applied_at": applied.applied_at,
            "earn_until": applied.earn_until,
        }),
        persisted,
    )
}

/// `referral_status`.
pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let (mut session, cfg) = match read_session(shared, req).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::status(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let status = match call.result {
        Ok(status) => status,
        Err(error) => return refused(req, error),
    };
    let can_apply = !status.applied && status.apply_until.is_some_and(|until| until > Utc::now());
    finish(
        req,
        serde_json::json!({
            "applied": status.applied,
            "applied_at": status.applied_at,
            "applied_via": status.applied_via.map(|via| via.as_str()),
            "earn_until": status.earn_until,
            "apply_until": status.apply_until,
            "can_apply": can_apply,
        }),
        persisted,
    )
}

#[cfg(test)]
#[path = "referral_tests.rs"]
mod tests;
