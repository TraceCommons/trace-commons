// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `near_ai` payout rail against a loopback mock of cloud-api #1271
//! (payout spec section 4, "The NEAR AI client" and "`near_ai` refusal
//! mapping").
//!
//! The mock keeps cumulative program totals the way #1271 does
//! (`max(target - SUM(program), 0)` under one lock, a zero delta answers 200
//! `no_op` with no id), pages outgoing history newest first with a cursor
//! that is returned even on the last page, enforces the `oc_` bearer, and
//! records every request so a test can say what went on the wire. Every
//! value here is synthetic.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use trace_commons_gate_api::pipeline::AtomicUnits;
use trace_commons_gate_api::{
    PayoutRail, RailLine, RailRefusal, ReceiptLookup, RecipientRef, SendOutcome,
};
use trace_commons_server::near_ai_rail::{
    NEAR_AI_PROBE_PROGRAM, NearAiRail, near_ai_line_metadata_tag, near_ai_program_for_link,
    near_ai_receipt_hash,
};
use trace_commons_server::near_ai_transfer::{
    NearAiCreditSource, NearAiTransferClient, NearAiTransferCredentials,
};
use uuid::Uuid;

const TOKEN: &str = "oc_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn sender() -> Uuid {
    Uuid::from_u128(0x5e4d_0000_0000_4000_8000_0000_0000_0001)
}

fn credential_id() -> Uuid {
    Uuid::from_u128(0xc4ed_0000_0000_4000_8000_0000_0000_0002)
}

fn recipient_org(n: u128) -> Uuid {
    Uuid::from_u128(0x4ec1_0000_0000_4000_8000_0000_0000_0000 + n)
}

fn line_ref(n: u64) -> String {
    format!("sha256:{n:064x}")
}

#[derive(Debug, Clone)]
struct Recorded {
    method: Method,
    path: String,
    query: Option<String>,
    idempotency_key: bool,
    body: Option<Value>,
}

#[derive(Debug, Clone)]
struct Transfer {
    id: Uuid,
    recipient: Uuid,
    program: Option<String>,
    amount: i64,
    cumulative: Option<i64>,
    created_at: DateTime<Utc>,
}

/// A scripted reply: answers the next request, whatever its route.
#[derive(Debug, Clone)]
enum Scripted {
    Error(u16, &'static str),
    Raw(u16, Value),
    /// A reply whose body is not JSON (empty, or a proxy's HTML page).
    Text(u16, &'static str),
    Redirect(&'static str),
    Delay(Duration),
}

#[derive(Default)]
struct MockState {
    transfers: Vec<Transfer>,
    requests: Vec<Recorded>,
    inactive: Vec<Uuid>,
    script: VecDeque<Scripted>,
    redirect_target_hits: usize,
    capacity: Option<Value>,
}

type Shared = Arc<Mutex<MockState>>;

/// What cloud-api's `CreateTransferRequest` accepts: anything else is a 400.
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

fn record(state: &Shared, method: Method, uri: &Uri, headers: &HeaderMap, body: Option<Value>) {
    state.lock().unwrap().requests.push(Recorded {
        method,
        path: uri.path().to_string(),
        query: uri.query().map(str::to_string),
        idempotency_key: headers.contains_key("idempotency-key"),
        body,
    });
}

/// Pops a scripted reply. A delay is awaited and then the request carries on.
async fn scripted(state: &Shared) -> Option<Response> {
    let next = state.lock().unwrap().script.pop_front();
    match next? {
        Scripted::Error(status, code) => Some(error_reply(status, code)),
        Scripted::Raw(status, body) => {
            Some((StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response())
        }
        Scripted::Text(status, body) => {
            Some((StatusCode::from_u16(status).unwrap(), body).into_response())
        }
        Scripted::Redirect(location) => Some(
            Response::builder()
                .status(StatusCode::TEMPORARY_REDIRECT)
                .header("location", location)
                .body(axum::body::Body::empty())
                .unwrap(),
        ),
        Scripted::Delay(delay) => {
            tokio::time::sleep(delay).await;
            None
        }
    }
}

async fn transfer(
    State(state): State<Shared>,
    Path(org): Path<Uuid>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let parsed: Option<Value> = serde_json::from_slice(&body).ok();
    record(&state, method, &uri, &headers, parsed.clone());
    if let Some(reply) = scripted(&state).await {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    let Ok(request) = serde_json::from_slice::<CreateTransferRequest>(&body) else {
        return error_reply(400, "invalid_transfer_terms");
    };
    let cumulative_shape = request.amount.is_none()
        && request
            .recipient_cumulative
            .is_some_and(|target| target >= 0)
        && request
            .payout_program
            .as_ref()
            .is_some_and(|program| !program.is_empty() && program.len() <= 100)
        && !headers.contains_key("idempotency-key");
    if org != sender()
        || org == request.recipient_organization_id
        || !matches!(
            request.source_credit_type.as_str(),
            "grant" | "payment" | "staking_farm"
        )
        || !cumulative_shape
        || request.memo.len() > 2000
    {
        return error_reply(400, "invalid_transfer_terms");
    }
    let mut guard = state.lock().unwrap();
    if guard.inactive.contains(&request.recipient_organization_id) {
        return error_reply(404, "organization_unavailable");
    }
    let program = request.payout_program.clone().unwrap();
    let total: i64 = guard
        .transfers
        .iter()
        .filter(|t| {
            t.recipient == request.recipient_organization_id
                && t.program.as_deref() == Some(&program)
        })
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
    let id = Uuid::new_v4();
    let created_at = Utc::now();
    guard.transfers.push(Transfer {
        id,
        recipient: request.recipient_organization_id,
        program: Some(program.clone()),
        amount,
        cumulative: Some(total + amount),
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
            "memo": request.memo,
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
    direction: Option<String>,
}

async fn history(
    State(state): State<Shared>,
    Path(org): Path<Uuid>,
    Query(query): Query<HistoryQuery>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    record(&state, method, &uri, &headers, None);
    if let Some(reply) = scripted(&state).await {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    if org != sender()
        || query.before_time.is_some() != query.before_id.is_some()
        || query.direction.as_deref() != Some("outgoing")
    {
        return error_reply(400, "invalid_cursor");
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
                "source_credit_type": "grant",
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
    // As #1271 does: the cursor is the last row's, returned on every page
    // that has a row, the last one included.
    let next = rows
        .last()
        .map(|t| json!({"before_time": t.created_at, "before_id": t.id}));
    axum::Json(json!({"transfers": items, "next_cursor": next})).into_response()
}

async fn capacity(
    State(state): State<Shared>,
    Path(_org): Path<Uuid>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    record(&state, method, &uri, &headers, None);
    if let Some(reply) = scripted(&state).await {
        return reply;
    }
    if !authorized(&headers) {
        return error_reply(403, "credential_refused");
    }
    let body = state.lock().unwrap().capacity.clone().unwrap_or_else(|| {
        json!({
            "credits": [],
            "sender_reconciled": true,
            "sending_enabled": true,
            "credential_can_transfer": true,
            "sends_remaining_in_window": 57,
            "sources": [
                {"source_credit_type": "payment", "max_amount_per_transfer": 11},
                {"source_credit_type": "staking_farm", "max_amount_per_transfer": 0},
                {"source_credit_type": "grant", "max_amount_per_transfer": 5_000_000_000_i64},
            ],
            "recipient_restrictions_apply": true,
            "is_reservation": false,
        })
    });
    axum::Json(body).into_response()
}

async fn redirect_target(State(state): State<Shared>) -> Response {
    state.lock().unwrap().redirect_target_hits += 1;
    (
        StatusCode::OK,
        axum::Json(json!({"amount": 0, "no_op": true})),
    )
        .into_response()
}

struct Mock {
    state: Shared,
    base: String,
}

impl Mock {
    async fn start() -> Self {
        let state: Shared = Arc::default();
        let app = Router::new()
            .route("/v1/organizations/{org}/credit/transfer", post(transfer))
            .route("/v1/organizations/{org}/credit/transfers", get(history))
            .route(
                "/v1/organizations/{org}/credit/transfer/capacity",
                get(capacity),
            )
            .route("/elsewhere", any(redirect_target))
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

    fn rail(&self) -> NearAiRail {
        self.rail_with_timeout(Duration::from_secs(5))
    }

    fn rail_with_timeout(&self, timeout: Duration) -> NearAiRail {
        NearAiRail::new(
            NearAiTransferClient::at_loopback(&self.base, credentials(), timeout).unwrap(),
        )
    }

    fn script(&self, reply: Scripted) {
        self.state.lock().unwrap().script.push_back(reply);
    }

    fn requests(&self) -> Vec<Recorded> {
        self.state.lock().unwrap().requests.clone()
    }

    fn transfers(&self) -> Vec<Transfer> {
        self.state.lock().unwrap().transfers.clone()
    }

    /// Seeds an outgoing transfer `minutes_ago`, as if sent earlier.
    fn seed(
        &self,
        recipient: Uuid,
        program: &str,
        amount: i64,
        cumulative: i64,
        minutes_ago: i64,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.state.lock().unwrap().transfers.push(Transfer {
            id,
            recipient,
            program: Some(program.to_string()),
            amount,
            cumulative: Some(cumulative),
            created_at: Utc::now() - ChronoDuration::minutes(minutes_ago),
        });
        id
    }
}

fn credentials() -> NearAiTransferCredentials {
    NearAiTransferCredentials {
        token: SecretString::from(TOKEN.to_string()),
        credential_id: Some(credential_id()),
        sender_org: sender(),
        source: NearAiCreditSource::Grant,
    }
}

fn recipient(org: Uuid) -> RecipientRef {
    RecipientRef::new(org.to_string()).unwrap()
}

fn units(value: u128) -> AtomicUnits {
    AtomicUnits::from_raw(value)
}

#[tokio::test]
async fn a_positive_delta_commits_and_hashes_the_transfer_id() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let to = recipient(recipient_org(1));
    let program = near_ai_program_for_link(Uuid::from_u128(1));
    let reference = line_ref(1);
    let line = RailLine::new(&reference, &to, &program, units(5_000), units(0)).unwrap();

    let outcome = rail.send(&line).await.unwrap();
    let transfers = mock.transfers();
    assert_eq!(transfers.len(), 1);
    assert_eq!(
        outcome,
        SendOutcome::Committed {
            receipt_hash: near_ai_receipt_hash(transfers[0].id),
            delta: units(5_000),
            total_after: Some(units(5_000)),
        }
    );
    // The hash is of the id and nothing else: domain-separated sha256.
    let expected = {
        use sha2::{Digest, Sha256};
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                format!("nearai-transfer-v1:{}", transfers[0].id).as_bytes()
            ))
        )
    };
    assert_eq!(near_ai_receipt_hash(transfers[0].id), expected);

    // The same line again has no second effect.
    let again = rail.send(&line).await.unwrap();
    assert_eq!(
        again,
        SendOutcome::AlreadyAtOrAbove {
            total: units(5_000)
        }
    );
    assert_eq!(mock.transfers().len(), 1);

    // A later line on the same program sends only the difference.
    let reference = line_ref(2);
    let next = RailLine::new(&reference, &to, &program, units(7_500), units(5_000)).unwrap();
    match rail.send(&next).await.unwrap() {
        SendOutcome::Committed {
            delta, total_after, ..
        } => {
            assert_eq!(delta, units(2_500));
            assert_eq!(total_after, Some(units(7_500)));
        }
        other => panic!("expected a commit, got {other:?}"),
    }
}

#[tokio::test]
async fn a_zero_delta_is_already_at_or_above() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(2);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(2));
    mock.seed(org, &program, 7_000, 7_000, 5);

    let reference = line_ref(3);
    let at = RailLine::new(&reference, &to, &program, units(7_000), units(7_000)).unwrap();
    assert_eq!(
        rail.send(&at).await.unwrap(),
        SendOutcome::AlreadyAtOrAbove {
            total: units(7_000)
        }
    );
    let below = RailLine::new(&reference, &to, &program, units(6_000), units(0)).unwrap();
    assert_eq!(
        rail.send(&below).await.unwrap(),
        SendOutcome::AlreadyAtOrAbove {
            total: units(7_000)
        }
    );
    assert_eq!(mock.transfers().len(), 1, "a no-op writes no transfer");
}

#[tokio::test]
async fn read_total_is_a_zero_cumulative_send() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(3);
    let program = near_ai_program_for_link(Uuid::from_u128(3));
    mock.seed(org, &program, 4_000, 4_000, 9);
    mock.seed(org, &program, 200, 4_200, 8);
    // Another program on the same recipient is not this program's total.
    mock.seed(org, "tc1-other", 999, 999, 7);

    let total = rail.read_total(&recipient(org), &program).await.unwrap();
    assert_eq!(total, units(4_200));

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(
        requests[0].path,
        format!("/v1/organizations/{}/credit/transfer", sender())
    );
    let body = requests[0].body.as_ref().unwrap();
    assert_eq!(body["recipient_cumulative"], json!(0));
    assert_eq!(body["payout_program"], json!(program));
    assert_eq!(mock.transfers().len(), 3, "a read writes nothing");

    // The liveness probe is the same zero send on the fixed probe program.
    rail.probe_recipient(&recipient(org)).await.unwrap();
    let requests = mock.requests();
    let probe = requests.last().unwrap().body.as_ref().unwrap();
    assert_eq!(probe["recipient_cumulative"], json!(0));
    assert_eq!(probe["payout_program"], json!(NEAR_AI_PROBE_PROGRAM));
    assert!(!NEAR_AI_PROBE_PROGRAM.starts_with("tc1-"));
    assert_eq!(mock.transfers().len(), 3);
}

#[tokio::test]
async fn find_receipt_pages_history_and_skips_claimed_receipts() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(4);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(4));

    // Oldest first: two records that match the line (one claimed by another
    // line already), then 158 that do not, so the unclaimed match is on the
    // second page of 100.
    let claimed = mock.seed(org, &program, 3_000, 3_000, 1_000);
    let unclaimed = mock.seed(org, &program, 3_000, 3_000, 999);
    for n in 0..158_u128 {
        let (other, other_program) = if n % 2 == 0 {
            (recipient_org(100 + n), program.clone())
        } else {
            (org, "tc1-not-this-program".to_string())
        };
        mock.seed(other, &other_program, 1, 3_000, 900 - n as i64);
    }

    let reference = line_ref(4);
    let line = RailLine::new(&reference, &to, &program, units(3_000), units(0)).unwrap();
    let found = rail
        .find_receipt(&line, &[near_ai_receipt_hash(claimed)])
        .await
        .unwrap();
    assert_eq!(
        found,
        ReceiptLookup::Found {
            receipt_hash: near_ai_receipt_hash(unclaimed),
            delta: units(3_000),
        }
    );
    let pages: Vec<Recorded> = mock
        .requests()
        .into_iter()
        .filter(|r| r.method == Method::GET)
        .collect();
    assert_eq!(pages.len(), 2, "stops on the short second page");
    for page in &pages {
        let query = page.query.as_deref().unwrap();
        assert!(query.contains("direction=outgoing"), "{query}");
        assert!(query.contains("limit=100"), "{query}");
    }
    assert!(pages[1].query.as_deref().unwrap().contains("before_id="));

    // With both matches claimed there is no receipt, and the walk still
    // stops on the short page.
    let none = rail
        .find_receipt(
            &line,
            &[
                near_ai_receipt_hash(claimed),
                near_ai_receipt_hash(unclaimed),
            ],
        )
        .await
        .unwrap();
    assert_eq!(none, ReceiptLookup::NotFound);

    // A different target does not match either record.
    let reference = line_ref(5);
    let other = RailLine::new(&reference, &to, &program, units(3_001), units(0)).unwrap();
    assert_eq!(
        rail.find_receipt(&other, &[]).await.unwrap(),
        ReceiptLookup::NotFound
    );

    // History lists this recipient's records on this program only.
    let history = rail.history(&to, &program).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|entry| entry.program == program));
}

/// One case per row of the spec's refusal table, plus the rows the table
/// leaves to the closed fallthrough.
#[tokio::test]
async fn each_reply_maps_to_its_refusal() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let to = recipient(recipient_org(5));
    let program = near_ai_program_for_link(Uuid::from_u128(5));
    let reference = line_ref(6);
    let line = RailLine::new(&reference, &to, &program, units(10), units(0)).unwrap();

    let cases: Vec<(Scripted, RailRefusal)> = vec![
        (
            Scripted::Error(429, "transfer_rate_limited"),
            RailRefusal::Retry("near_ai_transfer_rate_limited"),
        ),
        (
            Scripted::Error(503, "transfer_unavailable"),
            RailRefusal::Retry("near_ai_transfer_unavailable"),
        ),
        (
            Scripted::Error(503, "accounting_busy"),
            RailRefusal::Retry("near_ai_accounting_busy"),
        ),
        (
            Scripted::Error(502, "bad_gateway"),
            RailRefusal::Retry("near_ai_transfer_unavailable"),
        ),
        (
            Scripted::Error(409, "insufficient_posted_capacity"),
            RailRefusal::StopPass("near_ai_insufficient_posted_capacity"),
        ),
        (
            Scripted::Error(409, "sender_not_reconciled"),
            RailRefusal::StopPass("near_ai_sender_not_reconciled"),
        ),
        // A budget or policy limit on our own credential or org is the
        // sender's, never the recipient's: stop the rail, hold nobody.
        (
            Scripted::Error(409, "credential_transfer_limit_exceeded"),
            RailRefusal::StopPass("near_ai_credential_transfer_limit_exceeded"),
        ),
        (
            Scripted::Error(409, "organization_transfer_limit_exceeded"),
            RailRefusal::StopPass("near_ai_organization_transfer_limit_exceeded"),
        ),
        (
            Scripted::Error(409, "capacity_overflow"),
            RailRefusal::HoldLine("near_ai_capacity_overflow"),
        ),
        (
            Scripted::Error(404, "organization_unavailable"),
            RailRefusal::RecipientUnavailable,
        ),
        (
            Scripted::Error(403, "credential_refused"),
            RailRefusal::StopPass("near_ai_credential_refused"),
        ),
        (
            Scripted::Error(403, "sender_not_enabled"),
            RailRefusal::StopPass("near_ai_sender_not_enabled"),
        ),
        (
            Scripted::Error(403, "credential_management_disabled"),
            RailRefusal::StopPass("near_ai_credential_refused"),
        ),
        (
            Scripted::Error(401, "unauthorized"),
            RailRefusal::StopPass("near_ai_credential_refused"),
        ),
        (
            Scripted::Error(400, "invalid_transfer_terms"),
            RailRefusal::Failed("near_ai_request_refused"),
        ),
        (
            Scripted::Error(400, "something_new"),
            RailRefusal::Failed("near_ai_request_refused"),
        ),
        // Not in the table: closed fallthrough, never a guess.
        (
            Scripted::Error(409, "idempotency_conflict"),
            RailRefusal::Failed("near_ai_reply_unexpected"),
        ),
        (
            Scripted::Error(418, "teapot"),
            RailRefusal::Failed("near_ai_reply_unexpected"),
        ),
        // A 2xx on a send may have moved money, whatever its shape: the
        // line stays pending and the resend or recovery resolves it.
        (
            Scripted::Raw(200, json!({"amount": 0, "recipient_cumulative": 3})),
            RailRefusal::Retry("near_ai_reply_unexpected"),
        ),
        (
            Scripted::Raw(201, json!({"id": "not-a-uuid", "amount": 10})),
            RailRefusal::Retry("near_ai_reply_unreadable"),
        ),
        (
            Scripted::Raw(
                201,
                json!({"id": Uuid::from_u128(9), "amount": -1, "recipient_cumulative": 10}),
            ),
            RailRefusal::Retry("near_ai_reply_unexpected"),
        ),
        (
            Scripted::Raw(201, json!({"id": Uuid::from_u128(9), "amount": 10})),
            RailRefusal::Retry("near_ai_reply_unexpected"),
        ),
        // Only `organization_unavailable` names the recipient. Any other
        // 404 (a wrong route, a proxy, a host without #1271) is ours, and
        // must not mark a link.
        (
            Scripted::Error(404, "not_found"),
            RailRefusal::StopPass("near_ai_route_unavailable"),
        ),
        (
            Scripted::Error(404, "credential_not_found"),
            RailRefusal::StopPass("near_ai_route_unavailable"),
        ),
        (
            Scripted::Text(404, ""),
            RailRefusal::StopPass("near_ai_route_unavailable"),
        ),
        (
            Scripted::Text(404, "<html><body>Not Found</body></html>"),
            RailRefusal::StopPass("near_ai_route_unavailable"),
        ),
    ];
    for (reply, expected) in cases {
        let shown = format!("{reply:?}");
        mock.script(reply);
        let refusal = rail.send(&line).await.unwrap_err();
        assert_eq!(refusal, expected, "{shown}");
        assert!(refusal.label_is_safe(), "{shown}");
    }

    // A 404 on our own org's reads is our fault, not a recipient's.
    mock.script(Scripted::Error(404, "organization_unavailable"));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_sender_unavailable")
    );
    mock.script(Scripted::Error(404, "organization_unavailable"));
    assert_eq!(
        rail.find_receipt(&line, &[]).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_sender_unavailable")
    );
    // Any other 404 on a read is a route fault, not a missing sender.
    mock.script(Scripted::Text(404, ""));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_route_unavailable")
    );
    // A read has no line to fail: a reply we cannot place stops the pass.
    mock.script(Scripted::Error(418, "teapot"));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    mock.script(Scripted::Raw(202, json!({})));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    mock.script(Scripted::Error(409, "idempotency_conflict"));
    assert_eq!(
        rail.history(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    mock.script(Scripted::Raw(
        200,
        json!({"transfers": [{
            "id": Uuid::from_u128(11),
            "sender_organization_id": sender(),
            "recipient_organization_id": recipient_org(5),
            "payout_program": program,
            "amount": -1,
            "recipient_cumulative": 10,
        }], "next_cursor": null}),
    ));
    assert_eq!(
        rail.find_receipt(&line, &[]).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    // So does a sender limit on the zero read, which is a send.
    for code in [
        "credential_transfer_limit_exceeded",
        "organization_transfer_limit_exceeded",
    ] {
        mock.script(Scripted::Error(409, code));
        assert!(matches!(
            rail.read_total(&to, &program).await.unwrap_err(),
            RailRefusal::StopPass(_)
        ));
    }
    // A credential fault on a read stops the pass too.
    mock.script(Scripted::Error(403, "credential_refused"));
    assert_eq!(
        rail.read_total(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_credential_refused")
    );
    mock.script(Scripted::Error(401, "unauthorized"));
    assert_eq!(
        rail.history(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_credential_refused")
    );
    // The probe: a closed recipient list refuses as not permitted.
    mock.script(Scripted::Error(409, "credential_transfer_limit_exceeded"));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::HoldLine("near_ai_org_not_permitted")
    );
    mock.script(Scripted::Error(404, "organization_unavailable"));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::RecipientUnavailable
    );
    // At a zero target #1271 checks no budget, so the org-level code means
    // our org's policy refused the source or the recipient: ours to fix.
    mock.script(Scripted::Error(409, "organization_transfer_limit_exceeded"));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_organization_transfer_limit_exceeded")
    );
    // A 404 that does not name the org is not the contributor's fault.
    mock.script(Scripted::Text(404, ""));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_route_unavailable")
    );
    // A probe reply we cannot place, or a send at a zero target, says our
    // model of #1271 is wrong: stop, and blame no one's org.
    mock.script(Scripted::Raw(
        200,
        json!({"amount": 0, "recipient_cumulative": 3}),
    ));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    mock.script(Scripted::Raw(
        201,
        json!({"id": Uuid::from_u128(9), "amount": 1, "recipient_cumulative": 1}),
    ));
    assert_eq!(
        rail.probe_recipient(&to).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    mock.script(Scripted::Raw(
        201,
        json!({"id": Uuid::from_u128(9), "amount": 1, "recipient_cumulative": 1}),
    ));
    assert_eq!(
        rail.read_total(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );

    // A recipient that is not an org id never reaches the wire.
    let before = mock.requests().len();
    let garbled = RecipientRef::new("not-an-org").unwrap();
    let reference = line_ref(7);
    let bad = RailLine::new(&reference, &garbled, &program, units(10), units(0)).unwrap();
    assert_eq!(
        rail.send(&bad).await.unwrap_err(),
        RailRefusal::HoldLine("payout_link_unreadable")
    );
    assert_eq!(mock.requests().len(), before);

    // A timeout is a retry, and so is an endpoint that does not answer.
    let slow = mock.rail_with_timeout(Duration::from_millis(200));
    mock.script(Scripted::Delay(Duration::from_secs(2)));
    assert_eq!(
        slow.send(&line).await.unwrap_err(),
        RailRefusal::Retry("near_ai_timeout")
    );
    // Port 1 is below every ephemeral range, so a server another test binds
    // to port 0 can never be handed it, as a port bound and released could
    // be (#1759), and the refusal is immediate on every platform.
    let closed = "http://127.0.0.1:1/v1".to_string();
    let unreachable = NearAiRail::new(
        NearAiTransferClient::at_loopback(&closed, credentials(), Duration::from_secs(2)).unwrap(),
    );
    assert_eq!(
        unreachable.send(&line).await.unwrap_err(),
        RailRefusal::Retry("near_ai_unreachable")
    );
    assert!(mock.transfers().is_empty(), "no case moved money");
}

#[tokio::test]
async fn near_ai_target_above_i64_holds() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let to = recipient(recipient_org(6));
    let program = near_ai_program_for_link(Uuid::from_u128(6));
    let reference = line_ref(8);
    let above = i64::MAX as u128 + 1;
    let line = RailLine::new(&reference, &to, &program, units(above), units(0)).unwrap();
    assert_eq!(
        rail.send(&line).await.unwrap_err(),
        RailRefusal::HoldLine("payout_amount_out_of_range")
    );
    let huge = RailLine::new(&reference, &to, &program, units(10_u128.pow(30)), units(0)).unwrap();
    assert_eq!(
        rail.send(&huge).await.unwrap_err(),
        RailRefusal::HoldLine("payout_amount_out_of_range")
    );
    assert!(mock.requests().is_empty(), "nothing went on the wire");

    // i64::MAX itself is carried.
    let at = RailLine::new(&reference, &to, &program, units(i64::MAX as u128), units(0)).unwrap();
    assert!(rail.send(&at).await.is_ok());
    assert_eq!(rail.descriptor().wire_max, Some(units(i64::MAX as u128)));
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let to = recipient(recipient_org(7));
    let program = near_ai_program_for_link(Uuid::from_u128(7));
    let reference = line_ref(9);
    let line = RailLine::new(&reference, &to, &program, units(10), units(0)).unwrap();

    mock.script(Scripted::Redirect("/elsewhere"));
    assert_eq!(
        rail.send(&line).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_redirect_refused")
    );
    mock.script(Scripted::Redirect("/elsewhere"));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_redirect_refused")
    );
    mock.script(Scripted::Redirect("/elsewhere"));
    assert_eq!(
        rail.history(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_redirect_refused")
    );
    assert_eq!(mock.state.lock().unwrap().redirect_target_hits, 0);
    assert!(mock.transfers().is_empty());
}

#[tokio::test]
async fn the_body_never_carries_a_memo_and_metadata_is_one_hash() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(8);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(8));
    let reference = line_ref(10);
    let line = RailLine::new(&reference, &to, &program, units(42), units(0)).unwrap();
    rail.send(&line).await.unwrap();

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    assert!(
        !requests[0].idempotency_key,
        "cumulative mode forbids an Idempotency-Key"
    );
    let body = requests[0].body.as_ref().unwrap().as_object().unwrap();
    let mut keys: Vec<&str> = body.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "metadata",
            "payout_program",
            "recipient_cumulative",
            "recipient_organization_id",
            "source_credit_type",
        ]
    );
    assert_eq!(body["recipient_organization_id"], json!(org));
    assert_eq!(body["source_credit_type"], json!("grant"));
    assert_eq!(body["recipient_cumulative"], json!(42));
    assert_eq!(body["payout_program"], json!(program));
    let metadata = body["metadata"].as_object().unwrap();
    assert_eq!(metadata.len(), 1);
    let tag = near_ai_line_metadata_tag(&reference);
    assert_eq!(metadata["tc"], json!(tag));
    // The tag is a domain-separated hash of the line reference, not the
    // reference itself.
    let expected = {
        use sha2::{Digest, Sha256};
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                format!("nearai-payout-line-v1:{reference}").as_bytes()
            ))
        )
    };
    assert_eq!(tag, expected);
    assert_ne!(tag, reference);
}

/// A buffer the test's tracing subscriber writes into.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The process-wide log capture. A scoped (`set_default`) subscriber races
/// with the other tests in this binary over tracing's per-callsite interest
/// cache and can capture nothing, so the capture is the global subscriber,
/// installed once. It then also holds every other test's logs, which only
/// makes the absence checks below stronger.
fn global_logs() -> LogBuffer {
    static LOGS: std::sync::OnceLock<LogBuffer> = std::sync::OnceLock::new();
    LOGS.get_or_init(|| {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buffer.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("no other test in this binary installs a global subscriber");
        buffer
    })
    .clone()
}

#[tokio::test]
async fn logs_carry_no_org_id_token_or_rate() {
    let buffer = global_logs();

    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(9);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(9));
    let reference = line_ref(11);
    // A synthetic amount that stands in for a rate-derived figure: amounts
    // may be logged, so it is not on the forbidden list; the rate is never
    // known to the rail at all.
    let line = RailLine::new(&reference, &to, &program, units(31_337), units(0)).unwrap();
    rail.preflight().await.unwrap();
    rail.send(&line).await.unwrap();
    rail.read_total(&to, &program).await.unwrap();
    rail.find_receipt(&line, &[]).await.unwrap();
    mock.script(Scripted::Error(403, "credential_refused"));
    rail.send(&line).await.unwrap_err();
    mock.script(Scripted::Error(404, "organization_unavailable"));
    rail.send(&line).await.unwrap_err();

    let logs = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert!(!logs.is_empty(), "the subscriber captured nothing");
    // Captured, and carrying the credential id's hash.
    let credential_hash = NearAiTransferClient::credential_label(Some(credential_id()));
    assert!(logs.contains(&credential_hash), "{logs}");
    assert!(logs.contains("near_ai"), "{logs}");
    let transfer_id = mock.transfers()[0].id.to_string();
    for forbidden in [
        TOKEN,
        &TOKEN[3..],
        &sender().to_string(),
        &sender().simple().to_string(),
        &org.to_string(),
        &org.simple().to_string(),
        &credential_id().to_string(),
        &transfer_id,
        &program,
        "Bearer",
    ] {
        assert!(
            !logs.contains(forbidden),
            "a log line carries {forbidden:?}:\n{logs}"
        );
    }
}

/// `history` is the reconciliation read; its entries carry the receipt hash
/// and the program total after each record.
#[tokio::test]
async fn history_reports_each_record_with_its_receipt_and_total() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(10);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(10));
    let first = mock.seed(org, &program, 100, 100, 3);
    let second = mock.seed(org, &program, 50, 150, 2);
    let mut history = rail.history(&to, &program).await.unwrap();
    history.sort_by_key(|entry| entry.total_after);
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].receipt_hash, near_ai_receipt_hash(first));
    assert_eq!(history[0].delta, units(100));
    assert_eq!(history[0].total_after, Some(units(100)));
    assert_eq!(history[1].receipt_hash, near_ai_receipt_hash(second));
    assert_eq!(history[1].total_after, Some(units(150)));

    // preflight reads the configured source's per-send maximum.
    let capacity = rail.preflight().await.unwrap();
    assert!(capacity.sending_enabled);
    assert_eq!(capacity.per_send_max, units(5_000_000_000));
    assert_eq!(capacity.sends_remaining, Some(57));
    for (field, label) in [
        ("sending_enabled", "near_ai_sending_disabled"),
        ("sender_reconciled", "near_ai_sender_not_reconciled"),
        (
            "credential_can_transfer",
            "near_ai_credential_cannot_transfer",
        ),
    ] {
        let mut body = json!({
            "sender_reconciled": true,
            "sending_enabled": true,
            "credential_can_transfer": true,
            "sends_remaining_in_window": 3,
            "sources": [{"source_credit_type": "grant", "max_amount_per_transfer": 0}],
        });
        body[field] = json!(false);
        mock.state.lock().unwrap().capacity = Some(body);
        let capacity = rail.preflight().await.unwrap();
        assert!(!capacity.sending_enabled, "{field}");
        assert_eq!(capacity.stop_label, Some(label));
    }
}

/// #1271's capacity lists every source, a disallowed one at 0, so a reply
/// without the configured source is not #1271: the pass stops. And when no
/// sends are left in the minute, every source's maximum reads 0; the rail
/// reports `sends_remaining: Some(0)` and the pass waits on that, never on
/// the zero maximum.
#[tokio::test]
async fn capacity_quirks_are_settled_inside_the_rail() {
    let mock = Mock::start().await;
    let rail = mock.rail();

    mock.state.lock().unwrap().capacity = Some(json!({
        "sender_reconciled": true,
        "sending_enabled": true,
        "credential_can_transfer": true,
        "sends_remaining_in_window": 3,
        "sources": [{"source_credit_type": "payment", "max_amount_per_transfer": 9}],
    }));
    assert_eq!(
        rail.preflight().await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );

    mock.state.lock().unwrap().capacity = Some(json!({
        "sender_reconciled": true,
        "sending_enabled": true,
        "credential_can_transfer": true,
        "sends_remaining_in_window": 0,
        "sources": [
            {"source_credit_type": "payment", "max_amount_per_transfer": 0},
            {"source_credit_type": "staking_farm", "max_amount_per_transfer": 0},
            {"source_credit_type": "grant", "max_amount_per_transfer": 0},
        ],
    }));
    let capacity = rail.preflight().await.unwrap();
    assert!(capacity.sending_enabled);
    assert_eq!(capacity.stop_label, None);
    assert_eq!(capacity.sends_remaining, Some(0));
}

fn history_item(id: Uuid, sender: Uuid, recipient: Uuid, program: &str) -> Value {
    json!({
        "id": id,
        "sender_organization_id": sender,
        "recipient_organization_id": recipient,
        "payout_program": program,
        "amount": 1,
        "recipient_cumulative": 1,
    })
}

/// `direction=outgoing` returns only our sends. A record from another
/// sender is not #1271; dropping it would make a full page look short and
/// end the walk early, so the walk stops instead.
#[tokio::test]
async fn a_foreign_sender_in_outgoing_history_stops_the_walk() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(8);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(8));
    let mut items: Vec<Value> = (0..99_u128)
        .map(|n| {
            history_item(
                Uuid::from_u128(1_000 + n),
                sender(),
                recipient_org(200),
                "tc1-x",
            )
        })
        .collect();
    items.push(history_item(
        Uuid::from_u128(999),
        recipient_org(300),
        org,
        &program,
    ));
    mock.script(Scripted::Raw(
        200,
        json!({"transfers": items, "next_cursor": {
            "before_time": Utc::now(), "before_id": Uuid::from_u128(999)}}),
    ));
    let reference = line_ref(9);
    let line = RailLine::new(&reference, &to, &program, units(1), units(0)).unwrap();
    assert_eq!(
        rail.find_receipt(&line, &[]).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_reply_unexpected")
    );
    assert_eq!(mock.requests().len(), 1);
}

/// A cursor that does not move strictly older stops the walk, rather than
/// asking for the same page until the page cap.
#[tokio::test]
async fn a_stalled_history_cursor_stops_the_walk() {
    let mock = Mock::start().await;
    let rail = mock.rail();
    let org = recipient_org(9);
    let to = recipient(org);
    let program = near_ai_program_for_link(Uuid::from_u128(9));
    let at = Utc::now();
    let page = |cursor_id: u128| {
        let items: Vec<Value> = (0..100_u128)
            .map(|n| {
                history_item(
                    Uuid::from_u128(5_000 + n),
                    sender(),
                    recipient_org(200),
                    "tc1-x",
                )
            })
            .collect();
        json!({"transfers": items, "next_cursor": {
            "before_time": at, "before_id": Uuid::from_u128(cursor_id)}})
    };
    // The same cursor twice.
    mock.script(Scripted::Raw(200, page(77)));
    mock.script(Scripted::Raw(200, page(77)));
    assert_eq!(
        rail.history(&to, &program).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_history_cursor_stalled")
    );
    assert_eq!(mock.requests().len(), 2);

    // A cursor that moves newer is no better.
    mock.script(Scripted::Raw(200, page(77)));
    mock.script(Scripted::Raw(200, page(78)));
    let reference = line_ref(10);
    let line = RailLine::new(&reference, &to, &program, units(1), units(0)).unwrap();
    assert_eq!(
        rail.find_receipt(&line, &[]).await.unwrap_err(),
        RailRefusal::StopPass("near_ai_history_cursor_stalled")
    );
    assert_eq!(mock.requests().len(), 4);
}
