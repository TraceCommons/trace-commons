//! The native account session: its status, sign-in and sign-out. Account
//! authority stays in the OS credential store; only the session's state
//! crosses IPC.
use anyhow::{Result, anyhow, bail};
use reqwest::Method;
use serde_json::{Value, json};
use trace_commons_operator_client::Client;

use super::commons_credentials::{self, Kind};
use super::ipc::{DaemonShared, ERR_UNAVAILABLE, Request, Response};
use crate::{account_auth, config::ConfigStore};

fn canonical_origin(input: &str) -> Result<String> {
    let url = reqwest::Url::parse(input).map_err(|_| anyhow!("account-origin-refused"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("account-origin-refused");
    }
    // Syntax normalization is independent of host policy.
    Ok(url.origin().ascii_serialization())
}

// The account API deployed for the native app is the channel's.
// An IPC URL or a retained credential cannot establish a new trust root.
const NATIVE_ACCOUNT_ORIGIN: &str = crate::channel::NATIVE_ACCOUNT_ORIGIN;

fn unconfigured_origin(origin: &str) -> Result<()> {
    if canonical_origin(origin)? != NATIVE_ACCOUNT_ORIGIN {
        bail!("account-origin-refused");
    }
    Ok(())
}

pub(super) fn resolve_origin(store: &ConfigStore, params: &Value) -> Result<String> {
    let configured = store.load_config()?.map(|c| c.ingest_url);
    let retained = commons_credentials::load(store, Kind::Account)?
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .and_then(|v| {
            v.get("ingest_origin")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    let requested = params.get("ingest_url").and_then(Value::as_str);
    let pinned = configured.as_deref().or(retained.as_deref());
    let origin = canonical_origin(pinned.or(requested).unwrap_or(NATIVE_ACCOUNT_ORIGIN))?;
    if configured.is_none() {
        unconfigured_origin(&origin)?;
    }
    if let Some(requested) = requested {
        if canonical_origin(requested)? != origin {
            bail!("account-origin-refused");
        }
    }
    if let Some(retained) = retained {
        if canonical_origin(&retained)? != origin {
            bail!("account-origin-refused");
        }
    }
    Ok(origin)
}

pub(super) fn client(origin: &str, token: &str) -> Result<Client> {
    let allowed = super::account_onboarding::signup_allowlist(origin, &[])?;
    Client::builder(origin, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
        .bearer_token(token)
        .max_response_bytes(256 * 1024)
        .host_allowlist(allowed)
        .build()
        .map_err(|_| anyhow!("account-origin-refused"))
}

/// Whether a usable account session is held. An expired session on a
/// NEAR AI-provisioned account is renewed first, so the app's signed-in
/// state follows a renewal ([`super::account_session_renewal`]). A usable
/// session is read only, never presented to the commons, so a poll rotates
/// nothing; an expired one renews once, and a refused renewal is not tried
/// again for the same record and login for its backoff.
///
/// Never waits long for a renewal: past `READ_WAIT` it answers
/// `renewal: "renewing"` and the renewal finishes on its own. A refused one
/// answers `renewal: "failed"` with its class (`renewal_failure`), and
/// `sign_in: "near_ai"` says Sign in again (`near_ai_account_sign_in`) is
/// this account's way back.
async fn status(shared: &DaemonShared) -> Value {
    use super::account_session_renewal as renewal;
    let sign_in = renewal::near_ai_enrolled(shared).then_some("near_ai");
    match renewal::account_session_for_read(shared).await {
        Ok((Some(loaded), _)) => json!({
            "state": "known",
            "signed_in": true,
            "expires_at": loaded.session.expires_at,
            "renewal": null,
            "renewal_failure": null,
            "sign_in": sign_in,
        }),
        Ok((None, renewing)) => {
            let failure = (!renewing).then(|| renewal::last_failure(shared)).flatten();
            let state = if renewing {
                Some("renewing")
            } else {
                failure.map(|_| "failed")
            };
            json!({
                "state": "known",
                "signed_in": false,
                "expires_at": null,
                "renewal": state,
                "renewal_failure": failure,
                "sign_in": sign_in,
            })
        }
        Err(_) => json!({
            "state": "unknown",
            "signed_in": null,
            "expires_at": null,
            "renewal": null,
            "renewal_failure": null,
            "sign_in": sign_in,
        }),
    }
}
async fn sign_out(shared: &DaemonShared) -> Result<Value> {
    // Clear first even when Keychain is unavailable.
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).ok();
    commons_credentials::clear(&shared.store, &[Kind::Account])?;
    let config = expected
        .as_ref()
        .and_then(|s| s.configuration().ok().flatten());
    let raw = expected.as_ref().and_then(|s| {
        commons_credentials::removed_payload(&shared.store, s)
            .ok()
            .flatten()
    });
    let _ = commons_credentials::cleanup(&shared.store);
    let origin = config
        .as_ref()
        .map(|c| c.ingest_url.clone())
        .or_else(|| {
            raw.as_ref()
                .and_then(|raw| serde_json::from_slice::<Value>(raw).ok())
                .and_then(|v| {
                    v.get("ingest_origin")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        })
        .and_then(|v| canonical_origin(&v).ok());
    if let (Some(origin), Some(raw)) = (origin, raw) {
        if let Ok(session) = serde_json::from_slice::<account_auth::AccountSession>(&raw) {
            let allowed = config
                .as_ref()
                .map(crate::config::config_allowlist)
                .map(Ok)
                .unwrap_or_else(|| {
                    unconfigured_origin(&origin)?;
                    super::account_onboarding::signup_allowlist(&origin, &[])
                });
            if let Ok(allowed) = allowed {
                if let Ok(client) =
                    Client::builder(&origin, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
                        .bearer_token(session.access_token)
                        .max_response_bytes(256 * 1024)
                        .host_allowlist(allowed)
                        .build()
                {
                    let _ = client
                        .call_raw::<()>(Method::POST, "/v1/account/logout", &[], None)
                        .await;
                }
            }
        }
    }
    Ok(json!({"signed_out":true}))
}
async fn handle_inner(shared: &DaemonShared, req: &Request) -> Result<Value> {
    match req.method.as_str() {
        "account_session_status" => Ok(status(shared).await),
        "account_sign_out" => sign_out(shared).await,
        "account_sign_in" => {
            let cfg = shared
                .store
                .load_config()?
                .ok_or_else(|| anyhow!("account-enrollment-required"))?;
            resolve_origin(&shared.store, &req.params)?;
            let result = account_auth::sign_in(&shared.store, &cfg, true, |_| {})
                .await
                .map_err(|_| anyhow!("account-sign-in-refused"))?;
            Ok(json!({"signed_in":true,"expires_at":result.expires_at}))
        }
        _ => bail!("account-unavailable"),
    }
}
pub(super) async fn handle(shared: &DaemonShared, req: &Request) -> Response {
    match handle_inner(shared, req).await {
        Ok(value) => Response::ok(req.id, value),
        Err(error) => Response::err(req.id, ERR_UNAVAILABLE, ipc_label(&error.to_string())),
    }
}

/// The label an identity call answers with: one of this module's own, or
/// `account-unavailable`. An error string from a transport or a remote body is
/// never forwarded.
fn ipc_label(error: &str) -> &'static str {
    match error {
        "account-origin-refused" => "account-origin-refused",
        "account-enrollment-required" => "account-enrollment-required",
        "account-sign-in-refused" => "account-sign-in-refused",
        _ => "account-unavailable",
    }
}

#[cfg(test)]
#[path = "native_identity_tests.rs"]
mod tests;
