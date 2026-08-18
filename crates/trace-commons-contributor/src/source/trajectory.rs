//! Reader for Letta Trajectory v1 files (`https://letta.ai/schemas/trajectory/v1.json`).
//!
//! Accepts either a JSON array of records or JSONL (one record per line).
//! Contributors produce these with `npx @letta-ai/trajectory`; this reader
//! never shells out and never touches a harness's native session store.
//!
//! Fails closed: any malformed record, unparseable timestamp, orphaned
//! `tool_call_id`, or invalid `meta.source` rejects the entire file. Error
//! strings are reason labels only and never carry record content.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::source::{
    SOURCE_TRAJECTORY, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    session_hash,
};

const MAX_SOURCE_LEN: usize = 64;

#[derive(Debug)]
pub(crate) struct ParsedTrajectory {
    pub source: String,
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub events: Vec<SessionEvent>,
}

/// Validate an untrusted `meta.source` before it becomes provenance. The
/// value reaches `feature_flags["agent"]` and the local receipt, so it is
/// constrained to a conservative slug charset.
pub(crate) fn validate_source_name(raw: &str) -> Result<String> {
    if raw.is_empty() || raw.len() > MAX_SOURCE_LEN {
        bail!("invalid_source_name");
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        bail!("invalid_source_name");
    }
    Ok(raw.to_string())
}

/// Parse a trajectory-v1 file: either a JSON array of records or JSONL.
fn records_from(bytes: &[u8]) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).map_err(|_| anyhow!("invalid_utf8"))?;
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        let parsed: Value = serde_json::from_str(trimmed).map_err(|_| anyhow!("malformed_json"))?;
        match parsed {
            Value::Array(items) => Ok(items),
            _ => bail!("malformed_json"),
        }
    } else {
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            out.push(serde_json::from_str::<Value>(line).map_err(|_| anyhow!("malformed_json"))?);
        }
        Ok(out)
    }
}

fn parse_timestamp(record: &Value) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    let Some(raw) = record.get("timestamp").and_then(|v| v.as_str()) else {
        bail!("invalid_timestamp");
    };
    let parsed =
        chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| anyhow!("invalid_timestamp"))?;
    Ok(Some(parsed.with_timezone(&chrono::Utc)))
}

/// Read an optional string field. Absent or JSON null yields `None`;
/// present-but-not-a-string is a malformed record.
fn optional_string(record: &Value, key: &str) -> Result<Option<String>> {
    match record.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => bail!("malformed_record"),
    }
}

fn required_str(record: &Value, key: &str) -> Result<String> {
    record
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("malformed_record"))
}

pub(crate) fn parse_trajectory(bytes: &[u8]) -> Result<ParsedTrajectory> {
    let records = records_from(bytes)?;
    let Some(first) = records.first() else {
        bail!("missing_meta_record");
    };
    if first.get("role").and_then(|v| v.as_str()) != Some("meta") {
        bail!("missing_meta_record");
    }

    let source = validate_source_name(&required_str(first, "source")?)?;
    // A present-but-wrong-typed optional field is a malformed file, not an
    // absent field. Coercing it to None fails OPEN, and for `cwd` that
    // silently drops a redactor path prefix, weakening path stripping on a
    // file the module documents as fail-closed.
    let model = optional_string(first, "model")?;
    // `meta.cwd` feeds the redactor's path-prefix stripping and is never
    // serialized. `meta.git_branch` is deliberately dropped: it has no home
    // in SessionTranscript and is identity-adjacent.
    let cwd = optional_string(first, "cwd")?;

    let mut events = Vec::new();
    let mut seen_call_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for record in records.iter().skip(1) {
        let role = record.get("role").and_then(|v| v.as_str()).unwrap_or("");
        match role {
            "meta" => bail!("duplicate_meta_record"),
            "user" | "reasoning" => {
                let kind = if role == "user" {
                    SessionEventKind::User
                } else {
                    SessionEventKind::Reasoning
                };
                events.push(SessionEvent {
                    kind,
                    timestamp: parse_timestamp(record)?,
                    content: Some(required_str(record, "content")?),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                });
            }
            "assistant" => {
                let timestamp = parse_timestamp(record)?;
                let has_content = !matches!(record.get("content"), None | Some(Value::Null));
                match record.get("tool_calls").and_then(|v| v.as_array()) {
                    Some(calls) if !calls.is_empty() => {
                        if has_content {
                            bail!("malformed_record");
                        }
                        for call in calls {
                            let id = required_str(call, "id")?;
                            let name = required_str(call, "name")?;
                            let args = required_str(call, "args")?;
                            // Args are a stringified JSON object per the
                            // schema. When a producer violates that, record
                            // only the length -- never the raw string, which
                            // would smuggle unparsed content into the payload.
                            let structured = serde_json::from_str::<Value>(&args)
                                .unwrap_or_else(|_| json!({ "arguments_raw_len": args.len() }));
                            // A duplicate id makes the orphan check
                            // meaningless: a later `tool` record would pair
                            // against the wrong call. The schema gives ids
                            // per call, so a repeat is a malformed file.
                            if !seen_call_ids.insert(id) {
                                bail!("duplicate_tool_call_id");
                            }
                            events.push(SessionEvent {
                                kind: SessionEventKind::ToolCall,
                                timestamp,
                                content: None,
                                structured,
                                tool_name: Some(name),
                                token_counts: None,
                            });
                        }
                    }
                    _ => {
                        // The schema requires a non-empty content string when
                        // there are no tool calls. An empty string is neither
                        // a valid content assistant nor a tool-call assistant.
                        let content = required_str(record, "content")?;
                        if content.is_empty() {
                            bail!("malformed_record");
                        }
                        events.push(SessionEvent {
                            kind: SessionEventKind::Assistant,
                            timestamp,
                            content: Some(content),
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                        });
                    }
                }
            }
            "tool" => {
                let id = required_str(record, "tool_call_id")?;
                if !seen_call_ids.contains(&id) {
                    bail!("orphaned_tool_result");
                }
                events.push(SessionEvent {
                    kind: SessionEventKind::ToolResult,
                    timestamp: parse_timestamp(record)?,
                    content: Some(required_str(record, "content")?),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                });
            }
            _ => bail!("unknown_record"),
        }
    }

    Ok(ParsedTrajectory {
        source,
        model,
        cwd,
        events,
    })
}

/// Reads trajectory-v1 files from an explicitly supplied path. Unlike the
/// native adapters there is no conventional local store to scan, so this
/// source is only constructed when the contributor passes `--trajectory`.
pub struct TrajectorySource {
    path: PathBuf,
}

impl TrajectorySource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

fn is_trajectory_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
    let metadata = std::fs::metadata(&path).ok()?;
    let started_at = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Utc>::from);
    // `cwd` is left None at discovery: determining it requires a full parse,
    // and unlike the codex adapter there is no cheap single-line peek. The
    // `--project` filter therefore falls back to the path heuristic for
    // trajectory files.
    Some(SessionRef {
        source: SOURCE_TRAJECTORY,
        path,
        project: None,
        cwd: None,
        started_at,
        size_bytes: metadata.len(),
        // One file per trajectory, in a flat directory, with no parent/child
        // convention to group on.
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for TrajectorySource {
    fn name(&self) -> &'static str {
        SOURCE_TRAJECTORY
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        if self.path.is_file() {
            return Ok(session_ref_for(self.path.clone()).into_iter().collect());
        }
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            let path = entry.path();
            if !path.is_file() || !is_trajectory_file(&path) {
                continue;
            }
            match session_ref_for(path) {
                Some(r) => sessions.push(r),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable trajectory entries during discovery"
            );
        }
        Ok(sessions)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        let bytes = std::fs::read(&r.path).map_err(|_| anyhow!("unreadable_trajectory_file"))?;
        let hash = session_hash(&bytes);
        let parsed = parse_trajectory(&bytes)?;

        let project = parsed
            .cwd
            .as_deref()
            .map(Path::new)
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_string());

        let started_at = parsed.events.iter().find_map(|e| e.timestamp);

        Ok(SessionTranscript {
            source: Cow::Owned(parsed.source),
            // Trajectory carries no harness version field.
            agent_version: None,
            model: parsed.model,
            project,
            cwd: parsed.cwd,
            started_at,
            session_hash: hash,
            events: parsed.events,
            subagent_count: 0,
            subagents_dropped: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SessionEventKind;

    const SAMPLE: &str = r#"[
      {"role":"meta","source":"openhands","cwd":"/home/dev/proj","model":"gpt-5","git_branch":"main"},
      {"role":"user","content":"Check the current directory.","timestamp":"2026-07-10T12:00:00.000Z"},
      {"role":"reasoning","content":"I should run pwd.","timestamp":"2026-07-10T12:00:01.000Z"},
      {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","name":"exec_command","args":"{\"cmd\":\"pwd\"}"}],"timestamp":"2026-07-10T12:00:02.000Z"},
      {"role":"tool","tool_call_id":"call_1","content":"/workspace","timestamp":"2026-07-10T12:00:03.000Z"},
      {"role":"assistant","content":"You are in /workspace.","timestamp":"2026-07-10T12:00:04.000Z"}
    ]"#;

    #[test]
    fn parses_every_record_type_in_order() {
        let p = parse_trajectory(SAMPLE.as_bytes()).unwrap();
        assert_eq!(p.source, "openhands");
        assert_eq!(p.model.as_deref(), Some("gpt-5"));
        assert_eq!(p.cwd.as_deref(), Some("/home/dev/proj"));

        let kinds: Vec<_> = p.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::User,
                SessionEventKind::Reasoning,
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
            ]
        );
        assert_eq!(p.events[2].tool_name.as_deref(), Some("exec_command"));
        assert_eq!(p.events[2].structured["cmd"], "pwd");
        assert_eq!(p.events[3].content.as_deref(), Some("/workspace"));
    }

    #[test]
    fn accepts_jsonl_as_well_as_a_json_array() {
        let jsonl = "{\"role\":\"meta\",\"source\":\"pi\"}\n\
                     {\"role\":\"user\",\"content\":\"hi\",\"timestamp\":\"2026-07-10T12:00:00Z\"}\n";
        let p = parse_trajectory(jsonl.as_bytes()).unwrap();
        assert_eq!(p.source, "pi");
        assert_eq!(p.events.len(), 1);
    }

    #[test]
    fn rejects_orphaned_tool_result() {
        let bad = r#"[
          {"role":"meta","source":"pi"},
          {"role":"tool","tool_call_id":"nope","content":"x","timestamp":"2026-07-10T12:00:00Z"}
        ]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("orphaned_tool_result"), "got: {err}");
        assert!(!err.contains("nope"), "error must not echo file content");
    }

    #[test]
    fn rejects_missing_or_non_leading_meta() {
        let bad = r#"[{"role":"user","content":"hi","timestamp":"2026-07-10T12:00:00Z"}]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("missing_meta_record"), "got: {err}");
    }

    #[test]
    fn rejects_unknown_record_role() {
        let bad = r#"[{"role":"meta","source":"pi"},{"role":"wat"}]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("unknown_record"), "got: {err}");
    }

    #[test]
    fn rejects_bad_timestamp() {
        let bad = r#"[
          {"role":"meta","source":"pi"},
          {"role":"user","content":"hi","timestamp":"not-a-time"}
        ]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("invalid_timestamp"), "got: {err}");
    }

    #[test]
    fn source_name_validation_rejects_injection_and_overlong() {
        assert_eq!(validate_source_name("open-hands").unwrap(), "open-hands");
        assert!(validate_source_name("").is_err());
        assert!(validate_source_name("Open Hands").is_err());
        assert!(validate_source_name("../../etc/passwd").is_err());
        assert!(validate_source_name(&"a".repeat(65)).is_err());
        assert!(validate_source_name(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn rejects_duplicate_tool_call_ids() {
        let bad = r#"[
          {"role":"meta","source":"pi"},
          {"role":"assistant","content":null,"tool_calls":[{"id":"c1","name":"t","args":"{}"},{"id":"c1","name":"u","args":"{}"}],"timestamp":"2026-07-10T12:00:00Z"}
        ]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("duplicate_tool_call_id"), "got: {err}");
    }

    #[test]
    fn rejects_wrong_typed_optional_meta_fields() {
        for bad in [
            r#"[{"role":"meta","source":"pi","cwd":5}]"#,
            r#"[{"role":"meta","source":"pi","model":["x"]}]"#,
        ] {
            let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
            assert!(err.contains("malformed_record"), "got: {err}");
        }
        // Absent and explicit null remain valid.
        let ok = r#"[{"role":"meta","source":"pi","cwd":null}]"#;
        assert!(parse_trajectory(ok.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_empty_assistant_content() {
        let bad = r#"[
          {"role":"meta","source":"pi"},
          {"role":"assistant","content":"","timestamp":"2026-07-10T12:00:00Z"}
        ]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("malformed_record"), "got: {err}");
    }

    #[test]
    fn rejects_assistant_with_both_content_and_tool_calls() {
        let bad = r#"[
          {"role":"meta","source":"pi"},
          {"role":"assistant","content":"hello","tool_calls":[{"id":"c1","name":"t","args":"{}"}],"timestamp":"2026-07-10T12:00:00Z"}
        ]"#;
        let err = parse_trajectory(bad.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("malformed_record"), "got: {err}");
        assert!(!err.contains("hello"), "error must not echo file content");
    }

    #[test]
    fn tool_call_args_that_are_not_json_do_not_leak_content() {
        let raw = r#"[
          {"role":"meta","source":"pi"},
          {"role":"assistant","content":null,"tool_calls":[{"id":"c1","name":"t","args":"not json"}],"timestamp":"2026-07-10T12:00:00Z"}
        ]"#;
        let p = parse_trajectory(raw.as_bytes()).unwrap();
        let s = serde_json::to_string(&p.events[0].structured).unwrap();
        assert!(!s.contains("not json"), "raw args must not be embedded");
        assert_eq!(p.events[0].structured["arguments_raw_len"], 8);
    }

    use crate::source::TraceSource;
    use std::io::Write;

    fn write_temp(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        p
    }

    #[test]
    fn discovers_a_single_file_and_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        write_temp(dir.path(), "a.json", SAMPLE);
        write_temp(
            dir.path(),
            "b.jsonl",
            "{\"role\":\"meta\",\"source\":\"pi\"}\n",
        );
        write_temp(dir.path(), "ignored.txt", "not a trajectory");

        let src = super::TrajectorySource::new(dir.path().to_path_buf());
        let mut found: Vec<_> = src
            .discover()
            .unwrap()
            .into_iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.json", "b.jsonl"]);

        let one = write_temp(dir.path(), "c.json", SAMPLE);
        let src = super::TrajectorySource::new(one.clone());
        assert_eq!(src.discover().unwrap().len(), 1);
    }

    #[test]
    fn load_carries_inner_source_as_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_temp(dir.path(), "a.json", SAMPLE);
        let src = super::TrajectorySource::new(p.clone());
        let r = &src.discover().unwrap()[0];

        // The routing key stays the adapter name so `source_for` can pair
        // the ref back to this adapter.
        assert_eq!(r.source, crate::source::SOURCE_TRAJECTORY);

        let t = src.load(r).unwrap();
        // Provenance is the harness that actually produced the session.
        assert_eq!(t.source.as_ref(), "openhands");
        assert_eq!(t.model.as_deref(), Some("gpt-5"));
        assert_eq!(t.project.as_deref(), Some("proj"));
        assert_eq!(t.cwd.as_deref(), Some("/home/dev/proj"));
        assert!(t.session_hash.starts_with("sha256:"));
        assert_eq!(
            t.started_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-07-10T12:00:00.000Z")
                    .unwrap()
                    .with_timezone(&chrono::Utc)
            )
        );
    }

    #[test]
    fn a_malformed_file_rejects_the_whole_session() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_temp(dir.path(), "bad.json", "{ not json");
        let src = super::TrajectorySource::new(p.clone());
        let r = &src.discover().unwrap()[0];
        assert!(src.load(r).is_err());
    }
    /// Conformance corpus: every file in `tests/fixtures/letta-conformance`
    /// is parsed and its outcome checked against the expectation encoded in
    /// its filename, `<expected>__<case>.jsonl`, where `<expected>` is either
    /// `ok` or one of the documented error codes.
    ///
    /// The point is that a harness vendor can drop a `.jsonl` in that
    /// directory and get a pass/fail without writing Rust, and that every
    /// documented rejection reason stays exercised as the parser changes.
    #[test]
    fn letta_conformance_corpus_matches_expected_outcomes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/letta-conformance");
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("conformance corpus unreadable at {dir:?}: {e}"))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        entries.sort();
        assert!(
            !entries.is_empty(),
            "conformance corpus is empty at {dir:?}"
        );

        let mut failures = Vec::new();
        let mut covered = std::collections::BTreeSet::new();
        for path in &entries {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let expected = name
                .split("__")
                .next()
                .expect("fixture name must be <expected>__<case>.jsonl")
                .to_string();
            covered.insert(expected.clone());
            let bytes = std::fs::read(path).expect("fixture readable");
            let result = parse_trajectory(&bytes);
            match (&expected[..], &result) {
                ("ok", Ok(_)) => {}
                ("ok", Err(e)) => failures.push(format!("{name}: expected ok, got {e}")),
                (code, Err(e)) => {
                    let actual = e.to_string();
                    if actual != code {
                        failures.push(format!("{name}: expected {code}, got {actual}"));
                    }
                }
                (code, Ok(_)) => {
                    failures.push(format!("{name}: expected {code}, but parsing succeeded"))
                }
            }
        }
        assert!(
            failures.is_empty(),
            "conformance failures:\n{}",
            failures.join("\n")
        );

        // Every rejection reason reachable from bytes must appear in the
        // corpus, so a newly added code cannot ship without a fixture.
        // `unreadable_trajectory_file` is excluded deliberately: it is raised
        // by the file-reading wrapper, not by `parse_trajectory`, so no
        // fixture content can produce it.
        for code in [
            "duplicate_meta_record",
            "invalid_utf8",
            "duplicate_tool_call_id",
            "invalid_source_name",
            "invalid_timestamp",
            "malformed_json",
            "malformed_record",
            "missing_meta_record",
            "orphaned_tool_result",
            "unknown_record",
        ] {
            assert!(
                covered.contains(code),
                "no conformance fixture covers rejection reason `{code}`"
            );
        }
        println!(
            "CONFORMANCE_FIXTURES={} CODES_COVERED={}",
            entries.len(),
            covered.len()
        );
    }
}
