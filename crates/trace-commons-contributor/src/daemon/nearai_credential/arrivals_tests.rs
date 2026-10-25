//! The arrival rules, without a network. Every figure is synthetic.

use super::*;

const DOLLAR: i64 = 1_000_000_000;

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn transfer(id: &str, amount: i64, when: &str) -> IncomingTransfer {
    IncomingTransfer {
        id: id.to_string(),
        amount,
        created_at: at(when),
    }
}

// ---- the notification rule ----

#[test]
fn the_first_credit_ever_received_always_notifies() {
    // However small: the first arrival is news on its own.
    assert_eq!(credit_candidacy(DOLLAR / 10, None), Some(DOLLAR / 10));
    assert_eq!(credit_candidacy(40 * DOLLAR, None), Some(40 * DOLLAR));
}

#[test]
fn nothing_received_is_never_a_notification() {
    assert_eq!(credit_candidacy(0, None), None);
    assert_eq!(credit_candidacy(5 * DOLLAR, Some(5 * DOLLAR)), None);
}

#[test]
fn below_25_percent_does_not_notify() {
    // $10 announced; $2.49 more is 24.9%.
    assert_eq!(
        credit_candidacy(10 * DOLLAR + 2_490_000_000, Some(10 * DOLLAR)),
        None
    );
}

#[test]
fn below_one_dollar_does_not_notify_even_past_25_percent() {
    // $2 announced; $0.90 more is 45% but under the dollar floor.
    assert_eq!(
        credit_candidacy(2 * DOLLAR + 900_000_000, Some(2 * DOLLAR)),
        None
    );
}

#[test]
fn crossing_both_notifies_with_the_amount_since_the_last_notification() {
    // $10 announced; $2.50 more is exactly 25% and over $1.
    assert_eq!(
        credit_candidacy(12 * DOLLAR + 500_000_000, Some(10 * DOLLAR)),
        Some(2 * DOLLAR + 500_000_000)
    );
    // $4 announced; $1 more is 25% and exactly the floor.
    assert_eq!(credit_candidacy(5 * DOLLAR, Some(4 * DOLLAR)), Some(DOLLAR));
}

#[test]
fn a_notification_resets_the_baseline() {
    let mut record = ArrivalsRecord {
        scanned: true,
        received_total_nanos: 10 * DOLLAR,
        notified_received_total_nanos: Some(8 * DOLLAR),
        ..Default::default()
    };
    // $2 on $8 is 25%: announce.
    assert_eq!(record.candidacy(), Some(2 * DOLLAR));
    // Announced: the baseline is now $10, and $2 more is only 20% of it.
    record.notified_received_total_nanos = Some(record.received_total_nanos);
    assert_eq!(record.candidacy(), None);
    record.received_total_nanos += 2 * DOLLAR;
    assert_eq!(record.candidacy(), None);
    record.received_total_nanos += DOLLAR / 2;
    assert_eq!(record.candidacy(), Some(2 * DOLLAR + DOLLAR / 2));
}

#[test]
fn nothing_is_announced_before_one_whole_read_of_the_ledger() {
    let record = ArrivalsRecord {
        scanned: false,
        received_total_nanos: 10 * DOLLAR,
        ..Default::default()
    };
    assert_eq!(record.candidacy(), None);
}

#[test]
fn an_overflowing_total_is_not_a_panic() {
    assert_eq!(credit_candidacy(i64::MAX, Some(i64::MAX - 1)), None);
    assert!(credit_candidacy(i64::MAX, Some(1)).is_some());
}

// ---- counting transfers ----

#[test]
fn a_transfer_seen_twice_is_counted_once() {
    let mut record = ArrivalsRecord::default();
    let page = [
        transfer("t2", 3 * DOLLAR, "2026-10-20T10:00:00Z"),
        transfer("t1", 2 * DOLLAR, "2026-10-19T10:00:00Z"),
    ];
    assert_eq!(record.apply_transfers(&page), 5 * DOLLAR);
    assert!(record.scanned);
    // The next poll reads back over the same rows, plus one new one.
    let next = [
        transfer("t3", DOLLAR, "2026-10-21T10:00:00Z"),
        transfer("t2", 3 * DOLLAR, "2026-10-20T10:00:00Z"),
        transfer("t1", 2 * DOLLAR, "2026-10-19T10:00:00Z"),
    ];
    assert_eq!(record.apply_transfers(&next), DOLLAR);
    assert_eq!(record.received_total_nanos, 6 * DOLLAR);
    // And a page with the same id twice counts it once.
    let doubled = [
        transfer("t4", DOLLAR, "2026-10-21T11:00:00Z"),
        transfer("t4", DOLLAR, "2026-10-21T11:00:00Z"),
    ];
    assert_eq!(record.apply_transfers(&doubled), DOLLAR);
    assert_eq!(record.received_total_nanos, 7 * DOLLAR);
}

#[test]
fn a_late_transfer_inside_the_lookback_is_still_counted() {
    let mut record = ArrivalsRecord::default();
    record.apply_transfers(&[transfer("t2", DOLLAR, "2026-10-20T10:00:00Z")]);
    // Committed after t2 was read, stamped an hour before it.
    let late = [
        transfer("t2", DOLLAR, "2026-10-20T10:00:00Z"),
        transfer("t1", 4 * DOLLAR, "2026-10-20T09:00:00Z"),
    ];
    assert_eq!(record.apply_transfers(&late), 4 * DOLLAR);
}

#[test]
fn a_transfer_older_than_the_lookback_was_already_read() {
    let mut record = ArrivalsRecord::default();
    record.apply_transfers(&[transfer("t9", DOLLAR, "2026-10-20T10:00:00Z")]);
    assert_eq!(record.scan_cutoff(), Some(at("2026-10-18T10:00:00Z")));
    // Its id was pruned long ago; its time says an earlier poll read it.
    let old = [transfer("t0", 7 * DOLLAR, "2026-10-10T10:00:00Z")];
    assert_eq!(record.apply_transfers(&old), 0);
    assert_eq!(record.received_total_nanos, DOLLAR);
}

#[test]
fn spending_cannot_trigger_a_notification() {
    // Spending is not in the transfer ledger, and the balance it moves is
    // not what the rule reads: samples falling or rising leave it alone.
    let mut record = ArrivalsRecord::default();
    record.apply_transfers(&[transfer("t1", 10 * DOLLAR, "2026-10-20T10:00:00Z")]);
    record.notified_received_total_nanos = Some(record.received_total_nanos);
    record.record_sample(at("2026-10-20T10:00:00Z"), Some(10 * DOLLAR));
    record.record_sample(at("2026-10-21T10:00:00Z"), Some(2 * DOLLAR));
    record.record_sample(at("2026-10-22T10:00:00Z"), Some(30 * DOLLAR));
    record.apply_transfers(&[]);
    assert_eq!(record.candidacy(), None);
}

#[test]
fn the_seen_list_stays_bounded() {
    let mut record = ArrivalsRecord::default();
    let start = at("2026-10-01T00:00:00Z");
    for day in 0..20 {
        let rows: Vec<_> = (0..200)
            .map(|i| IncomingTransfer {
                id: format!("d{day}-{i}"),
                amount: 1,
                created_at: start + Duration::days(day) + Duration::seconds(i),
            })
            .collect();
        record.apply_transfers(&rows);
    }
    assert_eq!(record.received_total_nanos, 20 * 200);
    assert!(record.seen.len() <= MAX_SEEN, "{}", record.seen.len());
    // Only what the next poll can read back over is kept.
    let cutoff = record.scan_cutoff().unwrap();
    assert!(
        record
            .seen
            .iter()
            .all(|s| s.at >= cutoff - Duration::days(1))
    );
}

// ---- incoming credit in the rolling 24-hour window ----

#[test]
fn received_24h_counts_unique_positive_transfers_in_the_window_not_balance() {
    let now = at("2026-10-24T12:00:00Z");
    let rows = [
        transfer("recent", 2 * DOLLAR, "2026-10-24T11:00:00Z"),
        transfer("recent", 2 * DOLLAR, "2026-10-24T11:00:00Z"),
        transfer("boundary", DOLLAR, "2026-10-23T12:00:00Z"),
        transfer("inside", 3 * DOLLAR, "2026-10-23T12:01:00Z"),
        transfer("future", 9 * DOLLAR, "2026-10-24T12:01:00Z"),
        transfer("negative", -DOLLAR, "2026-10-24T11:00:00Z"),
    ];
    let mut record = ArrivalsRecord::default();
    record.record_received_window(&rows, now);
    record.record_sample(now - CHANGE_WINDOW, Some(100 * DOLLAR));
    record.record_sample(now, Some(DOLLAR));
    assert_eq!(record.received_24h(now), Some(5 * DOLLAR));
    assert_eq!(
        record.received_24h(now + Duration::minutes(2)),
        Some(2 * DOLLAR)
    );
    assert_eq!(record.received_24h(now - Duration::seconds(1)), None);
    assert_eq!(
        record.received_24h(now + POLL_INTERVAL + Duration::seconds(1)),
        None
    );
}

#[test]
fn received_24h_legacy_is_unknown_until_complete_scan_and_empty_is_known_zero() {
    let now = at("2026-10-24T12:00:00Z");
    let mut record: ArrivalsRecord = serde_json::from_value(serde_json::json!({
        "scanned":true,"received_total_nanos":9_000_000_000i64,
        "seen":[{"id":"old","at":"2026-10-24T11:00:00Z"}]
    }))
    .unwrap();
    assert_eq!(record.received_24h(now), None);
    record.record_received_window(&[transfer("old", DOLLAR, "2026-10-24T11:00:00Z")], now);
    assert_eq!(record.received_24h(now), Some(DOLLAR));
    assert_eq!(record.received_total_nanos, 9 * DOLLAR);
    record.record_received_window(&[], now);
    assert_eq!(record.received_24h(now), Some(0));
    record.adopt("another-org");
    assert_eq!(record.received_24h(now), None);
}

#[test]
fn received_24h_overflow_or_unbounded_window_is_unknown() {
    let now = at("2026-10-24T12:00:00Z");
    let mut record = ArrivalsRecord::default();
    record.record_received_window(
        &[
            transfer("one", i64::MAX, "2026-10-24T11:00:00Z"),
            transfer("two", 1, "2026-10-24T11:00:00Z"),
        ],
        now,
    );
    assert_eq!(record.received_24h(now), None);
    let rows: Vec<_> = (0..=MAX_SEEN)
        .map(|i| IncomingTransfer {
            id: format!("transfer-{i}"),
            amount: 1,
            created_at: now,
        })
        .collect();
    record.record_received_window(&rows, now);
    assert_eq!(record.received_24h(now), None);
}

// ---- the 24-hour change ----

#[test]
fn the_change_is_up_after_credit_arrived() {
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-22T09:00:00Z"), Some(5 * DOLLAR));
    record.record_sample(at("2026-10-22T20:00:00Z"), Some(6 * DOLLAR));
    let now = at("2026-10-23T10:00:00Z");
    assert_eq!(
        record.change_24h(now, Some(8 * DOLLAR + 200_000_000)),
        Some(3 * DOLLAR + 200_000_000)
    );
}

#[test]
fn the_change_is_down_after_spending() {
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-22T10:00:00Z"), Some(5 * DOLLAR));
    let now = at("2026-10-23T10:00:00Z");
    assert_eq!(record.change_24h(now, Some(4 * DOLLAR)), Some(-DOLLAR));
}

#[test]
fn no_reading_from_a_day_ago_is_no_change_rather_than_zero() {
    let now = at("2026-10-23T10:00:00Z");
    // No samples at all.
    assert_eq!(
        ArrivalsRecord::default().change_24h(now, Some(DOLLAR)),
        None
    );
    // Only recent samples: the daemon started this morning.
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-23T08:00:00Z"), Some(DOLLAR));
    assert_eq!(record.change_24h(now, Some(DOLLAR)), None);
    // Only a sample from too long ago: the Mac was off for two days.
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-22T03:00:00Z"), Some(DOLLAR));
    assert_eq!(record.change_24h(now, Some(DOLLAR)), None);
    // A known past, an unknown present (no limit set now).
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-22T10:00:00Z"), Some(DOLLAR));
    assert_eq!(record.change_24h(now, None), None);
}

#[test]
fn a_sample_with_no_figure_is_not_kept() {
    let mut record = ArrivalsRecord::default();
    record.record_sample(at("2026-10-22T10:00:00Z"), None);
    assert!(record.samples.is_empty());
}

#[test]
fn samples_are_spaced_and_kept_for_two_days() {
    let mut record = ArrivalsRecord::default();
    let start = at("2026-10-20T00:00:00Z");
    // A read every minute for three days.
    for minute in 0..(3 * 24 * 60) {
        record.record_sample(start + Duration::minutes(minute), Some(minute));
    }
    let newest = record.samples.last().unwrap().at;
    assert!(
        record.samples.len() <= 48 * 6 + 1,
        "{}",
        record.samples.len()
    );
    assert!(
        record
            .samples
            .iter()
            .all(|s| s.at >= newest - SAMPLE_RETENTION)
    );
    // Still spread across the window, so a day-ago sample exists.
    let now = newest;
    assert!(record.change_24h(now, Some(0)).is_some());
}

#[test]
fn another_organization_starts_a_new_record() {
    let mut record = ArrivalsRecord::default();
    record.adopt("org-a");
    record.apply_transfers(&[transfer("t1", DOLLAR, "2026-10-20T10:00:00Z")]);
    record.record_sample(at("2026-10-20T10:00:00Z"), Some(DOLLAR));
    record.notified_received_total_nanos = Some(DOLLAR);
    record.adopt("org-a");
    assert_eq!(record.received_total_nanos, DOLLAR);
    record.adopt("org-b");
    assert_eq!(
        record,
        ArrivalsRecord {
            organization_id: Some("org-b".into()),
            ..Default::default()
        }
    );
}

// ---- against a stub of the service ----

mod live {
    use super::*;
    use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};
    use axum::{
        Json, Router,
        extract::{Path, Query, Request},
        http::StatusCode,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};

    const ORG: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";

    fn shared() -> DaemonShared {
        let (dir, store) = crate::config::tests_support::temp_store();
        std::mem::forget(dir);
        let s = DaemonShared::load(store).unwrap();
        {
            let mut settings = s.settings.lock().unwrap();
            settings.near_ai_session = Some(NearAiSession {
                refresh_token: "rt_first".into(),
                refresh_token_expires_at: None,
                stored_at: Utc::now(),
                user_agent: "Mozilla/5.0 Test".into(),
            });
            settings.near_ai_inference = Some(NearAiInferenceCredential {
                key: "sk-synthetic".into(),
                key_id: "key-1".into(),
                key_prefix: "sk-syn".into(),
                organization_id: ORG.into(),
                workspace_id: "ws-1".into(),
                minted_at: Utc::now(),
            });
            settings.save_for_test(&s.store).unwrap();
        }
        s
    }

    async fn link_recipient(s: &DaemonShared) -> crate::payout_link::test_support::Server {
        use trace_commons_protocol::payout_link::near_ai_recipient_ref;
        let ingest = crate::payout_link::test_support::spawn().await;
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest.base.clone();
        s.store.save_config(&cfg).unwrap();
        let account = crate::account_auth::AccountSession {
            access_token: "tcn1_dGVuYW50.account".into(),
            expires_at: Utc::now() + Duration::hours(6),
            account_id: "acct".into(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&account).unwrap(),
            )
            .unwrap();
        let key = uuid::Uuid::new_v4();
        let org = uuid::Uuid::parse_str(ORG).unwrap();
        s.store
            .write_daemon_file(
                crate::config::DAEMON_PAYOUT_LINK_FILE,
                &serde_json::to_vec(&serde_json::json!({
                    "rails": {"near_ai": {"idempotency_key": key, "organization_id": org}}
                }))
                .unwrap(),
            )
            .unwrap();
        {
            let mut status = ingest.stub.lock().unwrap();
            status.linked = true;
            status.version = 1;
            status.recipient_ref = Some(near_ai_recipient_ref(key, org));
        }
        ingest
    }

    /// What the stub serves and saw.
    #[derive(Clone, Default)]
    struct Stub {
        /// Every incoming transfer, newest first, as the ledger holds them.
        ledger: Arc<Mutex<Vec<serde_json::Value>>>,
        /// The status the transfers route answers, 200 by default.
        refuse: Arc<Mutex<Option<StatusCode>>>,
        remaining: Arc<Mutex<i64>>,
        /// `(path and query, bearer)` for every call.
        calls: Arc<Mutex<Vec<(String, String)>>>,
        exchanges: Arc<Mutex<u32>>,
        transfer_barrier: Arc<Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>>,
    }

    fn bearer(request: &Request) -> String {
        request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .trim_start_matches("Bearer ")
            .to_string()
    }

    async fn serve(stub: &Stub) -> CloudApi {
        let exchanges = stub.exchanges.clone();
        let ledger = stub.ledger.clone();
        let refuse = stub.refuse.clone();
        let remaining = stub.remaining.clone();
        let calls = stub.calls.clone();
        let transfer_barrier = stub.transfer_barrier.clone();
        let router = Router::new()
            .route(
                "/v1/users/me/access-tokens",
                post(move || {
                    let exchanges = exchanges.clone();
                    async move {
                        let mut n = exchanges.lock().unwrap();
                        *n += 1;
                        Json(serde_json::json!({
                            "access_token": format!("jwt_{n}"),
                            "refresh_token": format!("rt_rotated_{n}"),
                            "refresh_token_expiration": "2026-12-01T00:00:00Z",
                        }))
                    }
                }),
            )
            .route(
                "/v1/organizations/{org}/usage/balance",
                get(move |Path(org): Path<String>| {
                    let remaining = *remaining.lock().unwrap();
                    async move {
                        Json(serde_json::json!({
                            "organization_id": org, "total_spent": 0,
                            "total_requests": 0, "total_tokens": 0,
                            "remaining": remaining, "spend_limit": null,
                        }))
                    }
                }),
            )
            .route(
                "/v1/organizations/{org}/credit/transfers",
                get(
                    move |Query(query): Query<std::collections::HashMap<String, String>>| {
                        let ledger = ledger.lock().unwrap().clone();
                        let refuse = *refuse.lock().unwrap();
                        let barrier = transfer_barrier.lock().unwrap().take();
                        async move {
                            if let Some((started, release)) = barrier {
                                started.notify_one();
                                release.notified().await;
                            }
                            if let Some(status) = refuse {
                                return (status, Json(serde_json::json!({}))).into_response();
                            }
                            assert_eq!(
                                query.get("direction").map(String::as_str),
                                Some("incoming")
                            );
                            // Pages of two, so the cursor is exercised.
                            let start = match query.get("before_id") {
                                Some(id) => {
                                    ledger.iter().position(|row| row["id"] == **id).unwrap() + 1
                                }
                                None => 0,
                            };
                            let page: Vec<_> = ledger.iter().skip(start).take(2).cloned().collect();
                            let next = page.last().map(|row| {
                                serde_json::json!({
                                    "before_time": row["created_at"],
                                    "before_id": row["id"],
                                })
                            });
                            Json(serde_json::json!({"transfers": page, "next_cursor": next}))
                                .into_response()
                        }
                    },
                ),
            )
            .layer(axum::middleware::from_fn(
                move |request: Request, next: axum::middleware::Next| {
                    let calls = calls.clone();
                    let path = request
                        .uri()
                        .path_and_query()
                        .map(|p| p.to_string())
                        .unwrap_or_default();
                    let offered = bearer(&request);
                    async move {
                        calls.lock().unwrap().push((path, offered));
                        next.run(request).await
                    }
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        CloudApi::for_test(&format!("http://{address}")).unwrap()
    }

    fn row(id: &str, amount: i64, when: &str) -> serde_json::Value {
        // The recipient's view carries more than this; none of it is read.
        serde_json::json!({
            "id": id, "amount": amount, "created_at": when,
            "sender_organization_id": "sender-org", "recipient_organization_id": ORG,
            "memo": "synthetic", "metadata": {}, "destination_credit_type": "transfer",
        })
    }

    #[tokio::test]
    async fn recipient_verification_failure_invalidates_the_visible_summary_and_publishes_refresh()
    {
        let s = shared();
        let ingest = link_recipient(&s).await;
        let stub = Stub::default();
        let api = serve(&stub).await;
        poll_with(&s, &api).await.unwrap();
        assert_eq!(snapshot(&s.store).received_24h(Utc::now()), Some(0));
        ingest.stub.lock().unwrap().linked = false;
        let mut events = s.events.subscribe();
        let _ = poll_with(&s, &api).await;
        let record = snapshot(&s.store);
        assert!(record.recipient_ref.is_none());
        assert_eq!(record.received_24h(Utc::now()), None);
        assert_eq!(
            events.try_recv().unwrap().event,
            crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED
        );
    }

    #[tokio::test]
    async fn a_failed_followup_scan_hides_the_window_without_changing_the_balance() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        *stub.remaining.lock().unwrap() = 7 * DOLLAR;
        let api = serve(&stub).await;
        poll_with(&s, &api).await.unwrap();
        assert_eq!(snapshot(&s.store).received_24h(Utc::now()), Some(0));
        *stub.refuse.lock().unwrap() = Some(StatusCode::SERVICE_UNAVAILABLE);
        assert!(poll_with(&s, &api).await.is_err());
        assert_eq!(snapshot(&s.store).received_24h(Utc::now()), None);
        assert!(matches!(
            balance::read_via(&s, &api).await,
            BalanceReport::Known(_)
        ));
    }

    #[test]
    fn explicit_refresh_reads_receipts_after_five_seconds_without_changing_background_cadence() {
        let s = shared();
        let now = Utc::now();
        assert!(take_poll_due_with_fresh(&s.store, now, false));
        assert!(!take_poll_due_with_fresh(
            &s.store,
            now + Duration::seconds(4),
            true
        ));
        assert!(take_poll_due_with_fresh(
            &s.store,
            now + Duration::seconds(5),
            true
        ));
        assert!(!take_poll_due_with_fresh(
            &s.store,
            now + Duration::minutes(1),
            false
        ));
        assert!(take_poll_due_with_fresh(
            &s.store,
            now + POLL_INTERVAL + Duration::seconds(5),
            false
        ));
    }

    #[tokio::test]
    async fn a_capped_ledger_scan_never_becomes_a_complete_credit_total() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        *stub.ledger.lock().unwrap() = (0..=MAX_PAGES * 2)
            .map(|i| row(&format!("transfer-{i}"), DOLLAR, "2026-10-24T11:00:00Z"))
            .collect();
        let api = serve(&stub).await;
        assert!(poll_with(&s, &api).await.is_err());
        let record = snapshot(&s.store);
        assert!(!record.scanned);
        assert_eq!(record.received_total_nanos, 0);
        assert_eq!(record.received_24h(Utc::now()), None);
    }

    #[tokio::test]
    async fn a_poll_counts_every_page_once_and_shares_one_exchange_with_the_balance() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        *stub.remaining.lock().unwrap() = 7 * DOLLAR;
        *stub.ledger.lock().unwrap() = vec![
            row("t3", 3 * DOLLAR, "2026-10-22T10:00:00Z"),
            row("t2", 2 * DOLLAR, "2026-10-21T10:00:00Z"),
            row("t1", DOLLAR, "2026-10-20T10:00:00Z"),
        ];
        let api = serve(&stub).await;
        let mut events = s.events.subscribe();

        assert_eq!(poll_with(&s, &api).await.unwrap(), 6 * DOLLAR);
        let record = snapshot(&s.store);
        assert_eq!(record.organization_id.as_deref(), Some(ORG));
        assert!(record.scanned);
        assert_eq!(record.received_total_nanos, 6 * DOLLAR);
        let balance = balance_snapshot(&s.store);
        assert_eq!(balance.samples.len(), 1);
        assert_eq!(balance.samples[0].remaining_nanos, 7 * DOLLAR);
        // Credit already there at the first read is history, not an
        // arrival: it becomes the baseline, silently.
        assert_eq!(record.notified_received_total_nanos, Some(6 * DOLLAR));
        assert_eq!(record.candidacy(), None);
        // A shell showing the balance is told to read it again.
        let event = events.try_recv().unwrap();
        assert_eq!(
            event.event,
            crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED
        );

        // One exchange bought the balance and both ledger pages: the reads
        // share the balance module's session, never a second rotation.
        assert_eq!(*stub.exchanges.lock().unwrap(), 1);
        let calls = stub.calls.lock().unwrap().clone();
        let ledger_calls: Vec<_> = calls
            .iter()
            .filter(|(path, _)| path.contains("/credit/transfers"))
            .collect();
        assert_eq!(ledger_calls.len(), 3, "{calls:?}");
        assert!(ledger_calls.iter().all(|(_, bearer)| bearer == "jwt_1"));

        // A complete scan with nothing new still refreshes the rolling window.
        assert_eq!(poll_with(&s, &api).await.unwrap(), 0);
        assert_eq!(snapshot(&s.store).received_total_nanos, 6 * DOLLAR);
        assert_eq!(
            events.try_recv().unwrap().event,
            crate::daemon::ipc::EVENT_NEAR_AI_BALANCE_CHANGED
        );

        // A new transfer lands: only it is counted.
        stub.ledger
            .lock()
            .unwrap()
            .insert(0, row("t4", DOLLAR / 2, "2026-10-23T10:00:00Z"));
        assert_eq!(poll_with(&s, &api).await.unwrap(), DOLLAR / 2);
        assert_eq!(
            snapshot(&s.store).received_total_nanos,
            6 * DOLLAR + DOLLAR / 2
        );

        // And the file says the same thing, with no names in it.
        let on_disk = std::fs::read_to_string(s.store.daemon_path(ARRIVALS_FILE)).unwrap();
        assert!(!on_disk.contains("sender-org"), "{on_disk}");
        assert!(!on_disk.contains("synthetic"), "{on_disk}");
    }

    /// An empty ledger at the first read, then a first transfer: that is the
    /// first credit ever received, and it is announced, whatever its size.
    #[tokio::test]
    async fn the_first_credit_after_an_empty_ledger_is_announced() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        let api = serve(&stub).await;
        assert_eq!(poll_with(&s, &api).await.unwrap(), 0);
        let record = snapshot(&s.store);
        assert!(record.scanned);
        assert_eq!(record.notified_received_total_nanos, None);
        assert_eq!(record.candidacy(), None);

        stub.ledger
            .lock()
            .unwrap()
            .push(row("t1", DOLLAR / 4, "2026-10-23T10:00:00Z"));
        assert_eq!(poll_with(&s, &api).await.unwrap(), DOLLAR / 4);
        assert_eq!(snapshot(&s.store).candidacy(), Some(DOLLAR / 4));
    }

    #[tokio::test]
    async fn a_member_who_is_not_an_owner_is_refused_and_nothing_is_counted() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        *stub.refuse.lock().unwrap() = Some(StatusCode::FORBIDDEN);
        let api = serve(&stub).await;
        assert!(poll_with(&s, &api).await.is_err());
        let record = snapshot(&s.store);
        assert!(!record.scanned);
        assert_eq!(record.candidacy(), None);
        // The balance sample was still kept: the 24-hour change does not
        // need the ledger.
        assert_eq!(balance_snapshot(&s.store).samples.len(), 1);
    }

    #[tokio::test]
    async fn forgetting_the_sign_in_forgets_the_record() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        *stub.ledger.lock().unwrap() = vec![row("t1", DOLLAR, "2026-10-20T10:00:00Z")];
        let api = serve(&stub).await;
        poll_with(&s, &api).await.unwrap();
        assert!(s.store.daemon_path(ARRIVALS_FILE).exists());

        let response = crate::daemon::nearai_credential::handle_forget(
            &s,
            &crate::daemon::ipc::Request {
                id: 1,
                method: "near_ai_credential_forget".into(),
                params: serde_json::json!({}),
            },
        );
        assert!(response.error.is_none());
        assert!(!s.store.daemon_path(ARRIVALS_FILE).exists());
        assert_eq!(snapshot(&s.store), ArrivalsRecord::default());
        // With the session gone, a late write cannot bring it back.
        assert!(update_record(&s, records(), ARRIVALS_FILE, |record| record.adopt(ORG)).is_none());
        assert!(!s.store.daemon_path(ARRIVALS_FILE).exists());
    }

    #[tokio::test]
    async fn payout_arrivals_are_read_without_a_private_ai_key() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        {
            let mut settings = s.settings.lock().unwrap();
            settings.near_ai_inference = None;
            settings.save_for_test(&s.store).unwrap();
        }
        let stub = Stub::default();
        let api = serve(&stub).await;
        assert_eq!(poll_with(&s, &api).await.unwrap(), 0);
        assert!(
            snapshot(&s.store).scanned,
            "signed-in contributors can monitor the linked payout recipient without Private AI"
        );
        stub.ledger
            .lock()
            .unwrap()
            .push(row("payout-1", DOLLAR, "2026-10-23T10:00:00Z"));
        assert_eq!(poll_with(&s, &api).await.unwrap(), DOLLAR);
        assert_eq!(
            pending_notification(&s).map(|credit| (credit.amount, credit.received_total)),
            Some((DOLLAR, DOLLAR))
        );
    }

    #[tokio::test]
    async fn payout_arrivals_never_use_a_different_spending_organization() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        {
            let mut settings = s.settings.lock().unwrap();
            settings.near_ai_inference.as_mut().unwrap().organization_id = "spending-only".into();
            settings.save_for_test(&s.store).unwrap();
        }
        let stub = Stub::default();
        let api = serve(&stub).await;
        *stub.ledger.lock().unwrap() = vec![row("payout-1", DOLLAR, "2026-10-23T10:00:00Z")];
        assert_eq!(poll_with(&s, &api).await.unwrap(), DOLLAR);
        assert_eq!(snapshot(&s.store).organization_id.as_deref(), Some(ORG));
        // A later read of the spending balance cannot replace the payout
        // recipient's seen IDs and turn the same transfer into news again.
        note_balance(
            &s,
            Some("spending-only"),
            &balance::BalanceReading {
                remaining_nanos: Some(5 * DOLLAR),
                spend_limit_nanos: None,
                total_spent_nanos: 0,
                total_requests: 0,
                total_tokens: 0,
                observed_at: Utc::now() + Duration::hours(1),
            },
        );
        assert_eq!(snapshot(&s.store).received_total_nanos, DOLLAR);
        assert_eq!(poll_with(&s, &api).await.unwrap(), 0);
        let calls = stub.calls.lock().unwrap();
        assert!(calls.iter().any(|(path, _)| {
            path.starts_with(&format!("/v1/organizations/{ORG}/credit/transfers"))
        }));
        assert!(
            !calls
                .iter()
                .any(|(path, _)| path
                    .starts_with("/v1/organizations/spending-only/credit/transfers"))
        );
    }

    #[tokio::test]
    async fn no_linked_payout_recipient_never_counts_spending_org_transfers() {
        let s = shared();
        let stub = Stub::default();
        *stub.ledger.lock().unwrap() = vec![row("unrelated", DOLLAR, "2026-10-20T10:00:00Z")];
        let api = serve(&stub).await;
        assert_eq!(poll_with(&s, &api).await.unwrap(), 0);
        assert!(!snapshot(&s.store).scanned);
        assert_eq!(pending_notification(&s), None);
        let calls = stub.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .all(|(path, _)| !path.contains("/credit/transfers"))
        );
        assert!(
            calls
                .iter()
                .any(|(path, _)| path.contains("/usage/balance"))
        );
        assert_eq!(balance_snapshot(&s.store).samples.len(), 1);
    }

    async fn unlink(s: &DaemonShared) {
        for (method, params) in [
            (
                "payout_link_unlink",
                serde_json::json!({"rail_id":"near_ai", "expected_current_version":1}),
            ),
            (
                "payout_link_confirm",
                serde_json::json!({"rail_id":"near_ai", "action":"unlink", "confirmed":true}),
            ),
        ] {
            let response = crate::daemon::ipc::handle_request_async(
                s,
                &crate::daemon::ipc::Request {
                    id: 1,
                    method: method.into(),
                    params,
                },
            )
            .await;
            assert!(response.error.is_none(), "{method} was refused");
        }
    }

    #[tokio::test]
    async fn an_inflight_poll_cannot_restore_an_unlinked_recipient() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        let api = serve(&stub).await;
        poll_with(&s, &api).await.unwrap();
        stub.ledger
            .lock()
            .unwrap()
            .push(row("late-credit", DOLLAR, "2026-10-23T10:00:00Z"));
        let started = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *stub.transfer_barrier.lock().unwrap() = Some((started.clone(), release.clone()));
        let poll = poll_with(&s, &api);
        tokio::pin!(poll);
        tokio::select! {
            _ = started.notified() => {},
            result = &mut poll => panic!("poll finished before its blocked ledger response: {result:?}"),
        }
        unlink(&s).await;
        let mut events = s.events.subscribe();
        release.notify_one();
        assert_eq!(poll.await.unwrap(), 0);
        assert_eq!(snapshot(&s.store).received_total_nanos, 0);
        assert_eq!(pending_notification(&s), None);
        assert!(
            events.try_recv().is_err(),
            "a stale poll published a balance event"
        );
    }

    #[tokio::test]
    async fn a_prepared_credit_notification_is_suppressed_after_unlink() {
        let s = shared();
        let _ingest = link_recipient(&s).await;
        let stub = Stub::default();
        let api = serve(&stub).await;
        poll_with(&s, &api).await.unwrap();
        stub.ledger
            .lock()
            .unwrap()
            .push(row("credit", DOLLAR, "2026-10-23T10:00:00Z"));
        assert_eq!(poll_with(&s, &api).await.unwrap(), DOLLAR);
        let credit = pending_notification(&s).unwrap();
        unlink(&s).await;
        assert!(!publish_notification(&s, &credit, || panic!(
            "stale recipient credit was published"
        )));
        assert_eq!(pending_notification(&s), None);
        assert_eq!(snapshot(&s.store).notified_received_total_nanos, None);
    }

    #[test]
    fn the_balance_reply_carries_the_change_and_the_total_only_when_known() {
        let s = shared();
        let now = Utc::now();
        let mut record = ArrivalsRecord::default();
        record.adopt(ORG);
        record.record_sample(now - Duration::hours(25), Some(5 * DOLLAR));
        record.apply_transfers(&[IncomingTransfer {
            id: "t1".into(),
            amount: 4 * DOLLAR,
            created_at: now - Duration::hours(2),
        }]);
        record.record_received_window(
            &[IncomingTransfer {
                id: "t1".into(),
                amount: 4 * DOLLAR,
                created_at: now - Duration::hours(2),
            }],
            now,
        );
        set_for_test(&s.store, record);
        let known = |remaining| {
            BalanceReport::Known(balance::BalanceReading {
                remaining_nanos: remaining,
                spend_limit_nanos: None,
                total_spent_nanos: 0,
                total_requests: 0,
                total_tokens: 0,
                observed_at: now,
            })
        };

        let up = balance_value(&s, known(Some(8 * DOLLAR + 200_000_000)));
        assert_eq!(up["change_24h_nanos"], 3 * DOLLAR + 200_000_000);
        assert_eq!(up["change_24h_text"], "+$3.20 in the last 24 hours");
        assert_eq!(up["change_24h_direction"], 1);
        assert_eq!(up["received_total_nanos"], 4 * DOLLAR);
        assert_eq!(up["received_24h_nanos"], 4 * DOLLAR);
        assert_eq!(
            up["received_24h_text"],
            "$4.00 received in the last 24 hours"
        );
        assert_eq!(up["received_24h_checked_at"], now.to_rfc3339());

        let down = balance_value(&s, known(Some(4 * DOLLAR)));
        assert_eq!(down["change_24h_nanos"], -DOLLAR);
        assert_eq!(down["change_24h_text"], "-$1.00 in the last 24 hours");
        assert_eq!(down["change_24h_direction"], -1);
        let subcent = balance_value(&s, known(Some(5 * DOLLAR - 9_000_000)));
        assert_eq!(subcent["change_24h_nanos"], -9_000_000);
        assert_eq!(subcent["change_24h_direction"], 0);
        assert_eq!(subcent["change_24h_text"], "$0.00 in the last 24 hours");

        // No limit set now: no remaining figure, so no change -- not zero.
        let unknown = balance_value(&s, known(None));
        assert!(unknown["change_24h_nanos"].is_null(), "{unknown}");
        assert!(unknown["change_24h_direction"].is_null());
        assert_eq!(
            unknown["change_24h_unavailable_text"],
            "24-hour balance change unavailable"
        );

        // Any state but `known` carries none of it.
        let expired = balance_value(&s, BalanceReport::SessionExpired);
        assert!(expired["change_24h_nanos"].is_null());
        assert!(expired["received_total_nanos"].is_null());
        assert!(expired["received_24h_nanos"].is_null());
        assert!(expired["received_24h_text"].is_null());

        // Switching only the spending organization cannot surface payouts
        // into a different organization under its balance.
        s.settings
            .lock()
            .unwrap()
            .near_ai_inference
            .as_mut()
            .unwrap()
            .organization_id = "other".into();
        let other = balance_value(&s, known(Some(DOLLAR)));
        assert!(other["received_24h_nanos"].is_null());
        assert_eq!(
            other["received_24h_unavailable_text"],
            "Amount received in 24 hours unavailable"
        );
    }

    #[test]
    fn a_poll_is_due_every_interval_and_not_between() {
        let s = shared();
        let t0 = Utc::now();
        assert!(take_poll_due_with_fresh(&s.store, t0, false));
        assert!(!take_poll_due_with_fresh(
            &s.store,
            t0 + Duration::minutes(5),
            false
        ));
        assert!(take_poll_due_with_fresh(
            &s.store,
            t0 + POLL_INTERVAL,
            false
        ));
    }
}
