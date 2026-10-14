// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A fake of the NEAR AI (`nearai/cloud-api`) admin route the inference
//! credit rail uses: the proposed
//! `PUT|GET /v1/admin/organizations/{org_id}/trace-commons/credit`, modelled
//! on the staking-farm writer and on the local cloud-api draft:
//!
//! - PUT raises the organization's `trace_commons` total to the stated
//!   cumulative value, never lowers it, and answers the total with
//!   `applied`; a value at or below the current one writes nothing.
//! - 404 for an unknown or inactive organization; 400 for a negative value,
//!   one above 1e18, or a malformed `statementRef`.
//! - Admin keys are `adm_`-prefixed and bound to the User-Agent they were
//!   minted with (`admin_access_token.rs:140`); a read-only key gets 403 on
//!   the PUT and may GET.

#![allow(dead_code)]

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ReadOnly,
    ReadWrite,
}

#[derive(Default)]
pub struct FakeState {
    /// Organization id -> active.
    pub orgs: HashMap<Uuid, bool>,
    /// Organization id -> current cumulative trace_commons total.
    pub credit: HashMap<Uuid, i64>,
    /// Every raise, as (org, total, statementRef).
    pub raises: Vec<(Uuid, i64, String)>,
    pub keys: HashMap<String, (Permission, Option<String>)>,
    pub put_calls: u64,
    /// Commit the next PUT, then stall the response this long.
    pub stall_after_commit: Option<Duration>,
    /// Commit the next PUT, then answer 502 as a proxy would.
    pub bad_gateway_after_commit: bool,
    /// Fail the next N requests with 503 before doing anything.
    pub unavailable: u32,
    /// Answer the next PUT with this total instead of the real one (a
    /// misbehaving server), after committing nothing.
    pub lie_next_put: Option<i64>,
}

impl FakeState {
    pub fn current(&self, org: Uuid) -> i64 {
        self.credit.get(&org).copied().unwrap_or(0)
    }
}

pub type Shared = Arc<Mutex<FakeState>>;

fn error(status: StatusCode, kind: &str) -> (StatusCode, Json<Value>) {
    (
        status,
        // NEAR AI's real messages echo the organization id; the client must
        // never read them, so the fake puts something recognisable here.
        Json(json!({"error": {"message": "LEAKY-BODY", "type": kind}})),
    )
}

fn authorize(
    state: &FakeState,
    headers: &HeaderMap,
    needs_write: bool,
) -> Result<(), (StatusCode, Json<Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok());
    let (permission, bound_ua) = state
        .keys
        .get(token)
        .filter(|_| token.starts_with("adm_"))
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    if let Some(bound) = bound_ua {
        if ua != Some(bound.as_str()) {
            return Err(error(StatusCode::UNAUTHORIZED, "unauthorized"));
        }
    }
    if needs_write && *permission == Permission::ReadOnly {
        return Err(error(StatusCode::FORBIDDEN, "insufficient_permissions"));
    }
    Ok(())
}

fn active_org(state: &FakeState, org: &str) -> Result<Uuid, (StatusCode, Json<Value>)> {
    let org =
        Uuid::parse_str(org).map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    if state.orgs.get(&org).copied().unwrap_or(false) {
        Ok(org)
    } else {
        Err(error(StatusCode::NOT_FOUND, "not_found"))
    }
}

fn body(org: Uuid, total: i64, applied: bool) -> Json<Value> {
    Json(
        json!({"organization_id": org.to_string(), "cumulativeNanoUsd": total, "applied": applied}),
    )
}

async fn read(
    State(state): State<Shared>,
    Path(org): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut s = state.lock().unwrap();
    if s.unavailable > 0 {
        s.unavailable -= 1;
        return Err(error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
    }
    authorize(&s, &headers, false)?;
    let org = active_org(&s, &org)?;
    Ok(body(org, s.current(org), false))
}

async fn write(
    State(state): State<Shared>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (response, stall) = {
        let mut s = state.lock().unwrap();
        if s.unavailable > 0 {
            s.unavailable -= 1;
            return Err(error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
        }
        authorize(&s, &headers, true)?;
        let total = request["cumulativeNanoUsd"]
            .as_i64()
            .filter(|v| (0..=1_000_000_000_000_000_000).contains(v))
            .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
        let statement = request["statementRef"]
            .as_str()
            .filter(|v| {
                !v.is_empty()
                    && v.len() <= 128
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
            })
            .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid_request"))?
            .to_string();
        let org = active_org(&s, &org)?;
        s.put_calls += 1;
        if let Some(lie) = s.lie_next_put.take() {
            return Ok(body(org, lie, false));
        }
        let current = s.current(org);
        let applied = total > current;
        if applied {
            s.credit.insert(org, total);
            s.raises.push((org, total, statement));
        }
        if std::mem::take(&mut s.bad_gateway_after_commit) {
            return Err(error(StatusCode::BAD_GATEWAY, "bad_gateway"));
        }
        (
            body(org, s.current(org), applied),
            s.stall_after_commit.take(),
        )
    };
    if let Some(stall) = stall {
        tokio::time::sleep(stall).await;
    }
    Ok(response)
}

/// Start the fake on an ephemeral loopback port; returns its `/v1` base URL.
pub async fn start(state: Shared) -> String {
    let app = Router::new()
        .route(
            "/v1/admin/organizations/{org_id}/trace-commons/credit",
            get(read).put(write),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1")
}
