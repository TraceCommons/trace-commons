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
    real_file_within_root, session_hash,
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
            // Upstream added `system` and `observation` to trajectory-v1 after
            // this reader was written, and the catch-all below rejects the
            // whole file on an unrecognised role. A contributor with a valid,
            // schema-conforming trajectory was being refused, so both are
            // mapped rather than tolerated: silently dropping them would lose
            // an observation's content, which is real work.
            //
            // Both land on `Opaque` rather than earning their own
            // `SessionEventKind`. A new kind would shift
            // `canonical_whole_trace_representation`, which renders the event
            // type and truncates at twelve events, creating a second
            // novelty-comparison cohort boundary for no gain.
            "observation" => {
                // Content is kept. An observation is environment output --
                // test results, command output -- and is redacted on the way
                // out like every other content field.
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Opaque,
                    timestamp: parse_timestamp(record)?,
                    content: Some(required_str(record, "content")?),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
            "system" => {
                // Content is deliberately dropped: a system prompt is harness
                // boilerplate that is near-identical across every session of a
                // given harness, so it adds nothing to novelty while carrying
                // whatever project context the harness injected into it. The
                // record is still required to be well formed -- a missing
                // content or timestamp is a malformed file, not a shrug.
                let timestamp = parse_timestamp(record)?;
                let _ = required_str(record, "content")?;
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Opaque,
                    timestamp,
                    content: None,
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
            "user" | "reasoning" => {
                let kind = if role == "user" {
                    SessionEventKind::User
                } else {
                    SessionEventKind::Reasoning
                };
                events.push(SessionEvent {
                    served_by: None,
                    kind,
                    timestamp: parse_timestamp(record)?,
                    content: Some(required_str(record, "content")?),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
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
                            if !seen_call_ids.insert(id.clone()) {
                                bail!("duplicate_tool_call_id");
                            }
                            events.push(SessionEvent {
                                served_by: None,
                                kind: SessionEventKind::ToolCall,
                                timestamp,
                                content: None,
                                structured,
                                tool_name: Some(name),
                                token_counts: None,
                                // The id the orphan check above already
                                // relies on; it was validated and discarded.
                                tool_call_id: Some(id),
                                success: None,
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
                            served_by: None,
                            kind: SessionEventKind::Assistant,
                            timestamp,
                            content: Some(content),
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                            tool_call_id: None,
                            success: None,
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
                    served_by: None,
                    kind: SessionEventKind::ToolResult,
                    timestamp: parse_timestamp(record)?,
                    content: Some(required_str(record, "content")?),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: Some(id),
                    success: None,
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

/// One place trajectory files may be read from.
///
/// Trajectory has no conventional per-user store, so unlike the native
/// adapters every location here was either named by the contributor or is
/// one of the two bounded defaults. There is deliberately no scope that
/// means `$HOME`, and none that recurses.
#[derive(Debug, Clone)]
enum Scope {
    /// The path `--trajectory` named: a file, or a directory whose direct
    /// `.json`/`.jsonl` children are all offered. The contributor named it,
    /// so no suffix is required and nothing is silently skipped -- a file
    /// that will not parse is offered and fails loudly at load.
    Declared(PathBuf),
    /// The working directory, direct children only, and only files whose
    /// name ends `.trajectory.json` or `.trajectory.jsonl`.
    ///
    /// The suffix is what keeps a stray `session.json` out of a submission:
    /// nobody opted this directory in, so the file has to say what it is.
    WorkingDirectory(PathBuf),
    /// The staging folder inside the contributor state directory, direct
    /// children only, any `.json`/`.jsonl`.
    ///
    /// No suffix required, because putting a file THERE is the opt-in. It
    /// inherits the state directory's `0700` mode, honours
    /// `TRACE_COMMONS_CONTRIBUTOR_DIR`, and `logout` clears it.
    Staging(PathBuf),
}

/// Reads trajectory-v1 files from the locations this source was built over.
pub struct TrajectorySource {
    scopes: Vec<Scope>,
}

impl TrajectorySource {
    /// The path the contributor named with `--trajectory`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            scopes: vec![Scope::Declared(path)],
        }
    }

    /// Bounded auto-discovery over the working directory and the staging
    /// folder, for a contributor who passed no `--trajectory` at all.
    /// Either location may be absent.
    pub fn auto(working_dir: Option<PathBuf>, staging_dir: Option<PathBuf>) -> Self {
        Self {
            scopes: working_dir
                .map(Scope::WorkingDirectory)
                .into_iter()
                .chain(staging_dir.map(Scope::Staging))
                .collect(),
        }
    }
}

fn is_trajectory_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

/// The name a file must carry to be auto-discovered from a directory nobody
/// opted in -- the working directory.
fn has_trajectory_suffix(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(".trajectory.json") || name.ends_with(".trajectory.jsonl")
}

/// The largest file auto-discovery will read to decide whether it is a
/// trajectory at all.
///
/// Only auto-discovery is bounded by it: a path the contributor named is
/// still read whole, because they said what it was. Here the read is a
/// guess about an unrelated file that happened to be sitting in a
/// directory, and a guess must not be able to pull an arbitrarily large
/// file into memory.
const AUTO_PARSE_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

/// Whether a file auto-discovery found really is a trajectory.
///
/// A file under an auto scope never claimed to be one, so it is skipped
/// rather than refused -- the opposite of the explicit path, which
/// hard-rejects with its reason label because the contributor named it.
/// Checked here rather than at load so the skip happens before a session is
/// ever offered.
/// The `meta.source` of a file auto-discovery found, or `None` when it is
/// not a trajectory at all.
///
/// The declared source comes back with the decision rather than being
/// fetched separately, because the decision IS a parse and it runs under a
/// 64 MB budget. Reading the file a second time to learn a string the first
/// read already had would make `list` walk every staged conversation twice.
///
/// `meta.source` is required and validated, so a file that parses always has
/// one: `Some` here means both "this is a trajectory" and "this is what it
/// says it came from".
fn parsed_source(path: &Path) -> Option<String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() <= AUTO_PARSE_BUDGET => {}
        _ => return None,
    }
    let bytes = std::fs::read(path).ok()?;
    parse_trajectory(&bytes).ok().map(|p| p.source)
}

/// What a scope decided about a file.
enum Accepted {
    /// Not this scope's file.
    No,
    /// Accepted on its name alone. The contributor named this path, so it is
    /// offered without a discovery-time parse and nothing knows what it
    /// declares until it is loaded.
    WithoutParse,
    /// Accepted by parsing it, which also read what it declares itself to be
    /// -- `antigravity` for an imported conversation, and so on.
    Parsed(String),
}

impl Scope {
    /// Whether this scope offers `path`, given that it is a real file
    /// directly inside the scope's directory.
    fn accepts(&self, path: &Path) -> Accepted {
        let parsed = |path: &Path| match parsed_source(path) {
            Some(source) => Accepted::Parsed(source),
            None => Accepted::No,
        };
        match self {
            Scope::Declared(_) => {
                if is_trajectory_file(path) {
                    Accepted::WithoutParse
                } else {
                    Accepted::No
                }
            }
            Scope::WorkingDirectory(_) if has_trajectory_suffix(path) => parsed(path),
            Scope::Staging(_) if is_trajectory_file(path) => parsed(path),
            Scope::WorkingDirectory(_) | Scope::Staging(_) => Accepted::No,
        }
    }

    fn dir(&self) -> &Path {
        match self {
            Scope::Declared(p) | Scope::WorkingDirectory(p) | Scope::Staging(p) => p,
        }
    }
}

fn session_ref_for(path: PathBuf, declared_source: Option<String>) -> Option<SessionRef> {
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
        declared_source,
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
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        for scope in &self.scopes {
            // A declared path that names a file IS the session; every other
            // scope is a directory read non-recursively.
            if matches!(scope, Scope::Declared(_)) && scope.dir().is_file() {
                // No declared source: a named path is offered without a
                // discovery-time parse, and inventing one by parsing here
                // would undo that.
                match session_ref_for(scope.dir().to_path_buf(), None) {
                    Some(r) => sessions.push(r),
                    None => skipped += 1,
                }
                continue;
            }
            let Ok(entries) = std::fs::read_dir(scope.dir()) else {
                continue;
            };
            for entry in entries {
                let Ok(entry) = entry else {
                    skipped += 1;
                    continue;
                };
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let declared_source = match scope.accepts(&path) {
                    Accepted::No => continue,
                    Accepted::WithoutParse => None,
                    Accepted::Parsed(source) => Some(source),
                };
                match session_ref_for(path, declared_source) {
                    Some(r) => sessions.push(r),
                    None => skipped += 1,
                }
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

    /// A changed trajectory file is its own session, on exactly the terms
    /// `discover` uses: the declared path itself when it names a file, and
    /// otherwise a file sitting DIRECTLY in one of this source's scopes and
    /// accepted by that scope's own rule -- suffix and parse included.
    /// Trajectory discovery does not recurse, so neither does this: a
    /// nested file the walk never sees must not be addressable, and a
    /// mapping laxer than discovery would be a way to name a file the
    /// contributor never agreed to.
    /// Returns the address AND what the accepting scope decided about it,
    /// so `session_at` can describe the file exactly as `discover` would.
    /// Re-deriving the declared source here instead -- by parsing the file
    /// unconditionally -- would make a watch event report a source that a
    /// full sweep of the same scope does not, which
    /// `session_at_describes_a_trajectory_exactly_as_discover_does` exists
    /// to forbid.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        self.addressed(path).map(|(address, _)| address)
    }

    /// The ref for whichever trajectory file a changed path names.
    ///
    /// `addressed` resolves the address AND carries the accepting scope's
    /// decision, and `session_ref_for` describes it -- the same function
    /// `discover` uses -- so a scoped scan and a full sweep cannot
    /// disagree. `session_ref_for` already answers `None` for a file it
    /// cannot stat, which is what a file deleted between the event and this
    /// lookup looks like.
    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some((address, declared_source)) = self.addressed(path) else {
            return Ok(None);
        };
        Ok(session_ref_for(address, declared_source))
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

        // The trajectory file's own stem -- the identifier this session is
        // already addressed by. Trajectory carries no separate in-file
        // session id, so this is not an invented one; it is the file's
        // existing name.
        let conversation_id = r
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());

        Ok(SessionTranscript {
            source: Cow::Owned(parsed.source),
            // Trajectory carries no harness version field.
            agent_version: None,
            model: parsed.model,
            project,
            cwd: parsed.cwd,
            started_at,
            session_hash: hash,
            conversation_id,
            events: parsed.events,
            subagent_count: 0,
            subagents_dropped: 0,
            routing: Vec::new(),
            attested_call: None,
        })
    }
}

impl TrajectorySource {
    /// The address a changed path maps to, together with what the accepting
    /// scope decided about it.
    ///
    /// The decision travels with the address so `session_at` can describe a
    /// file exactly as `discover` would. Re-deriving the declared source by
    /// parsing here instead would make a watch event report an origin that
    /// a full sweep of the same scope does not, which
    /// `session_at_describes_a_trajectory_exactly_as_discover_does` exists
    /// to forbid.
    fn addressed(&self, path: &Path) -> Option<(PathBuf, Option<String>)> {
        for scope in &self.scopes {
            if matches!(scope, Scope::Declared(_)) && scope.dir().is_file() {
                // A single declared file is the whole scope. Compared
                // rather than resolved: the contributor named this exact
                // path.
                if path == scope.dir() {
                    return Some((scope.dir().to_path_buf(), None));
                }
                continue;
            }
            let Some(real) = real_file_within_root(scope.dir(), path) else {
                continue;
            };
            if real.parent() != Some(scope.dir()) {
                continue;
            }
            match scope.accepts(&real) {
                Accepted::No => continue,
                Accepted::WithoutParse => return Some((real, None)),
                Accepted::Parsed(source) => return Some((real, Some(source))),
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SessionEventKind;

    /// A trajectory file is its own session, on the same terms `discover`
    /// uses: flat, direct children only, and nothing outside the declared
    /// path.
    #[test]
    fn a_trajectory_file_maps_to_itself_and_nothing_else_does() {
        let dir = tempfile::tempdir().unwrap();
        let flat = dir.path().join("run.jsonl");
        std::fs::write(&flat, SAMPLE).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("run.jsonl"), SAMPLE).unwrap();

        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("run.jsonl");
        std::fs::write(&elsewhere, SAMPLE).unwrap();

        let source = TrajectorySource::new(dir.path().to_path_buf());
        assert_eq!(source.session_for_path(&flat), Some(flat.clone()));
        assert!(source.discover().unwrap().iter().any(|r| r.path == flat));

        for path in [
            dir.path().join("notes.txt"),
            // Discovery does not recurse, so a nested file is not a session
            // this source owns.
            nested.join("run.jsonl"),
            nested,
            dir.path().to_path_buf(),
            dir.path().join("never-written.jsonl"),
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

    #[test]
    fn a_source_declared_as_one_file_maps_only_that_file() {
        let dir = tempfile::tempdir().unwrap();
        let declared = dir.path().join("run.jsonl");
        std::fs::write(&declared, SAMPLE).unwrap();
        let sibling = dir.path().join("other.jsonl");
        std::fs::write(&sibling, SAMPLE).unwrap();

        let source = TrajectorySource::new(declared.clone());
        assert_eq!(source.session_for_path(&declared), Some(declared));
        assert_eq!(source.session_for_path(&sibling), None);
    }

    #[test]
    fn session_at_describes_a_trajectory_exactly_as_discover_does() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.jsonl");
        std::fs::write(&file, SAMPLE).unwrap();

        let source = TrajectorySource::new(dir.path().to_path_buf());
        let discovered = source.discover().unwrap();
        assert_eq!(discovered.len(), 1);
        let scoped = source.session_at(&file).unwrap().expect("a session");

        // `Debug` rather than a hand-listed field set, so a field added
        // later is covered too.
        assert_eq!(format!("{scoped:?}"), format!("{:?}", discovered[0]));
        assert_eq!(scoped.path, file);
        assert_eq!(scoped.source, SOURCE_TRAJECTORY);
    }

    #[test]
    fn a_vanished_or_foreign_trajectory_is_ok_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("run.jsonl");
        std::fs::write(&file, SAMPLE).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let elsewhere = outside.path().join("run.jsonl");
        std::fs::write(&elsewhere, SAMPLE).unwrap();

        let source = TrajectorySource::new(dir.path().to_path_buf());
        assert!(source.session_at(&file).unwrap().is_some());
        assert!(source.session_at(&elsewhere).unwrap().is_none());

        std::fs::remove_file(&file).unwrap();
        assert!(
            source.session_at(&file).unwrap().is_none(),
            "a deleted trajectory must be Ok(None), not an error"
        );
    }

    #[test]
    #[cfg(unix)]
    fn path_mapping_refuses_symlinks_and_traversal_out_of_the_trajectory_root() {
        use std::os::unix::fs::symlink;

        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.jsonl");
        std::fs::write(&secret, SAMPLE).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("run.jsonl");
        std::fs::write(&real, SAMPLE).unwrap();
        let linked = dir.path().join("link.jsonl");
        symlink(&secret, &linked).unwrap();

        let source = TrajectorySource::new(dir.path().to_path_buf());
        assert_eq!(source.session_for_path(&linked), None);
        assert_eq!(
            source.session_for_path(&dir.path().join("..").join("secret.jsonl")),
            None
        );
        assert_eq!(source.session_for_path(&real), Some(real.clone()));
    }

    /// Auto-discovery in the working directory is suffix-gated. A stray
    /// `session.json` sitting in a repo is not a trajectory and must not
    /// join a submission just because it parses.
    #[test]
    fn the_working_directory_yields_only_suffixed_files() {
        let cwd = tempfile::tempdir().unwrap();
        for name in [
            "run.trajectory.json",
            "run.trajectory.jsonl",
            "session.json",
            "notes.jsonl",
            "notes.txt",
        ] {
            std::fs::write(cwd.path().join(name), SAMPLE).unwrap();
        }
        let nested = cwd.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.trajectory.json"), SAMPLE).unwrap();

        let source = TrajectorySource::auto(Some(cwd.path().to_path_buf()), None);
        let mut names: Vec<String> = source
            .discover()
            .unwrap()
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["run.trajectory.json", "run.trajectory.jsonl"]);
        assert_eq!(
            source.session_for_path(&nested.join("deep.trajectory.json")),
            None,
            "auto-discovery never recurses, so a nested file is not addressable"
        );
        assert_eq!(
            source.session_for_path(&cwd.path().join("session.json")),
            None
        );
    }

    /// The staging directory needs no suffix: putting a file there IS the
    /// opt-in, and it is inside the contributor's own 0700 state directory.
    #[test]
    fn the_staging_directory_yields_any_json_or_jsonl() {
        let staging = tempfile::tempdir().unwrap();
        for name in ["one.json", "two.jsonl", "notes.txt"] {
            std::fs::write(staging.path().join(name), SAMPLE).unwrap();
        }
        let source = TrajectorySource::auto(None, Some(staging.path().to_path_buf()));
        let mut names: Vec<String> = source
            .discover()
            .unwrap()
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["one.json", "two.jsonl"]);
    }

    /// A file that does not parse is skipped under auto-discovery, because
    /// it never claimed to be a trajectory -- and still hard-rejects under
    /// an explicit `--trajectory`, because the contributor named it.
    #[test]
    fn auto_discovery_skips_what_does_not_parse_and_an_explicit_path_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.trajectory.json");
        let bad = dir.path().join("bad.trajectory.json");
        std::fs::write(&good, SAMPLE).unwrap();
        std::fs::write(&bad, br#"{"not":"a trajectory"}"#).unwrap();

        let auto = TrajectorySource::auto(Some(dir.path().to_path_buf()), None);
        let found = auto.discover().unwrap();
        assert_eq!(found.len(), 1, "the unparseable file is skipped");
        assert_eq!(found[0].path, good);
        assert_eq!(
            auto.session_for_path(&bad),
            None,
            "and it is not addressable either"
        );

        let declared = TrajectorySource::new(bad.clone());
        let r = declared
            .discover()
            .unwrap()
            .into_iter()
            .next()
            .expect("a named file is offered even when it will not parse");
        let err = declared.load(&r).unwrap_err();
        assert_eq!(err.to_string(), "missing_meta_record");
    }

    /// Auto-discovery is two locations and no more. Neither an absent
    /// working directory nor an absent staging directory is an error, and
    /// nothing outside them is ever read.
    #[test]
    fn auto_discovery_is_bounded_to_the_two_locations_it_was_given() {
        let cwd = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(cwd.path().join("a.trajectory.json"), SAMPLE).unwrap();
        std::fs::write(staging.path().join("b.json"), SAMPLE).unwrap();
        std::fs::write(elsewhere.path().join("c.trajectory.json"), SAMPLE).unwrap();

        let source = TrajectorySource::auto(
            Some(cwd.path().to_path_buf()),
            Some(staging.path().to_path_buf()),
        );
        assert_eq!(source.discover().unwrap().len(), 2);
        assert_eq!(
            source.session_for_path(&elsewhere.path().join("c.trajectory.json")),
            None
        );

        // A location that does not exist is simply empty.
        let missing = TrajectorySource::auto(
            Some(cwd.path().join("gone")),
            Some(staging.path().join("gone")),
        );
        assert!(missing.discover().unwrap().is_empty());
        assert!(
            TrajectorySource::auto(None, None)
                .discover()
                .unwrap()
                .is_empty()
        );
    }

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

    #[test]
    fn tool_call_ids_reach_the_events() {
        // This parser already validated these ids -- duplicate calls and
        // orphaned results are both rejection reasons -- and then dropped
        // them, so nothing downstream could use what had just been checked.
        let file = concat!(
            "{\"role\":\"meta\",\"source\":\"openhands\",\"model\":\"m\"}\n",
            "{\"role\":\"user\",\"content\":\"list the files\",",
            "\"timestamp\":\"2026-08-08T10:00:00Z\"}\n",
            "{\"role\":\"assistant\",\"tool_calls\":[{\"id\":\"t1\",",
            "\"name\":\"shell\",\"args\":\"{\\\"cmd\\\":\\\"ls\\\"}\"}],",
            "\"timestamp\":\"2026-08-08T10:00:01Z\"}\n",
            "{\"role\":\"tool\",\"tool_call_id\":\"t1\",\"content\":\"src\",",
            "\"timestamp\":\"2026-08-08T10:00:02Z\"}\n",
        );
        let parsed = super::parse_trajectory(file.as_bytes()).expect("fixture parses");
        let call = parsed
            .events
            .iter()
            .find(|e| e.kind == SessionEventKind::ToolCall)
            .expect("a tool call");
        let result = parsed
            .events
            .iter()
            .find(|e| e.kind == SessionEventKind::ToolResult)
            .expect("a tool result");
        assert_eq!(call.tool_call_id.as_deref(), Some("t1"));
        assert_eq!(result.tool_call_id.as_deref(), Some("t1"));
    }
}
