//! Gemini CLI session adapter.
//!
//! Reads `<root>/<project>/chats/session-*.json`, where `<root>` is
//! `$GEMINI_CLI_HOME/tmp` (or `~/.gemini/tmp`). One file is one session: a
//! single JSON document, not JSONL, holding `sessionId`, `startTime`,
//! `lastUpdated` and a `messages` array.
//!
//! **Message-type dispatch is tolerant, and only that.** An unrecognised
//! `type` becomes an `Opaque` event carrying a type marker, the way
//! `claude_code` handles the same situation, rather than rejecting the file
//! the way `trajectory` does. Trajectory is a versioned schema with a
//! published conformance corpus, so an unknown record means the file is not
//! what it claims; Gemini's session format is unversioned and evolving, and
//! a strict parser would drop every session the day upstream adds a message
//! type. Everything a gate depends on -- path containment, the byte budget,
//! and the requirement that the document actually be a session document --
//! stays fail-closed.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::{
    SOURCE_GEMINI_CLI, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    real_file_within_root, session_hash,
};

/// The environment variable Gemini CLI uses to relocate its home directory,
/// and therefore its `tmp/<project>/chats/` session store.
pub const GEMINI_CLI_HOME_ENV: &str = "GEMINI_CLI_HOME";

/// The directory under the Gemini home that holds one directory per project.
pub const GEMINI_SESSION_SUBDIR: &str = "tmp";

/// The per-project subdirectory holding session documents.
const CHATS_DIR: &str = "chats";

/// The sibling file naming the project's true working directory. Older
/// hash-named project directories do not have one.
const PROJECT_ROOT_FILE: &str = ".project_root";

/// The largest session document this adapter will load, shared with every
/// other adapter's budget rather than picked separately: they bound the same
/// thing, which is how much of one conversation may become resident on its
/// way to being discarded.
pub(crate) const GEMINI_SESSION_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

/// The conventional store on this machine: `$GEMINI_CLI_HOME/tmp`, or
/// `~/.gemini/tmp` when the variable is unset or empty.
pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    env(GEMINI_CLI_HOME_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".gemini"))
        .join(GEMINI_SESSION_SUBDIR)
}

/// The conventional store, resolved against this machine's real home and
/// environment.
pub fn conventional_root_this_machine() -> PathBuf {
    conventional_root(&dirs::home_dir().unwrap_or_default(), |key| {
        std::env::var(key).ok()
    })
}

pub struct GeminiCliSource {
    root: PathBuf,
}

impl GeminiCliSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

/// The session naming rule, in one place: `session-<...>.json`.
fn is_session_file_name(file_name: &str) -> bool {
    file_name.starts_with("session-") && file_name.ends_with(".json")
}

/// The project's declared working directory, from the sibling
/// `.project_root`. Absent on older hash-named directories, and an absent
/// one is left absent rather than guessed: `cwd` feeds the redactor's
/// path-prefix stripping, and a wrong prefix strips nothing while looking
/// as though it had.
fn project_root_cwd(project_dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(project_dir.join(PROJECT_ROOT_FILE)).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The label a picker renders: the basename of the true working directory
/// when there is one, and otherwise the project directory's own name, which
/// is at least stable and distinguishes one session from another.
fn project_label(project_dir: &Path, cwd: Option<&str>) -> Option<String> {
    cwd.map(Path::new)
        .or(Some(project_dir))
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// The one way a Gemini `SessionRef` is built, shared by `discover` and
/// `session_at` so a scoped scan and a full sweep cannot disagree about a
/// session's size or cwd and therefore reach different eligibility
/// decisions for the same bytes.
///
/// `None` for a file that is no longer there, which on the event path is an
/// ordinary race rather than a failure.
fn session_ref_for(path: PathBuf, cwd: Option<String>) -> Option<SessionRef> {
    let project_dir = path.parent()?.parent()?.to_path_buf();
    let metadata = std::fs::metadata(&path).ok()?;
    let started_at = metadata
        .modified()
        .ok()
        .map(chrono::DateTime::<chrono::Utc>::from);
    let project = project_label(&project_dir, cwd.as_deref());
    Some(SessionRef {
        source: SOURCE_GEMINI_CLI,
        path,
        project,
        cwd,
        started_at,
        size_bytes: metadata.len(),
        // One document is one session. Subagent sessions are written as
        // separate files carrying no back-reference to a parent, so unlike
        // Claude Code there is no group to describe.
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for GeminiCliSource {
    fn name(&self) -> &'static str {
        SOURCE_GEMINI_CLI
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        let Ok(projects) = std::fs::read_dir(&self.root) else {
            return Ok(sessions);
        };
        for project in projects {
            let Ok(project) = project else {
                skipped += 1;
                continue;
            };
            // `file_type` does not follow, so a symlinked project directory
            // is not descended into: a link planted under the store by any
            // same-user process must not steer collection elsewhere.
            match project.file_type() {
                Ok(ft) if ft.is_dir() => {}
                Ok(_) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
            let project_dir = project.path();
            let cwd = project_root_cwd(&project_dir);
            let Ok(entries) = std::fs::read_dir(project_dir.join(CHATS_DIR)) else {
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    skipped += 1;
                    continue;
                };
                match entry.file_type() {
                    Ok(ft) if ft.is_file() => {}
                    Ok(_) => continue,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                }
                let file_name = entry.file_name();
                let Some(file_name) = file_name.to_str() else {
                    continue;
                };
                if !is_session_file_name(file_name) {
                    continue;
                }
                match session_ref_for(entry.path(), cwd.clone()) {
                    Some(r) => sessions.push(r),
                    None => skipped += 1,
                }
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable gemini session entries during discovery"
            );
        }
        Ok(sessions)
    }

    /// A changed session document is its own session, on exactly the terms
    /// `discover` uses: `<root>/<project>/chats/session-*.json`, three
    /// components deep and no deeper. Discovery does not recurse, so
    /// neither does this -- a mapping laxer than discovery would be a way
    /// to name a file the contributor never agreed to watch.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        if !is_session_file_name(path.file_name()?.to_str()?) {
            return None;
        }
        let chats = path.parent()?;
        if chats.file_name()? != CHATS_DIR {
            return None;
        }
        (chats.parent()?.parent() == Some(self.root.as_path())).then_some(path)
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        let cwd = address
            .parent()
            .and_then(|chats| chats.parent())
            .and_then(project_root_cwd);
        Ok(session_ref_for(address, cwd))
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path, r.cwd.clone())
    }
}

/// One text field that may be a bare string or an array of `{text}` parts.
///
/// `displayContent` is deliberately never consulted. Real sessions show
/// `content` carrying the relativised path (`@../../.gemini/...`) while
/// `displayContent` carries the absolute one under the contributor's home
/// directory, so reading the prettier field would put a real user name into
/// the corpus.
fn text_of(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(parts)) => Some(
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn timestamp_of(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn opaque(record_type: &str, timestamp: Option<chrono::DateTime<chrono::Utc>>) -> SessionEvent {
    SessionEvent {
        kind: SessionEventKind::Opaque,
        timestamp,
        content: None,
        structured: json!({ "record_type": record_type }),
        tool_name: None,
        token_counts: None,
        tool_call_id: None,
        success: None,
    }
}

/// A `gemini` turn expands to its thoughts, then its answer, then each tool
/// call paired with its result -- the order the turn happened in.
fn map_gemini_message(message: &Value, model: &mut Option<String>, events: &mut Vec<SessionEvent>) {
    let turn_timestamp = timestamp_of(message.get("timestamp"));
    if model.is_none() {
        if let Some(m) = message.get("model").and_then(|v| v.as_str()) {
            *model = Some(m.to_string());
        }
    }

    if let Some(thoughts) = message.get("thoughts").and_then(|v| v.as_array()) {
        for thought in thoughts {
            // Subject then description: the subject is a one-line heading
            // the CLI renders above the body, and dropping either would
            // lose reasoning the contributor is consenting to share.
            let parts: Vec<&str> = ["subject", "description"]
                .iter()
                .filter_map(|key| thought.get(*key).and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty())
                .collect();
            if parts.is_empty() {
                continue;
            }
            events.push(SessionEvent {
                kind: SessionEventKind::Reasoning,
                timestamp: timestamp_of(thought.get("timestamp")).or(turn_timestamp),
                content: Some(parts.join("\n")),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            });
        }
    }

    let tokens = message.get("tokens");
    let token_counts = match (
        tokens.and_then(|t| t.get("input")).and_then(|v| v.as_u64()),
        tokens
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64()),
    ) {
        (Some(input), Some(output)) => Some((input as u32, output as u32)),
        _ => None,
    };

    if let Some(content) = text_of(message.get("content")).filter(|c| !c.is_empty()) {
        events.push(SessionEvent {
            kind: SessionEventKind::Assistant,
            timestamp: turn_timestamp,
            content: Some(content),
            structured: Value::Null,
            tool_name: None,
            token_counts,
            tool_call_id: None,
            success: None,
        });
    }

    let Some(calls) = message.get("toolCalls").and_then(|v| v.as_array()) else {
        return;
    };
    for call in calls {
        let timestamp = timestamp_of(call.get("timestamp")).or(turn_timestamp);
        let tool_call_id = call
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        events.push(SessionEvent {
            kind: SessionEventKind::ToolCall,
            timestamp,
            content: None,
            structured: call.get("args").cloned().unwrap_or(Value::Null),
            tool_name: call
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            token_counts: None,
            tool_call_id: tool_call_id.clone(),
            success: None,
        });
        // `resultDisplay` is the rendered-for-a-terminal form of the same
        // answer; `result` is the answer. Only one of them is carried.
        let content = match call.get("result") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => serde_json::to_string(other).ok(),
            None => None,
        };
        // Only an explicit verdict counts, and only "success" is one:
        // "cancelled" is not a failure of the tool, but it is certainly not
        // a success, and both are recorded as "not success" rather than
        // guessed apart.
        let success = call
            .get("status")
            .and_then(|v| v.as_str())
            .map(|status| status == "success");
        events.push(SessionEvent {
            kind: SessionEventKind::ToolResult,
            timestamp,
            content,
            structured: Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id,
            success,
        });
    }
}

fn load_session(path: &Path, cwd: Option<String>) -> anyhow::Result<SessionTranscript> {
    // Declined rather than truncated, and named rather than silent: a
    // half-parsed transcript would upload as though it were the whole
    // conversation. The size is the contributor's own file's and safe to
    // state; the path is not, and is deliberately absent.
    let declared = std::fs::metadata(path)?.len();
    if declared > GEMINI_SESSION_BUDGET {
        return Err(super::SessionTooLarge {
            label: "gemini-session-too-large",
            declared_bytes: declared,
            budget_bytes: GEMINI_SESSION_BUDGET,
        }
        .into());
    }
    let bytes = std::fs::read(path)?;
    let hash = session_hash(&bytes);
    // One JSON document, so there is no streaming form to read it in: the
    // budget above is what bounds this read.
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("malformed_gemini_session"))?;
    // The tolerance is for message *types*, not for the document. A file
    // with no `messages` array is not a session document at all, and
    // accepting it would offer an empty transcript as though it were a
    // conversation.
    let messages = document
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("malformed_gemini_session"))?;

    let mut events = Vec::new();
    let mut model: Option<String> = None;
    for message in messages {
        let message_type = message.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match message_type {
            "user" => events.push(SessionEvent {
                kind: SessionEventKind::User,
                timestamp: timestamp_of(message.get("timestamp")),
                content: text_of(message.get("content")),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            }),
            "gemini" => map_gemini_message(message, &mut model, &mut events),
            other => events.push(opaque(other, timestamp_of(message.get("timestamp")))),
        }
    }

    let started_at = timestamp_of(document.get("startTime"));
    let project_dir = path.parent().and_then(|chats| chats.parent());
    let project = project_dir.and_then(|dir| project_label(dir, cwd.as_deref()));

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_GEMINI_CLI),
        // Gemini's session document carries no CLI version field.
        agent_version: None,
        model,
        project,
        cwd,
        started_at,
        session_hash: hash,
        // The session's own id, which is what the store addresses it by --
        // not the file name, which merely repeats it.
        conversation_id: document
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        events,
        // A subagent session is written as its own file with no
        // back-reference to a parent, so nothing is ever merged in.
        subagent_count: 0,
        subagents_dropped: 0,
    })
}

#[cfg(test)]
mod tests {
    use crate::source::gemini_cli::{GEMINI_SESSION_BUDGET, GeminiCliSource};
    use crate::source::{SOURCE_GEMINI_CLI, SessionEventKind, SessionTooLarge, TraceSource};
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/gemini-cli")
    }

    fn source() -> GeminiCliSource {
        GeminiCliSource::new(fixture_root())
    }

    /// Discovery is `<root>/<project>/chats/session-*.json` and nothing
    /// else. A stray file beside a session is not a session.
    #[test]
    fn discovery_finds_session_documents_and_nothing_else() {
        let mut found = source().discover().unwrap();
        found.sort_by(|a, b| a.path.cmp(&b.path));
        let names: Vec<String> = found
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec![
                "session-33333333-3333-4333-8333-333333333333.json",
                "session-11111111-1111-4111-8111-111111111111.json",
                "session-22222222-2222-4222-8222-222222222222.json",
            ],
            "only session-*.json under a project's chats/ is a session"
        );
        assert!(found.iter().all(|r| r.source == SOURCE_GEMINI_CLI));
        assert!(
            found.iter().all(|r| r.group_member_count == 0),
            "one file is one session; there is no group"
        );
    }

    /// The sibling `.project_root` supplies the cwd. An older hash-named
    /// directory has none, and then `cwd` is absent rather than guessed --
    /// while `project` still falls back to the directory name so the picker
    /// has something to render.
    #[test]
    fn the_project_root_file_supplies_the_cwd_and_a_missing_one_leaves_it_absent() {
        let found = source().discover().unwrap();
        let alpha = found
            .iter()
            .find(|r| r.path.to_string_lossy().contains("proj-alpha"))
            .expect("the declared project");
        assert_eq!(alpha.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(alpha.project.as_deref(), Some("alpha"));

        let legacy = found
            .iter()
            .find(|r| r.path.to_string_lossy().contains("legacyhash"))
            .expect("the hash-named project");
        assert_eq!(legacy.cwd, None, "an absent .project_root is not guessed");
        assert_eq!(
            legacy.project.as_deref(),
            Some("legacyhash"),
            "the directory name is the fallback label"
        );
    }

    /// A session file is its own session, and nothing else in the tree is
    /// addressable -- including a file one directory too shallow or too
    /// deep, and anything outside the root.
    #[test]
    fn a_session_maps_to_itself_and_nothing_else_maps_at_all() {
        let root = tempfile::tempdir().unwrap();
        let chats = root.path().join("proj/chats");
        std::fs::create_dir_all(&chats).unwrap();
        let session = chats.join("session-abc.json");
        std::fs::write(&session, b"{}").unwrap();
        std::fs::write(chats.join("notes.txt"), b"x").unwrap();
        std::fs::write(chats.join("session-abc.jsonl"), b"{}").unwrap();
        std::fs::write(root.path().join("proj/session-abc.json"), b"{}").unwrap();
        let nested = chats.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("session-deep.json"), b"{}").unwrap();

        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("session-xyz.json");
        std::fs::write(&elsewhere, b"{}").unwrap();

        let source = GeminiCliSource::new(root.path().to_path_buf());
        assert_eq!(source.session_for_path(&session), Some(session.clone()));
        let discovered = source.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].path, session);

        for path in [
            chats.join("notes.txt"),
            chats.join("session-abc.jsonl"),
            root.path().join("proj/session-abc.json"),
            nested.join("session-deep.json"),
            nested,
            chats.clone(),
            root.path().to_path_buf(),
            chats.join("session-never-written.json"),
            elsewhere,
        ] {
            assert_eq!(
                source.session_for_path(&path),
                None,
                "{} must not address a session",
                path.display()
            );
        }
    }

    /// A symlink under the root must not steer collection at a file
    /// somewhere else on disk, on the addressing surface the operating
    /// system feeds paths into.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_session_is_not_addressable() {
        let root = tempfile::tempdir().unwrap();
        let chats = root.path().join("proj/chats");
        std::fs::create_dir_all(&chats).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let real = outside.path().join("session-real.json");
        std::fs::write(&real, b"{}").unwrap();
        let link = chats.join("session-link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let source = GeminiCliSource::new(root.path().to_path_buf());
        assert_eq!(source.session_for_path(&link), None);
        assert!(
            source.discover().unwrap().is_empty(),
            "a symlinked session file is not collected either"
        );
    }

    /// A scoped lookup and a full sweep must describe a session
    /// identically, or the two paths judge the same bytes differently.
    #[test]
    fn session_at_describes_a_session_exactly_as_discover_does() {
        let source = source();
        let discovered = source.discover().unwrap();
        for r in &discovered {
            let scoped = source.session_at(&r.path).unwrap().expect("a session");
            assert_eq!(format!("{scoped:?}"), format!("{r:?}"));
        }
        let gone = fixture_root().join("proj-alpha/chats/session-never-written.json");
        assert!(source.session_at(&gone).unwrap().is_none());
    }

    /// Every record kind in the format, mapped once.
    #[test]
    fn a_loaded_session_maps_every_record_kind() {
        let source = source();
        let r = source
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("session-1111"))
            .expect("the main fixture session");
        let t = source.load(&r).unwrap();

        let kinds: Vec<&SessionEventKind> = t.events.iter().map(|e| &e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &SessionEventKind::User,
                // Thoughts come before the answer they produced.
                &SessionEventKind::Reasoning,
                &SessionEventKind::Reasoning,
                &SessionEventKind::Assistant,
                &SessionEventKind::ToolCall,
                &SessionEventKind::ToolResult,
                &SessionEventKind::ToolCall,
                &SessionEventKind::ToolResult,
                &SessionEventKind::User,
                &SessionEventKind::Opaque,
                &SessionEventKind::Opaque,
                &SessionEventKind::Opaque,
            ],
        );

        // A thought is its subject then its description, with its own stamp.
        let thought = &t.events[1];
        assert_eq!(
            thought.content.as_deref(),
            Some("Locating the parser\nThe parser lives in the manifest module.")
        );
        assert_eq!(
            thought.timestamp.map(|ts| ts.to_rfc3339()),
            Some("2026-08-25T10:00:05+00:00".to_string())
        );

        // Both halves of a call carry the harness's own id, so a result can
        // be paired with the call it answers without trusting array order.
        assert_eq!(t.events[4].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(t.events[5].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(t.events[4].tool_name.as_deref(), Some("write_file"));
        assert_eq!(
            t.events[4].structured["path"],
            "src/manifest/parser_test.rs"
        );
        assert_eq!(t.events[5].success, Some(true));
        assert_eq!(
            t.events[7].success,
            Some(false),
            "status other than success is not a success"
        );

        // Opaque records carry a type marker and nothing else.
        for (index, marker) in [(9, "info"), (10, "error"), (11, "compaction")] {
            let event = &t.events[index];
            assert_eq!(event.content, None, "an opaque record carries no content");
            assert_eq!(
                event.structured,
                serde_json::json!({ "record_type": marker })
            );
        }

        // Token counts ride on the assistant turn that spent them.
        assert_eq!(t.events[3].token_counts, Some((1200, 340)));
    }

    /// `content` may be a string or an array of parts, and `displayContent`
    /// is never read: real data shows `content` carrying the relativised
    /// path while `displayContent` carries the absolute one.
    #[test]
    fn content_parts_join_with_newline_and_display_content_is_never_read() {
        let source = source();
        let r = source
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("session-1111"))
            .unwrap();
        let t = source.load(&r).unwrap();
        let second_user = t
            .events
            .iter()
            .filter(|e| e.kind == SessionEventKind::User)
            .nth(1)
            .expect("the parts-array turn");
        assert_eq!(
            second_user.content.as_deref(),
            Some("now run it\nagainst @../../.gemini/skills/testing")
        );
        assert!(
            !t.events
                .iter()
                .filter_map(|e| e.content.as_deref())
                .any(|c| c.contains("/home/contributor/.gemini/skills")),
            "displayContent carries the absolute path and must never be read"
        );
    }

    /// Transcript-level fields come from the session document itself.
    #[test]
    fn transcript_fields_come_from_the_session_document() {
        let source = source();
        let r = source
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("session-1111"))
            .unwrap();
        let t = source.load(&r).unwrap();

        assert_eq!(t.source, SOURCE_GEMINI_CLI);
        assert_eq!(
            t.conversation_id.as_deref(),
            Some("11111111-1111-4111-8111-111111111111"),
            "the session's own id, not the file name"
        );
        assert_eq!(
            t.started_at.map(|ts| ts.to_rfc3339()),
            Some("2026-08-25T10:00:00+00:00".to_string())
        );
        assert_eq!(t.model.as_deref(), Some("gemini-3-pro"));
        assert_eq!(t.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(t.project.as_deref(), Some("alpha"));
        assert_eq!(t.subagent_count, 0);
        assert_eq!(t.subagents_dropped, 0);

        // The hash is over the raw file bytes, like every other adapter, so
        // `submission_id_for` stays deterministic across them.
        let bytes = std::fs::read(&r.path).unwrap();
        assert_eq!(t.session_hash, crate::source::session_hash(&bytes));
    }

    /// A subagent session carries no back-reference to a parent, so it
    /// ships standalone rather than being dropped or merged.
    #[test]
    fn a_subagent_session_ships_standalone() {
        let source = source();
        let r = source
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("session-2222"))
            .expect("the subagent session is discovered like any other");
        let t = source.load(&r).unwrap();
        assert_eq!(
            t.conversation_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
        assert_eq!(t.events.len(), 2);
    }

    /// The format is unversioned and evolving, so an unrecognised message
    /// type costs one `Opaque` event rather than the whole session. The
    /// departure from the trajectory reader's fail-closed parse is scoped
    /// to message-type dispatch: a document that is not a session document
    /// at all is still refused.
    #[test]
    fn an_unknown_message_type_is_tolerated_but_a_broken_document_is_not() {
        let root = tempfile::tempdir().unwrap();
        let chats = root.path().join("proj/chats");
        std::fs::create_dir_all(&chats).unwrap();
        let ok = chats.join("session-ok.json");
        std::fs::write(
            &ok,
            br#"{"sessionId":"s","messages":[{"type":"brand-new"},{"type":"user","content":"hi"}]}"#,
        )
        .unwrap();
        let broken = chats.join("session-broken.json");
        std::fs::write(&broken, b"not json at all").unwrap();
        let no_messages = chats.join("session-nomessages.json");
        std::fs::write(&no_messages, br#"{"sessionId":"s"}"#).unwrap();

        let source = GeminiCliSource::new(root.path().to_path_buf());
        let refs = source.discover().unwrap();

        let loaded = |name: &str| {
            let r = refs
                .iter()
                .find(|r| r.path.file_name().unwrap() == name)
                .unwrap();
            source.load(r)
        };

        let t = loaded("session-ok.json").unwrap();
        assert_eq!(t.events[0].kind, SessionEventKind::Opaque);
        assert_eq!(
            t.events[0].structured,
            serde_json::json!({ "record_type": "brand-new" })
        );
        assert_eq!(t.events[1].kind, SessionEventKind::User);

        assert!(loaded("session-broken.json").is_err());
        assert!(
            loaded("session-nomessages.json").is_err(),
            "a document with no messages array is not a session document"
        );
    }

    /// Over budget is declined by name, not truncated: a half-parsed
    /// transcript would upload as though it were the whole conversation.
    #[test]
    fn a_session_over_the_budget_is_declined_by_label() {
        let root = tempfile::tempdir().unwrap();
        let chats = root.path().join("proj/chats");
        std::fs::create_dir_all(&chats).unwrap();
        let big = chats.join("session-big.json");
        let filler = "x".repeat(1024);
        let mut body = String::from(r#"{"sessionId":"s","messages":[{"type":"user","content":""#);
        while body.len() as u64 <= GEMINI_SESSION_BUDGET {
            body.push_str(&filler);
        }
        body.push_str(r#""}]}"#);
        std::fs::write(&big, body.as_bytes()).unwrap();

        let source = GeminiCliSource::new(root.path().to_path_buf());
        let r = source.discover().unwrap().into_iter().next().unwrap();
        let err = source.load(&r).unwrap_err();
        let typed = err
            .downcast_ref::<SessionTooLarge>()
            .expect("declined for what the session is, not an IO blip");
        assert_eq!(typed.label, "gemini-session-too-large");
        assert_eq!(typed.budget_bytes, GEMINI_SESSION_BUDGET);
        assert!(
            !err.to_string().contains("session-big"),
            "a refusal names no path"
        );
    }
}
