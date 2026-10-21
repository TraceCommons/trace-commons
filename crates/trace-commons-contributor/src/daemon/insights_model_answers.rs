//! `insights_model_answers`: the Models card's six answers over the counter
//! pass (feed T).
//!
//! Gated exactly as `insights_sessions` is: off answers `enabled: false`
//! without opening the store or the key; on, a store, key or folder-rule
//! read that fails answers `readable: false` with its fixed reason, never an
//! empty list. Sessions in a Never folder enter no answer.
//!
//! # What crosses the socket
//!
//! Counts, fixed snake_case labels and the declared model labels the engine
//! (`insights::model_answers`) answers with. Money appears only as
//! `figure.value` under `kind: "usd_micros"` on the `docs_cost` row; the one
//! digest allowed is a shipped price table's `table_id`. No session
//! reference, path, folder or ranking word crosses. Labels are declared
//! labels, not verified serving identities.

use super::insights_sessions::gate;
use super::ipc::{DaemonShared, Request, Response};

/// `insights_model_answers`.
pub fn handle_model_answers(shared: &DaemonShared, req: &Request) -> Response {
    let (enabled, never) = match gate(shared) {
        Ok(gate) => gate,
        Err(answer) => return Response::ok(req.id, answer),
    };
    // A poisoned lock reads as off: fail closed, never a guess.
    let ledger_feed = shared
        .settings
        .lock()
        .map(|settings| settings.insights_ledger_feed)
        .unwrap_or(false);
    // Asked after the settings lock is let go. Only whether a ledger is
    // declared: the answer never reads the ledger itself.
    let ledger_present = ledger_feed && shared.routing_ledger().is_some();
    Response::ok(
        req.id,
        shared.insights_counter.model_answers_value(
            enabled,
            &never,
            ledger_feed,
            ledger_present,
            chrono::Utc::now(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use chrono::{DateTime, Duration, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    use super::super::insights_week::{
        COUNTER_ROWS_FILE, CounterCandidate, CounterPass, NeverFolders, UNREADABLE_POLICY,
    };
    use super::*;
    use crate::source::SOURCE_CLAUDE_CODE;

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";

    fn written() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        written() + Duration::hours(2)
    }

    struct Fixture {
        dir: tempfile::TempDir,
        pass: CounterPass,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([3; 32])),
            );
            Self { dir, pass }
        }

        /// One pass over the synthetic Claude Code session.
        fn run(&self) {
            let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl");
            let candidate = CounterCandidate {
                origin: 0,
                source: SOURCE_CLAUDE_CODE,
                path: path.clone(),
                size_bytes: std::fs::metadata(&path).unwrap().len(),
                modified_at: written(),
                started_at: None,
                group_member_count: 0,
            };
            self.pass
                .run_pass(&[candidate], &NeverFolders::NONE, now(), QUIET, &mut |_| {
                    (PROJECT.to_string(), false)
                })
                .unwrap();
        }

        fn answers(&self, never: &[&str], feed: bool) -> serde_json::Value {
            self.pass.model_answers_value(
                true,
                &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                feed,
                feed,
                now(),
            )
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(method: &str) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params: serde_json::json!({}),
        }
    }

    /// Every key and string value in a payload, lowercased.
    fn words(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    out.push(key.to_lowercase());
                    words(inner, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| words(i, out)),
            serde_json::Value::String(text) => out.push(text.to_lowercase()),
            _ => {}
        }
    }

    #[test]
    fn off_by_default_without_opening_the_store_or_making_a_key() {
        let f = Fixture::new();
        let off = f
            .pass
            .model_answers_value(false, &NeverFolders::NONE, true, true, now());
        assert_eq!(
            off,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        assert!(f.pass.key_is_absent_for_test(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        let (_dir, s) = shared();
        let r = handle_model_answers(&s, &call("insights_model_answers"));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
    }

    #[test]
    fn an_unreadable_policy_store_or_key_answers_readable_false() {
        let f = Fixture::new();
        std::fs::write(f.dir.path().join(COUNTER_ROWS_FILE), b"{not json").unwrap();
        f.pass.keys_load_or_create_for_test();
        let value = f.answers(&[], true);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["readable"], false);
        assert!(value["reason"].is_string());
        assert!(value.get("rows").is_none(), "never an empty answer");

        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _held = s.policy.lock().unwrap();
                    panic!("poison the policy lock");
                })
                .join();
        });
        let r = handle_model_answers(&s, &call("insights_model_answers"));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({
                "enabled": true,
                "feed": "counter_pass",
                "readable": false,
                "reason": UNREADABLE_POLICY,
            })
        );
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        let method = "insights_model_answers";
        assert!(super::super::ipc::METHODS.contains(&method));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        let r = super::super::ipc::handle_request(&s, &call(method));
        assert_eq!(r.result.unwrap()["enabled"], false);
    }

    #[test]
    fn never_folder_sessions_enter_no_answer() {
        let f = Fixture::new();
        f.run();
        let empty = Fixture::new().answers(&[], true);
        let hidden = f.answers(&[PROJECT], true);
        assert_eq!(hidden["readable"], true);
        assert_eq!(hidden["rows"], empty["rows"]);
        let shown = f.answers(&[], true);
        assert_eq!(shown["rows"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn the_payload_has_the_documented_shape() {
        let f = Fixture::new();
        f.run();
        let v = f.answers(&[], true);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["feed"], "counter_pass");
        assert_eq!(v["readable"], true);
        assert_eq!(v["pricing"]["basis"], "api_list");
        assert!(v["pricing"]["tables"].is_array());
        assert!(v["window_start"].is_string() && v["window_end"].is_string());
        assert!(v.get("updated_at").is_some());
        let questions: Vec<&str> = v["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["question"].as_str().unwrap())
            .collect();
        assert_eq!(
            questions,
            [
                "bug_fix_rejected",
                "refactor_rejected",
                "tests_rejected",
                "docs_cost",
                "rejected_across_tasks",
                "finish_time"
            ]
        );
    }

    #[test]
    fn nothing_identifying_crosses() {
        let f = Fixture::new();
        f.run();
        let value = f.answers(&[], true);
        let mut text = serde_json::to_string(&value).unwrap();
        // The pinned price tables' content digests are the one digest.
        for table in value["pricing"]["tables"].as_array().unwrap() {
            text = text.replace(table["table_id"].as_str().unwrap(), "TABLE");
        }
        for forbidden in [
            "PRIVATE",
            "/Users",
            "msg_",
            "\"t0\"",
            "\"t1\"",
            "session_ref",
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} crossed: {text}");
        }
        let hex_run = text
            .split(|c: char| !c.is_ascii_hexdigit())
            .map(str::len)
            .max()
            .unwrap_or(0);
        assert!(hex_run < 16, "a digest-like run crossed: {text}");
    }

    #[test]
    fn money_crosses_only_as_usd_micros_on_the_docs_cost_row() {
        fn walk(value: &serde_json::Value, path: &mut Vec<String>, hits: &mut Vec<Vec<String>>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, inner) in map {
                        let k = key.to_lowercase();
                        path.push(key.clone());
                        // `left_out.unpriced` is a count of sessions, not money.
                        let count = k == "unpriced"
                            && path.len() >= 2
                            && path[path.len() - 2] == "left_out";
                        if !count
                            && ["usd", "price", "cost", "dollar", "micros"]
                                .iter()
                                .any(|word| k.contains(word))
                        {
                            hits.push(path.clone());
                        }
                        walk(inner, path, hits);
                        path.pop();
                    }
                }
                serde_json::Value::Array(items) => {
                    for (ix, inner) in items.iter().enumerate() {
                        path.push(ix.to_string());
                        walk(inner, path, hits);
                        path.pop();
                    }
                }
                _ => {}
            }
        }
        let f = Fixture::new();
        f.run();
        let value = f.answers(&[], true);
        let mut hits = Vec::new();
        walk(&value, &mut Vec::new(), &mut hits);
        assert!(hits.is_empty(), "money-shaped keys crossed: {hits:?}");
        // The value itself is allowed only under kind usd_micros.
        for row in value["rows"].as_array().unwrap() {
            for side in ["leader", "next"] {
                let figure = &row[side]["figure"];
                if figure["kind"] == "usd_micros" {
                    assert_eq!(row["question"], "docs_cost");
                }
            }
        }
    }

    #[test]
    fn no_label_reads_as_a_ranking_claim() {
        let f = Fixture::new();
        f.run();
        let mut all = Vec::new();
        words(&f.answers(&[], true), &mut all);
        words(&f.answers(&[], false), &mut all);
        for word in all {
            for banned in ["best", "better", "fastest", "benchmark", "winner"] {
                assert!(!word.contains(banned), "{banned} in {word}");
            }
        }
    }

    #[test]
    fn insights_week_still_carries_no_price() {
        let f = Fixture::new();
        f.run();
        let week = f.pass.week_value(
            true,
            None,
            chrono::FixedOffset::east_opt(0).unwrap(),
            now(),
            &NeverFolders::NONE,
            super::super::insights_week::WeekOptions::default(),
        );
        let mut all = Vec::new();
        words(&week, &mut all);
        for word in all {
            for banned in ["usd", "price", "cost"] {
                assert!(!word.contains(banned), "{banned} in {word}");
            }
        }
    }
}
