//! Native platform credentials. Only challenges and authenticator bytes cross
//! IPC; account authority stays in the OS credential store. Ceremonies are
//! process-local, bounded, single-use and pinned to durable credential state.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use reqwest::Method;
use serde_json::{Value, json};
use trace_commons_operator_client::Client;
use trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER;

use super::commons_credentials::{self, Kind, Snapshot};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::{account_auth, config::ConfigStore};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Create,
    Login,
    Add,
}

pub(crate) struct Pending {
    action: Action,
    origin: String,
    server_id: String,
    challenge: String,
    expires: Instant,
    snapshot: Snapshot,
    label: Option<String>,
    session: Option<account_auth::LoadedAccountSession>,
}
pub(crate) type Ceremonies = HashMap<String, Pending>;

fn invalid() -> anyhow::Error {
    anyhow!("passkey-invalid")
}
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(invalid)
}
fn bytes(value: &Value, key: &str) -> Result<String> {
    let raw = string(value, key)?;
    if raw.len() > 128 * 1024 {
        return Err(invalid());
    }
    let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| invalid())?;
    if decoded.is_empty() || URL_SAFE_NO_PAD.encode(&decoded) != raw {
        return Err(invalid());
    }
    Ok(raw.into())
}
fn binding(value: &Value) -> Result<&str> {
    let value = string(value, "binding_state")?;
    if !matches!(value, "unbound" | "bound" | "closed" | "legacy") {
        return Err(invalid());
    }
    Ok(value)
}

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
    // Syntax normalization is independent of host policy. The captured
    // configuration's explicit policy takes precedence over the environment;
    // scoped_client applies that one policy immediately before the request.
    Ok(url.origin().ascii_serialization())
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
    let origin = canonical_origin(
        pinned
            .or(requested)
            .ok_or_else(|| anyhow!("account-origin-required"))?,
    )?;
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

fn scoped_client(
    store: &ConfigStore,
    expected: &Snapshot,
    origin: &str,
    token: &str,
) -> Result<Client> {
    let result = if let Some(config) = expected.configuration()? {
        let url = reqwest::Url::parse(origin).map_err(|_| anyhow!("account-origin-refused"))?;
        let configured = reqwest::Url::parse(&config.ingest_url)
            .map_err(|_| anyhow!("account-origin-refused"))?;
        if url.origin() != configured.origin() {
            bail!("account-origin-refused");
        }
        Client::builder(origin, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
            .bearer_token(token)
            .max_response_bytes(256 * 1024)
            .host_allowlist(crate::config::config_allowlist(&config))
            .build()
            .map_err(|_| anyhow!("account-origin-refused"))?
    } else {
        client(origin, token)?
    };
    if commons_credentials::snapshot(store, Kind::Account)? != *expected {
        bail!("account-session-changed");
    }
    Ok(result)
}

/// Persist rotation even on an HTTP refusal. A changed local authority wins.
pub(super) async fn authenticated(
    shared: &DaemonShared,
    origin: &str,
    session: &mut account_auth::LoadedAccountSession,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value> {
    if commons_credentials::snapshot(&shared.store, Kind::Account)? != session.snapshot {
        bail!("account-session-changed");
    }
    let response = scoped_client(
        &shared.store,
        &session.snapshot,
        origin,
        &session.session.access_token,
    )?
    .call_json_with_response_header::<Value, Value>(
        method,
        path,
        &[],
        body,
        ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
    )
    .await;
    if let Some(token) = response.response_header {
        account_auth::store_rotated_token(&shared.store, session, token)
            .map_err(|_| anyhow!("account-session-changed"))?;
        let next = account_auth::try_load_session_with_snapshot(&shared.store)?
            .ok_or_else(|| anyhow!("account-session-changed"))?;
        if next.session.account_id != session.session.account_id {
            bail!("account-session-changed");
        }
        *session = next;
    } else if commons_credentials::snapshot(&shared.store, Kind::Account)? != session.snapshot {
        bail!("account-session-changed");
    }
    response
        .result
        .map_err(|_| anyhow!("account-request-refused"))
}

fn normalize_options(action: Action, start: &Value) -> Result<Value> {
    let expires = start
        .get("expires_in_secs")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0 && *n <= 600)
        .ok_or_else(invalid)?;
    let key = start
        .get("public_key")
        .and_then(|v| v.get("publicKey"))
        .ok_or_else(invalid)?;
    let rp = if action == Action::Login {
        string(key, "rpId")?
    } else {
        string(key.get("rp").ok_or_else(invalid)?, "id")?
    };
    // The signed app's associated domain and the production RP are fixed.
    if rp != "tracecommons.ai" {
        return Err(invalid());
    }
    let challenge = bytes(key, "challenge")?;
    if URL_SAFE_NO_PAD
        .decode(&challenge)
        .map_err(|_| invalid())?
        .len()
        < 16
    {
        return Err(invalid());
    }
    let mut out = json!({"rp_id":rp,"challenge":challenge,"expires_in_secs":expires});
    let selection = key.get("authenticatorSelection").unwrap_or(&Value::Null);
    let uv = if action == Action::Login {
        key.get("userVerification")
    } else {
        selection.get("userVerification")
    }
    .and_then(Value::as_str)
    .unwrap_or("preferred");
    if !matches!(uv, "required" | "preferred" | "discouraged") {
        return Err(invalid());
    }
    out["user_verification"] = json!(uv);
    if action != Action::Login {
        let user = key.get("user").ok_or_else(invalid)?;
        out["user_id"] = json!(bytes(user, "id")?);
        if URL_SAFE_NO_PAD
            .decode(string(&out, "user_id")?)
            .map_err(|_| invalid())?
            .len()
            > 64
        {
            return Err(invalid());
        }
        let name = string(user, "name")?;
        if name.len() > 256 {
            return Err(invalid());
        }
        out["user_name"] = json!(name);
        let display_name = user
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or(name);
        if display_name.len() > 256 {
            return Err(invalid());
        }
        out["user_display_name"] = json!(display_name);
        let attestation = key
            .get("attestation")
            .and_then(Value::as_str)
            .unwrap_or("none");
        if !matches!(attestation, "none" | "direct" | "indirect") {
            return Err(invalid());
        }
        out["attestation"] = json!(attestation);
        if selection
            .get("authenticatorAttachment")
            .and_then(Value::as_str)
            .is_some_and(|v| v != "platform")
        {
            return Err(invalid());
        }
        out["authenticator_attachment"] = json!("platform");
        let resident = selection
            .get("residentKey")
            .and_then(Value::as_str)
            .unwrap_or(
                if selection.get("requireResidentKey").and_then(Value::as_bool) == Some(true) {
                    "required"
                } else {
                    "discouraged"
                },
            );
        if !matches!(resident, "required" | "preferred" | "discouraged") {
            return Err(invalid());
        }
        out["resident_key"] = json!(resident);
        let algorithms = key
            .get("pubKeyCredParams")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?;
        if !algorithms.iter().any(|v| {
            v.get("type").and_then(Value::as_str) == Some("public-key")
                && v.get("alg").and_then(Value::as_i64) == Some(-7)
        }) {
            return Err(invalid());
        }
        out["algorithms"] = json!([-7]);
        let mut excludes = Vec::new();
        if let Some(list) = key.get("excludeCredentials") {
            for entry in list.as_array().ok_or_else(invalid)? {
                if string(entry, "type")? != "public-key" {
                    return Err(invalid());
                }
                excludes.push(bytes(entry, "id")?);
            }
        }
        if excludes.len() > 128 {
            return Err(invalid());
        }
        out["exclude_credentials"] = json!(excludes);
    } else {
        let mut allowed = Vec::new();
        if let Some(list) = key.get("allowCredentials") {
            for entry in list.as_array().ok_or_else(invalid)? {
                if string(entry, "type")? != "public-key" {
                    return Err(invalid());
                }
                allowed.push(bytes(entry, "id")?);
            }
        }
        if allowed.len() > 128 {
            return Err(invalid());
        }
        out["allowed_credentials"] = json!(allowed);
    }
    Ok(out)
}

/// Apple's raw fields are assembled into WebAuthn JSON in exactly one place.
fn credential(action: Action, params: &Value) -> Result<Value> {
    let id = bytes(params, "credential_id")?;
    let mut response = json!({"clientDataJSON":bytes(params,"raw_client_data_json")?});
    if action == Action::Login {
        response["authenticatorData"] = json!(bytes(params, "raw_authenticator_data")?);
        response["signature"] = json!(bytes(params, "signature")?);
        response["userHandle"] = json!(bytes(params, "user_handle")?);
    } else {
        response["attestationObject"] = json!(bytes(params, "raw_attestation_object")?);
    }
    Ok(json!({"id":id,"rawId":id,"type":"public-key","response":response,"extensions":{}}))
}
fn validate_client_data(pending: &Pending, params: &Value) -> Result<()> {
    let data: Value = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(bytes(params, "raw_client_data_json")?)
            .map_err(|_| invalid())?,
    )
    .map_err(|_| invalid())?;
    let expected = if pending.action == Action::Login {
        "webauthn.get"
    } else {
        "webauthn.create"
    };
    if string(&data, "type")? != expected
        || string(&data, "challenge")? != pending.challenge
        || string(&data, "origin")? != "https://tracecommons.ai"
        || data.get("crossOrigin").and_then(Value::as_bool) == Some(true)
    {
        return Err(invalid());
    }
    Ok(())
}
fn route(action: Action, finish: bool) -> &'static str {
    match (action, finish) {
        (Action::Create, false) => "/v1/account/native/passkey/create/start",
        (Action::Create, true) => "/v1/account/native/passkey/create/finish",
        (Action::Login, false) => "/v1/account/native/passkey/login/start",
        (Action::Login, true) => "/v1/account/native/passkey/login/finish",
        (Action::Add, false) => "/v1/account/passkeys/native/register/start",
        (Action::Add, true) => "/v1/account/passkeys/native/register/finish",
    }
}
async fn begin(shared: &DaemonShared, action: Action, params: &Value) -> Result<Value> {
    if action == Action::Create && shared.store.load_config()?.is_some() {
        bail!("account-already-enrolled");
    }
    let mut snapshot = commons_credentials::snapshot(&shared.store, Kind::Account)?;
    let origin = resolve_origin(&shared.store, params)?;
    let mut session = if action == Action::Add {
        Some(
            account_auth::try_load_session_with_snapshot(&shared.store)?
                .ok_or_else(|| anyhow!("account-session-required"))?,
        )
    } else {
        None
    };
    if commons_credentials::snapshot(&shared.store, Kind::Account)? != snapshot {
        bail!("account-session-changed");
    }
    let label = params
        .get("label")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_str()
                .map(str::trim)
                .filter(|v| {
                    !v.is_empty() && v.chars().count() <= 64 && !v.chars().any(char::is_control)
                })
                .map(str::to_owned)
                .ok_or_else(invalid)
        })
        .transpose()?;
    let body = if action == Action::Create {
        json!({"label":label})
    } else {
        json!({})
    };
    let start = if let Some(session) = &mut session {
        let value = authenticated(
            shared,
            &origin,
            session,
            Method::POST,
            route(action, false),
            Some(&body),
        )
        .await?;
        snapshot = session.snapshot.clone();
        value
    } else {
        scoped_client(&shared.store, &snapshot, &origin, "unauthenticated")?
            .call_json::<Value, Value>(Method::POST, route(action, false), &[], Some(&body))
            .await
            .map_err(|_| anyhow!("passkey-start-refused"))?
    };
    if commons_credentials::snapshot(&shared.store, Kind::Account)? != snapshot {
        bail!("account-session-changed");
    }
    let mut normalized = normalize_options(action, &start)?;
    let server_id = string(&start, "ceremony_id")?;
    if server_id.len() > 256 {
        return Err(invalid());
    }
    let local_id = uuid::Uuid::new_v4().to_string();
    let pending = Pending {
        action,
        origin,
        server_id: server_id.into(),
        challenge: string(&normalized, "challenge")?.into(),
        expires: Instant::now()
            + Duration::from_secs(normalized["expires_in_secs"].as_u64().ok_or_else(invalid)?),
        snapshot,
        label,
        session,
    };
    let mut pending_map = shared.native_identity.lock().map_err(|_| invalid())?;
    pending_map.retain(|_, v| v.expires > Instant::now());
    if pending_map.len() >= 16 {
        bail!("passkey-busy");
    }
    pending_map.insert(local_id.clone(), pending);
    normalized["ceremony"] = json!(local_id);
    Ok(normalized)
}

fn persist_session(
    store: &ConfigStore,
    snapshot: &Snapshot,
    origin: &str,
    value: &Value,
) -> Result<()> {
    binding(value)?;
    let token = string(value, "access_token")?;
    let seconds = value["expires_in_secs"]
        .as_i64()
        .filter(|s| *s > 120 && *s <= 43200)
        .ok_or_else(invalid)?;
    let account = string(value, "account_id")?;
    uuid::Uuid::parse_str(account).map_err(|_| invalid())?;
    if string(value, "token_type")? != "Bearer"
        || !token.starts_with("tcn1_")
        || token.trim() != token
        || token.len() > 4096
    {
        return Err(invalid());
    }
    if let Some(config) = store.load_config()? {
        let tenant = token
            .strip_prefix("tcn1_")
            .and_then(|v| v.split_once('.'))
            .and_then(|(v, _)| URL_SAFE_NO_PAD.decode(v).ok())
            .and_then(|v| String::from_utf8(v).ok())
            .ok_or_else(invalid)?;
        if tenant != config.tenant_id {
            bail!("account-enrollment-mismatch");
        }
        if let Some(previous) = account_auth::try_load_session_with_snapshot(store)? {
            if previous.session.account_id != account {
                bail!("account-enrollment-mismatch");
            }
        }
    }
    let payload = json!({"access_token":token,"account_id":account,"expires_at":Utc::now()+chrono::Duration::seconds(seconds),"ingest_origin":origin,"binding_state":binding(value)?});
    commons_credentials::replace(store, snapshot, &serde_json::to_vec(&payload)?, None)
        .map_err(|_| anyhow!("account-session-changed"))
}
async fn complete(shared: &DaemonShared, action: Action, params: &Value) -> Result<Value> {
    let id = string(params, "ceremony")?;
    let mut pending = shared
        .native_identity
        .lock()
        .map_err(|_| invalid())?
        .remove(id)
        .ok_or_else(|| anyhow!("passkey-ceremony-expired"))?;
    if pending.action != action || pending.expires <= Instant::now() {
        bail!("passkey-ceremony-expired");
    }
    if commons_credentials::snapshot(&shared.store, Kind::Account)? != pending.snapshot {
        bail!("account-session-changed");
    }
    validate_client_data(&pending, params)?;
    let mut body = json!({"ceremony_id":pending.server_id,"credential":credential(action,params)?});
    if let Some(session) = &mut pending.session {
        body["label"] = json!(pending.label);
        authenticated(
            shared,
            &pending.origin,
            session,
            Method::POST,
            route(action, true),
            Some(&body),
        )
        .await?;
        let result = authenticated(
            shared,
            &pending.origin,
            session,
            Method::GET,
            "/v1/account/binding",
            None,
        )
        .await?;
        Ok(json!({"binding_state":binding(&result)?}))
    } else {
        let result: Value = scoped_client(
            &shared.store,
            &pending.snapshot,
            &pending.origin,
            "unauthenticated",
        )?
        .call_json(Method::POST, route(action, true), &[], Some(&body))
        .await
        .map_err(|_| anyhow!("passkey-finish-refused"))?;
        persist_session(&shared.store, &pending.snapshot, &pending.origin, &result)?;
        Ok(json!({"binding_state":binding(&result)?}))
    }
}

fn status(store: &ConfigStore) -> Value {
    match account_auth::try_load_session_with_snapshot(store) {
        Ok(Some(loaded)) => {
            json!({"state":"known","signed_in":true,"account_id":loaded.session.account_id,"expires_at":loaded.session.expires_at})
        }
        Ok(None) => json!({"state":"known","signed_in":false,"account_id":null,"expires_at":null}),
        Err(_) => json!({"state":"unknown","signed_in":null,"account_id":null,"expires_at":null}),
    }
}
async fn sign_out(shared: &DaemonShared) -> Result<Value> {
    // Clear first even when Keychain is unavailable. Durable invalidation also
    // rejects an already in-flight completion after restart.
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).ok();
    commons_credentials::clear(&shared.store, &[Kind::Account])?;
    shared
        .native_identity
        .lock()
        .map_err(|_| invalid())?
        .clear();
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
                .unwrap_or_else(|| super::account_onboarding::signup_allowlist(&origin, &[]));
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
        "passkey_create_begin" => begin(shared, Action::Create, &req.params).await,
        "passkey_login_begin" => begin(shared, Action::Login, &req.params).await,
        "passkey_add_begin" => begin(shared, Action::Add, &req.params).await,
        "passkey_create_complete" => complete(shared, Action::Create, &req.params).await,
        "passkey_login_complete" => complete(shared, Action::Login, &req.params).await,
        "passkey_add_complete" => complete(shared, Action::Add, &req.params).await,
        "passkey_cancel" => {
            let id = string(&req.params, "ceremony")?;
            shared
                .native_identity
                .lock()
                .map_err(|_| invalid())?
                .remove(id);
            Ok(json!({"cancelled":true}))
        }
        "account_session_status" => Ok(status(&shared.store)),
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
            Ok(
                json!({"signed_in":true,"account_id":result.account_id,"expires_at":result.expires_at}),
            )
        }
        "account_bind" => super::nearai_onboarding::bind(shared).await,
        "account_binding" | "passkey_state" => {
            let Some(mut session) = account_auth::try_load_session_with_snapshot(&shared.store)?
            else {
                if req.method == "passkey_state" {
                    return Ok(
                        json!({"state":"none","passkey_count":null,"near_ai_connected":null}),
                    );
                }
                bail!("account-session-required");
            };
            let origin = resolve_origin(&shared.store, &req.params)?;
            let value = authenticated(
                shared,
                &origin,
                &mut session,
                Method::GET,
                "/v1/account/binding",
                None,
            )
            .await?;
            let state = binding(&value)?;
            if req.method == "passkey_state" {
                Ok(
                    json!({"state":state,"passkey_count":null,"near_ai_connected":match state {"bound"=>Some(true),"unbound"|"closed"=>Some(false),_=>None}}),
                )
            } else {
                Ok(json!({"binding_state":state}))
            }
        }
        _ => Err(invalid()),
    }
}
pub(super) async fn handle(shared: &DaemonShared, req: &Request) -> Response {
    match handle_inner(shared, req).await {
        Ok(value) => Response::ok(req.id, value),
        Err(error) => {
            if req.method == "passkey_state" {
                return Response::ok(
                    req.id,
                    json!({"state":"unknown","passkey_count":null,"near_ai_connected":null}),
                );
            }
            let label = match error.to_string().as_str() {
                "passkey-invalid" => "passkey-invalid",
                "passkey-busy" => "passkey-busy",
                "passkey-start-refused" => "passkey-start-refused",
                "passkey-finish-refused" => "passkey-finish-refused",
                "passkey-ceremony-expired" => "passkey-ceremony-expired",
                "account-session-required" => "account-session-required",
                "account-session-changed" => "account-session-changed",
                "account-origin-required" => "account-origin-required",
                "account-origin-refused" => "account-origin-refused",
                "account-enrollment-required" => "account-enrollment-required",
                "account-sign-in-refused" => "account-sign-in-refused",
                "account-request-refused" => "account-request-refused",
                "account-already-enrolled" => "account-already-enrolled",
                "account-enrollment-mismatch" => "account-enrollment-mismatch",
                "account-bind-invalid" => "account-bind-invalid",
                "account-bind-refused" => "account-bind-refused",
                "near_ai_enroll_no_session" => "near_ai_enroll_no_session",
                _ => "account-unavailable",
            };
            Response::err(
                req.id,
                if label == "passkey-invalid" {
                    ERR_BAD_PARAMS
                } else {
                    ERR_UNAVAILABLE
                },
                label,
            )
        }
    }
}

#[cfg(test)]
#[path = "native_identity_tests.rs"]
mod tests;
