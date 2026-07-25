//! Reader for Letta Trajectory v1 files (`https://letta.ai/schemas/trajectory/v1.json`).
//!
//! Accepts either a JSON array of records or JSONL (one record per line).
//! Contributors produce these with `npx @letta-ai/trajectory`; this reader
//! never shells out and never touches a harness's native session store.
//!
//! Fails closed: any malformed record, unparseable timestamp, orphaned
//! `tool_call_id`, or invalid `meta.source` rejects the entire file. Error
//! strings are reason labels only and never carry record content.
//!
//! `ParsedTrajectory`, `parse_trajectory`, and `validate_source_name` have no
//! caller yet -- Task 6 wires this reader into a `TraceSource` impl. The
//! `#[allow(dead_code)]` below is temporary and Task 6 removes it.

#![allow(dead_code)]

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::source::{SessionEvent, SessionEventKind};

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
    let parsed = chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| anyhow!("invalid_timestamp"))?;
    Ok(Some(parsed.with_timezone(&chrono::Utc)))
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
    let model = first
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    // `meta.cwd` feeds the redactor's path-prefix stripping and is never
    // serialized. `meta.git_branch` is deliberately dropped: it has no home
    // in SessionTranscript and is identity-adjacent.
    let cwd = first
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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
                match record.get("tool_calls").and_then(|v| v.as_array()) {
                    Some(calls) if !calls.is_empty() => {
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
                            seen_call_ids.insert(id);
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
                        events.push(SessionEvent {
                            kind: SessionEventKind::Assistant,
                            timestamp,
                            content: Some(required_str(record, "content")?),
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
}
