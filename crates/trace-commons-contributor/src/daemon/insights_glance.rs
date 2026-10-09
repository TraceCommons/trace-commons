//! `insights_glance`: today's routed tokens for the menu-bar glance, read from
//! the proxy ledger this daemon already holds (Insights feed L), and the
//! coalesced `usage_changed` pulse.
//!
//! Gated through the `insights_ledger_feed` setting, on by default (owner
//! decision D3, settled 2026-10-09): while a contributor has it off, the
//! method answers `enabled: false` without touching the ledger, and no pulse
//! is published.
//!
//! Synchronous and read-only. It reads the committed ledger snapshot, never
//! refreshes it, never forwards to the Insights service, and makes no network
//! call, so it is on the developer dry-run allowlist.
//!
//! # What crosses the socket
//!
//! Fixed labels, counts, a local date and the time the ledger last answered.
//! Never a session id, a model name, a body reference, a digest, the
//! provider's exchange id, an endpoint, a backend name or a price (owner
//! decision D5, open: `cost_usd` never reaches Insights). Nothing here is
//! logged.

use chrono::{DateTime, FixedOffset, NaiveTime, TimeZone, Utc};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use crate::insights::analytics_constants::{
    ADVICE_SHOWN, CONTEXT_TIP_WINDOW_SECS, LEDGER_GLANCE_STALE_SECS,
};
use crate::insights::ledger_series::{ContextTip, context_tip, today_by_tool};
use crate::routing::RoutedExchange;

/// The feed label every answer carries, so a shell can say which feed it
/// shows.
pub const FEED_LEDGER: &str = "ledger";
/// `tz` missing, not an integer, or not a UTC offset.
pub const ERR_TZ_INVALID: &str = "tz-invalid";
/// The widest UTC offset accepted, in seconds (18 hours either way).
pub const MAX_TZ_OFFSET_SECS: i64 = 18 * 3_600;

/// The `tz` parameter: the shell's current UTC offset in seconds east.
pub(crate) fn parse_tz(params: &serde_json::Value) -> Option<FixedOffset> {
    let seconds = params.get("tz")?.as_i64()?;
    if seconds.abs() > MAX_TZ_OFFSET_SECS {
        return None;
    }
    FixedOffset::east_opt(i32::try_from(seconds).ok()?)
}

/// The wire form of a context tip state.
fn tip_value(tip: ContextTip) -> serde_json::Value {
    match tip {
        ContextTip::Held => serde_json::json!({"state": "held"}),
        ContextTip::ThresholdUnset => serde_json::json!({"state": "threshold_unset"}),
        ContextTip::NoSession => serde_json::json!({"state": "no_session"}),
        ContextTip::NoFigure => serde_json::json!({"state": "no_figure"}),
        ContextTip::Quiet => serde_json::json!({"state": "quiet"}),
        ContextTip::Lit { context, threshold } => serde_json::json!({
            "state": "lit",
            "context": context,
            "threshold": threshold,
        }),
    }
}

/// The glance over the rows the ledger holds, as the method answers it when
/// the feed is on and a ledger has answered.
pub(crate) fn glance_value(
    rows: &[RoutedExchange],
    speakers: &[(&'static str, &'static str)],
    now: DateTime<Utc>,
    tz: FixedOffset,
    threshold: Option<u32>,
    updated_at: DateTime<Utc>,
    unreadable_rows: usize,
) -> serde_json::Value {
    let today = now.with_timezone(&tz).date_naive();
    let today_rows: Vec<(&str, &RoutedExchange)> = rows
        .iter()
        .filter(|row| row.started_at.with_timezone(&tz).date_naive() == today)
        .map(|row| (super::inference_map::attribute(row, speakers), row))
        .collect();
    let tools = today_by_tool(&today_rows);
    let calls: u64 = tools.iter().map(|day| u64::from(day.calls)).sum();
    let known: u64 = tools.iter().map(|day| u64::from(day.known_calls)).sum();
    let tools: Vec<serde_json::Value> = tools
        .into_iter()
        .map(|day| {
            serde_json::json!({
                "tool": day.tool,
                "calls": day.calls,
                "known_calls": day.known_calls,
                "tokens": day.tokens,
                "cache_share": day.cache_share.map(|share| serde_json::json!({
                    "numerator": share.numerator,
                    "denominator": share.denominator,
                    "permille": share.permille(),
                })),
            })
        })
        .collect();
    let stale = (now - updated_at).num_seconds() > LEDGER_GLANCE_STALE_SECS;
    serde_json::json!({
        "enabled": true,
        "feed": FEED_LEDGER,
        "readable": true,
        "updated_at": updated_at.to_rfc3339(),
        "stale": stale,
        "date": today.to_string(),
        "tools": tools,
        "coverage": {
            "calls": calls,
            "known": known,
            "unknown": calls - known,
            "unreadable_rows": unreadable_rows,
        },
        "context_tip": tip_value(context_tip(rows, now, threshold, ADVICE_SHOWN)),
    })
}

/// `insights_glance`.
pub fn handle_glance(shared: &DaemonShared, req: &Request) -> Response {
    let Some(tz) = parse_tz(&req.params) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_TZ_INVALID);
    };
    // A poisoned lock reads as off: fail closed, never a guess.
    let (enabled, threshold) = shared.settings.lock().map_or((false, None), |settings| {
        (
            settings.insights_ledger_feed,
            settings.insights_context_threshold,
        )
    });
    if !enabled {
        // Turned off by the contributor: nothing reads the ledger for Insights.
        return Response::ok(
            req.id,
            serde_json::json!({"enabled": false, "feed": FEED_LEDGER}),
        );
    }
    let unreadable = serde_json::json!({"enabled": true, "feed": FEED_LEDGER, "readable": false});
    let Some(ledger) = shared.routing_ledger() else {
        return Response::ok(req.id, unreadable);
    };
    let Some(updated_at) = ledger.last_refresh_at() else {
        return Response::ok(req.id, unreadable);
    };
    let now = Utc::now();
    // Local midnight at this offset is at most a day ago, inside the
    // ledger's own 24-hour window.
    let midnight = tz
        .from_local_datetime(&now.with_timezone(&tz).date_naive().and_time(NaiveTime::MIN))
        .single()
        .map_or(now, |at| at.with_timezone(&Utc));
    let since = midnight.min(now - chrono::Duration::seconds(CONTEXT_TIP_WINDOW_SECS));
    let rows = crate::routing::RoutingLedger::exchanges_since(ledger.as_ref(), since);
    let speakers = super::inference_map::speakers_from_rows(&super::harness::rows_now(shared));
    Response::ok(
        req.id,
        glance_value(
            &rows,
            &speakers,
            now,
            tz,
            threshold,
            updated_at,
            ledger.unreadable_rows(),
        ),
    )
}

/// Publish one `usage_changed` when this tick's ledger read added any call
/// and the feed is on (the default; owner decision D3, settled 2026-10-09). A pulse: the data is `{}`, and a shell re-reads
/// `insights_glance` for the figures.
pub(crate) fn publish_usage_changed(shared: &DaemonShared, added: &[RoutedExchange]) {
    if added.is_empty()
        || !shared
            .settings
            .lock()
            .is_ok_and(|settings| settings.insights_ledger_feed)
    {
        return;
    }
    shared.publish(super::ipc::EVENT_USAGE_CHANGED, serde_json::json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::ProofStatus;
    use chrono::Duration;

    const SESSION: &str = "SESSION-SECRET";
    const UPSTREAM_ID: &str = "0123456789abcdef0123456789abcdef";
    const BODY_REF: &str = "00000000000000000009-000000";
    const DIGEST: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
    const MODEL: &str = "SECRET-MODEL-NAME";
    const BACKEND: &str = "my-backend";

    fn row(id: i64, started_at: DateTime<Utc>) -> RoutedExchange {
        RoutedExchange {
            id: Some(id),
            started_at,
            client_session_id: Some(SESSION.to_string()),
            facade: "anthropic".to_string(),
            path: Some("/v1/messages".to_string()),
            backend: BACKEND.to_string(),
            requested_model: Some(MODEL.to_string()),
            served_model: Some(MODEL.to_string()),
            upstream_id: Some(UPSTREAM_ID.to_string()),
            request_sha256: Some(DIGEST.to_string()),
            response_sha256: Some(DIGEST.to_string()),
            body_ref: Some(BODY_REF.to_string()),
            rung: "full".to_string(),
            attempts: 2,
            input_tokens: Some(100),
            cache_read_tokens: Some(800),
            cache_write_tokens: Some(100),
            output_tokens: Some(50),
            cost_usd: Some(1.2345),
            status: 200,
            proof: Some(ProofStatus::Verified),
            ..Default::default()
        }
    }

    /// 2026-10-08 10:00 UTC.
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 8, 10, 0, 0).unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: "insights_glance".to_string(),
            params,
        }
    }

    #[test]
    fn today_is_per_tool_and_counts_unknown_calls_as_unknown() {
        let mut unknown = row(3, now() - Duration::minutes(5));
        unknown.output_tokens = None;
        let rows = vec![
            row(1, now() - Duration::hours(11)), // yesterday in UTC
            row(2, now() - Duration::hours(1)),
            unknown,
        ];
        let value = glance_value(&rows, &[], now(), utc(), None, now(), 0);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["feed"], "ledger");
        assert_eq!(value["readable"], true);
        assert_eq!(value["date"], "2026-10-08");
        assert_eq!(value["stale"], false);
        assert_eq!(
            value["tools"],
            serde_json::json!([{
                "tool": "claude-code",
                "calls": 2,
                "known_calls": 1,
                "tokens": 1_050,
                "cache_share": {"numerator": 800, "denominator": 1_000, "permille": 800},
            }])
        );
        assert_eq!(
            value["coverage"],
            serde_json::json!({"calls": 2, "known": 1, "unknown": 1, "unreadable_rows": 0})
        );
    }

    /// The answers the shells pin, recorded whole: printed by
    /// `cargo test ... a_recorded_glance_for_the_shells -- --nocapture` and
    /// pasted into the Swift wire tests, never reconstructed by hand.
    ///
    /// Claude Code is partial (one call's output unknown), Codex is fully
    /// known through the OpenAI facade, and an endpoint no connection writes
    /// is `unknown` with no figure. The stale answer's one call read no input,
    /// so its tokens are known and its cache share is `null`. The lit tip is
    /// recorded from `tip_value`, since advice is held on the wire.
    #[test]
    fn a_recorded_glance_for_the_shells() {
        let mut unknown_output = row(2, now() - Duration::minutes(30));
        unknown_output.output_tokens = None;
        let mut codex = row(3, now() - Duration::minutes(20));
        codex.facade = "openai".to_string();
        codex.path = Some("/v1/responses".to_string());
        codex.input_tokens = Some(1_000);
        codex.cache_read_tokens = Some(600);
        codex.cache_write_tokens = Some(0);
        codex.output_tokens = Some(25);
        let mut elsewhere = row(4, now() - Duration::minutes(10));
        elsewhere.facade = "openai".to_string();
        elsewhere.path = Some("/v1/chat/completions".to_string());
        elsewhere.output_tokens = None;
        let rows = vec![
            row(1, now() - Duration::hours(1)),
            unknown_output,
            codex,
            elsewhere,
        ];
        let glance = glance_value(&rows, &[], now(), utc(), None, now(), 0);
        assert_eq!(
            glance,
            serde_json::json!({
                "enabled": true,
                "feed": "ledger",
                "readable": true,
                "updated_at": "2026-10-08T10:00:00+00:00",
                "stale": false,
                "date": "2026-10-08",
                "tools": [
                    {
                        "tool": "claude-code",
                        "calls": 2,
                        "known_calls": 1,
                        "tokens": 1_050,
                        "cache_share": {"numerator": 800, "denominator": 1_000, "permille": 800},
                    },
                    {
                        "tool": "codex",
                        "calls": 1,
                        "known_calls": 1,
                        "tokens": 1_025,
                        "cache_share": {"numerator": 600, "denominator": 1_000, "permille": 600},
                    },
                    {
                        "tool": "unknown",
                        "calls": 1,
                        "known_calls": 0,
                        "tokens": null,
                        "cache_share": null,
                    },
                ],
                "coverage": {"calls": 4, "known": 2, "unknown": 2, "unreadable_rows": 0},
                "context_tip": {"state": "held"},
            })
        );

        let mut output_only = row(5, now() - Duration::minutes(5));
        output_only.input_tokens = Some(0);
        output_only.cache_read_tokens = Some(0);
        output_only.cache_write_tokens = Some(0);
        output_only.output_tokens = Some(40);
        let updated = now() - Duration::seconds(LEDGER_GLANCE_STALE_SECS + 1);
        let stale = glance_value(&[output_only], &[], now(), utc(), None, updated, 2);
        assert_eq!(
            stale,
            serde_json::json!({
                "enabled": true,
                "feed": "ledger",
                "readable": true,
                "updated_at": "2026-10-08T09:49:59+00:00",
                "stale": true,
                "date": "2026-10-08",
                "tools": [{
                    "tool": "claude-code",
                    "calls": 1,
                    "known_calls": 1,
                    "tokens": 40,
                    "cache_share": null,
                }],
                "coverage": {"calls": 1, "known": 1, "unknown": 0, "unreadable_rows": 2},
                "context_tip": {"state": "held"},
            })
        );

        let lit = tip_value(ContextTip::Lit {
            context: 180_000,
            threshold: 200_000,
        });
        assert_eq!(
            lit,
            serde_json::json!({"state": "lit", "context": 180_000, "threshold": 200_000})
        );

        for value in [&glance, &stale, &lit] {
            println!("{}", serde_json::to_string(value).unwrap());
        }
    }

    #[test]
    fn the_day_is_the_shells_local_day() {
        // 23:30 UTC on the 7th is 00:30 on the 8th at UTC+1.
        let late = Utc.with_ymd_and_hms(2026, 10, 7, 23, 30, 0).unwrap();
        let rows = vec![row(1, late)];
        let plus_one = FixedOffset::east_opt(3_600).unwrap();
        let value = glance_value(&rows, &[], now(), plus_one, None, now(), 0);
        assert_eq!(value["coverage"]["calls"], 1);
        let value = glance_value(&rows, &[], now(), utc(), None, now(), 0);
        assert_eq!(value["coverage"]["calls"], 0);
        assert_eq!(value["tools"], serde_json::json!([]));
    }

    #[test]
    fn an_old_answer_is_stale_and_unreadable_rows_are_reported() {
        let updated = now() - Duration::seconds(LEDGER_GLANCE_STALE_SECS + 1);
        let value = glance_value(&[], &[], now(), utc(), None, updated, 3);
        assert_eq!(value["stale"], true);
        assert_eq!(value["updated_at"], updated.to_rfc3339());
        assert_eq!(value["coverage"]["unreadable_rows"], 3);
    }

    #[test]
    fn the_tip_is_held_on_the_wire_while_advice_is_held() {
        const { assert!(!ADVICE_SHOWN) };
        let rows = vec![row(1, now() - Duration::seconds(30))];
        let value = glance_value(&rows, &[], now(), utc(), Some(1_000), now(), 0);
        assert_eq!(value["context_tip"], serde_json::json!({"state": "held"}));
    }

    #[test]
    fn every_tip_state_has_a_label_and_only_lit_carries_figures() {
        assert_eq!(
            tip_value(ContextTip::Held),
            serde_json::json!({"state": "held"})
        );
        assert_eq!(
            tip_value(ContextTip::ThresholdUnset),
            serde_json::json!({"state": "threshold_unset"})
        );
        assert_eq!(
            tip_value(ContextTip::NoSession),
            serde_json::json!({"state": "no_session"})
        );
        assert_eq!(
            tip_value(ContextTip::NoFigure),
            serde_json::json!({"state": "no_figure"})
        );
        assert_eq!(
            tip_value(ContextTip::Quiet),
            serde_json::json!({"state": "quiet"})
        );
        assert_eq!(
            tip_value(ContextTip::Lit {
                context: 190_000,
                threshold: 200_000
            }),
            serde_json::json!({"state": "lit", "context": 190_000, "threshold": 200_000})
        );
    }

    #[test]
    fn nothing_identifying_or_priced_crosses() {
        let rows = vec![row(1, now() - Duration::minutes(1))];
        let text = glance_value(&rows, &[], now(), utc(), Some(1_000), now(), 0).to_string();
        for forbidden in [
            SESSION,
            UPSTREAM_ID,
            BODY_REF,
            DIGEST,
            MODEL,
            BACKEND,
            "/v1/",
            "cost",
            "priced",
            "1.2345",
            "attempts",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
        }
    }

    #[test]
    fn tz_is_an_offset_in_seconds_and_nothing_else() {
        assert_eq!(
            parse_tz(&serde_json::json!({"tz": -14_400})),
            FixedOffset::east_opt(-14_400)
        );
        assert_eq!(parse_tz(&serde_json::json!({"tz": 0})), Some(utc()));
        for bad in [
            serde_json::json!({}),
            serde_json::json!({"tz": null}),
            serde_json::json!({"tz": "America/New_York"}),
            serde_json::json!({"tz": 1.5}),
            serde_json::json!({"tz": MAX_TZ_OFFSET_SECS + 1}),
            serde_json::json!({"tz": -MAX_TZ_OFFSET_SECS - 1}),
        ] {
            assert_eq!(parse_tz(&bad), None, "{bad}");
        }
    }

    #[test]
    fn with_the_feed_off_the_ledger_is_not_read() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_ledger_feed = false;
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(vec![row(1, Utc::now())]),
        );
        let r = handle_glance(&s, &call(serde_json::json!({"tz": 0})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "ledger"})
        );
    }

    #[test]
    fn with_the_feed_on_it_reads_the_held_ledger() {
        let (_dir, s) = shared();
        assert!(
            s.settings.lock().unwrap().insights_ledger_feed,
            "on by default (owner decision D3, settled 2026-10-09)"
        );
        let r = handle_glance(&s, &call(serde_json::json!({"tz": 0})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": true, "feed": "ledger", "readable": false}),
            "no ledger answered: not evidence of no calls"
        );
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(vec![row(1, Utc::now())]),
        );
        let value = handle_glance(&s, &call(serde_json::json!({"tz": 0})))
            .result
            .unwrap();
        assert_eq!(value["readable"], true);
        assert_eq!(value["coverage"]["calls"], 1);
        assert_eq!(value["context_tip"]["state"], "held");
    }

    #[test]
    fn a_bad_tz_is_refused_whatever_the_setting() {
        let (_dir, s) = shared();
        for on in [false, true] {
            s.settings.lock().unwrap().insights_ledger_feed = on;
            let r = handle_glance(&s, &call(serde_json::json!({"tz": "UTC"})));
            let err = r.error.unwrap();
            assert_eq!(err.code, ERR_BAD_PARAMS);
            assert_eq!(err.message, ERR_TZ_INVALID);
        }
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        assert!(super::super::ipc::METHODS.contains(&"insights_glance"));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&"insights_glance"));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        let r = super::super::ipc::handle_request(&s, &call(serde_json::json!({"tz": 0})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": true, "feed": "ledger", "readable": false}),
            "on by default; no ledger answered"
        );
        let hello = super::super::ipc::handle_request(
            &s,
            &Request {
                id: 2,
                method: "hello".to_string(),
                params: serde_json::json!({}),
            },
        );
        let events = hello.result.unwrap()["events"].clone();
        assert!(
            events
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(super::super::ipc::EVENT_USAGE_CHANGED))
        );
    }

    #[test]
    fn usage_changed_is_one_pulse_per_tick_and_only_with_the_feed_on() {
        let (_dir, s) = shared();
        let mut rx = s.events.subscribe();
        let added: Vec<RoutedExchange> = (1..=5).map(|id| row(id, now())).collect();
        s.settings.lock().unwrap().insights_ledger_feed = false;
        publish_usage_changed(&s, &added);
        assert!(rx.try_recv().is_err(), "off: nothing published");

        s.settings.lock().unwrap().insights_ledger_feed = true;
        publish_usage_changed(&s, &[]);
        assert!(rx.try_recv().is_err(), "nothing added: nothing published");
        publish_usage_changed(&s, &added);
        let event = rx.try_recv().expect("one pulse");
        assert_eq!(event.event, super::super::ipc::EVENT_USAGE_CHANGED);
        assert_eq!(event.data, serde_json::json!({}));
        assert!(rx.try_recv().is_err(), "coalesced to one per tick");
    }

    /// The off switch through the path a shell takes: `set_settings` with
    /// `insights_ledger_feed: false` stops all three readers of feed L --
    /// the glance, the per-call `tokens` and the `usage_changed` pulse --
    /// and a daemon loaded again from the same store keeps it off.
    #[test]
    fn turning_the_feed_off_through_set_settings_stops_every_reader() {
        let (dir, store) = crate::config::tests_support::temp_store();
        let s = DaemonShared::load(store.clone()).unwrap();
        let mut r = row(1, Utc::now());
        r.input_tokens = Some(7);
        let ledger =
            || crate::routing::ironwire::IronWireLedger::with_rows_for_test(vec![r.clone()]);
        s.install_routing_ledger_for_test(ledger());
        let request = |method: &str, params: serde_json::Value| Request {
            id: 1,
            method: method.to_string(),
            params,
        };
        let calls = |s: &DaemonShared| {
            super::super::ipc::handle_request(s, &request("inference_calls", serde_json::json!({})))
                .result
                .unwrap()
        };
        assert_eq!(calls(&s)["calls"][0]["tokens"]["input"], 7, "on by default");

        let written = super::super::ipc::handle_request(
            &s,
            &request(
                "set_settings",
                serde_json::json!({"insights_ledger_feed": false}),
            ),
        );
        assert_eq!(written.result.unwrap()["insights_ledger_feed"], false);

        let check_off = |s: &DaemonShared| {
            assert_eq!(
                handle_glance(s, &call(serde_json::json!({"tz": 0})))
                    .result
                    .unwrap(),
                serde_json::json!({"enabled": false, "feed": "ledger"})
            );
            let page = calls(s);
            assert!(!page["calls"].as_array().unwrap().is_empty());
            assert!(page["calls"][0].get("tokens").is_none(), "{page}");
            let mut rx = s.events.subscribe();
            publish_usage_changed(s, &[r.clone()]);
            assert!(rx.try_recv().is_err(), "off: no usage_changed");
        };
        check_off(&s);

        drop(s);
        let again = DaemonShared::load(store).unwrap();
        again.install_routing_ledger_for_test(ledger());
        check_off(&again);
        drop(dir);
    }

    /// Owner decision D3, settled 2026-10-09: with the setting never touched,
    /// a tick that added a call publishes the pulse.
    #[test]
    fn usage_changed_is_published_by_default() {
        let (_dir, s) = shared();
        let mut rx = s.events.subscribe();
        publish_usage_changed(&s, &[row(1, now())]);
        let event = rx.try_recv().expect("on by default: one pulse");
        assert_eq!(event.event, super::super::ipc::EVENT_USAGE_CHANGED);
    }
}
