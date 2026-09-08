//! Explicit OpenCode JSON export reader, qualified against 1.18.29.
//!
//! No native database discovery, credential access, network requests or receipt
//! inference. The export is a local transcript, never verbatim provider evidence.
//! A declared directory contains `.json` exports directly (not recursively).

use super::{
    SOURCE_OPENCODE, SessionEvent, SessionEventKind, SessionRef, SessionTooLarge,
    SessionTranscript, TraceSource, real_file_within_root, session_hash,
};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::HashSet,
    io::Read,
    path::{Path, PathBuf},
};

pub const QUALIFIED_VERSION: &str = "1.18.29";
const BYTE_BUDGET: u64 = 16 * 1024 * 1024;
const RECORD_BUDGET: usize = 100_000;
const DISCOVERY_ENTRY_BUDGET: usize = 256;
const HEADER_BYTE_BUDGET: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("opencode-export-version-unsupported")]
pub(crate) struct UnsupportedExportVersion;

#[derive(serde::Deserialize)]
struct ExportHeader {
    directory: String,
    time: ExportTime,
}
#[derive(serde::Deserialize)]
struct ExportTime {
    created: i64,
}

// Read only the bounded leading info object. Upstream writes it before messages;
// a missing/malformed/late header leaves metadata absent, never invents an mtime.
fn export_header(path: &Path) -> Option<ExportHeader> {
    use serde::de::{Deserializer, MapAccess, Visitor};
    struct HeaderVisitor<'a>(&'a mut Option<ExportHeader>);
    impl<'de> Visitor<'de> for HeaderVisitor<'_> {
        type Value = ();
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("export header")
        }
        fn visit_map<M: MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == "info" {
                    *self.0 = Some(map.next_value()?);
                    // Stop before the (potentially multi-megabyte) message
                    // array. The full document is validated only by load.
                    return Err(serde::de::Error::custom("header-complete"));
                }
                map.next_value::<serde::de::IgnoredAny>()?;
            }
            Err(serde::de::Error::custom("missing export header"))
        }
    }
    let file = open_export(path)?;
    let mut de = serde_json::Deserializer::from_reader(std::io::BufReader::new(
        file.take(HEADER_BYTE_BUDGET),
    ));
    let mut header = None;
    let _ = (&mut de).deserialize_map(HeaderVisitor(&mut header));
    header
}

pub struct OpenCodeSource {
    root: PathBuf,
}
impl OpenCodeSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    fn address(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        (path.parent() == Some(self.root.as_path()) && path.extension()?.to_str()? == "json")
            .then_some(path)
    }
    fn reference(&self, path: &Path) -> Option<SessionRef> {
        let path = self.address(path)?;
        let size_bytes = std::fs::metadata(&path).ok()?.len();
        let header = export_header(&path);
        let started_at = header
            .as_ref()
            .and_then(|h| (h.time.created >= 0).then_some(h.time.created))
            .and_then(DateTime::from_timestamp_millis);
        let cwd = header.map(|h| h.directory);
        let project = cwd
            .as_deref()
            .and_then(|c| Path::new(c).file_name())
            .and_then(|n| n.to_str())
            .map(str::to_owned);
        Some(SessionRef {
            source: SOURCE_OPENCODE,
            declared_source: None,
            path,
            project,
            cwd,
            started_at,
            size_bytes,
            group_modified_at: None,
            group_member_count: 0,
        })
    }
}
impl TraceSource for OpenCodeSource {
    fn name(&self) -> &'static str {
        SOURCE_OPENCODE
    }
    fn discover(&self) -> Result<Vec<SessionRef>> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(Vec::new());
        };
        let mut refs = Vec::new();
        for (index, entry) in entries.enumerate() {
            if index >= DISCOVERY_ENTRY_BUDGET {
                bail!("opencode-discovery-entry-budget");
            }
            if let Some(reference) = entry.ok().and_then(|e| self.reference(&e.path())) {
                refs.push(reference);
            }
        }
        refs.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(refs)
    }
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        self.address(path)
    }
    fn session_at(&self, path: &Path) -> Result<Option<SessionRef>> {
        Ok(self.reference(path))
    }
    fn load(&self, r: &SessionRef) -> Result<SessionTranscript> {
        if r.source != SOURCE_OPENCODE {
            bail!("opencode_source_mismatch");
        }
        let path = self
            .address(&r.path)
            .ok_or_else(|| anyhow!("opencode_outside_declared_root"))?;
        let file = open_export(&path).ok_or_else(|| anyhow!("opencode_unreadable_export"))?;
        let mut bytes = Vec::new();
        file.take(BYTE_BUDGET + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("opencode_unreadable_export"))?;
        parse_export(&bytes)
    }
}
// Final-component races are refused on shipped platforms. This does not
// claim to defend against an adversary replacing ancestor directories.
fn open_export(path: &Path) -> Option<std::fs::File> {
    let checked = std::fs::symlink_metadata(path).ok()?;
    if !checked.is_file() {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Shipped Darwin / Linux x86_64 / Linux ARM64 UAPI, respectively.
        let flags = if cfg!(target_os = "macos") {
            0x4 | 0x100
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            0x800 | 0x20000
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            0x800 | 0x8000
        } else {
            return None;
        };
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    #[cfg(not(any(unix, windows)))]
    {
        return None;
    }
    let file = options.open(path).ok()?;
    let opened = file.metadata().ok()?;
    if !opened.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != checked.dev() || opened.ino() != checked.ino() {
            return None;
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if opened.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return None;
        }
    }
    Some(file)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("opencode_invalid_field"))
}
fn id<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    let s = field(v, key)?;
    if s.is_empty()
        || s.len() > 256
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        bail!("opencode_invalid_id");
    }
    Ok(s)
}
fn timestamp(v: &Value) -> Result<DateTime<Utc>> {
    v.as_i64()
        .filter(|n| *n >= 0)
        .and_then(DateTime::from_timestamp_millis)
        .ok_or_else(|| anyhow!("opencode_invalid_timestamp"))
}

/// Convert qualified export bytes without assigning any attestation or routing.
/// Unknown versions/roles/parts and ambiguous IDs fail closed with content-free labels.
pub fn parse_export(bytes: &[u8]) -> Result<SessionTranscript> {
    parse_export_with_record_budget(bytes, RECORD_BUDGET)
}
fn parse_export_with_record_budget(
    bytes: &[u8],
    record_budget: usize,
) -> Result<SessionTranscript> {
    if bytes.len() as u64 > BYTE_BUDGET {
        return Err(SessionTooLarge {
            label: "opencode-export-too-large",
            declared_bytes: bytes.len() as u64,
            budget_bytes: BYTE_BUDGET,
        }
        .into());
    }
    let doc: Value =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("opencode_malformed_json"))?;
    let info = &doc["info"];
    let session_id = id(info, "id")?;
    if field(info, "version")? != QUALIFIED_VERSION {
        return Err(UnsupportedExportVersion.into());
    }
    let cwd = field(info, "directory")?.to_owned();
    let started_at = timestamp(&info["time"]["created"])?;
    let messages = doc["messages"]
        .as_array()
        .ok_or_else(|| anyhow!("opencode_missing_messages"))?;
    if messages.len() > record_budget {
        bail!("opencode_record_budget");
    }
    let mut seen_messages = HashSet::new();
    let mut seen_parts = HashSet::new();
    let mut seen_calls = HashSet::new();
    let mut events = Vec::new();
    let mut models = HashSet::new();
    let mut previous = None;
    for message in messages {
        let mi = &message["info"];
        let message_id = id(mi, "id")?;
        if id(mi, "sessionID")? != session_id || !seen_messages.insert(message_id) {
            bail!("opencode_ambiguous_message_id");
        }
        let role = field(mi, "role")?;
        if !matches!(role, "user" | "assistant") {
            bail!("opencode_unknown_role");
        }
        let created = timestamp(&mi["time"]["created"])?;
        if previous.is_some_and(|p| p > created) {
            bail!("opencode_reordered_messages");
        }
        previous = Some(created);
        if role == "assistant" {
            let parent = id(mi, "parentID")?;
            if parent == message_id || !seen_messages.contains(parent) {
                bail!("opencode_missing_parent");
            }
            models.insert(format!(
                "{}/{}",
                field(mi, "providerID")?,
                field(mi, "modelID")?
            ));
        }
        let parts = message["parts"]
            .as_array()
            .ok_or_else(|| anyhow!("opencode_missing_parts"))?;
        for part in parts {
            let part_id = id(part, "id")?;
            if id(part, "sessionID")? != session_id
                || id(part, "messageID")? != message_id
                || !seen_parts.insert(part_id)
            {
                bail!("opencode_ambiguous_part_id");
            }
            if seen_parts.len() > record_budget {
                bail!("opencode_record_budget");
            }
            let kind = field(part, "type")?;
            let mut marker = json!({"message_id":message_id,"part_id":part_id});
            if role == "assistant" {
                marker["provider_id"] = field(mi, "providerID")?.into();
                marker["model_id"] = field(mi, "modelID")?.into();
            }
            for flag in ["synthetic", "ignored"] {
                if let Some(value) = part.get(flag) {
                    if !value.is_boolean() {
                        bail!("opencode_invalid_part_flag");
                    }
                    marker[flag] = value.clone();
                }
            }
            match kind {
                "text" | "reasoning" => {
                    if kind == "reasoning" && role != "assistant" {
                        bail!("opencode_invalid_role_part");
                    }
                    let text = field(part, "text")?;
                    if part.get("synthetic").and_then(Value::as_bool) == Some(true)
                        || part.get("ignored").and_then(Value::as_bool) == Some(true)
                    {
                        events.push(SessionEvent {
                            kind: SessionEventKind::Opaque,
                            timestamp: Some(created),
                            ..Default::default()
                        });
                        continue;
                    }
                    let time = match part.get("time") {
                        Some(t) => Some(timestamp(&t["start"])?),
                        None => Some(created),
                    };
                    events.push(SessionEvent {
                        kind: if kind == "reasoning" {
                            SessionEventKind::Reasoning
                        } else if role == "user" {
                            SessionEventKind::User
                        } else {
                            SessionEventKind::Assistant
                        },
                        timestamp: time,
                        content: Some(text.to_owned()),
                        // Identity/flag strings are not tool content. Keep them
                        // in the validated export, not the submitted payload.
                        structured: Value::Null,
                        ..Default::default()
                    });
                }
                "tool" => {
                    if role != "assistant" {
                        bail!("opencode_invalid_role_part");
                    }
                    let call = id(part, "callID")?;
                    if !seen_calls.insert(call) {
                        bail!("opencode_duplicate_tool_call");
                    }
                    let state = &part["state"];
                    let status = field(state, "status")?;
                    if !matches!(status, "pending" | "running" | "completed" | "error") {
                        bail!("opencode_unknown_tool_state");
                    }
                    let input = state["input"]
                        .as_object()
                        .ok_or_else(|| anyhow!("opencode_invalid_tool_input"))?;
                    let start = if status == "pending" {
                        None
                    } else {
                        Some(timestamp(&state["time"]["start"])?)
                    };
                    events.push(SessionEvent {
                        kind: SessionEventKind::ToolCall,
                        timestamp: start,
                        tool_name: Some(field(part, "tool")?.to_owned()),
                        tool_call_id: Some(call.to_owned()),
                        // raw_event_for wraps this as `arguments`; do not
                        // mix import identity markers into executable inputs.
                        structured: Value::Object(input.clone()),
                        ..Default::default()
                    });
                    if matches!(status, "completed" | "error") {
                        let end = timestamp(&state["time"]["end"])?;
                        if start.is_some_and(|s| s > end) {
                            bail!("opencode_invalid_tool_time");
                        }
                        let content = field(
                            state,
                            if status == "completed" {
                                "output"
                            } else {
                                "error"
                            },
                        )?;
                        events.push(SessionEvent {
                            structured: marker,
                            ..SessionEvent::tool_result(
                                Some(end),
                                Some(content.to_owned()),
                                Some(call.to_owned()),
                                Some(status == "completed"),
                            )
                        });
                    }
                }
                // No file URLs, snapshot content, embedded attachments or harness
                // metadata are opened or smuggled into the ordinary envelope.
                "file" | "snapshot" | "patch" | "step-start" | "step-finish" | "agent"
                | "retry" | "compaction" | "subtask" => {
                    let mut event = SessionEvent::opaque(kind, Some(created));
                    // No contributed body: do not manufacture a tool-payload
                    // consent requirement from an export-only type/ID marker.
                    event.structured = Value::Null;
                    events.push(event);
                }
                _ => bail!("opencode_unknown_part"),
            }
        }
    }
    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_OPENCODE),
        agent_version: Some(QUALIFIED_VERSION.into()),
        model: if models.len() == 1 {
            models.into_iter().next()
        } else {
            None
        },
        project: Path::new(&cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned),
        cwd: Some(cwd),
        started_at: Some(started_at),
        conversation_id: Some(session_id.to_owned()),
        session_hash: session_hash(bytes),
        events,
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
        attested_call: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        daemon::settings::SourceDeclaration,
        source::{SourceRoots, all_sources},
    };
    const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/opencode/completed.json");
    fn mutated(change: impl FnOnce(&mut Value)) -> Result<SessionTranscript> {
        let mut value: Value = serde_json::from_slice(FIXTURE).unwrap();
        change(&mut value);
        parse_export(&serde_json::to_vec(&value).unwrap())
    }
    #[test]
    fn export_preserves_observed_identity_roles_and_tool_outcome_without_evidence() {
        let t = parse_export(FIXTURE).unwrap();
        assert_eq!(t.conversation_id.as_deref(), Some("ses_synthetic"));
        assert_eq!(t.model.as_deref(), Some("fixture/model"));
        assert_eq!(t.session_hash, session_hash(FIXTURE));
        assert_eq!(
            t.events.iter().map(|e| &e.kind).collect::<Vec<_>>(),
            vec![
                &SessionEventKind::User,
                &SessionEventKind::Reasoning,
                &SessionEventKind::ToolCall,
                &SessionEventKind::ToolResult,
                &SessionEventKind::Assistant
            ]
        );
        assert_eq!(t.events[2].tool_call_id, t.events[3].tool_call_id);
        assert_eq!(t.events[3].success, Some(true));
        assert!(t.events[4].structured.is_null());
        assert!(t.routing.is_empty() && t.attested_call.is_none());
        assert!(
            t.events
                .iter()
                .all(|e| e.served_by.is_none() && e.token_counts.is_none())
        );
    }
    #[test]
    fn malformed_truncated_unknown_version_and_wrong_types_refuse_without_content() {
        assert_eq!(
            parse_export(&FIXTURE[..30]).unwrap_err().to_string(),
            "opencode_malformed_json"
        );
        assert!(mutated(|v| v["info"]["version"] = "99.0".into()).is_err());
        assert!(mutated(|v| v["info"]["directory"] = 42.into()).is_err());
        assert!(mutated(|v| v["messages"][0]["parts"][0]["text"] = false.into()).is_err());
        assert!(
            mutated(|v| v["messages"][0]["info"]["role"] = "secret_role".into())
                .unwrap_err()
                .to_string()
                .contains("unknown_role")
        );
        assert!(mutated(|v| v["messages"][0]["parts"][0]["type"] = "future".into()).is_err());
    }
    #[test]
    fn duplicate_reordered_and_cross_session_records_refuse() {
        assert!(
            mutated(|v| {
                let m = v["messages"][0].clone();
                v["messages"].as_array_mut().unwrap().push(m);
            })
            .is_err()
        );
        assert!(mutated(|v| v["messages"].as_array_mut().unwrap().reverse()).is_err());
        assert!(
            mutated(|v| v["messages"][0]["parts"][0]["sessionID"] = "ses_other".into()).is_err()
        );
        assert!(mutated(|v| v["messages"][1]["parts"][2]["id"] = "prt_reason".into()).is_err());
        assert!(mutated(|v| v["messages"][1]["info"]["parentID"] = "msg_missing".into()).is_err());
    }
    #[test]
    fn running_and_failed_tools_never_invent_success() {
        let running =
            mutated(|v| v["messages"][1]["parts"][1]["state"]["status"] = "running".into())
                .unwrap();
        assert!(
            !running
                .events
                .iter()
                .any(|e| e.kind == SessionEventKind::ToolResult)
        );
        let failed = mutated(|v| {
            let s = &mut v["messages"][1]["parts"][1]["state"];
            s["status"] = "error".into();
            s["error"] = "synthetic failure".into();
        })
        .unwrap();
        assert_eq!(failed.events[3].success, Some(false));
        assert_eq!(
            failed.events[3].content.as_deref(),
            Some("synthetic failure")
        );
    }
    #[test]
    fn undeclared_and_conventional_are_off_explicit_root_loads_export() {
        for roots in [SourceRoots::new(), SourceRoots::conventional()] {
            assert!(
                !all_sources(&roots)
                    .iter()
                    .any(|s| s.name() == SOURCE_OPENCODE)
            );
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("session.json");
        std::fs::write(&path, FIXTURE).unwrap();
        let sources = all_sources(&SourceRoots::new().declare(
            SOURCE_OPENCODE,
            Some(SourceDeclaration::Watch {
                path: tmp.path().to_owned(),
            }),
        ));
        let source = sources
            .iter()
            .find(|s| s.name() == SOURCE_OPENCODE)
            .unwrap();
        let refs = source.discover().unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(
            source.session_at(&path).unwrap().unwrap().size_bytes,
            refs[0].size_bytes
        );
        assert_eq!(
            source.load(&refs[0]).unwrap().conversation_id.as_deref(),
            Some("ses_synthetic")
        );
        assert!(
            source
                .session_for_path(&tmp.path().join("../outside.json"))
                .is_none()
        );
    }
    #[test]
    fn unsupported_payloads_stay_opaque_and_flags_and_models_are_observed() {
        let t = mutated(|v| {
            v["messages"][0]["parts"][0]["synthetic"] = true.into();
            v["messages"][1]["parts"][2]["type"] = "file".into();
            v["messages"][1]["parts"][2]["url"] = "file:///synthetic/private".into();
        })
        .unwrap();
        assert!(t.events[0].structured.is_null());
        assert_eq!(t.events[2].structured["command"], "echo synthetic");
        let last = t.events.last().unwrap();
        assert_eq!(last.kind, SessionEventKind::Opaque);
        assert!(last.content.is_none());
        assert!(!last.structured.to_string().contains("private"));
        assert!(mutated(|v| v["messages"][0]["parts"][0]["ignored"] = 1.into()).is_err());
    }

    #[test]
    fn explicit_off_and_foreign_reference_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let roots =
            SourceRoots::conventional().declare(SOURCE_OPENCODE, Some(SourceDeclaration::Off));
        assert!(
            !all_sources(&roots)
                .iter()
                .any(|s| s.name() == SOURCE_OPENCODE)
        );
        let source = OpenCodeSource::new(tmp.path().join("declared"));
        let other = OpenCodeSource::new(tmp.path().to_owned());
        let path = tmp.path().join("foreign.json");
        std::fs::write(&path, FIXTURE).unwrap();
        let r = other.reference(&path).unwrap();
        assert_eq!(
            source.load(&r).unwrap_err().to_string(),
            "opencode_outside_declared_root"
        );
    }

    #[test]
    fn discovery_metadata_uses_session_time_and_directory_not_export_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("export.json");
        std::fs::write(&path, FIXTURE).unwrap();
        let source = OpenCodeSource::new(tmp.path().to_owned());
        let r = source.discover().unwrap().remove(0);
        let t = source.load(&r).unwrap();
        assert_eq!(r.started_at, t.started_at);
        assert!(
            r.started_at
                .is_some_and(|t| t >= DateTime::from_timestamp_millis(1788739199999).unwrap())
        );
        assert_eq!(r.cwd.as_deref(), Some("/synthetic/project"));
        assert_eq!(r.project.as_deref(), Some("project"));
    }

    #[test]
    fn record_and_discovery_entry_limits_refuse_instead_of_truncating() {
        assert!(parse_export_with_record_budget(FIXTURE, 4).is_ok());
        assert_eq!(
            parse_export_with_record_budget(FIXTURE, 3)
                .unwrap_err()
                .to_string(),
            "opencode_record_budget"
        );
        let mut doc: Value = serde_json::from_slice(FIXTURE).unwrap();
        doc["messages"] = serde_json::json!([null, null, null, null, null]);
        assert_eq!(
            parse_export_with_record_budget(&serde_json::to_vec(&doc).unwrap(), 4)
                .unwrap_err()
                .to_string(),
            "opencode_record_budget"
        );
        doc["messages"] = Value::Array(vec![Value::Null; RECORD_BUDGET + 1]);
        assert_eq!(
            parse_export(&serde_json::to_vec(&doc).unwrap())
                .unwrap_err()
                .to_string(),
            "opencode_record_budget"
        );
        let tmp = tempfile::tempdir().unwrap();
        for i in 0..DISCOVERY_ENTRY_BUDGET {
            std::fs::write(tmp.path().join(format!("{i}.txt")), b"").unwrap();
        }
        let source = OpenCodeSource::new(tmp.path().to_owned());
        assert!(source.discover().unwrap().is_empty());
        std::fs::write(tmp.path().join("over-budget.txt"), b"").unwrap();
        assert_eq!(
            source.discover().unwrap_err().to_string(),
            "opencode-discovery-entry-budget"
        );
    }

    #[test]
    fn oversize_export_has_typed_stable_refusal() {
        let e = parse_export(&vec![b' '; BYTE_BUDGET as usize + 1]).unwrap_err();
        assert!(e.downcast_ref::<SessionTooLarge>().is_some());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_export_is_not_discovered_or_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target");
        std::fs::write(&target, FIXTURE).unwrap();
        let link = tmp.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_export(&link).is_none());
        assert!(
            OpenCodeSource::new(tmp.path().to_owned())
                .discover()
                .unwrap()
                .is_empty()
        );
    }
}
