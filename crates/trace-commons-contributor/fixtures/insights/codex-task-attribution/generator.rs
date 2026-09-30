use anyhow::Context;
use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_protocol::protocol::EventMsg;
use codex_protocol::user_input::UserInput;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use wiremock::MockServer;

const OUTPUT_DIR: &str = "/private/tmp/trace-insights-codex-writer-fixtures";
const PINNED_REVISION: &str = "c4017a87aacc7558002b7cb510025e967c1d765e";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn writes_two_direct_profile_fixtures() -> Result<()> {
    let output_dir = Path::new(OUTPUT_DIR);
    std::fs::create_dir_all(output_dir)?;

    let alpha = write_fixture("model-alpha", "alpha", 1).await?;
    let beta = write_fixture("model-beta", "beta", 2).await?;
    assert_fixture_invariants(output_dir)?;

    let manifest = serde_json::json!({
        "schema_version": 1,
        "provenance": {
            "upstream_repository": "https://github.com/openai/codex",
            "upstream_revision": PINNED_REVISION,
            "workspace_package_version": env!("CARGO_PKG_VERSION"),
            "build_qualification_limit": "The workspace package version is build provenance for this pinned source checkout only; it does not map the fixture to a released or production Codex version.",
            "generator_test": "codex-core/all::suite::insights_direct_profile_writer_fixture::writes_two_direct_profile_fixtures",
            "transport": "localhost wiremock Responses SSE; no live provider",
            "sanitization": "deterministic value replacement after writer flush; record order, type, field presence, and JSON value kinds preserved"
        },
        "observed_profile": {
            "session_meta": {
                "source": "exec",
                "history_mode": "legacy",
                "originator": "codex_cli_rs"
            },
            "turn_context": {
                "collaboration_mode": "default",
                "multi_agent_version": "v1",
                "realtime_active": false
            },
            "top_level_record_types": [
                "session_meta",
                "event_msg",
                "response_item",
                "world_state",
                "turn_context",
                "token_usage_record"
            ],
            "event_msg_payload_types": [
                "task_started",
                "user_message",
                "agent_message",
                "token_count",
                "task_complete"
            ],
            "response_item_shapes": [
                "message/developer/permissions.instructions",
                "message/user/environments.environment_context",
                "message/user/user.text",
                "message/assistant/unknown"
            ],
            "not_qualified_by_these_fixtures": [
                "reasoning",
                "function_call",
                "function_call_output",
                "custom_tool_call",
                "custom_tool_call_output"
            ]
        },
        "fixtures": [alpha, beta]
    });
    std::fs::write(
        output_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

async fn write_fixture(model: &str, cohort: &str, turns: usize) -> Result<Value> {
    let server = MockServer::start().await;
    if turns == 1 {
        mount_sse_once(
            &server,
            sse(vec![
                ev_response_created("response-alpha-1"),
                ev_assistant_message("message-alpha-1", "synthetic alpha result"),
                ev_completed("response-alpha-1"),
            ]),
        )
        .await;
    } else {
        mount_sse_sequence(
            &server,
            vec![
                sse(vec![
                    ev_response_created("response-beta-1"),
                    ev_assistant_message("message-beta-1", "synthetic beta result one"),
                    ev_completed("response-beta-1"),
                ]),
                sse(vec![
                    ev_response_created("response-beta-2"),
                    ev_assistant_message("message-beta-2", "synthetic beta result two"),
                    ev_completed("response-beta-2"),
                ]),
            ],
        )
        .await;
    }

    let test = test_codex()
        .with_model(model)
        .with_config(|config| {
            config.base_instructions = Some("Synthetic fixture instructions".to_string());
        })
        .build(&server)
        .await?;

    for turn in 0..turns {
        test.codex
            .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
                text: format!("synthetic {cohort} refactor turn {}", turn + 1),
                text_elements: Vec::new(),
            }]))
            .await?;
        wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::TurnComplete(_))
        })
        .await;
    }

    test.codex.flush_rollout().await?;
    let rollout_path = test
        .codex
        .rollout_path()
        .context("writer created no rollout")?;
    let raw = std::fs::read_to_string(&rollout_path)?;
    let sanitized = sanitize_rollout(&raw, cohort)?;
    let fixture_name = format!("codex-{cohort}-direct.jsonl");
    std::fs::write(Path::new(OUTPUT_DIR).join(&fixture_name), &sanitized)?;
    let fixture_path = Path::new(OUTPUT_DIR).join(&fixture_name);
    let digest_output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&fixture_path)
        .output()
        .context("run shasum for generated fixture")?;
    anyhow::ensure!(digest_output.status.success(), "shasum failed");
    let digest = String::from_utf8(digest_output.stdout)?
        .split_whitespace()
        .next()
        .context("shasum produced no digest")?
        .to_string();
    let records = sanitized.lines().count();

    test.codex.shutdown_and_wait().await?;
    Ok(serde_json::json!({
        "file": fixture_name,
        "declared_model": model,
        "turns": turns,
        "records": records,
        "sha256": digest
    }))
}

fn sanitize_rollout(raw: &str, cohort: &str) -> Result<String> {
    let mut sanitizer = Sanitizer::new(cohort);
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut value: Value = serde_json::from_str(line)?;
            sanitizer.sanitize(&mut value, None, None);
            Ok(serde_json::to_string(&value)?)
        })
        .collect::<Result<Vec<_>>>()
        .map(|lines| format!("{}\n", lines.join("\n")))
}

struct Sanitizer {
    cohort: String,
    ids: BTreeMap<String, String>,
    paths: BTreeMap<String, String>,
    next_id: usize,
    next_timestamp: usize,
    next_text: usize,
    cohort_namespace: usize,
    temporal_numbers: BTreeMap<String, serde_json::Number>,
    next_temporal_number: u64,
}

impl Sanitizer {
    fn new(cohort: &str) -> Self {
        Self {
            cohort: cohort.to_string(),
            ids: BTreeMap::new(),
            paths: BTreeMap::new(),
            next_id: 1,
            next_timestamp: 0,
            next_text: 1,
            cohort_namespace: match cohort {
                "alpha" => 1,
                "beta" => 2,
                _ => 9,
            },
            temporal_numbers: BTreeMap::new(),
            next_temporal_number: 100,
        }
    }

    fn sanitize(&mut self, value: &mut Value, key: Option<&str>, parent: Option<&str>) {
        match value {
            Value::Object(object) => {
                for (child_key, child) in object {
                    self.sanitize(child, Some(child_key), key);
                }
            }
            Value::Array(values) => {
                for value in values {
                    self.sanitize(value, key, parent);
                }
            }
            Value::String(text) => match key {
                Some("timestamp" | "created_at" | "updated_at") => {
                    let hours = self.next_timestamp / 3600;
                    let minutes = (self.next_timestamp / 60) % 60;
                    let seconds = self.next_timestamp % 60;
                    *text = format!("2026-01-01T{hours:02}:{minutes:02}:{seconds:02}Z");
                    self.next_timestamp += 1;
                }
                Some(name)
                    if matches!(name, "cwd" | "path" | "working_directory")
                        || name.ends_with("_root")
                        || name.ends_with("_roots") =>
                {
                    let replacement = if let Some(existing) = self.paths.get(text) {
                        existing.clone()
                    } else {
                        let replacement = if self.paths.is_empty() {
                            "/w".to_string()
                        } else {
                            format!("/w/path-{}", self.paths.len() + 1)
                        };
                        self.paths.insert(text.clone(), replacement.clone());
                        replacement
                    };
                    *text = replacement;
                }
                Some(name)
                    if (name == "id" && parent != Some("active_permission_profile"))
                        || matches!(
                            name,
                            "session_id"
                                | "thread_id"
                                | "root_turn_id"
                                | "turn_id"
                                | "response_id"
                                | "call_id"
                                | "parent_thread_id"
                                | "forked_from_id"
                                | "window_id"
                                | "limit_id"
                        ) =>
                {
                    let replacement = if let Some(existing) = self.ids.get(text) {
                        existing.clone()
                    } else {
                        let replacement = format!(
                            "00000000-0000-4000-8{}00-{:012}",
                            self.cohort_namespace, self.next_id
                        );
                        self.ids.insert(text.clone(), replacement.clone());
                        self.next_id += 1;
                        replacement
                    };
                    *text = replacement;
                }
                Some("current_date") => *text = "2026-01-01".to_string(),
                Some("timezone") => *text = "UTC".to_string(),
                Some("text") if parent == Some("base_instructions") => {
                    *text = "synthetic-shared-base-instructions".to_string();
                }
                Some("text" | "message" | "last_agent_message") => {
                    *text = format!("synthetic-{}-text-{}", self.cohort, self.next_text);
                    self.next_text += 1;
                }
                _ => {
                    for (source, replacement) in self.paths.iter() {
                        if text.contains(source) {
                            *text = text.replace(source, replacement);
                        }
                    }
                }
            },
            Value::Number(number) if matches!(key, Some("duration_ms")) => {
                *number = serde_json::Number::from(10);
            }
            Value::Number(number) if matches!(key, Some("time_to_first_token_ms")) => {
                *number = serde_json::Number::from(5);
            }
            Value::Number(number)
                if matches!(key, Some("create_time" | "started_at" | "completed_at")) =>
            {
                let source = number.to_string();
                let replacement = if let Some(existing) = self.temporal_numbers.get(&source) {
                    existing.clone()
                } else {
                    let replacement = if number.is_f64() {
                        serde_json::Number::from_f64(self.next_temporal_number as f64 / 10.0)
                            .expect("finite fixture number")
                    } else {
                        serde_json::Number::from(self.next_temporal_number)
                    };
                    self.temporal_numbers.insert(source, replacement.clone());
                    self.next_temporal_number += 1;
                    replacement
                };
                *number = replacement;
            }
            _ => {}
        }
    }
}

fn assert_fixture_invariants(output_dir: &Path) -> Result<()> {
    let alpha = read_fixture(output_dir.join("codex-alpha-direct.jsonl"))?;
    let beta = read_fixture(output_dir.join("codex-beta-direct.jsonl"))?;

    let alpha_meta = &alpha[0]["payload"];
    let beta_meta = &beta[0]["payload"];
    anyhow::ensure!(alpha_meta["session_id"] == alpha_meta["id"]);
    anyhow::ensure!(beta_meta["session_id"] == beta_meta["id"]);
    anyhow::ensure!(alpha_meta["session_id"] != beta_meta["session_id"]);
    anyhow::ensure!(
        alpha_meta["base_instructions"] == beta_meta["base_instructions"],
        "cohort fixtures changed shared base instructions"
    );

    let alpha_context = first_record(&alpha, "turn_context")?;
    let beta_context = first_record(&beta, "turn_context")?;
    for field in [
        "active_permission_profile",
        "approval_policy",
        "approvals_reviewer",
        "current_date",
        "cwd",
        "multi_agent_version",
        "permission_profile",
        "sandbox_policy",
        "timezone",
        "workspace_roots",
    ] {
        anyhow::ensure!(
            alpha_context["payload"][field] == beta_context["payload"][field],
            "cross-fixture config differs at {field}"
        );
    }
    anyhow::ensure!(alpha_context["payload"]["current_date"] == "2026-01-01");
    anyhow::ensure!(alpha_context["payload"]["timezone"] == "UTC");
    anyhow::ensure!(alpha_context["payload"]["active_permission_profile"]["id"] == ":read-only");

    for records in [&alpha, &beta] {
        let serialized = serde_json::to_string(records)?;
        anyhow::ensure!(!serialized.contains("/private/") && !serialized.contains("/Users/"));
        let mut starts = BTreeMap::new();
        for record in records {
            let payload = &record["payload"];
            match payload["type"].as_str() {
                Some("task_started") => {
                    starts.insert(
                        payload["turn_id"]
                            .as_str()
                            .context("task_started turn_id")?,
                        payload["started_at"]
                            .as_u64()
                            .context("task_started started_at")?,
                    );
                }
                Some("task_complete") => {
                    let turn_id = payload["turn_id"]
                        .as_str()
                        .context("task_complete turn_id")?;
                    let started = payload["started_at"]
                        .as_u64()
                        .context("task_complete started_at")?;
                    let completed = payload["completed_at"]
                        .as_u64()
                        .context("task_complete completed_at")?;
                    anyhow::ensure!(starts.get(turn_id) == Some(&started));
                    anyhow::ensure!(completed >= started);
                    anyhow::ensure!(
                        payload["time_to_first_token_ms"].as_u64()
                            <= payload["duration_ms"].as_u64()
                    );
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn read_fixture(path: impl AsRef<Path>) -> Result<Vec<Value>> {
    std::fs::read_to_string(path)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn first_record<'a>(records: &'a [Value], record_type: &str) -> Result<&'a Value> {
    records
        .iter()
        .find(|record| record["type"] == record_type)
        .with_context(|| format!("missing {record_type}"))
}
