//! Source-bound structural attribution for an observed Claude Code agent branch.
//!
//! This adapter retains hashes, counts, and physical line references. It does
//! not retain source bodies or claim a complete task, a human prompt, provider
//! serving identity, successful work, or completed asynchronous processes.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_RECORDS: usize = 262_144;
const MAX_REFERENCES: usize = 262_144;
const WRITER_VERSION: &str = "2.1.260";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeTaskUnavailableReason {
    SourceProfileMismatch,
    UnsupportedRecord,
    UnsupportedAttachment,
    UnsupportedTool,
    InvalidRecordOrder,
    MissingRequiredRecord,
    IdentityConflict,
    ModelConflict,
    NestedDelegation,
    Interrupted,
    RecordLimitExceeded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeObservedTermination {
    ObservedBranchSegmentEnded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AsyncWorkStatus {
    ObservedBackgroundLaunch,
    AsyncWorkStatusUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeAttachmentRef {
    pub kind: String,
    pub physical_line: u64,
    pub canonical_payload_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeBackgroundWorkRef {
    pub physical_line: u64,
    pub tool_name: String,
    pub task_identity_sha256: Option<String>,
    pub monitor_persistent: Option<bool>,
    pub monitor_timeout_ms: Option<u64>,
    pub launch_status: AsyncWorkStatus,
    pub completion_status: AsyncWorkStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeToolPairRef {
    pub tool_name: String,
    pub call_physical_line: u64,
    pub call_block_index: u64,
    pub response_physical_line: u64,
    pub response_block_index: u64,
    pub call_payload_sha256: String,
    pub response_payload_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeRepeatedAssistantRef {
    pub message_identity_sha256: String,
    pub first_physical_line: u64,
    pub repeated_physical_line: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeAttributedBranch {
    pub terminal_physical_line: u64,
    pub termination: ClaudeObservedTermination,
    pub tool_calls: u64,
    pub tool_protocol_responses: u64,
    pub repeated_assistant_records: u64,
    pub tool_pairs: Vec<ClaudeToolPairRef>,
    pub repeated_assistant_refs: Vec<ClaudeRepeatedAssistantRef>,
    pub material_attachment_provenance_sha256: String,
    pub attachments: Vec<ClaudeAttachmentRef>,
    pub background_work: Vec<ClaudeBackgroundWorkRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ClaudeTaskAttributionState {
    Attributed {
        branch: ClaudeAttributedBranch,
    },
    Unavailable {
        reason: ClaudeTaskUnavailableReason,
        physical_line: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClaudeTaskAttributionEvidence {
    pub schema_version: u32,
    pub extractor_version: u32,
    pub profile_id: String,
    pub observed_writer_version: String,
    pub qualification_scope: String,
    pub source_digest: String,
    pub record_count: u64,
    pub recognized_records: u64,
    pub root_session_identity_sha256: Option<String>,
    pub agent_branch_identity_sha256: Option<String>,
    pub declared_model: Option<String>,
    pub model_selector: Option<String>,
    pub context_window_selector: Option<String>,
    pub state: ClaudeTaskAttributionState,
}

fn invalid() -> anyhow::Error {
    anyhow!("insights-claude-task-attribution-invalid")
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn safe_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
}

fn supported_tool(value: &str) -> bool {
    matches!(
        value,
        "Bash" | "Read" | "Edit" | "ToolSearch" | "Monitor" | "TaskStop"
    )
}

fn supported_attachment(value: &str) -> bool {
    matches!(
        value,
        "deferred_tools_delta"
            | "environment"
            | "model"
            | "skill_listing"
            | "instructions"
            | "session_context"
            | "date"
            | "hook_success"
            | "agent_listing_delta"
            | "mcp_instructions_delta"
            | "auto_mode"
            | "total_tokens_reminder"
            | "queued_command"
            | "edited_text_file"
            | "nested_memory"
    )
}

fn selector_matches(model: Option<&str>, selector: Option<&str>, context: Option<&str>) -> bool {
    match (model, selector, context) {
        (Some(model), Some(selector), None) => selector == model,
        (Some(model), Some(selector), Some("1m")) => selector.strip_suffix("[1m]") == Some(model),
        _ => false,
    }
}

fn hash(domain: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn canonical_json(value: &Value, output: &mut String) {
    match value {
        Value::Null => output.push_str("null"),
        Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        Value::Number(value) => output.push_str(&value.to_string()),
        Value::String(value) => output.push_str(&serde_json::to_string(value).expect("string")),
        Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(value, output);
            }
            output.push(']');
        }
        Value::Object(values) => {
            output.push('{');
            let sorted = values.iter().collect::<BTreeMap<_, _>>();
            for (index, (key, value)) in sorted.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).expect("key"));
                output.push(':');
                canonical_json(value, output);
            }
            output.push('}');
        }
    }
}

fn value_digest(value: &Value) -> String {
    let mut canonical = String::new();
    canonical_json(value, &mut canonical);
    hash("trace-insights-claude-json-v1", &[&canonical])
}

fn attachment_provenance(references: &[ClaudeAttachmentRef]) -> String {
    let projection = references
        .iter()
        .map(|reference| {
            format!(
                "{}:{}:{}",
                reference.physical_line, reference.kind, reference.canonical_payload_sha256
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    hash("trace-insights-claude-attachments-v1", &[&projection])
}

impl ClaudeTaskAttributionEvidence {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.extractor_version != 1
            || self.profile_id != "claude-code-v2.1.260-observed-agent-branch-v1"
            || self.observed_writer_version != WRITER_VERSION
            || self.qualification_scope != "observed_writer_agent_branch_records"
            || !valid_hash(&self.source_digest)
            || self.record_count == 0
            || self.recognized_records > self.record_count
            || self
                .root_session_identity_sha256
                .as_deref()
                .is_some_and(|value| !valid_hash(value))
            || self
                .agent_branch_identity_sha256
                .as_deref()
                .is_some_and(|value| !valid_hash(value))
        {
            return Err(invalid());
        }
        match &self.state {
            ClaudeTaskAttributionState::Attributed { branch } => {
                let declared_model = self.declared_model.as_deref();
                let model_selector = self.model_selector.as_deref();
                let attachment_lines = branch
                    .attachments
                    .iter()
                    .map(|reference| reference.physical_line)
                    .collect::<BTreeSet<_>>();
                let call_lines = branch
                    .tool_pairs
                    .iter()
                    .map(|reference| reference.call_physical_line)
                    .collect::<BTreeSet<_>>();
                let response_coordinates = branch
                    .tool_pairs
                    .iter()
                    .map(|reference| {
                        (
                            reference.response_physical_line,
                            reference.response_block_index,
                        )
                    })
                    .collect::<BTreeSet<_>>();
                let background_unique = branch
                    .background_work
                    .iter()
                    .map(|reference| (reference.physical_line, reference.tool_name.as_str()))
                    .collect::<BTreeSet<_>>();
                if self.recognized_records != self.record_count
                    || self.record_count > MAX_RECORDS as u64
                    || self.root_session_identity_sha256.is_none()
                    || self.agent_branch_identity_sha256.is_none()
                    || self.declared_model.is_none()
                    || self.model_selector.is_none()
                    || declared_model.is_none_or(|value| !safe_label(value))
                    || !selector_matches(
                        declared_model,
                        model_selector,
                        self.context_window_selector.as_deref(),
                    )
                    || branch.terminal_physical_line == 0
                    || branch.terminal_physical_line != self.record_count
                    || branch.tool_calls != branch.tool_protocol_responses
                    || !valid_hash(&branch.material_attachment_provenance_sha256)
                    || branch.material_attachment_provenance_sha256
                        != attachment_provenance(&branch.attachments)
                    || branch.attachments.len() > MAX_REFERENCES
                    || branch.tool_pairs.len() > MAX_REFERENCES
                    || branch.repeated_assistant_refs.len() > MAX_REFERENCES
                    || branch.background_work.len() > MAX_REFERENCES
                    || branch
                        .attachments
                        .iter()
                        .filter(|reference| reference.kind == "model")
                        .count()
                        != 1
                    || branch.tool_pairs.len() as u64 != branch.tool_calls
                    || branch.repeated_assistant_refs.len() as u64
                        != branch.repeated_assistant_records
                    || branch.attachments.iter().any(|reference| {
                        reference.physical_line == 0
                            || reference.physical_line >= branch.terminal_physical_line
                            || !valid_hash(&reference.canonical_payload_sha256)
                            || !supported_attachment(&reference.kind)
                    })
                    || !branch
                        .attachments
                        .windows(2)
                        .all(|pair| pair[0].physical_line < pair[1].physical_line)
                    || branch.tool_pairs.iter().any(|reference| {
                        !supported_tool(&reference.tool_name)
                            || reference.call_physical_line == 0
                            || reference.call_physical_line >= reference.response_physical_line
                            || reference.response_physical_line >= branch.terminal_physical_line
                            || !valid_hash(&reference.call_payload_sha256)
                            || !valid_hash(&reference.response_payload_sha256)
                    })
                    || !branch.tool_pairs.windows(2).all(|pair| {
                        (pair[0].call_physical_line, pair[0].call_block_index)
                            < (pair[1].call_physical_line, pair[1].call_block_index)
                    })
                    || branch.repeated_assistant_refs.iter().any(|reference| {
                        !valid_hash(&reference.message_identity_sha256)
                            || reference.first_physical_line == 0
                            || reference.first_physical_line >= reference.repeated_physical_line
                            || reference.repeated_physical_line > self.record_count
                    })
                    || !branch
                        .repeated_assistant_refs
                        .windows(2)
                        .all(|pair| pair[0].repeated_physical_line < pair[1].repeated_physical_line)
                    || branch.background_work.iter().any(|reference| {
                        reference.physical_line == 0
                            || reference.physical_line >= branch.terminal_physical_line
                            || reference
                                .task_identity_sha256
                                .as_deref()
                                .is_none_or(|value| !valid_hash(value))
                            || !matches!(reference.tool_name.as_str(), "Bash" | "Monitor")
                            || (reference.tool_name == "Monitor"
                                && (reference.monitor_persistent.is_none()
                                    || reference.monitor_timeout_ms.is_none()))
                            || (reference.tool_name == "Bash"
                                && (reference.monitor_persistent.is_some()
                                    || reference.monitor_timeout_ms.is_some()))
                            || reference.launch_status != AsyncWorkStatus::ObservedBackgroundLaunch
                            || reference.completion_status
                                != AsyncWorkStatus::AsyncWorkStatusUnavailable
                    })
                    || response_coordinates.len() != branch.tool_pairs.len()
                    || background_unique.len() != branch.background_work.len()
                    || !attachment_lines.is_disjoint(&call_lines)
                    || branch.tool_pairs.iter().any(|reference| {
                        attachment_lines.contains(&reference.response_physical_line)
                            || call_lines.contains(&reference.response_physical_line)
                    })
                    || branch.background_work.iter().any(|background| {
                        !branch.tool_pairs.iter().any(|pair| {
                            pair.response_physical_line == background.physical_line
                                && pair.tool_name == background.tool_name
                        })
                    })
                    || branch.tool_pairs.iter().any(|pair| {
                        pair.tool_name == "Monitor"
                            && !branch.background_work.iter().any(|background| {
                                background.physical_line == pair.response_physical_line
                                    && background.tool_name == pair.tool_name
                            })
                    })
                    || branch.repeated_assistant_refs.iter().any(|reference| {
                        attachment_lines.contains(&reference.first_physical_line)
                            || attachment_lines.contains(&reference.repeated_physical_line)
                            || branch.tool_pairs.iter().any(|pair| {
                                pair.response_physical_line == reference.first_physical_line
                                    || pair.response_physical_line
                                        == reference.repeated_physical_line
                            })
                    })
                {
                    return Err(invalid());
                }
            }
            ClaudeTaskAttributionState::Unavailable {
                reason,
                physical_line,
            } => {
                if physical_line.is_some_and(|line| line == 0 || line > self.record_count)
                    || (self.record_count > MAX_RECORDS as u64
                        && *reason != ClaudeTaskUnavailableReason::RecordLimitExceeded)
                    || (*reason == ClaudeTaskUnavailableReason::RecordLimitExceeded
                        && self.record_count <= MAX_RECORDS as u64)
                    || self.declared_model.is_some()
                    || self.model_selector.is_some()
                    || self.context_window_selector.is_some()
                {
                    return Err(invalid());
                }
            }
        }
        Ok(())
    }
}

fn unavailable(
    source_digest: String,
    record_count: u64,
    recognized_records: u64,
    root: Option<String>,
    branch: Option<String>,
    reason: ClaudeTaskUnavailableReason,
    physical_line: Option<u64>,
) -> Result<ClaudeTaskAttributionEvidence> {
    let evidence = ClaudeTaskAttributionEvidence {
        schema_version: 1,
        extractor_version: 1,
        profile_id: "claude-code-v2.1.260-observed-agent-branch-v1".into(),
        observed_writer_version: WRITER_VERSION.into(),
        qualification_scope: "observed_writer_agent_branch_records".into(),
        source_digest,
        record_count,
        recognized_records,
        root_session_identity_sha256: root,
        agent_branch_identity_sha256: branch,
        declared_model: None,
        model_selector: None,
        context_window_selector: None,
        state: ClaudeTaskAttributionState::Unavailable {
            reason,
            physical_line,
        },
    };
    evidence.validate()?;
    Ok(evidence)
}

fn allowed_envelope(kind: &str, object: &serde_json::Map<String, Value>) -> bool {
    let common = [
        "agentId",
        "cwd",
        "entrypoint",
        "gitBranch",
        "isSidechain",
        "parentUuid",
        "sessionId",
        "timestamp",
        "type",
        "userType",
        "uuid",
        "version",
    ];
    let specific: &[&str] = match kind {
        "user" => &[
            "message",
            "isMeta",
            "promptId",
            "sourceToolAssistantUUID",
            "toolUseResult",
            "classifierMetaLines",
        ],
        "assistant" => &["message", "apiBlockIndex", "effort", "requestId"],
        "attachment" => &["attachment", "rendered", "renderedInHumanTurn"],
        _ => return false,
    };
    let keys_allowed = object
        .keys()
        .all(|key| common.contains(&key.as_str()) || specific.contains(&key.as_str()))
        && object
            .get("classifierMetaLines")
            .is_none_or(Value::is_string);
    let common_types = [
        "agentId",
        "cwd",
        "entrypoint",
        "gitBranch",
        "sessionId",
        "timestamp",
        "type",
        "userType",
        "uuid",
        "version",
    ]
    .iter()
    .all(|key| object.get(*key).is_some_and(Value::is_string))
        && object.get("isSidechain").is_some_and(Value::is_boolean)
        && object
            .get("parentUuid")
            .is_some_and(|value| value.is_null() || value.is_string());
    let specific_types = match kind {
        "user" => {
            object.get("message").is_some_and(Value::is_object)
                && object.get("isMeta").is_none_or(Value::is_boolean)
                && object.get("promptId").is_none_or(Value::is_string)
                && object
                    .get("sourceToolAssistantUUID")
                    .is_none_or(|value| value.is_null() || value.is_string())
        }
        "assistant" => {
            object.get("message").is_some_and(Value::is_object)
                && object.get("apiBlockIndex").is_none_or(Value::is_number)
                && object.get("effort").is_none_or(Value::is_string)
                && object.get("requestId").is_none_or(Value::is_string)
        }
        "attachment" => {
            object.get("attachment").is_some_and(Value::is_object)
                && object.get("rendered").is_none_or(Value::is_array)
                && object
                    .get("renderedInHumanTurn")
                    .is_none_or(Value::is_array)
        }
        _ => false,
    };
    keys_allowed && common_types && specific_types
}

fn allowed_attachment(kind: &str, object: &serde_json::Map<String, Value>) -> bool {
    let keys = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let exact = |shape: &[&str]| keys == shape.iter().copied().collect::<BTreeSet<_>>();
    let strings = |key: &str| {
        object.get(key).is_some_and(|value| {
            value
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_string))
        })
    };
    let string = |key: &str| object.get(key).is_some_and(Value::is_string);
    let boolean = |key: &str| object.get(key).is_some_and(Value::is_boolean);
    let number = |key: &str| object.get(key).is_some_and(Value::is_number);
    match kind {
        "deferred_tools_delta" => {
            exact(&[
                "type",
                "addedLines",
                "addedNames",
                "readdedNames",
                "removedNames",
                "wireHiddenNames",
            ]) && strings("addedLines")
                && strings("addedNames")
                && strings("readdedNames")
                && strings("removedNames")
                && strings("wireHiddenNames")
        }
        "environment" => {
            (exact(&["type", "snapshot"]) || exact(&["type", "snapshot", "changes"]))
                && object.get("snapshot").is_some_and(Value::is_object)
                && object.get("changes").is_none_or(Value::is_array)
        }
        "model" => exact(&["type", "identity", "text"]) && string("text"),
        "skill_listing" => {
            exact(&["type", "content", "isInitial", "names", "skillCount"])
                && string("content")
                && boolean("isInitial")
                && strings("names")
                && number("skillCount")
        }
        "instructions" => {
            exact(&["type", "files"]) && object.get("files").is_some_and(Value::is_array)
        }
        "session_context" => {
            exact(&["type", "context"]) && object.get("context").is_some_and(Value::is_object)
        }
        "date" => exact(&["type", "date"]) && string("date"),
        "hook_success" => {
            exact(&[
                "type",
                "command",
                "content",
                "durationMs",
                "exitCode",
                "hookEvent",
                "hookName",
                "stderr",
                "stdout",
                "toolUseID",
            ]) && string("command")
                && string("content")
                && number("durationMs")
                && number("exitCode")
                && string("hookEvent")
                && string("hookName")
                && string("stderr")
                && string("stdout")
                && string("toolUseID")
        }
        "agent_listing_delta" => {
            exact(&[
                "type",
                "addedLines",
                "addedTypes",
                "isInitial",
                "removedTypes",
                "showConcurrencyNote",
            ]) && strings("addedLines")
                && strings("addedTypes")
                && boolean("isInitial")
                && strings("removedTypes")
                && boolean("showConcurrencyNote")
        }
        "mcp_instructions_delta" => {
            exact(&["type", "addedBlocks", "addedNames", "removedNames"])
                && strings("addedBlocks")
                && strings("addedNames")
                && strings("removedNames")
        }
        "auto_mode" => {
            exact(&[
                "type",
                "autoModeConsentFlow",
                "bashFirst",
                "bashFirstSteer",
                "bypass",
                "steerOnly",
            ]) && boolean("autoModeConsentFlow")
                && boolean("bashFirst")
                && string("bashFirstSteer")
                && boolean("bypass")
                && boolean("steerOnly")
        }
        "total_tokens_reminder" => exact(&["type", "text"]) && string("text"),
        "queued_command" => {
            exact(&["type", "commandMode", "prompt", "timestamp"])
                && string("commandMode")
                && string("prompt")
                && string("timestamp")
        }
        "edited_text_file" => {
            exact(&["type", "filename", "snippet"]) && string("filename") && string("snippet")
        }
        "nested_memory" => {
            exact(&["type", "content", "displayPath", "path"])
                && object.get("content").is_some_and(Value::is_object)
                && string("displayPath")
                && string("path")
        }
        _ => false,
    }
}

fn source_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|parsed| parsed.hyphenated().to_string() == value)
}

fn bounded_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn supported_assistant_block(block: &serde_json::Map<String, Value>) -> bool {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block.get("text").is_some_and(Value::is_string),
        Some("thinking") => block.get("thinking").is_some_and(Value::is_string),
        Some("tool_use") => {
            block.get("id").is_some_and(Value::is_string)
                && block.get("name").is_some_and(Value::is_string)
                && block.get("input").is_some_and(Value::is_object)
        }
        _ => false,
    }
}

fn supported_user_block(block: &serde_json::Map<String, Value>) -> bool {
    let content_supported = block.get("content").is_some_and(|content| {
        content.is_string()
            || content.as_array().is_some_and(|items| {
                items.iter().all(|item| {
                    item.as_object().is_some_and(|item| {
                        item.keys().map(String::as_str).collect::<BTreeSet<_>>()
                            == ["tool_name", "type"].into_iter().collect()
                            && item.get("type").and_then(Value::as_str) == Some("tool_reference")
                            && item
                                .get("tool_name")
                                .and_then(Value::as_str)
                                .is_some_and(bounded_identity)
                    })
                })
            })
    });
    block.get("type").and_then(Value::as_str) == Some("tool_result")
        && block.get("tool_use_id").is_some_and(Value::is_string)
        && content_supported
        && block.get("is_error").is_none_or(|value| value.is_boolean())
}

fn supported_native_response(name: &str, value: Option<&Value>) -> bool {
    let Some(value) = value else {
        return false;
    };
    match name {
        "Monitor" => value.as_object().is_some_and(|object| {
            object.keys().map(String::as_str).collect::<BTreeSet<_>>()
                == ["persistent", "taskId", "timeoutMs"].into_iter().collect()
                && object.get("persistent").is_some_and(Value::is_boolean)
                && object.get("taskId").is_some_and(Value::is_string)
                && object.get("timeoutMs").and_then(Value::as_u64).is_some()
        }),
        "TaskStop" => value.as_object().is_some_and(|object| {
            object.keys().map(String::as_str).collect::<BTreeSet<_>>()
                == ["command", "message", "task_id", "task_type"]
                    .into_iter()
                    .collect()
                && ["command", "message", "task_id", "task_type"]
                    .iter()
                    .all(|key| object.get(*key).is_some_and(Value::is_string))
        }),
        "Bash" | "Read" | "Edit" | "ToolSearch" => value.is_object() || value.is_string(),
        _ => false,
    }
}

fn supported_tool_input(name: &str, input: &serde_json::Map<String, Value>) -> bool {
    let keys = input.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let allowed = |names: &[&str]| keys.iter().all(|key| names.contains(key));
    let string = |key: &str| input.get(key).is_some_and(Value::is_string);
    match name {
        "Bash" => {
            allowed(&["command", "description", "timeout", "run_in_background"])
                && string("command")
                && input.get("description").is_none_or(Value::is_string)
                && input.get("timeout").is_none_or(Value::is_number)
                && input.get("run_in_background").is_none_or(Value::is_boolean)
        }
        "Read" => keys == ["file_path"].into_iter().collect() && string("file_path"),
        "Edit" => {
            keys == ["file_path", "new_string", "old_string", "replace_all"]
                .into_iter()
                .collect()
                && string("file_path")
                && string("new_string")
                && string("old_string")
                && input.get("replace_all").is_some_and(Value::is_boolean)
        }
        "ToolSearch" => {
            keys == ["max_results", "query"].into_iter().collect()
                && string("query")
                && input.get("max_results").is_some_and(Value::is_number)
        }
        "Monitor" => {
            keys == ["command", "description", "persistent", "timeout_ms"]
                .into_iter()
                .collect()
                && string("command")
                && string("description")
                && input.get("persistent").is_some_and(Value::is_boolean)
                && input.get("timeout_ms").is_some_and(Value::is_number)
        }
        "TaskStop" => keys == ["task_id"].into_iter().collect() && string("task_id"),
        _ => false,
    }
}

pub fn classify_claude_task_attribution(bytes: &[u8]) -> Result<ClaudeTaskAttributionEvidence> {
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_BYTES {
        return Err(anyhow!("insights_source_size_invalid"));
    }
    let digest = source_digest(bytes);
    let lines = std::str::from_utf8(bytes)
        .map_err(|_| anyhow!("insights_claude_task_attribution_invalid_utf8"))?
        .lines()
        .collect::<Vec<_>>();
    let count = u64::try_from(lines.len()).map_err(|_| invalid())?;
    if lines.is_empty() || lines.len() > MAX_RECORDS {
        return unavailable(
            digest,
            count.max(1),
            0,
            None,
            None,
            ClaudeTaskUnavailableReason::RecordLimitExceeded,
            None,
        );
    }

    let mut session: Option<String> = None;
    let mut agent: Option<String> = None;
    let mut seen_uuids = BTreeSet::new();
    let mut recognized = 0_u64;
    let mut attachments = Vec::new();
    let mut model_selector: Option<String> = None;
    let mut declared_model: Option<String> = None;
    let mut message_ids = BTreeMap::<String, u64>::new();
    let mut repeated_assistant_records = 0_u64;
    let mut repeated_assistant_refs = Vec::new();
    let mut calls = BTreeMap::<String, (u64, u64, String, String, bool, String)>::new();
    let mut responses = BTreeSet::new();
    let mut tool_pairs = Vec::new();
    let mut hook_ids = Vec::<(u64, String)>::new();
    let mut terminal = None;
    let mut background = Vec::new();

    macro_rules! fail {
        ($reason:expr, $line:expr) => {{
            let root = session
                .as_deref()
                .map(|value| hash("trace-insights-claude-root-session-v1", &[value]));
            let branch = session
                .as_deref()
                .zip(agent.as_deref())
                .map(|(s, a)| hash("trace-insights-claude-agent-branch-v1", &[s, a]));
            return unavailable(
                digest,
                count,
                recognized,
                root,
                branch,
                $reason,
                Some($line),
            );
        }};
    }

    for (offset, raw) in lines.iter().enumerate() {
        let line = u64::try_from(offset + 1).map_err(|_| invalid())?;
        let value: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(_) => fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line),
        };
        let Some(object) = value.as_object() else {
            fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
        };
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
        };
        if !allowed_envelope(kind, object) {
            fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
        }
        let (Some(version), Some(session_id), Some(agent_id), Some(uuid), Some(sidechain)) = (
            object.get("version").and_then(Value::as_str),
            object.get("sessionId").and_then(Value::as_str),
            object.get("agentId").and_then(Value::as_str),
            object.get("uuid").and_then(Value::as_str),
            object.get("isSidechain").and_then(Value::as_bool),
        ) else {
            fail!(ClaudeTaskUnavailableReason::IdentityConflict, line)
        };
        if version != WRITER_VERSION {
            fail!(ClaudeTaskUnavailableReason::SourceProfileMismatch, line)
        }
        if !sidechain
            || !canonical_uuid(uuid)
            || !canonical_uuid(session_id)
            || !bounded_identity(agent_id)
            || session.as_deref().is_some_and(|known| known != session_id)
            || agent.as_deref().is_some_and(|known| known != agent_id)
            || seen_uuids.contains(uuid)
        {
            fail!(ClaudeTaskUnavailableReason::IdentityConflict, line)
        }
        match object.get("parentUuid") {
            Some(Value::Null) if line == 1 => {}
            Some(Value::String(parent))
                if canonical_uuid(parent) && seen_uuids.contains(parent) => {}
            _ => fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line),
        }
        seen_uuids.insert(uuid.to_owned());
        session.get_or_insert_with(|| session_id.to_owned());
        agent.get_or_insert_with(|| agent_id.to_owned());

        match kind {
            "attachment" => {
                let Some(attachment) = object.get("attachment").and_then(Value::as_object) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedAttachment, line)
                };
                let Some(attachment_kind) = attachment.get("type").and_then(Value::as_str) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedAttachment, line)
                };
                if !allowed_attachment(attachment_kind, attachment) {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedAttachment, line)
                }
                if attachment_kind == "model" {
                    if model_selector.is_some() {
                        fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                    }
                    let Some(identity) = attachment.get("identity").and_then(Value::as_object)
                    else {
                        fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                    };
                    if identity.keys().map(String::as_str).collect::<BTreeSet<_>>()
                        != ["knowledgeCutoff", "marketingName", "modelId"]
                            .iter()
                            .copied()
                            .collect::<BTreeSet<_>>()
                        || identity.values().any(|value| !value.is_string())
                    {
                        fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                    }
                    let Some(model_id) = identity.get("modelId").and_then(Value::as_str) else {
                        fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                    };
                    model_selector = Some(model_id.to_owned());
                }
                if attachment_kind == "hook_success" {
                    let Some(id) = attachment.get("toolUseID").and_then(Value::as_str) else {
                        fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line)
                    };
                    if !calls.contains_key(id) {
                        fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line)
                    }
                    hook_ids.push((line, id.to_owned()));
                }
                attachments.push(ClaudeAttachmentRef {
                    kind: attachment_kind.to_owned(),
                    physical_line: line,
                    canonical_payload_sha256: value_digest(&Value::Object(attachment.clone())),
                });
            }
            "assistant" => {
                let Some(message) = object.get("message").and_then(Value::as_object) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                };
                let Some(model) = message.get("model").and_then(Value::as_str) else {
                    fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                };
                if model == "<synthetic>"
                    || model.is_empty()
                    || declared_model
                        .as_deref()
                        .is_some_and(|known| known != model)
                {
                    fail!(ClaudeTaskUnavailableReason::ModelConflict, line)
                }
                declared_model.get_or_insert_with(|| model.to_owned());
                let Some(message_id) = message.get("id").and_then(Value::as_str) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                };
                if !bounded_identity(message_id) {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                }
                {
                    let id = message_id;
                    if let Some(first_line) = message_ids.get(id).copied() {
                        repeated_assistant_records += 1;
                        repeated_assistant_refs.push(ClaudeRepeatedAssistantRef {
                            message_identity_sha256: hash(
                                "trace-insights-claude-message-v1",
                                &[id],
                            ),
                            first_physical_line: first_line,
                            repeated_physical_line: line,
                        });
                    } else {
                        message_ids.insert(id.to_owned(), line);
                    }
                }
                let Some(content) = message.get("content").and_then(Value::as_array) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                };
                for (block_index, block) in content.iter().enumerate() {
                    let Some(block) = block.as_object() else {
                        fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                    };
                    if !supported_assistant_block(block) {
                        fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                    }
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        let (Some(id), Some(name)) = (
                            block.get("id").and_then(Value::as_str),
                            block.get("name").and_then(Value::as_str),
                        ) else {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                        };
                        if matches!(name, "Agent" | "Task") {
                            fail!(ClaudeTaskUnavailableReason::NestedDelegation, line)
                        }
                        if !matches!(
                            name,
                            "Bash" | "Read" | "Edit" | "ToolSearch" | "Monitor" | "TaskStop"
                        ) {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                        }
                        let Some(input) = block.get("input").and_then(Value::as_object) else {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                        };
                        if !supported_tool_input(name, input) {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                        }
                        let digest = value_digest(&Value::Object(block.clone()));
                        let background_bash = name == "Bash"
                            && block
                                .get("input")
                                .and_then(Value::as_object)
                                .and_then(|input| input.get("run_in_background"))
                                .and_then(Value::as_bool)
                                == Some(true);
                        if let Some((
                            _,
                            _,
                            known_name,
                            known_digest,
                            known_background,
                            known_message,
                        )) = calls.get(id)
                        {
                            if known_name != name
                                || known_digest != &digest
                                || *known_background != background_bash
                                || known_message != message_id
                            {
                                fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line)
                            }
                        } else {
                            calls.insert(
                                id.to_owned(),
                                (
                                    line,
                                    u64::try_from(block_index).map_err(|_| invalid())?,
                                    name.to_owned(),
                                    digest,
                                    background_bash,
                                    message_id.to_owned(),
                                ),
                            );
                        }
                    }
                }
                match message.get("stop_reason") {
                    Some(Value::String(reason)) if reason == "end_turn" => {
                        if terminal.replace(line).is_some()
                            || message
                                .get("stop_sequence")
                                .is_some_and(|value| !value.is_null())
                        {
                            fail!(ClaudeTaskUnavailableReason::Interrupted, line)
                        }
                    }
                    Some(Value::String(reason)) if reason == "stop_sequence" => {
                        fail!(ClaudeTaskUnavailableReason::Interrupted, line)
                    }
                    _ => {}
                }
            }
            "user" => {
                let Some(message) = object.get("message").and_then(Value::as_object) else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                };
                let Some(content_value) = message.get("content") else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                };
                if content_value.is_string() {
                    // A recorded user text starts the selected agent branch. It is
                    // not attributed as a human-authored task prompt.
                } else if let Some(content) = content_value.as_array() {
                    for (block_index, block) in content.iter().enumerate() {
                        let Some(block_object) = block.as_object() else {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                        };
                        if !supported_user_block(block_object) {
                            fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                        }
                        if let Some(id) = block_object.get("tool_use_id").and_then(Value::as_str) {
                            if !calls.contains_key(id) || !responses.insert(id.to_owned()) {
                                fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line)
                            }
                            let (
                                call_line,
                                call_block_index,
                                name,
                                call_digest,
                                background_bash,
                                _,
                            ) = &calls[id];
                            if block_object.get("content").is_some_and(Value::is_array)
                                && name != "ToolSearch"
                            {
                                fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                            }
                            if !supported_native_response(name, object.get("toolUseResult")) {
                                fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                            }
                            tool_pairs.push(ClaudeToolPairRef {
                                tool_name: name.clone(),
                                call_physical_line: *call_line,
                                call_block_index: *call_block_index,
                                response_physical_line: line,
                                response_block_index: u64::try_from(block_index)
                                    .map_err(|_| invalid())?,
                                call_payload_sha256: call_digest.clone(),
                                response_payload_sha256: value_digest(block),
                            });
                            let native = object.get("toolUseResult").and_then(Value::as_object);
                            let native_background_bash = name == "Bash"
                                && native
                                    .and_then(|result| result.get("backgroundTaskId"))
                                    .and_then(Value::as_str)
                                    .is_some();
                            if name == "Monitor" || *background_bash || native_background_bash {
                                let task_id = if name == "Monitor" {
                                    native.and_then(|result| result.get("taskId"))
                                } else {
                                    native.and_then(|result| result.get("backgroundTaskId"))
                                }
                                .and_then(Value::as_str);
                                let Some(task_id) = task_id else {
                                    fail!(ClaudeTaskUnavailableReason::UnsupportedTool, line)
                                };
                                let (monitor_persistent, monitor_timeout_ms) = if name == "Monitor"
                                {
                                    (
                                        native
                                            .and_then(|result| result.get("persistent"))
                                            .and_then(Value::as_bool),
                                        native
                                            .and_then(|result| result.get("timeoutMs"))
                                            .and_then(Value::as_u64),
                                    )
                                } else {
                                    (None, None)
                                };
                                background.push(ClaudeBackgroundWorkRef {
                                    physical_line: line,
                                    tool_name: name.clone(),
                                    task_identity_sha256: Some(hash(
                                        "trace-insights-claude-background-task-v1",
                                        &[task_id],
                                    )),
                                    monitor_persistent,
                                    monitor_timeout_ms,
                                    launch_status: AsyncWorkStatus::ObservedBackgroundLaunch,
                                    completion_status: AsyncWorkStatus::AsyncWorkStatusUnavailable,
                                });
                            }
                        }
                    }
                } else {
                    fail!(ClaudeTaskUnavailableReason::UnsupportedRecord, line)
                }
            }
            _ => unreachable!(),
        }
        if terminal.is_some_and(|terminal_line| line > terminal_line) {
            fail!(ClaudeTaskUnavailableReason::InvalidRecordOrder, line)
        }
        recognized += 1;
    }

    let Some(terminal_line) = terminal else {
        return unavailable(
            digest,
            count,
            recognized,
            session
                .as_deref()
                .map(|s| hash("trace-insights-claude-root-session-v1", &[s])),
            session
                .as_deref()
                .zip(agent.as_deref())
                .map(|(s, a)| hash("trace-insights-claude-agent-branch-v1", &[s, a])),
            ClaudeTaskUnavailableReason::MissingRequiredRecord,
            None,
        );
    };
    if responses.len() != calls.len() {
        return unavailable(
            digest,
            count,
            recognized,
            session
                .as_deref()
                .map(|s| hash("trace-insights-claude-root-session-v1", &[s])),
            session
                .as_deref()
                .zip(agent.as_deref())
                .map(|(s, a)| hash("trace-insights-claude-agent-branch-v1", &[s, a])),
            ClaudeTaskUnavailableReason::MissingRequiredRecord,
            None,
        );
    }
    if hook_ids.iter().any(|(_, id)| !calls.contains_key(id)) {
        return unavailable(
            digest,
            count,
            recognized,
            session
                .as_deref()
                .map(|s| hash("trace-insights-claude-root-session-v1", &[s])),
            session
                .as_deref()
                .zip(agent.as_deref())
                .map(|(s, a)| hash("trace-insights-claude-agent-branch-v1", &[s, a])),
            ClaudeTaskUnavailableReason::InvalidRecordOrder,
            None,
        );
    }
    tool_pairs.sort_by_key(|reference| (reference.call_physical_line, reference.call_block_index));
    let known_root = session
        .as_deref()
        .map(|value| hash("trace-insights-claude-root-session-v1", &[value]));
    let known_branch = session
        .as_deref()
        .zip(agent.as_deref())
        .map(|(s, a)| hash("trace-insights-claude-agent-branch-v1", &[s, a]));
    let (Some(model), Some(selector)) = (declared_model, model_selector) else {
        return unavailable(
            digest,
            count,
            recognized,
            known_root,
            known_branch,
            ClaudeTaskUnavailableReason::ModelConflict,
            None,
        );
    };
    if !safe_label(&model) || selector.len() > 132 {
        return unavailable(
            digest,
            count,
            recognized,
            known_root,
            known_branch,
            ClaudeTaskUnavailableReason::ModelConflict,
            None,
        );
    }
    let context_window_selector = if selector == model {
        None
    } else if selector.strip_suffix("[1m]") == Some(model.as_str()) {
        Some("1m".to_owned())
    } else {
        return unavailable(
            digest,
            count,
            recognized,
            known_root,
            known_branch,
            ClaudeTaskUnavailableReason::ModelConflict,
            None,
        );
    };
    let material_attachment_provenance_sha256 = attachment_provenance(&attachments);
    let root = hash(
        "trace-insights-claude-root-session-v1",
        &[session.as_deref().ok_or_else(invalid)?],
    );
    let branch_id = hash(
        "trace-insights-claude-agent-branch-v1",
        &[
            session.as_deref().ok_or_else(invalid)?,
            agent.as_deref().ok_or_else(invalid)?,
        ],
    );
    let evidence = ClaudeTaskAttributionEvidence {
        schema_version: 1,
        extractor_version: 1,
        profile_id: "claude-code-v2.1.260-observed-agent-branch-v1".into(),
        observed_writer_version: WRITER_VERSION.into(),
        qualification_scope: "observed_writer_agent_branch_records".into(),
        source_digest: digest,
        record_count: count,
        recognized_records: recognized,
        root_session_identity_sha256: Some(root),
        agent_branch_identity_sha256: Some(branch_id),
        declared_model: Some(model),
        model_selector: Some(selector),
        context_window_selector,
        state: ClaudeTaskAttributionState::Attributed {
            branch: ClaudeAttributedBranch {
                terminal_physical_line: terminal_line,
                termination: ClaudeObservedTermination::ObservedBranchSegmentEnded,
                tool_calls: calls.len() as u64,
                tool_protocol_responses: responses.len() as u64,
                repeated_assistant_records,
                tool_pairs,
                repeated_assistant_refs,
                material_attachment_provenance_sha256,
                attachments,
                background_work: background,
            },
        },
    };
    evidence.validate()?;
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_uuid(label: &str) -> String {
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, label.as_bytes()).to_string()
    }

    fn record(kind: &str, uuid: &str, parent: Option<&str>, body: Value) -> Value {
        let mut value = json!({
            "type": kind,
            "uuid": test_uuid(uuid),
            "parentUuid": parent.map(test_uuid),
            "sessionId": "11111111-1111-4111-8111-111111111111",
            "agentId": "agent-a",
            "isSidechain": true,
            "version": "2.1.260",
            "cwd": "/redacted",
            "entrypoint": "cli",
            "gitBranch": "branch",
            "timestamp": "2026-09-12T00:00:00Z",
            "userType": "external"
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        value
    }

    fn bytes(records: &[Value]) -> Vec<u8> {
        let mut encoded = records
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        encoded.push('\n');
        encoded.into_bytes()
    }

    fn minimal(model: &str, stop_reason: &str) -> Vec<Value> {
        vec![
            record(
                "user",
                "u1",
                None,
                json!({"message":{"role":"user","content":"not retained"}}),
            ),
            record(
                "attachment",
                "u2",
                Some("u1"),
                json!({"attachment":{"type":"model","identity":{"modelId":"claude-opus-5[1m]","marketingName":"Opus","knowledgeCutoff":"redacted"},"text":"not retained"}}),
            ),
            record(
                "assistant",
                "u3",
                Some("u2"),
                json!({"message":{"id":"m1","type":"message","role":"assistant","model":model,"content":[{"type":"text","text":"not retained"}],"stop_reason":stop_reason,"stop_sequence":null}}),
            ),
        ]
    }

    #[test]
    fn qualifies_observed_terminal_branch_without_completion_claims() {
        let evidence =
            classify_claude_task_attribution(&bytes(&minimal("claude-opus-5", "end_turn")))
                .unwrap();
        evidence.validate().unwrap();
        let ClaudeTaskAttributionState::Attributed { branch } = evidence.state else {
            panic!("expected attributed branch")
        };
        assert_eq!(
            branch.termination,
            ClaudeObservedTermination::ObservedBranchSegmentEnded
        );
        assert_eq!(branch.terminal_physical_line, 3);
        assert_eq!(evidence.context_window_selector.as_deref(), Some("1m"));

        let mut bare = minimal("claude-opus-5", "end_turn");
        bare[0]["classifierMetaLines"] = json!("opaque and not interpreted");
        bare[1]["attachment"]["identity"]["modelId"] = json!("claude-opus-5");
        let bare = classify_claude_task_attribution(&bytes(&bare)).unwrap();
        assert!(bare.context_window_selector.is_none());
    }

    #[test]
    fn repeated_snapshots_do_not_drop_blocks_or_duplicate_logical_calls() {
        let tool = json!({"type":"tool_use","id":"call-1","name":"Monitor","input":{"command":"run","description":"watch","persistent":true,"timeout_ms":0}});
        let mut records = minimal("claude-opus-5", "tool_use");
        records[2]["message"]["content"] = json!([tool.clone()]);
        records.push(record(
            "assistant",
            "u4",
            Some("u3"),
            json!({"message":{"id":"m1","type":"message","role":"assistant","model":"claude-opus-5","content":[tool],"stop_reason":"tool_use","stop_sequence":null}}),
        ));
        records.push(record(
            "user",
            "u5",
            Some("u4"),
            json!({"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"not retained"}]},"toolUseResult":{"taskId":"background-private","timeoutMs":0,"persistent":true},"sourceToolAssistantUUID":"u4"}),
        ));
        records.push(record(
            "assistant",
            "u6",
            Some("u5"),
            json!({"message":{"id":"m2","type":"message","role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn","stop_sequence":null}}),
        ));

        let mut invalid_timeout = records.clone();
        invalid_timeout[4]["toolUseResult"]["timeoutMs"] = json!(-1);
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&invalid_timeout))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::UnsupportedTool,
                ..
            }
        ));
        let evidence = classify_claude_task_attribution(&bytes(&records)).unwrap();
        let mut missing_background = evidence.clone();
        let ClaudeTaskAttributionState::Attributed { branch } = &mut missing_background.state
        else {
            panic!("expected attributed branch")
        };
        branch.background_work.clear();
        assert!(missing_background.validate().is_err());
        let ClaudeTaskAttributionState::Attributed { branch } = evidence.state else {
            panic!("expected attributed branch")
        };
        assert_eq!(branch.tool_calls, 1);
        assert_eq!(branch.tool_protocol_responses, 1);
        assert_eq!(branch.repeated_assistant_records, 1);
        assert_eq!(branch.background_work.len(), 1);
        assert_eq!(
            branch.background_work[0].completion_status,
            AsyncWorkStatus::AsyncWorkStatusUnavailable
        );
        assert_ne!(
            branch.background_work[0].task_identity_sha256.as_deref(),
            Some("background-private")
        );
    }

    #[test]
    fn synthetic_interrupted_and_nested_delegation_remain_typed_unavailable() {
        let synthetic =
            classify_claude_task_attribution(&bytes(&minimal("<synthetic>", "stop_sequence")))
                .unwrap();
        assert!(matches!(
            synthetic.state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::ModelConflict,
                ..
            }
        ));

        let mut delegated = minimal("claude-opus-5", "tool_use");
        delegated[2]["message"]["content"] =
            json!([{"type":"tool_use","id":"call-1","name":"Agent","input":{}}]);
        let delegated = classify_claude_task_attribution(&bytes(&delegated)).unwrap();
        assert!(matches!(
            delegated.state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::NestedDelegation,
                ..
            }
        ));
    }

    #[test]
    fn native_background_marker_is_retained_without_an_explicit_input_flag() {
        let mut records = minimal("claude-opus-5", "tool_use");
        records[2]["message"]["content"] = json!([{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":"run","description":"background"}}]);
        records.push(record(
            "user",
            "u4",
            Some("u3"),
            json!({"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call-1","content":"not retained"}]},"toolUseResult":{"backgroundTaskId":"background-private","stdout":"","stderr":"","interrupted":false},"sourceToolAssistantUUID":"u3"}),
        ));
        records.push(record(
            "assistant",
            "u5",
            Some("u4"),
            json!({"message":{"id":"m2","type":"message","role":"assistant","model":"claude-opus-5","content":[{"type":"text","text":"done"}],"stop_reason":"end_turn","stop_sequence":null}}),
        ));
        let evidence = classify_claude_task_attribution(&bytes(&records)).unwrap();
        let ClaudeTaskAttributionState::Attributed { branch } = evidence.state else {
            panic!("expected attributed branch")
        };
        assert_eq!(branch.background_work.len(), 1);
        assert_eq!(branch.background_work[0].tool_name, "Bash");
        assert_eq!(
            branch.background_work[0].completion_status,
            AsyncWorkStatus::AsyncWorkStatusUnavailable
        );
    }

    #[test]
    fn finite_profile_rejects_unknown_shapes_and_self_parent_cycles() {
        let base = record(
            "user",
            "1",
            None,
            json!({"message":{"role":"user","content":"synthetic"}}),
        );
        let malformed_attachment = record(
            "attachment",
            "2",
            Some("1"),
            json!({"attachment":{"type":"auto_mode","autoModeConsentFlow":{},"bashFirst":false,"bashFirstSteer":"","bypass":false,"steerOnly":false}}),
        );
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&[base.clone(), malformed_attachment]))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::UnsupportedAttachment,
                ..
            }
        ));

        let rendered_arrays = record(
            "attachment",
            "2a",
            Some("1"),
            json!({"attachment":{"type":"model","identity":{"modelId":"claude-opus-5","marketingName":"Opus","knowledgeCutoff":"fixture"},"text":"selector"},"rendered":[],"renderedInHumanTurn":[]}),
        );
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&[base.clone(), rendered_arrays]))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::MissingRequiredRecord,
                ..
            }
        ));
        let malformed_rendered = record(
            "attachment",
            "2b",
            Some("1"),
            json!({"attachment":{"type":"model","identity":{"modelId":"claude-opus-5","marketingName":"Opus","knowledgeCutoff":"fixture"},"text":"selector"},"rendered":false}),
        );
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&[base.clone(), malformed_rendered]))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::UnsupportedRecord,
                ..
            }
        ));

        let unknown_block = record(
            "assistant",
            "2",
            Some("1"),
            json!({"message":{"id":"message-a","type":"message","role":"assistant","model":"claude-opus-5","content":[{"type":"unsupported_delegation"}],"stop_reason":"end_turn","stop_sequence":null}}),
        );
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&[base, unknown_block]))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::UnsupportedRecord,
                ..
            }
        ));

        let self_parent = record(
            "user",
            "1",
            Some("1"),
            json!({"message":{"role":"user","content":"synthetic"}}),
        );
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&[self_parent]))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::InvalidRecordOrder,
                ..
            }
        ));

        let mut uppercase_session = minimal("claude-opus-5", "end_turn");
        for record in &mut uppercase_session {
            record["sessionId"] = json!("AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA");
        }
        assert!(matches!(
            classify_claude_task_attribution(&bytes(&uppercase_session))
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::IdentityConflict,
                ..
            }
        ));
    }

    #[test]
    fn persisted_cross_references_and_optional_failures_fail_closed() {
        let fixture =
            include_bytes!("../../fixtures/insights/claude-task-attribution/agent-alpha.jsonl");
        let valid = classify_claude_task_attribution(fixture).unwrap();
        valid.validate().unwrap();
        let mut missing_model = valid.clone();
        let ClaudeTaskAttributionState::Attributed { branch } = &mut missing_model.state else {
            panic!("fixture must qualify")
        };
        branch.attachments.clear();
        branch.material_attachment_provenance_sha256 = attachment_provenance(&[]);
        assert!(missing_model.validate().is_err());

        let mut early_terminal = valid;
        let ClaudeTaskAttributionState::Attributed { branch } = &mut early_terminal.state else {
            panic!("fixture must qualify")
        };
        branch.terminal_physical_line -= 1;
        assert!(early_terminal.validate().is_err());

        let too_many = b"{}\n".repeat(MAX_RECORDS + 1);
        assert!(matches!(
            classify_claude_task_attribution(&too_many).unwrap().state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::RecordLimitExceeded,
                ..
            }
        ));

        let invalid_model = String::from_utf8(fixture.to_vec())
            .unwrap()
            .replace("claude-opus-5", "bad model");
        assert!(matches!(
            classify_claude_task_attribution(invalid_model.as_bytes())
                .unwrap()
                .state,
            ClaudeTaskAttributionState::Unavailable {
                reason: ClaudeTaskUnavailableReason::ModelConflict,
                ..
            }
        ));
    }
}
