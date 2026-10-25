//! OpenCode 2's typed session-transfer envelope. Native exports do not contain
//! a producer version or per-text-part identities; neither is manufactured.
use super::super::{
    SOURCE_OPENCODE, SessionEvent, SessionEventKind, SessionTranscript, session_hash,
};
use super::{field, id, timestamp};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{borrow::Cow, collections::HashSet, path::Path};

fn observed_model(value: &Value) -> Result<String> {
    Ok(format!(
        "{}/{}",
        field(value, "providerID")?,
        field(value, "id")?
    ))
}

fn opaque(kind: &str, created: DateTime<Utc>) -> SessionEvent {
    SessionEvent {
        structured: Value::Null,
        ..SessionEvent::opaque(kind, Some(created))
    }
}

struct Budget {
    remaining: usize,
}
impl Budget {
    fn take(&mut self, count: usize) -> Result<()> {
        self.remaining = self
            .remaining
            .checked_sub(count)
            .ok_or_else(|| anyhow!("opencode_record_budget"))?;
        Ok(())
    }
}

pub(super) fn parse(doc: &Value, bytes: &[u8], record_budget: usize) -> Result<SessionTranscript> {
    let info = &doc["info"];
    let session_id = id(info, "id")?;
    if !session_id.starts_with("ses_") || info.get("directory").is_some() {
        bail!("opencode_invalid_v2_session");
    }
    id(info, "projectID")?;
    let cwd = field(&info["location"], "directory")?;
    if cwd.is_empty() || cwd.contains('\0') {
        bail!("opencode_invalid_field");
    }
    let started_at = timestamp(&info["time"]["created"])?;
    if timestamp(&info["time"]["updated"])? < started_at {
        bail!("opencode_invalid_timestamp");
    }
    if !info["cost"].is_number() {
        bail!("opencode_invalid_v2_session");
    }
    for value in [
        &info["tokens"]["input"],
        &info["tokens"]["output"],
        &info["tokens"]["reasoning"],
        &info["tokens"]["cache"]["read"],
        &info["tokens"]["cache"]["write"],
    ] {
        if value.as_u64().is_none() {
            bail!("opencode_invalid_v2_session");
        }
    }
    let messages = doc["messages"]
        .as_array()
        .ok_or_else(|| anyhow!("opencode_missing_messages"))?;
    if messages.len() > record_budget {
        bail!("opencode_record_budget");
    }
    let mut budget = Budget {
        remaining: record_budget,
    };
    let mut seen_messages = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut models = HashSet::new();
    let mut events = Vec::new();
    let mut previous = None;
    for message in messages {
        let message_id = id(message, "id")?;
        if !message_id.starts_with("msg_")
            || !seen_messages.insert(message_id)
            || message.get("sessionID").is_some()
            || message.get("info").is_some()
        {
            bail!("opencode_ambiguous_message_id");
        }
        let created = timestamp(&message["time"]["created"])?;
        if previous.is_some_and(|p| p > created) {
            bail!("opencode_reordered_messages");
        }
        previous = Some(created);
        let kind = field(message, "type")?;
        match kind {
            "user" => {
                events.push(SessionEvent::user(field(message, "text")?, Some(created)));
                // Attachment bodies and URLs are never opened or copied.
                if let Some(files) = message.get("files") {
                    let files = files
                        .as_array()
                        .ok_or_else(|| anyhow!("opencode_invalid_field"))?;
                    budget.take(files.len())?;
                    for _ in files {
                        events.push(opaque("file", created));
                    }
                }
            }
            "assistant" => {
                models.insert(observed_model(&message["model"])?);
                if let Some(completed) = message["time"].get("completed") {
                    if timestamp(completed)? < created {
                        bail!("opencode_invalid_timestamp");
                    }
                }
                let content = message["content"]
                    .as_array()
                    .ok_or_else(|| anyhow!("opencode_missing_parts"))?;
                budget.take(content.len())?;
                for part in content {
                    match field(part, "type")? {
                        "text" | "reasoning" => {
                            let reasoning = field(part, "type")? == "reasoning";
                            let start = match part.get("time") {
                                Some(time) => {
                                    let start = timestamp(&time["created"])?;
                                    if let Some(end) = time.get("completed") {
                                        if timestamp(end)? < start {
                                            bail!("opencode_invalid_timestamp");
                                        }
                                    }
                                    start
                                }
                                None => created,
                            };
                            events.push(SessionEvent {
                                kind: if reasoning {
                                    SessionEventKind::Reasoning
                                } else {
                                    SessionEventKind::Assistant
                                },
                                timestamp: Some(start),
                                content: Some(field(part, "text")?.to_owned()),
                                ..Default::default()
                            });
                        }
                        "tool" => {
                            parse_tool(part, message_id, &mut seen_calls, &mut events, &mut budget)?
                        }
                        _ => bail!("opencode_unknown_part"),
                    }
                }
            }
            // These native lifecycle and harness records remain opaque, just as
            // v1's snapshots, agent records and compaction parts do.
            "agent-switched" | "model-switched" | "location-switched" | "synthetic" | "system"
            | "skill" | "shell" | "compaction" | "idle" => {
                events.push(opaque(kind, created));
            }
            _ => bail!("opencode_unknown_role"),
        }
    }
    Ok(SessionTranscript {
        source_session: super::super::native_session_identity(SOURCE_OPENCODE, Some(session_id)),
        source: Cow::Borrowed(SOURCE_OPENCODE),
        agent_version: None,
        model: if models.len() == 1 {
            models.into_iter().next()
        } else {
            None
        },
        project: Path::new(cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned),
        cwd: Some(cwd.to_owned()),
        started_at: Some(started_at),
        conversation_id: Some(session_id.to_owned()),
        session_hash: session_hash(bytes),
        events,
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
        attested_call: None,
        attested_refusal: None,
    })
}

fn parse_tool<'a>(
    part: &'a Value,
    message_id: &str,
    seen_calls: &mut HashSet<&'a str>,
    events: &mut Vec<SessionEvent>,
    budget: &mut Budget,
) -> Result<()> {
    let call = id(part, "id")?;
    if !seen_calls.insert(call) {
        bail!("opencode_duplicate_tool_call");
    }
    let name = field(part, "name")?;
    let created = timestamp(&part["time"]["created"])?;
    let start = part["time"].get("ran").map(timestamp).transpose()?;
    let end = part["time"].get("completed").map(timestamp).transpose()?;
    if start.is_some_and(|s| s < created) || end.is_some_and(|e| e < start.unwrap_or(created)) {
        bail!("opencode_invalid_tool_time");
    }
    let state = &part["state"];
    let status = field(state, "status")?;
    if status == "streaming" {
        field(state, "input")?;
        events.push(opaque("streaming-tool", created));
        return Ok(());
    }
    if !matches!(status, "running" | "completed" | "error") {
        bail!("opencode_unknown_tool_state");
    }
    let input = state["input"]
        .as_object()
        .ok_or_else(|| anyhow!("opencode_invalid_tool_input"))?;
    events.push(SessionEvent {
        kind: SessionEventKind::ToolCall,
        timestamp: start.or(Some(created)),
        tool_name: Some(name.to_owned()),
        tool_call_id: Some(call.to_owned()),
        structured: Value::Object(input.clone()),
        ..Default::default()
    });
    if status == "running" {
        return Ok(());
    }
    let end = end.ok_or_else(|| anyhow!("opencode_invalid_tool_time"))?;
    let mut texts = Vec::new();
    let mut files = 0;
    if let Some(content) = state.get("content") {
        let content = content
            .as_array()
            .ok_or_else(|| anyhow!("opencode_invalid_tool_content"))?;
        if content.is_empty() {
            bail!("opencode_invalid_tool_content");
        }
        budget.take(content.len())?;
        for item in content {
            match field(item, "type")? {
                "text" => texts.push(field(item, "text")?.to_owned()),
                "file" => {
                    field(item, "uri")?;
                    field(item, "mime")?;
                    files += 1;
                }
                _ => bail!("opencode_unknown_part"),
            }
        }
    } else if status == "completed" {
        bail!("opencode_invalid_tool_content");
    }
    if status == "error" {
        field(&state["error"], "type")?;
        let message = field(&state["error"], "message")?;
        if texts.is_empty() {
            texts.push(message.to_owned());
        }
    }
    events.push(SessionEvent {
        structured: json!({"message_id":message_id}),
        ..SessionEvent::tool_result(
            Some(end),
            (!texts.is_empty()).then(|| texts.join("\n")),
            Some(call.to_owned()),
            Some(status == "completed"),
        )
    });
    for _ in 0..files {
        events.push(opaque("file", end));
    }
    Ok(())
}
