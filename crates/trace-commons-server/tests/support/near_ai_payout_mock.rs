// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A loopback mock of cloud-api #1271's transfer routes for the payout pass
//! suites: cumulative program totals kept the way #1271 keeps them, outgoing
//! history newest first with a cursor returned even on the last page, a
//! capacity reply shaped like staging's (paid credit as `payment`, the other
//! sources at 0, and the fields the rail does not read), the `oc_` bearer,
//! and a log of every request. Scripted replies answer the next request on
//! any route. Every value is synthetic.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use trace_commons_server::near_ai_rail::NearAiRail;
use trace_commons_server::near_ai_transfer::{
    NearAiCreditSource, NearAiTransferClient, NearAiTransferCredentials,
};
use uuid::Uuid;

pub const TOKEN: &str = "oc_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

pub fn sender() -> Uuid {
    Uuid::from_u128(0x5e4d_0000_0000_4000_8000_0000_0000_0001)
}

pub fn recipient_org(n: u128) -> Uuid {
    Uuid::from_u128(0x4ec1_0000_0000_4000_8000_0000_0000_0000 + n)
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: Method,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub id: Uuid,
    pub recipient: Uuid,
    pub program: String,
    pub amount: i64,
    pub cumulative: i64,
    pub created_at: DateTime<Utc>,
}

/// A scripted reply for the next request, on whichever route it lands.
#[derive(Debug, Clone, Copy)]
pub enum Scripted {
    /// Answer normally: lets a script refuse the second or third request.
    Pass,
    Error(u16, &'static str),
}

pub struct MockState {
    transfers: Vec<Transfer>,
    requests: Vec<Recorded>,
    inactive: Vec<Uuid>,
    script: VecDeque<Scripted>,
    per_send_max: i64,
    sends_remaining: i64,
    sending_enabled: bool,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            transfers: Vec::new(),
            requests: Vec::new(),
            inactive: Vec::new(),
            script: VecDeque::new(),
            per_send_max: 1_000_000_000_000,
            sends_remaining: 60,
            sending_enabled: true,
        }
    }
}

type Shared = Arc<Mutex<MockState>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct CreateTransferRequest {
    recipient_organization_id: Uuid,
    source_credit_type: String,
    amount: Option<i64>,
    recipient_cumulative: Option<i64>,
    payout_program: Option<String>,
    #[serde(default)]
    memo: String,
    #[serde(default)]
    metadata: serde_json::Map<String, Value>,
}

fn error_reply(status: u16, code: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        axum::Json(json!({"error": {"type": code, "message": code}})),
    )
        .into_response()
}

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(&format!("Bearer {TOKEN}"))
}

fn begin(state: &Shared, method: Method) -> Option<Response> {
    let mut guard = state.lock().unwrap();
    guard.requests.push(Recorded { method });
    match guard.script.pop_front()? {
        Scripted::Pass => None,
        Scripted::Error(status, code) => Some(error_reply(status, code)),
    }
}

async fn transfer(
    State(state): State<Shared>,
    Path(org): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(reply) = begin(&state, Method::POST) {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    let Ok(request) = serde_json::from_slice::<CreateTransferRequest>(&body) else {
        return error_reply(400, "invalid_transfer_terms");
    };
    let shape = request.amount.is_none()
        && request
            .recipient_cumulative
            .is_some_and(|target| target >= 0)
        && request
            .payout_program
            .as_ref()
            .is_some_and(|program| !program.is_empty() && program.len() <= 100)
        && request.memo.is_empty();
    if org != sender()
        || org == request.recipient_organization_id
        || request.source_credit_type != "payment"
        || !shape
    {
        return error_reply(400, "invalid_transfer_terms");
    }
    let mut guard = state.lock().unwrap();
    if guard.inactive.contains(&request.recipient_organization_id) {
        return error_reply(404, "organization_unavailable");
    }
    let program = request.payout_program.unwrap();
    let total: i64 = guard
        .transfers
        .iter()
        .filter(|t| t.recipient == request.recipient_organization_id && t.program == program)
        .map(|t| t.amount)
        .sum();
    let target = request.recipient_cumulative.unwrap();
    let amount = target.saturating_sub(total).max(0);
    if amount == 0 {
        return (
            StatusCode::OK,
            axum::Json(json!({
                "sender_organization_id": org,
                "recipient_organization_id": request.recipient_organization_id,
                "payout_program": program,
                "recipient_cumulative": total,
                "amount": 0,
                "no_op": true,
                "delivered": true,
            })),
        )
            .into_response();
    }
    if guard.per_send_max < amount {
        return error_reply(409, "credential_transfer_limit_exceeded");
    }
    let id = Uuid::new_v4();
    let created_at = Utc::now();
    guard.transfers.push(Transfer {
        id,
        recipient: request.recipient_organization_id,
        program: program.clone(),
        amount,
        cumulative: total + amount,
        created_at,
    });
    (
        StatusCode::CREATED,
        axum::Json(json!({
            "id": id,
            "sender_organization_id": org,
            "recipient_organization_id": request.recipient_organization_id,
            "source_credit_type": request.source_credit_type,
            "destination_credit_type": "transfer",
            "amount": amount,
            "payout_program": program,
            "recipient_cumulative": total + amount,
            "memo": "",
            "metadata": request.metadata,
            "created_at": created_at,
            "delivered": true,
            "schema_version": 1,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct HistoryQuery {
    before_time: Option<DateTime<Utc>>,
    before_id: Option<Uuid>,
    limit: Option<i64>,
}

async fn history(
    State(state): State<Shared>,
    Path(org): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Response {
    if let Some(reply) = begin(&state, Method::GET) {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    let limit = query.limit.unwrap_or(50).clamp(1, 100) as usize;
    let guard = state.lock().unwrap();
    let mut rows: Vec<&Transfer> = guard.transfers.iter().collect();
    rows.sort_by_key(|t| std::cmp::Reverse((t.created_at, t.id)));
    let rows: Vec<&Transfer> = rows
        .into_iter()
        .filter(|t| match (query.before_time, query.before_id) {
            (Some(time), Some(id)) => (t.created_at, t.id) < (time, id),
            _ => true,
        })
        .take(limit)
        .collect();
    let items: Vec<Value> = rows
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "sender_organization_id": org,
                "recipient_organization_id": t.recipient,
                "source_credit_type": "payment",
                "destination_credit_type": "transfer",
                "amount": t.amount,
                "payout_program": t.program,
                "recipient_cumulative": t.cumulative,
                "memo": "",
                "metadata": {},
                "created_at": t.created_at,
                "idempotency_key": null,
            })
        })
        .collect();
    let next = rows
        .last()
        .map(|t| json!({"before_time": t.created_at, "before_id": t.id}));
    axum::Json(json!({"transfers": items, "next_cursor": next})).into_response()
}

async fn capacity(
    State(state): State<Shared>,
    Path(_org): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if let Some(reply) = begin(&state, Method::GET) {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    let guard = state.lock().unwrap();
    let max = if guard.sends_remaining > 0 {
        guard.per_send_max
    } else {
        0
    };
    axum::Json(json!({
        "credits": [{"credit_type": "payment", "currency": "USD", "available": max}],
        "own_staking_earned_total": 0,
        "sender_reconciled": true,
        "sending_enabled": guard.sending_enabled,
        "credential_can_transfer": true,
        "organization_constraints": {"max_amount_per_transfer": null},
        "credential_constraints": {"max_amount_per_transfer": null},
        "organization_remaining_budget": max,
        "credential_remaining_budget": max,
        "sends_remaining_in_window": guard.sends_remaining,
        "sources": [
            {"source_credit_type": "payment", "max_amount_per_transfer": max},
            {"source_credit_type": "staking_farm", "max_amount_per_transfer": 0},
            {"source_credit_type": "grant", "max_amount_per_transfer": 0},
        ],
        "recipient_restrictions_apply": true,
        "is_reservation": false,
    }))
    .into_response()
}

pub struct Mock {
    state: Shared,
    base: String,
}

impl Mock {
    pub async fn start() -> Self {
        let state: Shared = Arc::default();
        let app = Router::new()
            .route("/v1/organizations/{org}/credit/transfer", post(transfer))
            .route("/v1/organizations/{org}/credit/transfers", get(history))
            .route(
                "/v1/organizations/{org}/credit/transfer/capacity",
                get(capacity),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            state,
            base: format!("http://{address}/v1"),
        }
    }

    /// The `near_ai` rail against this mock, sending from `payment`.
    pub fn rail(&self) -> NearAiRail {
        NearAiRail::new(
            NearAiTransferClient::at_loopback(&self.base, credentials(), Duration::from_secs(5))
                .unwrap(),
        )
    }

    pub fn script(&self, reply: Scripted) {
        self.state.lock().unwrap().script.push_back(reply);
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().requests.clone()
    }

    /// POSTs to the transfer route: every write call, zero reads included.
    pub fn posts(&self) -> usize {
        self.requests()
            .iter()
            .filter(|request| request.method == Method::POST)
            .count()
    }

    pub fn transfers(&self) -> Vec<Transfer> {
        self.state.lock().unwrap().transfers.clone()
    }

    pub fn set_per_send_max(&self, max: i64) {
        self.state.lock().unwrap().per_send_max = max;
    }

    pub fn set_sends_remaining(&self, remaining: i64) {
        self.state.lock().unwrap().sends_remaining = remaining;
    }

    pub fn deactivate(&self, org: Uuid) {
        self.state.lock().unwrap().inactive.push(org);
    }

    /// Seeds a transfer as if it had been sent earlier.
    pub fn seed(&self, recipient: Uuid, program: &str, amount: i64) -> Uuid {
        let mut guard = self.state.lock().unwrap();
        let total: i64 = guard
            .transfers
            .iter()
            .filter(|t| t.recipient == recipient && t.program == program)
            .map(|t| t.amount)
            .sum();
        let id = Uuid::new_v4();
        guard.transfers.push(Transfer {
            id,
            recipient,
            program: program.to_string(),
            amount,
            cumulative: total + amount,
            created_at: Utc::now() - chrono::Duration::seconds(1),
        });
        id
    }
}

pub fn credentials() -> NearAiTransferCredentials {
    NearAiTransferCredentials {
        token: SecretString::from(TOKEN.to_string()),
        credential_id: Some(Uuid::from_u128(0xc4ed_0000_0000_4000_8000_0000_0000_0002)),
        sender_org: sender(),
        source: NearAiCreditSource::Payment,
    }
}
