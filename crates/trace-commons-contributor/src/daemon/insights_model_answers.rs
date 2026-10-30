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
//! (`insights::model_answers`) answers with. Money appears only on the
//! `docs_cost` row, as `{kind: "usd_micros", value}`: in `figure` at API list
//! prices, and in `near_ai_charge.median` as what NEAR AI charged, from the
//! route tallies (owner decision D5, settled 2026-10-20), never added to the
//! list-priced figure. The one digest allowed is a shipped price table's
//! `table_id`. No session
//! reference, path, folder or ranking word crosses. Labels are declared
//! labels, not verified serving identities.
//!
//! The person's own labels on their sessions (`insights_session_label`)
//! are read here, by each row's reference: their task replaces the inferred
//! one, and their outcomes cross as counts only, in `kept_by_model` and each
//! rejection figure's `kept`.

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
    let ledger_present = ledger_feed && super::insights_week::any_ledger(shared);
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
                role: crate::daemon::insights_week::CandidateRole::Watched,
            };
            self.pass
                .run_pass(&[candidate], &NeverFolders::NONE, now(), QUIET, &mut |_| {
                    (PROJECT.to_string(), false)
                })
                .unwrap();
        }

        /// One pass over generated sessions, all in `PROJECT`.
        fn run_generated(&self, sessions: &[(String, Vec<u8>)]) {
            let candidates: Vec<CounterCandidate> = sessions
                .iter()
                .map(|(name, bytes)| {
                    let path = self.dir.path().join(name);
                    std::fs::write(&path, bytes).unwrap();
                    CounterCandidate {
                        origin: 0,
                        source: SOURCE_CLAUDE_CODE,
                        path,
                        size_bytes: bytes.len() as u64,
                        modified_at: written(),
                        started_at: None,
                        group_member_count: 0,
                        role: crate::daemon::insights_week::CandidateRole::Watched,
                    }
                })
                .collect();
            self.pass
                .run_pass(&candidates, &NeverFolders::NONE, now(), QUIET, &mut |_| {
                    (PROJECT.to_string(), false)
                })
                .unwrap();
        }

        /// Ten documentation sessions on two priced models, five each, so
        /// the docs cost row meets its floor and carries money.
        fn docs_priced() -> Self {
            let f = Self::new();
            let mut sessions = Vec::new();
            for n in 0..5u64 {
                sessions.push((
                    format!("PRIVATE-docs-a{n}.jsonl"),
                    docs_session(&format!("PRIVATE-A{n}"), SMALLER, 100 + n * 10),
                ));
                sessions.push((
                    format!("PRIVATE-docs-b{n}.jsonl"),
                    docs_session(&format!("PRIVATE-B{n}"), LARGER, 4_000 + n * 100),
                ));
            }
            f.run_generated(&sessions);
            f
        }

        /// `docs_priced`, plus seven Docs sessions declaring `SMALLER` sent
        /// through NEAR AI and verified: six served by `SMALLER` (one with
        /// an unknown charge), and one served by another model, the case a
        /// declared label would misattribute.
        fn docs_charged() -> Self {
            let f = Self::docs_priced();
            let sessions: Vec<(String, Vec<u8>)> = (0..7u64)
                .map(|n| {
                    (
                        format!("PRIVATE-docs-near{n}.jsonl"),
                        docs_session(&format!("PRIVATE-NEAR{n}"), SMALLER, 200 + n),
                    )
                })
                .collect();
            f.run_generated(&sessions);
            let call = |id: i64, session: &str, served: &str, usd: Option<f64>| {
                crate::routing::RoutedExchange {
                    id: Some(id),
                    started_at: now() - Duration::minutes(30 - id),
                    client_session_id: Some(session.to_string()),
                    facade: "anthropic".to_string(),
                    path: Some("/v1/messages".to_string()),
                    backend: "SECRET-BACKEND".to_string(),
                    served_model: Some(served.to_string()),
                    input_tokens: Some(100),
                    cache_read_tokens: Some(40),
                    cache_write_tokens: Some(0),
                    output_tokens: Some(10),
                    cost_usd: usd,
                    proof: Some(crate::routing::ProofStatus::Verified),
                    ..Default::default()
                }
            };
            let ledger = crate::routing::FixedLedger::new(vec![
                call(1, "PRIVATE-NEAR0", SMALLER, Some(0.0125)),
                call(2, "PRIVATE-NEAR1", SMALLER, Some(0.025)),
                call(3, "PRIVATE-NEAR1", SMALLER, Some(0.0125)),
                call(4, "PRIVATE-NEAR2", SMALLER, Some(0.01)),
                call(5, "PRIVATE-NEAR2", SMALLER, None),
                call(6, "PRIVATE-NEAR3", SMALLER, Some(0.02)),
                call(7, "PRIVATE-NEAR4", SMALLER, Some(0.03)),
                call(8, "PRIVATE-NEAR5", SMALLER, Some(0.04)),
                call(9, "PRIVATE-NEAR6", "PRIVATE-SERVED-glm", Some(0.05)),
            ]);
            f.pass
                .fold_ledger(&|| true, &ledger, &mut Vec::new, now())
                .unwrap();
            assert_eq!(
                f.pass.tallied_sessions_for_test(),
                7,
                "the calls were folded"
            );
            f
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

    /// Labels a shipped Anthropic price table names.
    const SMALLER: &str = "claude-sonnet-5-5";
    const LARGER: &str = "claude-opus-5-5";

    /// A Claude Code session whose only edit is to a Markdown file, so it
    /// reads as documentation work, with one turn on `model`.
    fn docs_session(id: &str, model: &str, output_tokens: u64) -> Vec<u8> {
        let common = |uuid: &str, at: &str| {
            serde_json::json!({
                "sessionId": id,
                "cwd": PROJECT,
                "version": "2.0.0",
                "uuid": uuid,
                "timestamp": at,
            })
        };
        let line = |kind: &str, at: &str, uuid: &str, message: serde_json::Value| {
            let mut value = common(uuid, at);
            value["type"] = kind.into();
            value["message"] = message;
            value.to_string()
        };
        let usage = serde_json::json!({
            "input_tokens": 50,
            "output_tokens": output_tokens,
            "cache_read_input_tokens": 1000,
            "cache_creation_input_tokens": 200,
            "cache_creation": {
                "ephemeral_5m_input_tokens": 200,
                "ephemeral_1h_input_tokens": 0
            },
            "service_tier": "standard"
        });
        [
            line(
                "user",
                "2026-09-14T09:00:00.000Z",
                "u1",
                serde_json::json!({"role": "user", "content": "PRIVATE_PROMPT update the readme"}),
            ),
            line(
                "assistant",
                "2026-09-14T09:00:05.000Z",
                "u2",
                serde_json::json!({
                    "id": "msg_PRIVATE_1",
                    "model": model,
                    "role": "assistant",
                    "content": [{"type": "tool_use", "id": "toolu_PRIVATE_A", "name": "Edit",
                        "input": {"file_path": "/Users/PRIVATE_USER/code/PRIVATE_REPO/README.md",
                            "old_string": "PRIVATE_OLD", "new_string": "PRIVATE_NEW"}}],
                    "usage": usage,
                }),
            ),
            line(
                "user",
                "2026-09-14T09:00:06.000Z",
                "u3",
                serde_json::json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": "toolu_PRIVATE_A", "content": "ok"}]}),
            ),
        ]
        .join("\n")
        .into_bytes()
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
    fn switched_off_without_opening_the_store_or_making_a_key() {
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
        s.settings.lock().unwrap().insights_counter_pass = false;
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
        s.settings.lock().unwrap().insights_counter_pass = false;
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
        // The session counts when its folder is not Never.
        assert_ne!(shown["rows"], empty["rows"]);
        let across = &shown["rows"][4];
        assert_eq!(across["question"], "rejected_across_tasks");
        assert!(!across["sessions_by_model"].as_array().unwrap().is_empty());
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

    /// Every figure of `kind` in a payload, with its row's question.
    fn figures<'v>(
        value: &'v serde_json::Value,
        kind: &str,
    ) -> Vec<(&'v str, &'v serde_json::Value)> {
        let mut found = Vec::new();
        for row in value["rows"].as_array().unwrap() {
            for side in ["leader", "next"] {
                let figure = &row[side]["figure"];
                if figure["kind"] == kind {
                    found.push((row["question"].as_str().unwrap(), figure));
                }
            }
        }
        found
    }

    /// A whole answer, printed between markers for shells' decode fixtures
    /// (`cargo test ... -- --nocapture`): the Docs cost row answered at API
    /// prices with a NEAR AI charge beside it, and the synthetic Claude Code
    /// session on the other rows, which are short of sessions.
    #[test]
    fn a_model_answers_answer_for_shell_fixtures() {
        let f = Fixture::docs_charged();
        f.run();
        let value = f.answers(&[], true);
        assert_eq!(value["rows"][3]["state"], "answered", "{value}");
        assert!(value["rows"][3]["near_ai_charge"].is_object(), "{value}");
        // The shells decode this answer from the committed fixture, so the
        // fixture is this answer: a change to the wire fails here first.
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/insights-analytics/insights_model_answers.json"
        ))
        .unwrap();
        println!("BEGIN insights_model_answers");
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        println!("END insights_model_answers");
        assert_eq!(
            value, fixture,
            "re-record tests/fixtures/insights-analytics/insights_model_answers.json \
             from this test's --nocapture output"
        );
    }

    #[test]
    fn the_priced_fixture_answers_docs_cost_with_money() {
        let v = Fixture::docs_priced().answers(&[], true);
        let money = figures(&v, "usd_micros");
        assert_eq!(money.len(), 2, "leader and next: {v}");
        // The wire form: exactly a kind and an integer value.
        for (question, figure) in &money {
            assert_eq!(*question, "docs_cost");
            assert_eq!(figure.as_object().unwrap().len(), 2, "{figure}");
            assert!(figure["value"].is_u64(), "{figure}");
        }
        assert_eq!(v["rows"][3]["leader"]["label"], SMALLER);
        assert_eq!(v["rows"][3]["next"]["label"], LARGER);
        let tables = v["pricing"]["tables"].as_array().unwrap();
        assert!(!tables.is_empty(), "a priced session names its table");
        for table in tables {
            assert!(table["table_id"].as_str().unwrap().starts_with("sha256:"));
        }
    }

    #[test]
    fn the_near_ai_charge_crosses_as_its_own_figure() {
        let f = Fixture::docs_charged();
        let v = f.answers(&[], true);
        let q4 = &v["rows"][3];
        assert_eq!(q4["question"], "docs_cost");
        // Every Docs session sent through NEAR AI, whoever served it.
        let charge = serde_json::json!({
            "basis": "near_ai_ledger",
            "median": {"kind": "usd_micros", "value": 33_750},
            "routed_sessions": 7,
            "charge_unknown": 1,
        });
        assert_eq!(q4["near_ai_charge"], charge, "{v}");
        // The routed sessions are out of the list-priced figures, which are
        // those of the unrouted sessions alone.
        assert_eq!(q4["left_out"]["routed"], 7);
        let plain = Fixture::docs_priced().answers(&[], true);
        for side in ["leader", "next"] {
            assert_eq!(q4[side]["figure"], plain["rows"][3][side]["figure"]);
            assert_eq!(q4[side]["sessions"], plain["rows"][3][side]["sessions"]);
        }
        assert_eq!(q4["leader"]["label"], SMALLER);
        // The label's figure takes only the sessions NEAR AI proved it
        // served: not the one served by another model.
        let served = serde_json::json!({
            "basis": "near_ai_ledger",
            "median": {"kind": "usd_micros", "value": 30_000},
            "routed_sessions": 6,
            "charge_unknown": 1,
        });
        assert_eq!(q4["leader"]["near_ai_charge"], served);
        assert!(q4["next"]["near_ai_charge"].is_null());
        assert!(plain["rows"][3]["near_ai_charge"].is_null());
        for (ix, row) in v["rows"].as_array().unwrap().iter().enumerate() {
            if ix != 3 {
                assert!(row["near_ai_charge"].is_null(), "{row}");
            }
        }
        // With the feed off no stored charge is read, as no route is.
        let off = f.answers(&[], false);
        assert!(off["rows"][3]["near_ai_charge"].is_null(), "{off}");
    }

    /// The ledger keeps calls for 24 hours. The charge is kept in the store
    /// when it is folded, so it is still answered after the ledger has let
    /// the calls go and the daemon has started again.
    #[test]
    fn the_near_ai_charge_outlives_the_ledger_and_a_restart() {
        let f = Fixture::docs_charged();
        let before = f.answers(&[], true)["rows"][3]["near_ai_charge"].clone();
        assert!(before.is_object());
        let empty = crate::routing::FixedLedger::new(Vec::new());
        f.pass
            .fold_ledger(&|| true, &empty, &mut Vec::new, now())
            .unwrap();
        assert_eq!(f.answers(&[], true)["rows"][3]["near_ai_charge"], before);
        let reopened = CounterPass::with_keys(
            f.dir.path().join(COUNTER_ROWS_FILE),
            Box::new(InMemoryDigestKeyStore::with_seed([3; 32])),
        );
        // The same seed loads the same key, as the keychain would.
        reopened.keys_load_or_create_for_test();
        let again = reopened.model_answers_value(true, &NeverFolders::NONE, true, true, now());
        assert_eq!(again["rows"][3]["near_ai_charge"], before);
        // The store holds the charge as integer micros, never the amount
        // as the ledger wrote it, an id, the backend or a list price.
        let text = std::fs::read_to_string(f.dir.path().join(COUNTER_ROWS_FILE)).unwrap();
        assert!(text.contains("\"usd_micros\":12500"), "charge kept");
        for leak in ["0.0125", "PRIVATE-NEAR", "SECRET-BACKEND", "cost", "price"] {
            assert!(!text.contains(leak), "{leak} in the store");
        }
    }

    #[test]
    fn nothing_identifying_crosses() {
        let plain = Fixture::new();
        plain.run();
        let priced = Fixture::docs_priced();
        let charged = Fixture::docs_charged();
        for f in [&plain, &priced, &charged] {
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
                "toolu_",
                "\"t0\"",
                "\"t1\"",
                "session_ref",
                "session.jsonl",
                ".jsonl",
                "README",
                "SECRET-BACKEND",
                "SERVED",
                "glm",
                "backend",
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
        // The exemption was used: the priced payload carries a table id.
        let value = priced.answers(&[], true);
        assert!(!value["pricing"]["tables"].as_array().unwrap().is_empty());
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
        let plain = Fixture::new();
        plain.run();
        let priced = Fixture::docs_priced();
        let charged = Fixture::docs_charged();
        for f in [&plain, &priced, &charged] {
            let value = f.answers(&[], true);
            let mut hits = Vec::new();
            walk(&value, &mut Vec::new(), &mut hits);
            assert!(hits.is_empty(), "money-shaped keys crossed: {hits:?}");
        }
        // The value itself is allowed only under kind usd_micros, on
        // docs_cost; the priced payload really has some.
        let value = priced.answers(&[], true);
        let money = figures(&value, "usd_micros");
        assert!(!money.is_empty(), "no money reached the walk: {value}");
        assert!(money.iter().all(|(question, _)| *question == "docs_cost"));
        // No other question carries a figure of money's kind or any value
        // outside a typed figure.
        for row in value["rows"].as_array().unwrap() {
            if row["question"] != "docs_cost" {
                for side in ["leader", "next"] {
                    assert_ne!(row[side]["figure"]["kind"], "usd_micros");
                }
            }
        }
        // The NEAR AI charge is money of the same kind, labelled, on the
        // docs_cost row only.
        let value = charged.answers(&[], true);
        for row in value["rows"].as_array().unwrap() {
            let charge = &row["near_ai_charge"];
            if row["question"] == "docs_cost" {
                assert_eq!(charge["basis"], "near_ai_ledger");
                assert_eq!(charge["median"]["kind"], "usd_micros");
                assert!(charge["median"]["value"].is_u64());
                assert_eq!(charge["median"].as_object().unwrap().len(), 2);
            } else {
                assert!(charge.is_null(), "{row}");
            }
        }
    }

    #[test]
    fn no_label_reads_as_a_ranking_claim() {
        let f = Fixture::new();
        f.run();
        let priced = Fixture::docs_priced();
        let mut all = Vec::new();
        words(&f.answers(&[], true), &mut all);
        words(&f.answers(&[], false), &mut all);
        words(&priced.answers(&[], true), &mut all);
        words(&priced.answers(&[], false), &mut all);
        // Leader and next labels are among the words walked.
        assert!(all.iter().any(|word| word == SMALLER));
        assert!(all.iter().any(|word| word == LARGER));
        for word in all {
            for banned in ["best", "better", "fastest", "benchmark", "winner"] {
                assert!(!word.contains(banned), "{banned} in {word}");
            }
        }
    }

    /// A pending ledger call keyed to the fixture session: with routing
    /// available it reads as routed and unverified, so the session is left
    /// out of the rejection comparison and counted in `left_out.routed`.
    fn routed_count(feed: bool, ledger_present: bool) -> u64 {
        let f = Fixture::new();
        f.run();
        let call = crate::routing::RoutedExchange {
            id: Some(1),
            started_at: now() - Duration::minutes(30),
            client_session_id: Some("11111111-2222-4333-8444-PRIVATESESS1".to_string()),
            facade: "anthropic".to_string(),
            path: Some("/v1/messages".to_string()),
            backend: "SECRET-BACKEND".to_string(),
            input_tokens: Some(100),
            cache_read_tokens: Some(40),
            cache_write_tokens: Some(0),
            output_tokens: Some(10),
            proof: Some(crate::routing::ProofStatus::Pending),
            ..Default::default()
        };
        let ledger = crate::routing::FixedLedger::new(vec![call]);
        f.pass
            .fold_ledger(&|| true, &ledger, &mut Vec::new, now())
            .unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 1, "the call was folded");
        let value =
            f.pass
                .model_answers_value(true, &NeverFolders::NONE, feed, ledger_present, now());
        let across = &value["rows"][4];
        assert_eq!(across["question"], "rejected_across_tasks");
        across["left_out"]["routed"].as_u64().unwrap()
    }

    #[test]
    fn with_the_feed_off_or_no_ledger_every_session_is_unobserved() {
        assert_eq!(routed_count(true, true), 1, "routing available");
        assert_eq!(routed_count(false, true), 0, "feed off");
        assert_eq!(routed_count(true, false), 0, "no ledger declared");
        assert_eq!(routed_count(false, false), 0);
    }

    /// The person's own labels (`insights_session_label`): their task
    /// moves a session off the inferred task's rows and onto theirs, and
    /// their outcomes cross as `kept` on the rejection rows.
    #[test]
    fn the_persons_labels_override_the_inferred_task_and_count_as_kept() {
        let f = Fixture::docs_priced();
        let before = f.answers(&[], true);
        let docs = &before["rows"][3];
        assert_eq!(docs["question"], "docs_cost");
        assert_eq!(
            docs["sessions_by_model"],
            serde_json::json!([[LARGER, 5], [SMALLER, 5]])
        );
        let listed = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 200);
        let references: Vec<String> = listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                assert_eq!(row["inferred_task"], "docs");
                row["session_ref"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(references.len(), 10);
        for (ix, reference) in references.iter().enumerate() {
            let outcome = if ix % 2 == 0 { "kept" } else { "thrown_away" };
            let (reference, change) = super::super::insights_sessions::label_params(
                &serde_json::json!({"session_ref": reference, "task": "bug_fix", "outcome": outcome}),
            )
            .unwrap();
            f.pass
                .label_value(true, &reference, change, &NeverFolders::NONE, &[])
                .unwrap();
        }
        let after = f.answers(&[], true);
        assert_eq!(
            after["rows"][3]["sessions_by_model"],
            serde_json::json!([]),
            "no longer Docs"
        );
        let bug_fix = &after["rows"][0];
        assert_eq!(bug_fix["question"], "bug_fix_rejected");
        assert_eq!(
            bug_fix["sessions_by_model"],
            serde_json::json!([[LARGER, 5], [SMALLER, 5]])
        );
        let kept = bug_fix["kept_by_model"].as_array().unwrap();
        assert_eq!(kept.len(), 2);
        let mut labelled = 0;
        for entry in kept {
            let figure = &entry[1];
            assert_eq!(
                figure["kept"].as_u64().unwrap() + figure["thrown_away"].as_u64().unwrap(),
                figure["labelled"].as_u64().unwrap()
            );
            labelled += figure["labelled"].as_u64().unwrap();
        }
        assert_eq!(labelled, 10);
        // A Never folder takes its sessions' labels out with them.
        let hidden = f.answers(&[PROJECT], true);
        assert_eq!(hidden["rows"][0]["kept_by_model"], serde_json::json!([]));
    }

    #[test]
    fn insights_week_carries_list_prices_only_in_work() {
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
        // List-price figures cross in `work` only (product owner,
        // 2026-10-29); NEAR AI money in `near_ai_credit`, null here.
        assert_eq!(week["work"]["basis"], "api_list");
        assert_eq!(week["near_ai_credit"], serde_json::Value::Null);
        let mut rest = week.clone();
        rest.as_object_mut().unwrap().remove("work");
        let mut all = Vec::new();
        words(&rest, &mut all);
        for word in all {
            for banned in ["usd", "price", "cost"] {
                assert!(!word.contains(banned), "{banned} in {word}");
            }
        }
    }
}
