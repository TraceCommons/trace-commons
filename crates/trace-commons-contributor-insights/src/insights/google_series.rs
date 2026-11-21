//! Per-turn usage series extractors for Antigravity (JSONL) and Gemini CLI
//! (single JSON document).
//!
//! Both formats report per-response input/output/cached token counts and model
//! labels. Neither reports cache-write durations, so `cache_write` is recorded
//! as `CacheWrite::TotalOnly(0)`.
//!
//! - **Antigravity**: `input_tokens` is uncached input and `cache_read_tokens`
//!   is cached input, so `uncached_input = input_tokens` and
//!   `cache_read = cache_read_tokens`.
//! - **Gemini CLI**: `tokens.input` is total prompt tokens (including
//!   `tokens.cached`), so `uncached_input = input.saturating_sub(cached)` and
//!   `cache_read = cached`.
//!
//! After that normalization, both produce standard `UsageSeries` records whose
//! `context() = uncached_input + cache_read` and cache share is
//! `cache_read / context()`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, DigestKey, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
    args_key as compute_args_key,
};

use super::MAX_SOURCE_BYTES;

/// Extracted per-turn usage series and declared model labels for one
/// Antigravity or Gemini CLI session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleTurnSeriesEvidence {
    pub series: UsageSeries,
    pub model_labels: Vec<String>,
    pub earliest: Option<DateTime<Utc>>,
    pub latest: Option<DateTime<Utc>>,
}

fn timestamp_of(value: Option<&Value>) -> Option<DateTime<Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

fn map_google_tool_name(name: &str) -> ToolKind {
    match name {
        "read_file" | "read_many_files" => ToolKind::Read,
        "edit_file" | "replace" => ToolKind::Edit,
        "write_file" => ToolKind::Write,
        "run_command" | "run_shell_command" => ToolKind::Bash,
        "grep_search" | "search_file_content" => ToolKind::Grep,
        "find_by_name" | "glob" | "list_dir" => ToolKind::Glob,
        other => ToolKind::from_tool_name(other),
    }
}

fn clamp_u32(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

fn gap_seconds(previous: Option<DateTime<Utc>>, current: Option<DateTime<Utc>>) -> Option<u32> {
    let (prev, curr) = (previous?, current?);
    let secs = (curr - prev).num_seconds();
    (secs >= 0).then(|| clamp_u32(secs as u64))
}

fn intern_model(
    label: Option<&str>,
    labels: &mut Vec<String>,
    index: &mut HashMap<String, u16>,
) -> Option<u16> {
    let label = label.filter(|s| !s.is_empty())?;
    if let Some(&ix) = index.get(label) {
        return Some(ix);
    }
    let ix = u16::try_from(labels.len()).ok()?;
    labels.push(label.to_string());
    index.insert(label.to_string(), ix);
    Some(ix)
}

/// Streaming line-by-line scanner for Antigravity `transcript[_full].jsonl`.
pub struct AntigravityTurnSeriesScan {
    key: DigestKey,
    series: UsageSeries,
    model_labels: Vec<String>,
    model_index: HashMap<String, u16>,
    ordinal: u32,
    previous_turn_at: Option<DateTime<Utc>>,
    earliest: Option<DateTime<Utc>>,
    latest: Option<DateTime<Utc>>,
    saw_step: bool,
}

impl AntigravityTurnSeriesScan {
    pub fn new(key: &DigestKey) -> Self {
        Self {
            key: key.clone(),
            series: UsageSeries::default(),
            model_labels: Vec::new(),
            model_index: HashMap::new(),
            ordinal: 0,
            previous_turn_at: None,
            earliest: None,
            latest: None,
            saw_step: false,
        }
    }

    pub fn observe(&mut self, record: &Value) -> Result<(), ()> {
        let Some(obj) = record.as_object() else {
            return Err(());
        };
        if !obj.contains_key("type") && !obj.contains_key("step_index") {
            return Err(());
        }
        self.saw_step = true;

        let at = timestamp_of(record.get("created_at"));
        if let Some(ts) = at {
            self.earliest = Some(self.earliest.map_or(ts, |e| e.min(ts)));
            self.latest = Some(self.latest.map_or(ts, |l| l.max(ts)));
        }

        if record.get("type").and_then(|v| v.as_str()) != Some("PLANNER_RESPONSE") {
            return Ok(());
        }

        let input = record.get("input_tokens").and_then(|v| v.as_u64());
        let output = record.get("output_tokens").and_then(|v| v.as_u64());
        let cache_read = record
            .get("cache_read_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        // Only count steps that actually carry token counters or tool calls.
        let calls = record.get("tool_calls").and_then(|v| v.as_array());
        if input.is_none() && output.is_none() && calls.is_none_or(Vec::is_empty) {
            return Ok(());
        }

        let turn_ordinal = self.ordinal;
        self.ordinal = self.ordinal.saturating_add(1);
        let model_label_ix = intern_model(
            record.get("model").and_then(|v| v.as_str()),
            &mut self.model_labels,
            &mut self.model_index,
        );
        let gap_secs = gap_seconds(self.previous_turn_at, at);
        if at.is_some() {
            self.previous_turn_at = at;
        }

        let (uncached_input, cache_read_opt, cache_write, output_opt) = match (input, output) {
            (Some(i), Some(o)) => (
                Some(clamp_u32(i)),
                Some(clamp_u32(cache_read)),
                Some(CacheWrite::TotalOnly(0)),
                Some(clamp_u32(o)),
            ),
            _ => (None, None, None, None),
        };

        self.series.push_turn(TurnRecord {
            ordinal: turn_ordinal,
            at,
            uncached_input,
            cache_read: cache_read_opt,
            cache_write,
            output: output_opt,
            model_label_ix,
            gap_secs,
            msg_key: None,
        });

        if let Some(calls) = calls {
            for call in calls {
                let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let tool = map_google_tool_name(name);
                let args = call.get("args").unwrap_or(&Value::Null);
                let Ok(args_key) = compute_args_key(&self.key, tool, args) else {
                    continue;
                };
                self.series.push_tool_call(ToolCallRecord {
                    turn_ordinal,
                    tool,
                    args_key,
                    path_key: None,
                    path_ext: None,
                    result_bytes: None,
                    success: None,
                    paired: false,
                });
            }
        }

        Ok(())
    }

    pub fn finish(self) -> Option<GoogleTurnSeriesEvidence> {
        if !self.saw_step || self.series.turns.is_empty() {
            return None;
        }
        Some(GoogleTurnSeriesEvidence {
            series: self.series,
            model_labels: self.model_labels,
            earliest: self.earliest,
            latest: self.latest,
        })
    }
}

/// Extracts a per-turn usage series from one Gemini CLI `session-*.json` file.
pub fn extract_gemini_turn_series(
    bytes: &[u8],
    key: &DigestKey,
) -> Option<GoogleTurnSeriesEvidence> {
    if (bytes.len() as u64) > MAX_SOURCE_BYTES {
        return None;
    }
    let doc: Value = serde_json::from_slice(bytes).ok()?;
    let messages = doc.get("messages")?.as_array()?;

    let mut series = UsageSeries::default();
    let mut model_labels = Vec::new();
    let mut model_index = HashMap::new();
    let mut ordinal: u32 = 0;
    let mut previous_turn_at: Option<DateTime<Utc>> = None;
    let mut earliest = timestamp_of(doc.get("startTime"));
    let mut latest = timestamp_of(doc.get("lastUpdated"));

    for message in messages {
        let at = timestamp_of(message.get("timestamp"));
        if let Some(ts) = at {
            earliest = Some(earliest.map_or(ts, |e| e.min(ts)));
            latest = Some(latest.map_or(ts, |l| l.max(ts)));
        }
        if message.get("type").and_then(|v| v.as_str()) != Some("gemini") {
            continue;
        }
        let tokens = message.get("tokens");
        let input = tokens.and_then(|t| t.get("input")).and_then(|v| v.as_u64());
        let output = tokens.and_then(|t| t.get("output")).and_then(|v| v.as_u64());
        let cached = tokens
            .and_then(|t| t.get("cached"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let calls = message.get("toolCalls").and_then(|v| v.as_array());
        if input.is_none() && output.is_none() && calls.is_none_or(Vec::is_empty) {
            continue;
        }

        let turn_ordinal = ordinal;
        ordinal = ordinal.saturating_add(1);
        let model_label_ix = intern_model(
            message.get("model").and_then(|v| v.as_str()),
            &mut model_labels,
            &mut model_index,
        );
        let gap_secs = gap_seconds(previous_turn_at, at);
        if at.is_some() {
            previous_turn_at = at;
        }

        let (uncached_input, cache_read, cache_write, output_opt) = match (input, output) {
            (Some(i), Some(o)) => {
                let capped_cache = cached.min(i);
                (
                    Some(clamp_u32(i - capped_cache)),
                    Some(clamp_u32(capped_cache)),
                    Some(CacheWrite::TotalOnly(0)),
                    Some(clamp_u32(o)),
                )
            }
            _ => (None, None, None, None),
        };

        series.push_turn(TurnRecord {
            ordinal: turn_ordinal,
            at,
            uncached_input,
            cache_read,
            cache_write,
            output: output_opt,
            model_label_ix,
            gap_secs,
            msg_key: None,
        });

        if let Some(calls) = calls {
            for call in calls {
                let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let tool = map_google_tool_name(name);
                let args = call.get("args").unwrap_or(&Value::Null);
                let Ok(args_key) = compute_args_key(key, tool, args) else {
                    continue;
                };
                let success = call
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s == "success");
                series.push_tool_call(ToolCallRecord {
                    turn_ordinal,
                    tool,
                    args_key,
                    path_key: None,
                    path_ext: None,
                    result_bytes: None,
                    success,
                    paired: success.is_some(),
                });
            }
        }
    }

    if series.turns.is_empty() {
        return None;
    }

    Some(GoogleTurnSeriesEvidence {
        series,
        model_labels,
        earliest,
        latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> DigestKey {
        DigestKey::from_bytes([7_u8; 32])
    }

    #[test]
    fn antigravity_scan_extracts_turns_cache_and_tools() {
        let key = test_key();
        let mut scan = AntigravityTurnSeriesScan::new(&key);
        let lines = [
            serde_json::json!({
                "step_index": 0, "type": "USER_INPUT",
                "created_at": "2026-11-19T02:06:58Z", "content": "hi"
            }),
            serde_json::json!({
                "step_index": 1, "type": "PLANNER_RESPONSE",
                "created_at": "2026-11-19T02:06:59Z",
                "model": "gemini-4-neon-medium",
                "input_tokens": 6704, "cache_read_tokens": 21220, "output_tokens": 263,
                "tool_calls": [{"name": "read_file", "args": {"Path": "/a.rs"}}]
            }),
        ];
        for line in &lines {
            scan.observe(line).unwrap();
        }
        let evidence = scan.finish().expect("evidence");
        assert_eq!(evidence.model_labels, vec!["gemini-4-neon-medium"]);
        assert_eq!(evidence.series.turns.len(), 1);
        let t = &evidence.series.turns[0];
        assert_eq!(t.uncached_input, Some(6704));
        assert_eq!(t.cache_read, Some(21220));
        assert_eq!(t.output, Some(263));
        assert_eq!(evidence.series.tool_calls.len(), 1);
        assert_eq!(evidence.series.tool_calls[0].tool, ToolKind::Read);
    }

    #[test]
    fn gemini_extracts_turns_and_splits_cached_from_total_input() {
        let key = test_key();
        let doc = serde_json::json!({
            "sessionId": "ses-1",
            "startTime": "2026-11-19T01:00:00Z",
            "lastUpdated": "2026-11-19T01:05:00Z",
            "messages": [
                {"type": "user", "timestamp": "2026-11-19T01:00:00Z", "content": "check"},
                {
                    "type": "gemini",
                    "timestamp": "2026-11-19T01:00:05Z",
                    "model": "gemini-2.5-pro",
                    "tokens": {"input": 1000, "cached": 400, "output": 120},
                    "toolCalls": [{"name": "run_shell_command", "args": {"cmd": "ls"}, "status": "success"}]
                }
            ]
        });
        let evidence =
            extract_gemini_turn_series(&serde_json::to_vec(&doc).unwrap(), &key).expect("evidence");
        assert_eq!(evidence.model_labels, vec!["gemini-2.5-pro"]);
        assert_eq!(evidence.series.turns.len(), 1);
        let t = &evidence.series.turns[0];
        assert_eq!(t.uncached_input, Some(600));
        assert_eq!(t.cache_read, Some(400));
        assert_eq!(t.output, Some(120));
        assert_eq!(evidence.series.tool_calls.len(), 1);
        assert_eq!(evidence.series.tool_calls[0].tool, ToolKind::Bash);
    }
}
