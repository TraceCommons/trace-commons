// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The account devices list
//! (design: "Linking more devices to a NEAR AI account", 2026-10-27).
//!
//! - `GET /v1/account/devices` lists the devices linked to the caller's
//!   account through NEAR sign-in, flagging the one whose session made the
//!   request.
//! - `PATCH /v1/account/devices/{device_key_id}` renames one.
//! - `DELETE /v1/account/devices/{device_key_id}` revokes one: its key, its
//!   tenant access grants and its native sessions, in one transaction.
//!
//! All three take the signed-in account session, weak or strong, like the
//! NEAR identity list. A NEAR AI account usually holds no strong
//! authenticator, so a strong-only revoke would leave most accounts unable to
//! revoke at all. Neither rename nor revoke adds authority: the worst a
//! stolen session can do here is cut off the owner's other devices and the
//! account's unattributed native sessions (below), all of which sign in
//! again. A revoked key is never revived; its next sign-in is refused by name
//! (`device_key_revoked`), and the device recovers by signing out, which
//! drops its key, and signing in again, which makes a new key and joins as a
//! new device. An unknown, revoked or foreign device is a uniform 404.
//!
//! A revoke ends the sessions that name the device's key. A sign-in reads
//! the key `FOR SHARE` and the revoke updates it, so the two serialize: the
//! sign-in either commits its session before the revoke reads the sessions,
//! or finds the key revoked. (Session validation does not read `device_keys`:
//! least-privilege session roles hold no grant on it.)
//!
//! Native sessions that name neither a key nor a credential (issued before
//! V139, or by the login-link native sign-in) cannot be attributed, so a
//! revoke ends those too, except the caller's own: a NEAR AI device among
//! them renews by itself with a keyed session, and any other signs in again.

use super::*;

/// Longest label a contributor may give a device, in characters. Matches the
/// `device_keys.label` CHECK in V139.
const DEVICE_LABEL_MAX_CHARS: usize = 64;

pub(super) fn routes() -> crate::ingest::app::account_routes::AccountRoutes {
    crate::ingest::app::account_routes::AccountRoutes::new()
        .get("/v1/account/devices", list_handler)
        .patch("/v1/account/devices/{device_key_id}", rename_handler)
        .delete("/v1/account/devices/{device_key_id}", revoke_handler)
        .route_layer_fn(no_store_middleware)
}

async fn no_store_middleware(request: Request, next: Next) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

/// One device as `GET /v1/account/devices` reports it.
#[derive(Debug, Serialize)]
struct AccountDeviceListItem {
    device_key_id: String,
    public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    /// How the device joined: `near_ai` (NEAR AI sign-in) or `near` (wallet).
    origin: String,
    created_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<DateTime<Utc>>,
    this_device: bool,
    /// How many of the device's traces still stand in the commons
    /// (`AccountDeviceSummary::contributed_traces`). Always present: zero
    /// is a count.
    contributed_traces: i64,
    /// When the newest of those was received.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_contributed_at: Option<DateTime<Utc>>,
}

async fn list_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<Json<serde_json::Value>> {
    let db = account_db(state.as_ref())?;
    let devices = db
        .list_account_devices(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            ctx.session_token_hash.as_deref(),
        )
        .await
        .map_err(internal_error)?;
    let devices: Vec<AccountDeviceListItem> = devices
        .into_iter()
        .map(|device| AccountDeviceListItem {
            device_key_id: device.device_key_id,
            public_key: device.public_key,
            label: device.label,
            origin: device.onboarding_origin,
            created_at: device.created_at,
            last_seen_at: device.last_seen_at,
            this_device: device.this_device,
            contributed_traces: device.contributed_traces,
            last_contributed_at: device.last_contributed_at,
        })
        .collect();
    Ok(Json(serde_json::json!({ "devices": devices })))
}

/// Request body for `PATCH /v1/account/devices/{device_key_id}`. A missing,
/// blank or whitespace-only `label` clears it.
#[derive(Debug, Deserialize)]
struct RenameBody {
    #[serde(default)]
    label: Option<String>,
}

/// The label to store, or a 400 for one that is too long or carries control
/// characters. Blank means "clear".
fn device_label(raw: Option<&str>) -> ApiResult<Option<&str>> {
    let Some(label) = raw.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };
    if label.chars().count() > DEVICE_LABEL_MAX_CHARS || label.chars().any(char::is_control) {
        return Err(api_error(StatusCode::BAD_REQUEST, "invalid device label"));
    }
    Ok(Some(label))
}

async fn rename_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    AxumPath(device_key_id): AxumPath<String>,
    Json(body): Json<RenameBody>,
) -> ApiResult<StatusCode> {
    let device_key_id = device_path_id(&device_key_id)?;
    let label = device_label(body.label.as_deref())?;
    let db = account_db(state.as_ref())?;
    let renamed = db
        .rename_account_device(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            &device_key_id,
            label,
        )
        .await
        .map_err(internal_error)?;
    if !renamed {
        return Err(device_not_found());
    }
    db.append_account_audit(
        &ctx.tenant_id,
        "account_device_renamed",
        &ctx.actor_ref,
        "success",
        serde_json::json!({ "labeled": label.is_some() }),
    )
    .await
    .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    AxumPath(device_key_id): AxumPath<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let device_key_id = device_path_id(&device_key_id)?;
    let db = account_db(state.as_ref())?;
    let result = db
        .revoke_account_device(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            &device_key_id,
            &ctx.actor_ref,
            ctx.session_token_hash.as_deref(),
        )
        .await
        .map_err(internal_error)?;
    if !result.removed {
        return Err(device_not_found());
    }
    db.append_account_audit(
        &ctx.tenant_id,
        "account_device_revoked",
        &ctx.actor_ref,
        "success",
        serde_json::json!({
            "sessions_revoked": result.sessions_revoked,
            "grants_revoked": result.grants_revoked,
            "unattributed_sessions_revoked": result.unattributed_sessions_revoked,
        }),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(serde_json::json!({
        "revoked": true,
        "sessions_revoked": result.sessions_revoked
            + result.unattributed_sessions_revoked,
    })))
}

/// A malformed id cannot name any device, so it gets the same 404 as an
/// unknown one rather than a distinguishable 400.
fn device_path_id(raw: &str) -> ApiResult<String> {
    validate_trace_sha256_hash(raw, "device_key_id").map_err(|_| device_not_found())
}

fn device_not_found() -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::NOT_FOUND, "device not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A device's contribution count is always on the wire, zero included,
    /// so a client can tell "nothing yet" from a server too old to say;
    /// its last time is left out when there is none.
    #[test]
    fn a_listed_device_always_carries_its_count() {
        let item = |traces: i64, last: Option<DateTime<Utc>>| AccountDeviceListItem {
            device_key_id: "sha256:aa".into(),
            public_key: "k".into(),
            label: None,
            origin: "near_ai".into(),
            created_at: Utc::now(),
            last_seen_at: None,
            this_device: false,
            contributed_traces: traces,
            last_contributed_at: last,
        };
        let none = serde_json::to_value(item(0, None)).unwrap();
        assert_eq!(none["contributed_traces"], 0);
        assert!(none.get("last_contributed_at").is_none());
        let at = Utc::now();
        let some = serde_json::to_value(item(3, Some(at))).unwrap();
        assert_eq!(some["contributed_traces"], 3);
        assert_eq!(
            some["last_contributed_at"],
            serde_json::to_value(at).unwrap()
        );
    }

    #[test]
    fn a_blank_label_clears_and_a_long_or_control_label_is_refused() {
        assert_eq!(device_label(None).unwrap(), None);
        assert_eq!(device_label(Some("   ")).unwrap(), None);
        assert_eq!(
            device_label(Some("  zaki-server ")).unwrap(),
            Some("zaki-server")
        );
        assert!(device_label(Some(&"x".repeat(DEVICE_LABEL_MAX_CHARS))).is_ok());
        assert!(device_label(Some(&"x".repeat(DEVICE_LABEL_MAX_CHARS + 1))).is_err());
        assert!(device_label(Some("line\nbreak")).is_err());
    }

    #[test]
    fn a_malformed_device_id_is_a_plain_not_found() {
        let error = device_path_id("not-a-hash").unwrap_err();
        assert_eq!(error.0, StatusCode::NOT_FOUND);
        assert!(device_path_id(&format!("sha256:{}", "a".repeat(64))).is_ok());
    }
}
