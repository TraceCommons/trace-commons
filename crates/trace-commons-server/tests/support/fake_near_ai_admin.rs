// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A fake of the NEAR AI (`nearai/cloud-api`) admin routes the inference
//! credit rail uses, modelling the semantics recorded in the design spec,
//! section 1 -- including the absence of compare-and-swap:
//!
//! - `PATCH .../limits` closes every active row of the request's credit
//!   type, **whatever its source**, and inserts the new absolute value
//!   (`organization_limits.rs:73-121`); 404 for a missing or inactive
//!   organization; 400 outside `0..=1e18`.
//! - `GET .../limits/history` is newest first, paginated, and answers 404 for
//!   an organization with no rows (`admin/mod.rs:590-594`).
//! - Admin keys are `adm_`-prefixed, bound to the User-Agent they were
//!   minted with when one was recorded (`admin_access_token.rs:140`), and a
//!   read-only key gets 403 `insufficient_permissions` on a write.
//! - `changedBy` over 255 or `changeReason` over 500 characters is a column
//!   overflow, which is a 500 there.

#![allow(dead_code)]

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Row {
    pub id: Uuid,
    pub credit_type: String,
    pub source: Option<String>,
    pub amount: i64,
    pub effective_from: u64,
    pub effective_until: Option<u64>,
    pub changed_by: Option<String>,
    pub change_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    ReadOnly,
    ReadWrite,
}

#[derive(Default)]
pub struct FakeState {
    pub orgs: HashMap<Uuid, bool>,
    pub rows: HashMap<Uuid, Vec<Row>>,
    pub keys: HashMap<String, (Permission, Option<String>)>,
    pub clock: u64,
    pub patch_calls: u64,
    /// Commit the next PATCH, then stall the response this long.
    pub stall_after_commit: Option<Duration>,
    /// Before applying the next PATCH, apply a foreign write of this amount.
    pub foreign_write_before_patch: Option<i64>,
    /// Fail the next N requests with 503 before doing anything.
    pub unavailable: u32,
}

impl FakeState {
    pub fn write(
        &mut self,
        org: Uuid,
        credit_type: &str,
        source: Option<&str>,
        amount: i64,
        changed_by: Option<&str>,
        change_reason: Option<&str>,
    ) {
        self.clock += 1;
        let now = self.clock;
        let rows = self.rows.entry(org).or_default();
        for row in rows.iter_mut() {
            if row.credit_type == credit_type && row.effective_until.is_none() {
                row.effective_until = Some(now);
            }
        }
        rows.push(Row {
            id: Uuid::new_v4(),
            credit_type: credit_type.to_string(),
            source: source.map(str::to_string),
            amount,
            effective_from: now,
            effective_until: None,
            changed_by: changed_by.map(str::to_string),
            change_reason: change_reason.map(str::to_string),
        });
    }

    pub fn active(&self, org: Uuid, credit_type: &str) -> Option<Row> {
        self.rows
            .get(&org)?
            .iter()
            .find(|r| r.credit_type == credit_type && r.effective_until.is_none())
            .cloned()
    }

    pub fn rows_with_reason(&self, org: Uuid, reason: &str) -> usize {
        self.rows
            .get(&org)
            .map(|rs| {
                rs.iter()
                    .filter(|r| r.change_reason.as_deref() == Some(reason))
                    .count()
            })
            .unwrap_or(0)
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

#[derive(Deserialize)]
struct HistoryQuery {
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn history(
    State(state): State<Shared>,
    Path(org): Path<String>,
    Query(q): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let mut s = state.lock().unwrap();
    if s.unavailable > 0 {
        s.unavailable -= 1;
        return Err(error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
    }
    authorize(&s, &headers, false)?;
    let org =
        Uuid::parse_str(&org).map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_request"))?;
    let limit = q.limit.unwrap_or(50);
    let offset = q.offset.unwrap_or(0);
    let mut rows = s.rows.get(&org).cloned().unwrap_or_default();
    if rows.is_empty() {
        return Err(error(StatusCode::NOT_FOUND, "organization_not_found"));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.effective_from));
    let total = rows.len() as i64;
    let page: Vec<Value> = rows
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|r| {
            json!({
                "id": r.id.to_string(),
                "organizationId": org.to_string(),
                "type": r.credit_type,
                "source": r.source,
                "spendLimit": {"amount": r.amount, "scale": 9, "currency": "USD"},
                "effectiveFrom": format!("2026-10-14T00:00:{:02}Z", r.effective_from % 60),
                "effectiveUntil": r.effective_until.map(|t| format!("2026-10-14T00:00:{:02}Z", t % 60)),
                "changedBy": r.changed_by,
                "changeReason": r.change_reason,
                "changedByUserId": "00000000-0000-4000-8000-000000000001",
                "changedByUserEmail": "admin@near.ai",
                "createdAt": "2026-10-14T00:00:00Z"
            })
        })
        .collect();
    Ok(Json(
        json!({"history": page, "total": total, "limit": limit, "offset": offset}),
    ))
}

async fn update(
    State(state): State<Shared>,
    Path(org): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let (response, stall) = {
        let mut s = state.lock().unwrap();
        if s.unavailable > 0 {
            s.unavailable -= 1;
            return Err(error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
        }
        authorize(&s, &headers, true)?;
        let org =
            Uuid::parse_str(&org).map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_id"))?;
        if !s.orgs.get(&org).copied().unwrap_or(false) {
            return Err(error(StatusCode::NOT_FOUND, "organization_not_found"));
        }
        let credit_type = body["type"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(
            credit_type.as_str(),
            "grant" | "payment" | "staking_farm" | "postpay"
        ) {
            return Err(error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"));
        }
        let amount = body["spendLimit"]["amount"]
            .as_i64()
            .ok_or_else(|| error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_request"))?;
        if !(0..=1_000_000_000_000_000_000).contains(&amount) {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_limits"));
        }
        let changed_by = body["changedBy"].as_str();
        let change_reason = body["changeReason"].as_str();
        if changed_by.is_some_and(|v| v.len() > 255) || change_reason.is_some_and(|v| v.len() > 500)
        {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_server_error",
            ));
        }
        s.patch_calls += 1;
        if let Some(foreign) = s.foreign_write_before_patch.take() {
            s.write(
                org,
                &credit_type,
                Some("nearai"),
                foreign,
                Some("ops"),
                Some("promo"),
            );
        }
        let source = body["source"].as_str().map(str::to_string);
        s.write(
            org,
            &credit_type,
            source.as_deref(),
            amount,
            changed_by,
            change_reason,
        );
        (
            json!({
                "organization_id": org.to_string(),
                "type": credit_type,
                "source": source,
                "spendLimit": {"amount": amount, "scale": 9, "currency": "USD"},
                "updated_at": "2026-10-14T00:00:00Z"
            }),
            s.stall_after_commit.take(),
        )
    };
    if let Some(stall) = stall {
        tokio::time::sleep(stall).await;
    }
    Ok(Json(response))
}

/// Start the fake on an ephemeral loopback port; returns its `/v1` base URL.
pub async fn start(state: Shared) -> String {
    let app = Router::new()
        .route("/v1/admin/organizations/{org_id}/limits", patch(update))
        .route(
            "/v1/admin/organizations/{org_id}/limits/history",
            get(history),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/v1")
}
