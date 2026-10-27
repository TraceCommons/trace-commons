//! IPC handlers for the devices linked to one account, and for linking a
//! new one with a one-time code (owner decisions 2026-10-27).
//!
//! # The devices of an account
//!
//! Every device that signs in to the same NEAR AI account joins the same
//! commons account. `account_devices_list`, `account_device_rename` and
//! `account_device_revoke` relay the account routes under
//! `/v1/account/devices`, on the account session, exactly as the payout
//! link relays its routes: the session is renewed first when it may be,
//! and a rotated session is stored whatever the answer.
//!
//! # Linking a server or headless device
//!
//! A server has no browser to sign in to NEAR AI with. A signed-in laptop
//! mints a one-time code instead (`account_device_code_mint`), signed with
//! its own device key, and the server redeems it
//! (`account_device_code_redeem`): it makes a device key, signs the code
//! with it, and is enrolled in the laptop's account. The code lasts ten
//! minutes and is spent once. The laptop can follow it with
//! `account_device_code_status`.
//!
//! A server that joined this way keeps [`JOINED_BY_CODE_FILE`] in its state
//! directory. It has no NEAR AI login, so
//! [`super::account_session_renewal`] renews its 12-hour session by signing
//! with the device key instead; revoking the device from another one ends
//! that.
//!
//! # Refusals
//!
//! Fixed labels only, never a remote body: see [`status_label`].

use base64::Engine as _;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use trace_commons_protocol::account_device_code::{
    DEVICE_CODE_LABEL_MAX_CHARS, DEVICE_CODE_MINT_PATH, DEVICE_CODE_PREFIX,
    DEVICE_CODE_REDEEM_PATH, DeviceCodeMintRequest, DeviceCodeMintResponse,
    DeviceCodeRedeemRequest, DeviceCodeStatusResponse, DeviceSignInResponse,
    device_code_mint_bytes, device_code_redeem_bytes, device_code_status_path,
};
use trace_commons_protocol::{
    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER, ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, keep_rotated_token};
use crate::account_auth::{LoadedAccountSession, RotatedToken};
use crate::config::{ConfigStore, ContributorConfig};
use crate::identity::DeviceIdentity;

/// The marker a device that joined by code keeps in its state directory
/// (module doc). An enrollment file, so sign-out and unenroll remove it.
pub use crate::config::JOINED_BY_CODE_FILE;

/// Refused: this device joined by code, and such a device never mints one.
pub const ERR_JOINED_BY_CODE: &str = "device_code_joined_by_code";

/// The account routes this module relays.
const DEVICES_PATH: &str = "/v1/account/devices";

/// The longest code this client will send; the server refuses longer.
const CODE_MAX_LEN: usize = 256;

/// Whether this device joined its account by code, and so renews by key.
pub(crate) fn joined_by_code(store: &ConfigStore) -> bool {
    matches!(store.read_daemon_file(JOINED_BY_CODE_FILE), Ok(Some(_)))
}

fn bad(req: &Request, name: &'static str) -> Response {
    Response::err(req.id, ERR_BAD_PARAMS, name)
}

fn unavailable(req: &Request, name: &'static str) -> Response {
    Response::err(req.id, ERR_UNAVAILABLE, name)
}

fn finish(req: &Request, mut value: Value, credential_persisted: bool) -> Response {
    if !credential_persisted {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

/// A device id as the server names one: `sha256:` and 64 lowercase hex.
/// Checked before it goes into a path.
pub(crate) fn valid_device_key_id(id: &str) -> bool {
    id.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// A device label: trimmed, at most [`DEVICE_CODE_LABEL_MAX_CHARS`]
/// characters, no control characters. Blank is `Ok(None)` (no label).
pub(crate) fn device_label(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(raw) = raw.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };
    if raw.chars().count() > DEVICE_CODE_LABEL_MAX_CHARS || raw.chars().any(char::is_control) {
        return Err(());
    }
    Ok(Some(raw.to_string()))
}

/// A code as the server mints one: the prefix, a tenant, a dot and a
/// secret. Only the shape is checked here; the server decides the rest.
/// Surrounding whitespace, as a paste leaves it, is dropped.
pub(crate) fn device_code(raw: &str) -> Option<&str> {
    let code = raw.trim();
    let body = code.strip_prefix(DEVICE_CODE_PREFIX)?;
    let (tenant, secret) = body.split_once('.')?;
    let url_safe = |part: &str| {
        part.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    (code.len() <= CODE_MAX_LEN
        && !tenant.is_empty()
        && !secret.is_empty()
        && url_safe(tenant)
        && url_safe(secret))
    .then_some(code)
}

/// The tenant a code was minted in, which the sign-in it buys must name.
fn code_tenant(code: &str) -> Option<String> {
    let body = code.strip_prefix(DEVICE_CODE_PREFIX)?;
    let (tenant, _) = body.split_once('.')?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(tenant)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// The fixed label for a refused account-route call, by status.
fn status_label(status: Option<StatusCode>, not_found: &'static str) -> &'static str {
    match status {
        Some(StatusCode::UNAUTHORIZED) => ERR_ACCOUNT_SESSION_REQUIRED,
        Some(StatusCode::NOT_FOUND) => not_found,
        Some(StatusCode::TOO_MANY_REQUESTS) => "account_devices_rate_limited",
        Some(StatusCode::FORBIDDEN) => "account_devices_refused",
        Some(StatusCode::BAD_REQUEST) => "account_devices_invalid",
        _ => "account_devices_unavailable",
    }
}

/// One account-route call's answer: its status, its body when it was
/// JSON, and whether a rotated session (if any) was stored.
struct Call {
    status: Option<StatusCode>,
    body: Option<Value>,
    persisted: bool,
    cfg: ContributorConfig,
}

/// The account session (renewed first when it may be) and the config.
async fn session_and_config(
    shared: &DaemonShared,
    req: &Request,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let session = match super::account_session_renewal::account_session_live(shared).await {
        Ok(Some(session)) => session,
        Ok(None) => return Err(Box::new(unavailable(req, ERR_ACCOUNT_SESSION_REQUIRED))),
        Err(_) => {
            return Err(Box::new(unavailable(
                req,
                "commons_credential_storage_unavailable",
            )));
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Err(Box::new(unavailable(req, "not-logged-in")));
    };
    Ok((session, cfg))
}

/// Call one account route on the account session and keep any rotation.
async fn account_call<B: serde::Serialize>(
    shared: &DaemonShared,
    req: &Request,
    method: Method,
    path: &str,
    body: Option<&B>,
) -> Result<Call, Box<Response>> {
    let (mut session, cfg) = session_and_config(shared, req).await?;
    let Ok(client) = super::native_identity::client(&cfg.ingest_url, &session.session.access_token)
    else {
        return Err(Box::new(unavailable(req, "account_devices_unavailable")));
    };
    let call = client
        .call_json_with_response_headers::<B, Value>(
            method,
            path,
            &[],
            body,
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
        )
        .await;
    // Stored whatever the result: the server rotates on a refusal too.
    let rotated = RotatedToken::from_headers(call.response_header, call.companion_header);
    let persisted = keep_rotated_token(shared, &mut session, rotated);
    Ok(Call {
        status: call.status,
        body: call.result.ok(),
        persisted,
        cfg,
    })
}

fn param_device_key_id(req: &Request) -> Result<&str, Box<Response>> {
    req.params
        .get("device_key_id")
        .and_then(Value::as_str)
        .filter(|id| valid_device_key_id(id))
        .ok_or_else(|| Box::new(bad(req, "account_device_id_invalid")))
}

fn param_label(req: &Request) -> Result<Option<String>, Box<Response>> {
    match req.params.get("label") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(label)) => device_label(Some(label))
            .map_err(|_| Box::new(bad(req, "account_device_label_invalid"))),
        Some(_) => Err(Box::new(bad(req, "account_device_label_invalid"))),
    }
}

/// `account_devices_list`: every device linked to this account.
pub(super) async fn handle_list(shared: &DaemonShared, req: &Request) -> Response {
    let call = match account_call::<()>(shared, req, Method::GET, DEVICES_PATH, None).await {
        Ok(call) => call,
        Err(response) => return *response,
    };
    let mut devices = match (call.status, call.body) {
        (Some(StatusCode::OK), Some(body)) => match body.get("devices") {
            Some(Value::Array(devices)) => devices.clone(),
            _ => return unavailable(req, "account_devices_unavailable"),
        },
        (status, _) => {
            return unavailable(req, status_label(status, "account_devices_unavailable"));
        }
    };
    let own = shared
        .store
        .load_config()
        .ok()
        .flatten()
        .map(|cfg| cfg.device_key_id);
    mark_this_device(&mut devices, own.as_deref());
    finish(req, json!({ "devices": devices }), call.persisted)
}

/// Flag this device in the commons's list by its key id as well as by the
/// session the commons matched: a session issued before the commons
/// recorded device keys matches nothing there, and this device must still
/// show as this device so it is never offered an Unlink.
fn mark_this_device(devices: &mut [Value], own_device_key_id: Option<&str>) {
    let Some(own) = own_device_key_id else {
        return;
    };
    for device in devices {
        if device.get("device_key_id").and_then(Value::as_str) == Some(own) {
            device["this_device"] = Value::Bool(true);
        }
    }
}

/// `account_device_rename {device_key_id, label?}`: a missing or blank
/// label clears it.
pub(super) async fn handle_rename(shared: &DaemonShared, req: &Request) -> Response {
    let id = match param_device_key_id(req) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let label = match param_label(req) {
        Ok(label) => label,
        Err(response) => return *response,
    };
    let path = format!("{DEVICES_PATH}/{id}");
    let body = json!({ "label": label });
    let call = match account_call(shared, req, Method::PATCH, &path, Some(&body)).await {
        Ok(call) => call,
        Err(response) => return *response,
    };
    match call.status {
        Some(status) if status.is_success() => finish(
            req,
            json!({ "renamed": true, "labeled": label.is_some() }),
            call.persisted,
        ),
        status => unavailable(req, status_label(status, "account_device_not_found")),
    }
}

/// `account_device_revoke {device_key_id}`: unlink another device. Its
/// sessions end and it can no longer renew. This device is refused: it
/// signs out instead, which keeps its own enrollment consistent.
pub(super) async fn handle_revoke(shared: &DaemonShared, req: &Request) -> Response {
    let id = match param_device_key_id(req) {
        Ok(id) => id.to_string(),
        Err(response) => return *response,
    };
    if matches!(shared.store.load_config(), Ok(Some(cfg)) if cfg.device_key_id == id) {
        return bad(req, "account_device_revoke_this_device");
    }
    let path = format!("{DEVICES_PATH}/{id}");
    let call = match account_call::<()>(shared, req, Method::DELETE, &path, None).await {
        Ok(call) => call,
        Err(response) => return *response,
    };
    match call.status {
        Some(status) if status.is_success() => {
            finish(req, json!({ "revoked": true }), call.persisted)
        }
        status => unavailable(req, status_label(status, "account_device_not_found")),
    }
}

/// `account_device_code_mint {label?}`: a one-time code for linking a
/// server, signed with this device's key. The code is shown once.
pub(super) async fn handle_mint(shared: &DaemonShared, req: &Request) -> Response {
    let label = match param_label(req) {
        Ok(label) => label,
        Err(response) => return *response,
    };
    // A device that joined by code never mints one; the commons refuses it
    // too. Refused here first so the person is told why.
    if joined_by_code(&shared.store) {
        return unavailable(req, ERR_JOINED_BY_CODE);
    }
    let store = shared.store.clone();
    let Ok(Ok(Some(identity))) =
        tokio::task::spawn_blocking(move || DeviceIdentity::load(&store)).await
    else {
        return unavailable(req, "not-logged-in");
    };
    let Ok(minter): Result<[u8; 32], _> = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)
        .map_err(|_| ())
        .and_then(|bytes| bytes.try_into().map_err(|_| ()))
    else {
        return unavailable(req, "device_code_unavailable");
    };
    // The proof names the account, so the session comes first.
    let (session, cfg) = match session_and_config(shared, req).await {
        Ok(found) => found,
        Err(response) => return *response,
    };
    if identity.device_key_id != cfg.device_key_id {
        return unavailable(req, "device_code_unavailable");
    }
    let mut nonce = [0u8; 32];
    if ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut nonce).is_err() {
        return unavailable(req, "device_code_unavailable");
    }
    let issued_at = chrono::Utc::now().timestamp();
    let request = DeviceCodeMintRequest {
        label,
        minter_public_key: identity.public_key_b64.clone(),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        issued_at,
        signature: identity.sign_b64(&device_code_mint_bytes(
            &cfg.tenant_id,
            &session.session.account_id,
            &minter,
            &nonce,
            issued_at,
        )),
    };
    drop(session);
    let call = match account_call(
        shared,
        req,
        Method::POST,
        DEVICE_CODE_MINT_PATH,
        Some(&request),
    )
    .await
    {
        Ok(call) => call,
        Err(response) => return *response,
    };
    // A renewal between the two reads would sign for a stale account.
    if call.cfg.tenant_id != cfg.tenant_id {
        return unavailable(req, "device_code_unavailable");
    }
    let minted = match call.status {
        Some(StatusCode::OK) => call
            .body
            .and_then(|body| serde_json::from_value::<DeviceCodeMintResponse>(body).ok()),
        Some(StatusCode::FORBIDDEN) => return unavailable(req, "device_code_minter_not_a_device"),
        Some(StatusCode::TOO_MANY_REQUESTS) => return unavailable(req, "device_code_too_many"),
        Some(StatusCode::UNAUTHORIZED) => return unavailable(req, ERR_ACCOUNT_SESSION_REQUIRED),
        _ => None,
    };
    let Some(minted) = minted.filter(|minted| device_code(&minted.code).is_some()) else {
        return unavailable(req, "device_code_unavailable");
    };
    finish(
        req,
        json!({
            "code_id": minted.code_id,
            "code": minted.code,
            "expires_at": minted.expires_at,
        }),
        call.persisted,
    )
}

/// `account_device_code_status {code_id}`: pending, redeemed (with the new
/// device's id) or expired.
pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let Some(code_id) = req
        .params
        .get("code_id")
        .and_then(Value::as_str)
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return bad(req, "device_code_id_invalid");
    };
    let path = device_code_status_path(code_id);
    let call = match account_call::<()>(shared, req, Method::GET, &path, None).await {
        Ok(call) => call,
        Err(response) => return *response,
    };
    let status = match (call.status, call.body) {
        (Some(StatusCode::OK), Some(body)) => {
            serde_json::from_value::<DeviceCodeStatusResponse>(body).ok()
        }
        (status, _) => return unavailable(req, status_label(status, "device_code_not_found")),
    };
    let Some(status) = status else {
        return unavailable(req, "device_code_unavailable");
    };
    finish(
        req,
        serde_json::to_value(status).unwrap_or(Value::Null),
        call.persisted,
    )
}

/// `account_device_code_redeem {code, ingest_url?}`: enroll this device in
/// the account that minted `code`. Refused when this device is already
/// enrolled.
pub(super) async fn handle_redeem(shared: &DaemonShared, req: &Request) -> Response {
    let Some(code) = req
        .params
        .get("code")
        .and_then(Value::as_str)
        .and_then(device_code)
        .map(str::to_string)
    else {
        return bad(req, "device_code_invalid");
    };
    let Some(tenant_id) = code_tenant(&code) else {
        return bad(req, "device_code_invalid");
    };
    match redeem(shared, req, &code, &tenant_id).await {
        Ok(value) => Response::ok(req.id, value),
        Err(label) => unavailable(req, label),
    }
}

async fn redeem(
    shared: &DaemonShared,
    req: &Request,
    code: &str,
    tenant_id: &str,
) -> Result<Value, &'static str> {
    let store = &shared.store;
    match store.load_config() {
        Ok(None) => {}
        Ok(Some(_)) => return Err("device_code_already_enrolled"),
        Err(_) => return Err("device_code_unavailable"),
    }
    let expected =
        super::commons_credentials::snapshot(store, super::commons_credentials::Kind::Account)
            .map_err(|_| "commons_credential_storage_unavailable")?;
    let ingest_url = super::nearai_onboarding::enroll_origin(store, &req.params)
        .map_err(|_| "device_code_endpoint_refused")?;
    let commons = super::nearai_onboarding::prepare_for_code(&ingest_url)
        .await
        .map_err(|error| match error.to_string().as_str() {
            "near_ai_enroll_endpoint_refused" => "device_code_endpoint_refused",
            "near_ai_enroll_commons_unsupported" => "device_code_commons_unsupported",
            _ => "device_code_commons_unreachable",
        })?;
    let identity = DeviceIdentity::load_or_generate_async(store)
        .await
        .map_err(|_| "device_code_unavailable")?;
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&identity.public_key_b64)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or("device_code_unavailable")?;
    let request = DeviceCodeRedeemRequest {
        code: code.to_string(),
        device_public_key: identity.public_key_b64.clone(),
        signature: identity.sign_b64(&device_code_redeem_bytes(code, &device)),
    };
    // A public route: the status names the two refusals a person can act on.
    let call = commons
        .commons
        .call_json_with_response_headers::<_, DeviceSignInResponse>(
            Method::POST,
            DEVICE_CODE_REDEEM_PATH,
            &[],
            Some(&request),
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
        )
        .await;
    let response = match (call.status, call.result) {
        (Some(StatusCode::OK), Ok(response)) => response,
        (Some(StatusCode::CONFLICT), _) => return Err("device_key_registered_elsewhere"),
        (Some(StatusCode::TOO_MANY_REQUESTS), _) => return Err("device_code_rate_limited"),
        (Some(status), _) if status.is_client_error() => return Err("device_code_refused"),
        _ => return Err("device_code_commons_unreachable"),
    };
    if response.tenant_id != tenant_id || response.device_key_id != identity.device_key_id {
        return Err("device_code_answer_mismatch");
    }
    let account_id = response.account_id.clone();
    let device_key_id = response.device_key_id.clone();
    let worker_store = store.clone();
    let persisted = tokio::task::spawn_blocking(move || {
        super::nearai_onboarding::persist_code_sign_in(
            &worker_store,
            &ingest_url,
            commons,
            &identity,
            response,
            &expected,
        )
    })
    .await
    .map_err(|_| "commons_credential_worker_unavailable")?;
    if persisted.is_err() {
        return Err("device_code_enroll_failed");
    }
    // Without the marker the session would lapse after 12 hours: report it,
    // the enrollment itself stands.
    let renews = store.write_daemon_file(JOINED_BY_CODE_FILE, b"1\n").is_ok();
    Ok(json!({
        "enrolled": true,
        "tenant_id": tenant_id,
        "account_id": account_id,
        "device_key_id": device_key_id,
        "renews_by_device_key": renews,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_id_must_be_a_lowercase_sha256() {
        let good = format!("sha256:{}", "a1".repeat(32));
        assert!(valid_device_key_id(&good));
        assert!(!valid_device_key_id(&good.to_uppercase()));
        assert!(!valid_device_key_id("sha256:abc"));
        assert!(!valid_device_key_id(&format!(
            "sha256:{}/x",
            "a".repeat(62)
        )));
        assert!(!valid_device_key_id(&"a".repeat(71)));
    }

    #[test]
    fn a_label_is_trimmed_bounded_and_free_of_control_characters() {
        assert_eq!(device_label(None), Ok(None));
        assert_eq!(device_label(Some("   ")), Ok(None));
        assert_eq!(
            device_label(Some("  build box ")),
            Ok(Some("build box".into()))
        );
        assert_eq!(
            device_label(Some(&"é".repeat(64))),
            Ok(Some("é".repeat(64)))
        );
        assert!(device_label(Some(&"x".repeat(65))).is_err());
        assert!(device_label(Some("a\nb")).is_err());
    }

    #[test]
    fn a_code_is_checked_for_shape_and_names_its_tenant() {
        let tenant = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("nearai-acme");
        let code = format!("{DEVICE_CODE_PREFIX}{tenant}.s3cr3t-_X");
        assert_eq!(device_code(&format!("  {code}\n")), Some(code.as_str()));
        assert_eq!(code_tenant(&code).as_deref(), Some("nearai-acme"));
        assert_eq!(device_code("tcn1_abc.def"), None);
        assert_eq!(device_code(&format!("{DEVICE_CODE_PREFIX}{tenant}.")), None);
        assert_eq!(device_code(&format!("{DEVICE_CODE_PREFIX}.secret")), None);
        assert_eq!(
            device_code(&format!("{DEVICE_CODE_PREFIX}{tenant}.a b")),
            None
        );
        let long = format!("{DEVICE_CODE_PREFIX}{tenant}.{}", "a".repeat(CODE_MAX_LEN));
        assert_eq!(device_code(&long), None);
    }

    #[test]
    fn a_refusal_is_a_fixed_label() {
        assert_eq!(
            status_label(Some(StatusCode::UNAUTHORIZED), "x"),
            ERR_ACCOUNT_SESSION_REQUIRED
        );
        assert_eq!(status_label(Some(StatusCode::NOT_FOUND), "gone"), "gone");
        assert_eq!(status_label(None, "x"), "account_devices_unavailable");
        assert_eq!(
            status_label(Some(StatusCode::INTERNAL_SERVER_ERROR), "x"),
            "account_devices_unavailable"
        );
    }

    #[test]
    fn this_device_is_flagged_by_its_key_even_when_its_session_is_unattributed() {
        let mut devices = vec![
            json!({"device_key_id": "sha256:aa", "this_device": false}),
            json!({"device_key_id": "sha256:bb", "this_device": false}),
        ];
        mark_this_device(&mut devices, Some("sha256:bb"));
        assert_eq!(devices[0]["this_device"], json!(false));
        assert_eq!(devices[1]["this_device"], json!(true));
        mark_this_device(&mut devices, None);
        assert_eq!(devices[1]["this_device"], json!(true));
    }

    #[tokio::test]
    async fn a_device_that_joined_by_code_is_refused_a_mint_before_any_request() {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = DaemonShared::load(store).unwrap();
        shared
            .store
            .write_daemon_file(JOINED_BY_CODE_FILE, b"1\n")
            .unwrap();
        let req = Request {
            id: 7,
            method: "account_device_code_mint".into(),
            params: serde_json::json!({}),
        };
        let response = handle_mint(&shared, &req).await;
        assert_eq!(
            response.error.map(|error| error.message).as_deref(),
            Some(ERR_JOINED_BY_CODE)
        );
        drop(dir);
    }

    #[test]
    fn the_marker_says_whether_a_device_joined_by_code() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        assert!(!joined_by_code(&store));
        store
            .write_daemon_file(JOINED_BY_CODE_FILE, b"1\n")
            .unwrap();
        assert!(joined_by_code(&store));
    }
}
