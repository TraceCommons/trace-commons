//! Local-only native usage observations. No pricing, source discovery, or I/O.
//! Codex cached input/reasoning output are subsets; Claude cache counters are
//! separate input categories. These representations must not be summed alike.
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageSource {
    Codex,
    ClaudeCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "accounting", rename_all = "snake_case")]
pub enum NativeTokenCounts {
    Codex {
        input: u64,
        cached_input: u64,
        output: u64,
        reasoning_output: u64,
        total: u64,
    },
    ClaudeCode {
        input: u64,
        cache_read_input: u64,
        cache_creation_input: u64,
        output: u64,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageUnavailableReason {
    NoUsage,
    IncompleteOrInvalidUsage,
    CumulativeReset,
    Overflow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    ExplicitFile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageSummary {
    pub source: UsageSource,
    /// Exactly one supplied file; no linked subagent files are read.
    pub scope: UsageScope,
    /// Distinct valid labels, capped at 32; not a per-request coverage measure.
    /// Declared model identifiers, not verified serving identities. Never used
    /// to attribute cumulative counters across model changes.
    pub observed_models: Vec<String>,
    pub model_labels_omitted: bool,
    /// Native candidate records; duplicates are included in this denominator.
    pub usage_records: u64,
    pub complete_records: u64,
    /// None when any candidate is incomplete, inconsistent, or overflows.
    pub counts: Option<NativeTokenCounts>,
    pub unavailable_reason: Option<UsageUnavailableReason>,
}

fn model_label(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (!text.is_empty()
        && text.len() <= 96
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b)))
    .then(|| text.to_owned())
}

fn counts(source: UsageSource, value: &Value) -> Option<[u64; 5]> {
    let keys = match source {
        UsageSource::Codex => [
            "input_tokens",
            "cached_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
            "total_tokens",
        ],
        UsageSource::ClaudeCode => [
            "input_tokens",
            "cache_read_input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "input_tokens",
        ],
    };
    let mut result = [0; 5];
    for (i, key) in keys.iter().enumerate() {
        result[i] = value.get(key)?.as_u64()?;
    }
    if source == UsageSource::Codex
        && (result[1] > result[0]
            || result[3] > result[2]
            || result[0].checked_add(result[2])? != result[4])
    {
        return None;
    }
    if source == UsageSource::ClaudeCode {
        result[4] = 0;
    }
    Some(result)
}

/// Extract only explicitly supplied JSONL, bounded to the local importer limit.
/// Claude assistant records require stable message IDs to avoid double counting
/// content blocks/replayed records. Repeated snapshots must be monotonic.
/// Codex uses only total_token_usage; last_token_usage is never added to it.
/// A source file is not proof of a complete session or complete API usage.
pub fn extract_usage(source: UsageSource, bytes: &[u8]) -> Result<UsageSummary> {
    if bytes.len() > 16 * 1024 * 1024 {
        bail!("insights_usage_source_too_large");
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("insights_usage_invalid_jsonl"))?;
    let mut summary = UsageSummary {
        source,
        scope: UsageScope::ExplicitFile,
        observed_models: Vec::new(),
        model_labels_omitted: false,
        usage_records: 0,
        complete_records: 0,
        counts: None,
        unavailable_reason: None,
    };
    let mut models = BTreeSet::new();
    let mut latest: Option<[u64; 5]> = None;
    let mut messages: BTreeMap<String, ([u64; 5], Option<String>)> = BTreeMap::new();
    let mut problem = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: Value = serde_json::from_str(line)
            .map_err(|_| anyhow::anyhow!("insights_usage_invalid_jsonl"))?;
        if !row.is_object() {
            bail!("insights_usage_invalid_jsonl");
        }
        let kind = row.get("type").and_then(Value::as_str);
        let raw_model = match source {
            UsageSource::Codex if kind == Some("turn_context") => row.pointer("/payload/model"),
            UsageSource::ClaudeCode if kind == Some("assistant") => row.pointer("/message/model"),
            _ => None,
        };
        let model = model_label(raw_model);
        if let Some(label) = &model {
            if models.len() < 32 || models.contains(label) {
                models.insert(label.clone());
            } else {
                summary.model_labels_omitted = true;
            }
        } else if raw_model.is_some() {
            summary.model_labels_omitted = true;
        }
        let usage = match source {
            UsageSource::Codex
                if kind == Some("event_msg")
                    && row.pointer("/payload/type").and_then(Value::as_str)
                        == Some("token_count") =>
            {
                row.pointer("/payload/info/total_token_usage")
            }
            UsageSource::ClaudeCode if kind == Some("assistant") => row.pointer("/message/usage"),
            _ => continue,
        };
        summary.usage_records += 1;
        let Some(current) = usage.and_then(|usage| counts(source, usage)) else {
            problem.get_or_insert(UsageUnavailableReason::IncompleteOrInvalidUsage);
            continue;
        };
        summary.complete_records += 1;
        match source {
            UsageSource::Codex => {
                if latest.is_some_and(|previous| current.iter().zip(previous).any(|(a, b)| *a < b))
                {
                    problem = Some(UsageUnavailableReason::CumulativeReset);
                }
                latest = Some(current);
            }
            UsageSource::ClaudeCode => {
                let Some(id) = row
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                else {
                    problem.get_or_insert(UsageUnavailableReason::IncompleteOrInvalidUsage);
                    continue;
                };
                if let Some((previous, previous_model)) = messages.get(id)
                    && (current.iter().zip(previous).any(|(a, b)| a < b)
                        || previous_model != &model)
                {
                    problem.get_or_insert(UsageUnavailableReason::IncompleteOrInvalidUsage);
                }
                messages.insert(id.to_owned(), (current, model));
            }
        }
    }
    if source == UsageSource::ClaudeCode && !messages.is_empty() {
        let mut total = [0u64; 5];
        for (entry, _) in messages.values() {
            for (sum, count) in total.iter_mut().zip(entry) {
                if let Some(value) = sum.checked_add(*count) {
                    *sum = value;
                } else {
                    problem = Some(UsageUnavailableReason::Overflow);
                }
            }
        }
        latest = Some(total);
    }
    summary.observed_models = models.into_iter().collect();
    summary.unavailable_reason =
        problem.or_else(|| latest.is_none().then_some(UsageUnavailableReason::NoUsage));
    if summary.unavailable_reason.is_none() {
        summary.counts = latest.map(|v| match source {
            UsageSource::Codex => NativeTokenCounts::Codex {
                input: v[0],
                cached_input: v[1],
                output: v[2],
                reasoning_output: v[3],
                total: v[4],
            },
            UsageSource::ClaudeCode => NativeTokenCounts::ClaudeCode {
                input: v[0],
                cache_read_input: v[1],
                output: v[2],
                cache_creation_input: v[3],
            },
        });
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn run(source: UsageSource, rows: Vec<Value>) -> UsageSummary {
        extract_usage(
            source,
            rows.iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                .as_bytes(),
        )
        .unwrap()
    }
    fn codex(input: u64, output: u64) -> Value {
        json!({"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":input/2,"output_tokens":output,"reasoning_output_tokens":output/2,"total_tokens":input+output},"last_token_usage":{"input_tokens":999}}}})
    }
    fn claude(id: &str, input: u64) -> Value {
        json!({"type":"assistant","message":{"id":id,"model":"claude-sonnet-4","content":[{"type":"tool_use","id":"synthetic"}],"usage":{"input_tokens":input,"output_tokens":4,"cache_read_input_tokens":10,"cache_creation_input_tokens":20}}})
    }
    #[test]
    fn codex_cumulative_is_not_summed_or_cache_counted_twice() {
        let s = run(
            UsageSource::Codex,
            vec![codex(100, 20), codex(200, 40), codex(200, 40)],
        );
        assert_eq!(
            s.counts,
            Some(NativeTokenCounts::Codex {
                input: 200,
                cached_input: 100,
                output: 40,
                reasoning_output: 20,
                total: 240
            })
        );
    }
    #[test]
    fn reset_is_unknown() {
        assert_eq!(
            run(UsageSource::Codex, vec![codex(200, 40), codex(100, 20)]).unavailable_reason,
            Some(UsageUnavailableReason::CumulativeReset)
        );
    }
    #[test]
    fn model_switch_does_not_assign_cumulative_usage_to_final_model() {
        let s = run(
            UsageSource::Codex,
            vec![
                json!({"type":"turn_context","payload":{"model":"gpt-5"}}),
                codex(100, 20),
                json!({"type":"turn_context","payload":{"model":"gpt-5-mini"}}),
                codex(200, 40),
            ],
        );
        assert_eq!(s.observed_models, vec!["gpt-5", "gpt-5-mini"]);
        assert!(s.counts.is_some());
    }
    #[test]
    fn missing_negative_fractional_or_inconsistent_counts_are_unknown() {
        for value in [Value::Null, json!(-1), json!(1.5), json!(999), json!("10")] {
            let mut row = codex(100, 20);
            row["payload"]["info"]["total_token_usage"]["cached_input_tokens"] = value;
            assert!(run(UsageSource::Codex, vec![row]).counts.is_none());
        }
        let mut row = codex(100, 20);
        row["payload"]["info"]["total_token_usage"]["total_tokens"] = json!(u64::MAX);
        assert!(run(UsageSource::Codex, vec![row]).counts.is_none());
    }
    #[test]
    fn claude_tool_only_deduplicates_message_and_preserves_cache_categories() {
        let row = claude("msg_1", 3);
        assert_eq!(
            run(
                UsageSource::ClaudeCode,
                vec![row.clone(), row, claude("msg_2", 7)]
            )
            .counts,
            Some(NativeTokenCounts::ClaudeCode {
                input: 10,
                cache_read_input: 20,
                cache_creation_input: 40,
                output: 8
            })
        );
    }
    #[test]
    fn claude_requires_all_fields_and_id() {
        for key in [
            "input_tokens",
            "output_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ] {
            let mut row = claude("msg", 1);
            row["message"]["usage"].as_object_mut().unwrap().remove(key);
            assert!(run(UsageSource::ClaudeCode, vec![row]).counts.is_none());
        }
        assert!(
            run(UsageSource::ClaudeCode, vec![claude("", 1)])
                .counts
                .is_none()
        );
    }
    #[test]
    fn overflow_is_unknown_not_saturated() {
        assert_eq!(
            run(
                UsageSource::ClaudeCode,
                vec![claude("a", u64::MAX), claude("b", 1)]
            )
            .unavailable_reason,
            Some(UsageUnavailableReason::Overflow)
        );
    }
    #[test]
    fn regressing_or_conflicting_duplicate_is_unknown() {
        assert!(
            run(
                UsageSource::ClaudeCode,
                vec![claude("a", 2), claude("a", 1)]
            )
            .counts
            .is_none()
        );
        let mut row = claude("a", 2);
        row["message"]["model"] = json!("another-model");
        assert!(
            run(UsageSource::ClaudeCode, vec![claude("a", 2), row])
                .counts
                .is_none()
        );
    }
    #[test]
    fn labels_are_bounded_and_control_characters_rejected() {
        let mut rows = vec![];
        for i in 0..40 {
            rows.push(json!({"type":"turn_context","payload":{"model":format!("model-{i}")}}));
        }
        rows.push(json!({"type":"turn_context","payload":{"model":"secret\ntext"}}));
        let s = run(UsageSource::Codex, rows);
        assert_eq!(s.observed_models.len(), 32);
        assert!(s.model_labels_omitted);
        assert_eq!(s.unavailable_reason, Some(UsageUnavailableReason::NoUsage));
    }
    #[test]
    fn malformed_json_has_static_error() {
        assert_eq!(
            extract_usage(UsageSource::Codex, b"private secret")
                .unwrap_err()
                .to_string(),
            "insights_usage_invalid_jsonl"
        );
    }
}
