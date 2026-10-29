//! `insights_benchmarks`: per kind of work, which of the person's own
//! models did best on Quality (share of edits kept), Value (list-priced
//! cost per session) and Speed (active time per session), over the counter
//! pass (feed T). Product owner, 2026-10-29, list prices included.
//!
//! Gated exactly as `insights_model_answers` is: off answers `enabled:
//! false` without opening the store or the key; on, a store, key or
//! folder-rule read that fails answers `readable: false` with its fixed
//! reason. Sessions in a Never folder enter no answer.
//!
//! What crosses: counts, fixed snake_case keys, declared model labels,
//! permille, seconds and USD micros at API list prices, and a shipped price
//! table's `table_id`. No session reference, path, folder or backend.

use super::insights_sessions::gate;
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use crate::insights::benchmarks::{BENCHMARK_DEFAULT_WEEKS, BENCHMARK_WEEKS, BenchTool};

/// `weeks` that is not 4 or 13.
pub const ERR_BENCHMARKS_WEEKS_INVALID: &str = "weeks_invalid";
/// `tool` that is not `claude_code`, `codex` or null.
pub const ERR_BENCHMARKS_TOOL_INVALID: &str = "tool_invalid";

/// The params: `weeks` (4 or 13, default 4) and `tool` (a tool word or
/// null, default both). Absent and `null` read as the default.
pub(crate) fn benchmark_params(
    params: &serde_json::Value,
) -> Result<(u32, Option<BenchTool>), &'static str> {
    let weeks = match params.get("weeks") {
        None | Some(serde_json::Value::Null) => BENCHMARK_DEFAULT_WEEKS,
        Some(value) => value
            .as_u64()
            .and_then(|weeks| u32::try_from(weeks).ok())
            .filter(|weeks| BENCHMARK_WEEKS.contains(weeks))
            .ok_or(ERR_BENCHMARKS_WEEKS_INVALID)?,
    };
    let tool = match params.get("tool") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .and_then(BenchTool::from_wire)
                .ok_or(ERR_BENCHMARKS_TOOL_INVALID)?,
        ),
    };
    Ok((weeks, tool))
}

/// `insights_benchmarks`.
pub fn handle_benchmarks(shared: &DaemonShared, req: &Request) -> Response {
    let (weeks, tool) = match benchmark_params(&req.params) {
        Ok(params) => params,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
    };
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
    let routing_available = ledger_feed && super::insights_week::any_ledger(shared);
    Response::ok(
        req.id,
        shared.insights_counter.benchmarks_value(
            enabled,
            &never,
            routing_available,
            weeks,
            tool,
            chrono::Utc::now(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    use super::super::insights_week::{
        COUNTER_ROWS_FILE, CounterCandidate, CounterPass, NeverFolders,
    };
    use super::*;
    use crate::source::SOURCE_CLAUDE_CODE;

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";
    const SMALLER: &str = "claude-sonnet-5-5";
    const LARGER: &str = "claude-opus-5-5";

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
                    }
                })
                .collect();
            self.pass
                .run_pass(&candidates, &NeverFolders::NONE, now(), QUIET, &mut |_| {
                    (PROJECT.to_string(), false)
                })
                .unwrap();
        }

        /// Ten documentation sessions, five on each of two priced models.
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

        fn answer(&self, never: &[&str], weeks: u32, tool: Option<BenchTool>) -> serde_json::Value {
            self.pass.benchmarks_value(
                true,
                &NeverFolders::These(never.iter().map(|k| k.to_string()).collect()),
                false,
                weeks,
                tool,
                now(),
            )
        }
    }

    /// A Claude Code session whose only edit is to a Markdown file, with
    /// one turn on `model`.
    fn docs_session(id: &str, model: &str, output_tokens: u64) -> Vec<u8> {
        let line = |kind: &str, at: &str, uuid: &str, message: serde_json::Value| {
            serde_json::json!({
                "sessionId": id,
                "cwd": PROJECT,
                "version": "2.0.0",
                "uuid": uuid,
                "timestamp": at,
                "type": kind,
                "message": message,
            })
            .to_string()
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

    fn call(params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: "insights_benchmarks".to_string(),
            params,
        }
    }

    #[test]
    fn params_default_to_four_weeks_and_both_tools_and_refuse_bad_values() {
        let ok = |params| benchmark_params(&params);
        assert_eq!(ok(serde_json::json!({})), Ok((4, None)));
        assert_eq!(ok(serde_json::Value::Null), Ok((4, None)));
        assert_eq!(
            ok(serde_json::json!({"weeks": null, "tool": null})),
            Ok((4, None))
        );
        assert_eq!(
            ok(serde_json::json!({"weeks": 13, "tool": "codex"})),
            Ok((13, Some(BenchTool::Codex)))
        );
        assert_eq!(
            ok(serde_json::json!({"weeks": 4, "tool": "claude_code"})),
            Ok((4, Some(BenchTool::ClaudeCode)))
        );
        for bad in [
            serde_json::json!({"weeks": 5}),
            serde_json::json!({"weeks": "4"}),
            serde_json::json!({"weeks": -4}),
            serde_json::json!({"weeks": 4.5}),
        ] {
            assert_eq!(ok(bad.clone()), Err(ERR_BENCHMARKS_WEEKS_INVALID), "{bad}");
        }
        for bad in [
            serde_json::json!({"tool": "cursor"}),
            serde_json::json!({"tool": 1}),
        ] {
            assert_eq!(ok(bad.clone()), Err(ERR_BENCHMARKS_TOOL_INVALID), "{bad}");
        }
        let (_dir, s) = shared();
        let r = handle_benchmarks(&s, &call(serde_json::json!({"weeks": 2})));
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn switched_off_without_opening_the_store_or_making_a_key() {
        let f = Fixture::new();
        let off = f
            .pass
            .benchmarks_value(false, &NeverFolders::NONE, false, 4, None, now());
        assert_eq!(
            off,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        assert!(f.pass.key_is_absent_for_test(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = handle_benchmarks(&s, &call(serde_json::json!({})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
    }

    #[test]
    fn an_unreadable_store_answers_readable_false() {
        let f = Fixture::new();
        std::fs::write(f.dir.path().join(COUNTER_ROWS_FILE), b"{not json").unwrap();
        f.pass.keys_load_or_create_for_test();
        let value = f.answer(&[], 4, None);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["readable"], false);
        assert!(value["reason"].is_string());
        assert!(value.get("categories").is_none(), "never an empty answer");
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        let method = "insights_benchmarks";
        assert!(super::super::ipc::METHODS.contains(&method));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&method));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = super::super::ipc::handle_request(&s, &call(serde_json::json!({"weeks": 13})));
        assert_eq!(r.result.unwrap()["enabled"], false);
    }

    #[test]
    fn the_payload_has_the_documented_shape_and_compares_two_models() {
        let f = Fixture::docs_priced();
        let v = f.answer(&[], 4, None);
        assert_eq!(v["enabled"], true);
        assert_eq!(v["feed"], "counter_pass");
        assert_eq!(v["readable"], true);
        assert!(v.get("updated_at").is_some());
        assert!(v["window_start"].is_string() && v["window_end"].is_string());
        assert_eq!(v["weeks"], 4);
        assert_eq!(v["pricing"]["basis"], "api_list");
        assert_eq!(v["pricing"]["tables"].as_array().unwrap().len(), 1);
        assert_eq!(v["sessions"], 10);
        assert_eq!(v["min_sessions"], 5);
        assert!(v["left_out"].is_object());
        let keys: Vec<&str> = v["categories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["key"].as_str().unwrap())
            .collect();
        assert_eq!(
            keys,
            [
                "bug_fix",
                "feature",
                "refactor",
                "tests",
                "docs",
                "understand",
                "review",
                "setup"
            ]
        );
        let docs = &v["categories"][4];
        assert_eq!(docs["sessions"], 10, "{v}");
        let models = docs["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        for model in models {
            assert_eq!(model["tool"], "claude_code");
            assert_eq!(model["sessions"], 5);
            assert_eq!(model["kept_permille"], 1000);
            assert!(model["usd_micros_median"].is_u64(), "{model}");
            assert!(model.get("active_secs_median").is_some());
        }
        // The smaller model writes less at a lower price.
        assert_eq!(docs["value"]["leader"]["label"], SMALLER, "{v}");
        assert_eq!(docs["value"]["next"]["label"], LARGER);
        assert!(
            docs["value"]["leader"]["figure"].as_u64() < docs["value"]["next"]["figure"].as_u64()
        );
        assert!(docs["quality"]["leader"]["figure"].is_u64());
        assert!(docs.get("tip").is_some());
        // Every other category is empty with no picks.
        assert_eq!(v["categories"][0]["sessions"], 0);
        assert_eq!(v["categories"][0]["quality"], serde_json::Value::Null);
    }

    #[test]
    fn the_tool_filter_and_never_folders_narrow_the_answer() {
        let f = Fixture::docs_priced();
        let codex = f.answer(&[], 13, Some(BenchTool::Codex));
        assert_eq!(codex["sessions"], 0);
        assert_eq!(codex["weeks"], 13);
        assert_eq!(codex["tool"], "codex");
        let claude = f.answer(&[], 4, Some(BenchTool::ClaudeCode));
        assert_eq!(claude["sessions"], 10);
        let hidden = f.answer(&[PROJECT], 4, None);
        assert_eq!(hidden["readable"], true);
        assert_eq!(hidden["sessions"], 0);
    }

    #[test]
    fn nothing_identifying_crosses() {
        let f = Fixture::docs_priced();
        let value = f.answer(&[], 4, None);
        let mut text = serde_json::to_string(&value).unwrap();
        for table in value["pricing"]["tables"].as_array().unwrap() {
            text = text.replace(table["table_id"].as_str().unwrap(), "TABLE");
        }
        for forbidden in [
            "PRIVATE",
            "/Users",
            "msg_",
            "toolu_",
            "session_ref",
            ".jsonl",
            "README",
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
}
