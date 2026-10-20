//! `GET /_ironwire/log?since=` returns a window, oldest first.
//!
//! The default view is the newest 20, which is right for a person reading
//! `ironwire log`. It is wrong for anything polling the ledger: newest-first
//! plus a limit drops the *oldest* rows in a window, and a reader that has
//! already advanced past them cannot tell that it missed any. So a windowed
//! read pages forward instead -- oldest first, bounded in SQL, with `after_id`
//! as the cursor.
//!
//! The cursor is the load-bearing half. `since` is inclusive against
//! `started_at`, which is not unique, so a caller paging on the timestamp
//! re-reads its boundary row every request and stops advancing entirely once
//! `limit` exchanges share an instant. `after_id` is exclusive over a unique,
//! insert-ordered column, so a reader walks the window exactly once.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::{Exchange, Ledger};
use ironwire_proxy::control::LogView;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use tower::ServiceExt;

fn at(offset: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000 + offset, 0).expect("valid timestamp")
}

fn exchange(started_at: DateTime<Utc>) -> Exchange {
    Exchange {
        id: None,
        model_alias_resolved: None,
        started_at,
        ttfb_ms: None,
        total_ms: Some(10),
        facade: "anthropic".into(),
        path: "/v1/messages".into(),
        conversation: "c-1".into(),
        client_session_id: Some(format!("s-{}", started_at.timestamp())),
        backend: "claude-sub".into(),
        requested_model: None,
        served_model: None,
        upstream_id: None,
        request_sha256: None,
        response_sha256: None,
        body_ref: None,
        rung: "same_model".into(),
        attempts: 1,
        input_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        output_tokens: None,
        cost_usd: None,
        substitutions: None,
        status: 200,
        error: None,
        confidence: None,
        proof: None,
    }
}

async fn fetch(ledger: Ledger, query: &str) -> LogView {
    let state = AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger));
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!("/_ironwire/log{query}"))
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("a LogView")
}

fn seeded() -> Ledger {
    let ledger = Ledger::in_memory().expect("ledger opens");
    for offset in 0..5 {
        ledger.record(&exchange(at(offset))).expect("record");
    }
    ledger
}

#[tokio::test]
async fn a_window_starts_at_the_instant_it_names_and_runs_forward() {
    let view = fetch(
        seeded(),
        &format!(
            "?since={}",
            at(2).to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
    )
    .await;
    let seen: Vec<_> = view
        .exchanges
        .iter()
        .map(|e| e.started_at.timestamp())
        .collect();
    assert_eq!(
        seen,
        vec![at(2).timestamp(), at(3).timestamp(), at(4).timestamp()],
        "the window excludes what came before it, oldest first"
    );
}

#[tokio::test]
async fn a_truncated_window_cuts_the_end_the_caller_has_not_reached() {
    // A page starts at the window's oldest row. The rows beyond it are not
    // lost: the caller advances past them with `after_id`, which is what makes
    // truncating here safe. Without that cursor this would be a reader that
    // only ever sees the oldest rows in its window.
    let view = fetch(
        seeded(),
        &format!(
            "?since={}&limit=2",
            at(0).to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
    )
    .await;
    let seen: Vec<_> = view
        .exchanges
        .iter()
        .map(|e| e.started_at.timestamp())
        .collect();
    assert_eq!(seen, vec![at(0).timestamp(), at(1).timestamp()]);
}

#[tokio::test]
async fn without_a_window_the_log_is_still_newest_first() {
    // The default view is what a person reads, and it must not change.
    let view = fetch(seeded(), "?limit=2").await;
    let seen: Vec<_> = view
        .exchanges
        .iter()
        .map(|e| e.started_at.timestamp())
        .collect();
    assert_eq!(seen, vec![at(4).timestamp(), at(3).timestamp()]);
}

#[tokio::test]
async fn the_windowed_log_still_needs_the_control_token() {
    let state = AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(seeded()));
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/_ironwire/log?since={}",
                    at(0).to_rfc3339_opts(SecondsFormat::Secs, true)
                ))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("served");
    assert_ne!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn the_summary_still_covers_the_last_day_regardless_of_the_window() {
    // `last_24h` answers "what has this machine been doing", which is not the
    // same question as the window, and must not follow it.
    let ledger = Ledger::in_memory().expect("ledger opens");
    ledger
        .record(&exchange(Utc::now() - Duration::hours(1)))
        .expect("record");
    let view = fetch(
        ledger,
        &format!(
            "?since={}",
            (Utc::now() + Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
    )
    .await;
    assert!(view.exchanges.is_empty(), "the window is in the future");
    assert_eq!(view.last_24h.exchanges, 1, "the summary is unaffected");
}

#[tokio::test]
async fn a_caller_pages_a_window_to_its_end_without_repeats_or_gaps() {
    // The endpoint's contract, end to end. A reader takes a page, advances on
    // the last id, and stops when a page is short -- seeing every row once.
    let ledger = seeded();
    let since = at(0).to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut seen: Vec<i64> = Vec::new();
    let mut cursor: Option<i64> = None;

    loop {
        let query = match cursor {
            Some(id) => format!("?since={since}&limit=2&after_id={id}"),
            None => format!("?since={since}&limit=2"),
        };
        let view = fetch(ledger.clone(), &query).await;
        if view.exchanges.is_empty() {
            break;
        }
        cursor = view.exchanges.last().and_then(|e| e.id);
        seen.extend(view.exchanges.iter().filter_map(|e| e.id));
        assert!(seen.len() <= 5, "paging must terminate rather than loop");
        if view.exchanges.len() < 2 {
            break;
        }
    }

    let mut unique = seen.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 5, "every row exactly once");
    assert_eq!(seen.len(), unique.len(), "no row returned twice");
}

#[tokio::test]
async fn a_window_larger_than_the_limit_still_reaches_its_newest_row() {
    // The bug this endpoint shipped with: oldest-first truncation handed back
    // the oldest `limit` rows and nothing else, so a caller that read one page
    // per tick never saw a recent exchange on a busy machine.
    let ledger = seeded();
    let since = at(0).to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut cursor: Option<i64> = None;
    let mut last_seen: Option<i64> = None;

    for _ in 0..10 {
        let query = match cursor {
            Some(id) => format!("?since={since}&limit=2&after_id={id}"),
            None => format!("?since={since}&limit=2"),
        };
        let view = fetch(ledger.clone(), &query).await;
        if view.exchanges.is_empty() {
            break;
        }
        last_seen = view.exchanges.last().map(|e| e.started_at.timestamp());
        cursor = view.exchanges.last().and_then(|e| e.id);
    }

    assert_eq!(
        last_seen,
        Some(at(4).timestamp()),
        "the newest row in the window is reachable"
    );
}
