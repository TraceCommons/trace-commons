# Cline Session Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a fourth native `TraceSource`, `cline`, that reads the sessions the current Cline release writes under `~/.cline/data/sessions/`, so an agent IronWire already routes becomes one we acquire traces from.

**Architecture:** A new adapter module `source/cline.rs` mirrors `source/gemini_cli.rs` exactly in shape: one conventional root resolved from Cline's own environment variables, one directory per session, one JSON document per session, tolerant message-type dispatch, fail-closed on path containment and byte budget. It is registered in `NATIVE_SOURCES` with `Undeclared::Nothing` (an absent declaration constructs nothing, as Gemini does), gets a `cline_source` settings key, a `cline_source_mode` IPC field, and a discovery candidate appended after Gemini. No GUI rows: GTK, macOS and Windows shells are untouched by this plan.

**Tech Stack:** Rust, `serde_json`, `chrono`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-03-ironwire-agent-alignment.md` section 4.2.

## Global Constraints

- Work only inside this worktree: `/Users/zakimanian/code/trace-commons-server/.claude/worktrees/ironwire-agent-alignment`. Relative paths for everything. Never `cd` elsewhere, never `git -C` another checkout.
- **No new dependencies.** If you think one is needed, stop and report.
- `trace-commons-contributor` is `MIT OR Apache-2.0`. Never add `trace-commons-server`, `-gate-api` or `-gate-enclave` to its dependencies. Every new `.rs` file in this crate needs no AGPL header (it is a permissive crate); copy the first lines of `source/gemini_cli.rs` to see what a file in this crate starts with and match it.
- **Hash-only logging.** No paths, session ids or content in `tracing` strings. Error strings are reason labels only (`malformed_cline_session`), never content.
- **Cwd is never serialized.** `SessionTranscript.cwd` feeds the redactor and hashing only. Do not put it in `structured` or `content`.
- **Fixture provenance.** No Cline is installed on this machine. The fixtures in this plan are transcribed from upstream source at `cline/cline` main (extension 4.1.17): `sdk/packages/shared/src/llms/messages.ts` (`MessageWithMetadata`, content blocks), `sdk/packages/core/src/services/session-data.ts` (`buildMessagesFilePayload`), `sdk/packages/core/src/session/models/session-manifest.ts` (`SessionManifestSchema`), `sdk/packages/shared/src/storage/paths.ts` (roots). Task 1 writes a README saying exactly that. A fixture and a parser written together can agree with each other and both be wrong; the final report must say this has not been run against a real Cline install.
- Verification for every task (plain `cargo check` hides what CI catches):
  ```bash
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
  RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
  cargo clippy -p trace-commons-contributor --all-targets -- \
    -A clippy::type_complexity -A clippy::collapsible_if \
    -A clippy::manual_option_as_slice -A clippy::useless_vec \
    -A clippy::redundant_pattern_matching
  cargo fmt --all
  ```
  If the disk is full (`No space left on device`), set `CARGO_TARGET_DIR=/Users/zakimanian/code/trace-commons-server/target` for every cargo command in this plan and say so in the report. Do not delete anything to make room.
- Commit after each task. Subjects: short imperative, no prefix, no emoji. `git add` named files only, never `git add -A` or `git add .`.
- No emojis anywhere.

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-contributor/tests/fixtures/cline/README.md` (create) | Provenance of the fixtures |
| `crates/trace-commons-contributor/tests/fixtures/cline/sessions/...` (create) | Two well-formed sessions, one malformed, one stray directory |
| `crates/trace-commons-contributor/src/source/cline.rs` (create) | `ClineSource`, conventional root, discovery, load, path mapping, event mapping |
| `crates/trace-commons-contributor/src/source/mod.rs` (modify) | `SOURCE_CLINE`, `pub mod cline`, `NATIVE_SOURCES` entry |
| `crates/trace-commons-contributor/src/source/discovery.rs` (modify) | Fourth `SourceCandidate` |
| `crates/trace-commons-contributor/src/daemon/settings.rs` (modify) | `cline_source` field, key mapping, apply arm |
| `crates/trace-commons-contributor/src/daemon/ipc.rs` (modify) | `cline_source_mode` in the settings view |
| `crates/trace-commons-contributor-ffi/tests/abi.rs` (modify) | Discovery now lists four sources |

## The format being read

From upstream. `<root>/<sessionId>/<sessionId>.messages.json`:

```json
{
  "version": 1,
  "updated_at": "2026-09-03T11:20:05.000Z",
  "agent": "lead",
  "sessionId": "1756900000000_k3x9q",
  "origin": { "source": "vscode", "mode": "act", "sessionId": "1756900000000_k3x9q" },
  "messages": [
    { "role": "user", "ts": 1756900000000,
      "content": [ { "type": "text", "text": "List the files in src" } ] },
    { "role": "assistant", "ts": 1756900001500,
      "modelInfo": { "id": "claude-sonnet-5", "provider": "anthropic" },
      "metrics": { "inputTokens": 1200, "outputTokens": 85, "cacheReadTokens": 0, "cacheWriteTokens": 0, "cost": 0.0041 },
      "content": [
        { "type": "thinking", "thinking": "The user wants a directory listing." },
        { "type": "text", "text": "I'll list the directory." },
        { "type": "tool_use", "id": "toolu_01", "name": "list_files", "input": { "path": "src", "recursive": false } } ] },
    { "role": "user", "ts": 1756900001600,
      "content": [ { "type": "tool_result", "tool_use_id": "toolu_01", "name": "list_files", "content": "index.ts\nutil.ts" } ] }
  ]
}
```

`content` may also be a bare string. `tool_result.content` may be a string or an array of `{type:"text",text}` / `{type:"image",...}` parts. `tool_result.is_error` is optional. `image` blocks carry base64 `data`; they are never copied into an event.

`<root>/<sessionId>/<sessionId>.json` (manifest, optional):

```json
{ "version": 1, "session_id": "1756900000000_k3x9q", "source": "vscode", "pid": 4242,
  "started_at": "2026-09-03T11:20:00.000Z", "status": "completed", "interactive": true,
  "provider": "anthropic", "model": "claude-sonnet-5",
  "cwd": "/home/contributor/code/alpha", "workspace_root": "/home/contributor/code/alpha",
  "enable_tools": true, "enable_spawn": false, "enable_teams": false }
```

Root resolution (`paths.ts`): `CLINE_SESSION_DATA_DIR`; else `CLINE_DATA_DIR/sessions`; else `CLINE_DIR/data/sessions`; else `~/.cline/data/sessions`. Empty values count as unset.

---

### Task 1: Fixtures and their provenance

**Files:**
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/README.md`
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/sessions/1756900000000_k3x9q/1756900000000_k3x9q.messages.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/sessions/1756900000000_k3x9q/1756900000000_k3x9q.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/sessions/1756900100000_p2m7z/1756900100000_p2m7z.messages.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/sessions/1756900200000_bad00/1756900200000_bad00.messages.json`
- Create: `crates/trace-commons-contributor/tests/fixtures/cline/sessions/not-a-session/notes.txt`

**Interfaces:**
- Produces: the fixture tree every later test reads via `env!("CARGO_MANIFEST_DIR")/tests/fixtures/cline/sessions`.

- [x] **Step 1: Write the README**

```markdown
# Cline session fixtures

Transcribed from upstream source, not captured from an install: no Cline was
available on the machine these were written on. Shapes follow `cline/cline`
main (extension 4.1.17) as of 2026-09-03:

- `sdk/packages/core/src/services/session-data.ts` -- `buildMessagesFilePayload`
  is the `.messages.json` wrapper (`version`, `updated_at`, `agent`,
  `sessionId`, `origin`, `messages`, optional `system_prompt`).
- `sdk/packages/shared/src/llms/messages.ts` -- `MessageWithMetadata` and the
  `text` / `thinking` / `tool_use` / `tool_result` / `image` blocks.
- `sdk/packages/core/src/session/models/session-manifest.ts` -- the sibling
  `<id>.json` manifest.
- `sdk/packages/shared/src/storage/paths.ts` -- `~/.cline/data/sessions` and
  the environment variables that relocate it.

Until a session written by a real Cline is dropped in here and the tests
still pass, treat the reader as unverified against the wild.

Layout:

- `1756900000000_k3x9q/` -- a full session: manifest, thinking, a tool call
  and its result, per-message model and metrics.
- `1756900100000_p2m7z/` -- no manifest, string content, a failed tool
  result, an image block, and an unknown block type.
- `1756900200000_bad00/` -- a document with no `messages` array. Must be
  refused, not offered as an empty transcript.
- `not-a-session/` -- a directory with no messages file. Must be skipped.
```

- [x] **Step 2: Write the full session**

`1756900000000_k3x9q/1756900000000_k3x9q.messages.json`:

```json
{
  "version": 1,
  "updated_at": "2026-09-03T11:20:05.000Z",
  "agent": "lead",
  "sessionId": "1756900000000_k3x9q",
  "origin": { "source": "vscode", "mode": "act", "sessionId": "1756900000000_k3x9q" },
  "messages": [
    {
      "role": "user",
      "ts": 1756900000000,
      "content": [ { "type": "text", "text": "List the files in src" } ]
    },
    {
      "role": "assistant",
      "ts": 1756900001500,
      "modelInfo": { "id": "claude-sonnet-5", "provider": "anthropic" },
      "metrics": { "inputTokens": 1200, "outputTokens": 85, "cacheReadTokens": 0, "cacheWriteTokens": 0, "cost": 0.0041 },
      "content": [
        { "type": "thinking", "thinking": "The user wants a directory listing." },
        { "type": "text", "text": "I'll list the directory." },
        { "type": "tool_use", "id": "toolu_01", "name": "list_files", "input": { "path": "src", "recursive": false } }
      ]
    },
    {
      "role": "user",
      "ts": 1756900001600,
      "content": [
        { "type": "tool_result", "tool_use_id": "toolu_01", "name": "list_files", "content": "index.ts\nutil.ts" }
      ]
    },
    {
      "role": "assistant",
      "ts": 1756900003000,
      "modelInfo": { "id": "claude-sonnet-5", "provider": "anthropic" },
      "metrics": { "inputTokens": 1300, "outputTokens": 20, "cacheReadTokens": 1100, "cacheWriteTokens": 0, "cost": 0.0012 },
      "content": [ { "type": "text", "text": "Two files: index.ts and util.ts." } ]
    }
  ]
}
```

`1756900000000_k3x9q/1756900000000_k3x9q.json`:

```json
{
  "version": 1,
  "session_id": "1756900000000_k3x9q",
  "source": "vscode",
  "pid": 4242,
  "started_at": "2026-09-03T11:20:00.000Z",
  "ended_at": "2026-09-03T11:20:05.000Z",
  "status": "completed",
  "interactive": true,
  "provider": "anthropic",
  "model": "claude-sonnet-5",
  "cwd": "/home/contributor/code/alpha",
  "workspace_root": "/home/contributor/code/alpha",
  "enable_tools": true,
  "enable_spawn": false,
  "enable_teams": false
}
```

- [x] **Step 3: Write the manifest-less session**

`1756900100000_p2m7z/1756900100000_p2m7z.messages.json`:

```json
{
  "version": 1,
  "updated_at": "2026-09-03T11:22:00.000Z",
  "agent": "lead",
  "sessionId": "1756900100000_p2m7z",
  "origin": { "source": "vscode", "mode": "act", "sessionId": "1756900100000_p2m7z" },
  "messages": [
    { "role": "user", "ts": 1756900100000, "content": "Run the tests" },
    {
      "role": "assistant",
      "ts": 1756900101000,
      "modelInfo": { "id": "gpt-5.5", "provider": "openai" },
      "content": [
        { "type": "tool_use", "id": "call_9", "name": "execute_command", "input": { "command": "cargo test" } }
      ]
    },
    {
      "role": "user",
      "ts": 1756900102000,
      "content": [
        { "type": "tool_result", "tool_use_id": "call_9", "name": "execute_command", "is_error": true,
          "content": [ { "type": "text", "text": "error: no such command" }, { "type": "image", "data": "AAAA", "mediaType": "image/png" } ] },
        { "type": "image", "data": "BBBB", "mediaType": "image/png" },
        { "type": "future_block", "payload": 1 }
      ]
    }
  ]
}
```

- [x] **Step 4: Write the malformed session and the stray directory**

`1756900200000_bad00/1756900200000_bad00.messages.json`:

```json
{ "version": 1, "sessionId": "1756900200000_bad00", "agent": "lead" }
```

`not-a-session/notes.txt`:

```
not a session
```

- [x] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/tests/fixtures/cline
git commit -m "Add Cline session fixtures transcribed from upstream"
```

---

### Task 2: The adapter: root, discovery, path mapping

**Files:**
- Create: `crates/trace-commons-contributor/src/source/cline.rs`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (add `pub mod cline;` beside `pub mod gemini_cli;`, and `pub const SOURCE_CLINE: &str = "cline";` after `SOURCE_GEMINI_CLI`)

**Interfaces:**
- Produces: `pub struct ClineSource`, `ClineSource::new(root: PathBuf)`, `pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf`, `pub fn conventional_root_this_machine() -> PathBuf`, `pub const CLINE_DIR_ENV`, `CLINE_DATA_DIR_ENV`, `CLINE_SESSION_DATA_DIR_ENV`, `pub(crate) const CLINE_SESSION_BUDGET: u64`.
- Consumes from `source/mod.rs`: `SOURCE_CLINE`, `SessionRef`, `SessionTranscript`, `SessionEvent`, `SessionEventKind`, `TraceSource`, `real_file_within_root`, `session_hash`, `SessionTooLarge`, and `claude_code::GROUP_RAW_BYTE_BUDGET`. Read `source/gemini_cli.rs:1-242` first; this task copies its structure.

- [x] **Step 1: Write the failing tests**

Create `cline.rs` with the module doc, the constants, and only this test module at the bottom (implementation comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SessionEventKind, TraceSource};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cline/sessions")
    }

    fn source() -> ClineSource {
        ClineSource::new(fixture_root())
    }

    #[test]
    fn the_conventional_root_follows_clines_own_precedence() {
        let home = Path::new("/home/c");
        let none = |_: &str| None;
        assert_eq!(
            conventional_root(home, none),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
        let dir = |k: &str| (k == CLINE_DIR_ENV).then(|| "/opt/cline".to_string());
        assert_eq!(
            conventional_root(home, dir),
            PathBuf::from("/opt/cline/data/sessions")
        );
        let data = |k: &str| match k {
            CLINE_DIR_ENV => Some("/opt/cline".to_string()),
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            _ => None,
        };
        assert_eq!(conventional_root(home, data), PathBuf::from("/data/cl/sessions"));
        let sessions = |k: &str| match k {
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            CLINE_SESSION_DATA_DIR_ENV => Some("/s".to_string()),
            _ => None,
        };
        assert_eq!(conventional_root(home, sessions), PathBuf::from("/s"));
        // An empty value is unset, as upstream's `.trim()` check treats it.
        let empty = |k: &str| (k == CLINE_SESSION_DATA_DIR_ENV).then(String::new);
        assert_eq!(
            conventional_root(home, empty),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
    }

    #[test]
    fn discovery_finds_each_messages_file_and_nothing_else() {
        let refs = source().discover().unwrap();
        let mut names: Vec<String> = refs
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "1756900000000_k3x9q.messages.json",
                "1756900100000_p2m7z.messages.json",
                "1756900200000_bad00.messages.json",
            ],
            "the stray directory is skipped; a malformed document is still discovered and refused at load"
        );
        for r in &refs {
            assert_eq!(r.source, SOURCE_CLINE);
            assert!(r.size_bytes > 0);
            assert_eq!(r.group_member_count, 0);
        }
    }

    #[test]
    fn a_manifest_gives_discovery_the_cwd_and_project() {
        let refs = source().discover().unwrap();
        let with = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("k3x9q"))
            .unwrap();
        assert_eq!(with.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(with.project.as_deref(), Some("alpha"));
        let without = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("p2m7z"))
            .unwrap();
        assert_eq!(without.cwd, None, "no manifest, no guess");
        assert_eq!(
            without.project.as_deref(),
            Some("1756900100000_p2m7z"),
            "the session directory name is the fallback label"
        );
    }

    #[test]
    fn a_changed_messages_file_maps_to_its_own_session_and_nothing_else_does() {
        let s = source();
        let messages = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.messages.json");
        assert_eq!(s.session_for_path(&messages), Some(messages.clone()));
        // The manifest changing does not change the transcript's bytes.
        let manifest = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.json");
        assert_eq!(s.session_for_path(&manifest), None);
        // Outside the root, and a name that does not follow the rule.
        assert_eq!(s.session_for_path(Path::new("/etc/passwd")), None);
        let stray = fixture_root().join("not-a-session/notes.txt");
        assert_eq!(s.session_for_path(&stray), None);
        // A messages file whose name disagrees with its directory is not a
        // session: the id is the directory, and the file must repeat it.
        let wrong = fixture_root().join("1756900000000_k3x9q/other.messages.json");
        assert_eq!(s.session_for_path(&wrong), None);
    }

    #[test]
    fn session_at_agrees_with_discover() {
        let s = source();
        for r in s.discover().unwrap() {
            let again = s.session_at(&r.path).unwrap().expect("the same session");
            assert_eq!(again.path, r.path);
            assert_eq!(again.size_bytes, r.size_bytes);
            assert_eq!(again.cwd, r.cwd);
            assert_eq!(again.project, r.project);
        }
    }
}
```

Note the `session_for_path` test for `wrong`: that file does not exist, and `real_file_within_root` canonicalises, so the answer is `None` for two reasons. That is fine; the test pins the outcome, not the reason.

- [x] **Step 2: Run tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib source::cline`
Expected: compile errors (`ClineSource`, `conventional_root` not found).

- [x] **Step 3: Implement**

Top of `cline.rs`, above the test module:

```rust
//! Cline session adapter.
//!
//! Reads `<root>/<session-id>/<session-id>.messages.json`, where `<root>` is
//! Cline's own session data directory: `$CLINE_SESSION_DATA_DIR`, else
//! `$CLINE_DATA_DIR/sessions`, else `$CLINE_DIR/data/sessions`, else
//! `~/.cline/data/sessions`. One directory is one session; the messages
//! document is a single JSON object, not JSONL, and a sibling
//! `<session-id>.json` manifest carries the working directory, the model and
//! the start time when the session had one.
//!
//! This is the store the current Cline release (extension 4.1.17, built on
//! the `@cline/core` SDK) writes. The pre-SDK layout under VS Code's global
//! storage (`tasks/<id>/api_conversation_history.json`) is not read: upstream
//! itself treats it as read-only legacy, and it carries neither timestamps
//! nor model information per message.
//!
//! **Message-type dispatch is tolerant, and only that**, on the same terms as
//! `gemini_cli`: an unrecognised content block becomes an `Opaque` event
//! with a type marker rather than rejecting the file, because the SDK's
//! message shape is young and moving. Everything a gate depends on -- path
//! containment, the byte budget, and the requirement that the document
//! actually carry a `messages` array -- stays fail-closed.
//!
//! Image blocks are never copied: their `data` is base64 pixels, which is
//! neither text a gate scores nor something a contributor reviewed.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde_json::{Value, json};

use super::{
    SOURCE_CLINE, SessionEvent, SessionEventKind, SessionRef, SessionTranscript, TraceSource,
    real_file_within_root, session_hash,
};

/// Overrides the whole Cline directory; sessions live under `data/sessions`.
pub const CLINE_DIR_ENV: &str = "CLINE_DIR";
/// Overrides the data directory; sessions live under `sessions`.
pub const CLINE_DATA_DIR_ENV: &str = "CLINE_DATA_DIR";
/// Overrides the session directory itself.
pub const CLINE_SESSION_DATA_DIR_ENV: &str = "CLINE_SESSION_DATA_DIR";

const MESSAGES_SUFFIX: &str = ".messages.json";
const MANIFEST_SUFFIX: &str = ".json";

/// The largest session document this adapter will load, shared with every
/// other adapter's budget: they all bound how much of one conversation may
/// become resident on its way to being discarded.
pub(crate) const CLINE_SESSION_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

/// The conventional store, resolved the way Cline's own `paths.ts` does it.
/// An empty variable counts as unset, matching upstream's `.trim()` check.
pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let set = |key: &str| env(key).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    if let Some(sessions) = set(CLINE_SESSION_DATA_DIR_ENV) {
        return sessions;
    }
    if let Some(data) = set(CLINE_DATA_DIR_ENV) {
        return data.join("sessions");
    }
    set(CLINE_DIR_ENV)
        .unwrap_or_else(|| home.join(".cline"))
        .join("data")
        .join("sessions")
}

/// The conventional store, resolved against this machine's real home and
/// environment.
pub fn conventional_root_this_machine() -> PathBuf {
    conventional_root(&dirs::home_dir().unwrap_or_default(), |key| {
        std::env::var(key).ok()
    })
}

pub struct ClineSource {
    root: PathBuf,
}

impl ClineSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

/// The messages file a session directory must hold: `<dir name>.messages.json`.
fn messages_file_for(session_dir: &Path) -> Option<PathBuf> {
    let id = session_dir.file_name()?.to_str()?;
    Some(session_dir.join(format!("{id}{MESSAGES_SUFFIX}")))
}

/// The sibling manifest, if the session wrote one.
fn manifest_for(messages_path: &Path) -> Option<PathBuf> {
    let dir = messages_path.parent()?;
    let id = dir.file_name()?.to_str()?;
    let candidate = dir.join(format!("{id}{MANIFEST_SUFFIX}"));
    candidate.is_file().then_some(candidate)
}

/// What the manifest says about the session, where it says it. Every field
/// is optional: a session interrupted before its manifest was written is
/// still a session.
#[derive(Default)]
struct Manifest {
    cwd: Option<String>,
    model: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn read_manifest(messages_path: &Path) -> Manifest {
    let Some(path) = manifest_for(messages_path) else {
        return Manifest::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Manifest::default();
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return Manifest::default();
    };
    let string = |key: &str| {
        doc.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    Manifest {
        cwd: string("cwd"),
        model: string("model"),
        started_at: timestamp_rfc3339(doc.get("started_at")),
    }
}

/// The label a picker renders: the basename of the working directory when
/// there is one, otherwise the session directory's own name.
fn project_label(session_dir: &Path, cwd: Option<&str>) -> Option<String> {
    cwd.map(Path::new)
        .or(Some(session_dir))
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// The one way a Cline `SessionRef` is built, shared by `discover` and
/// `session_at` so a scoped scan and a full sweep cannot disagree.
fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
    let session_dir = path.parent()?.to_path_buf();
    let metadata = std::fs::metadata(&path).ok()?;
    let manifest = read_manifest(&path);
    let started_at = manifest.started_at.or_else(|| {
        metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
    });
    let project = project_label(&session_dir, manifest.cwd.as_deref());
    Some(SessionRef {
        source: SOURCE_CLINE,
        declared_source: None,
        path,
        project,
        cwd: manifest.cwd,
        started_at,
        size_bytes: metadata.len(),
        // One document is one session. A subagent session is its own
        // directory with an `origin.parentThreadId` back-reference that this
        // adapter does not follow, so there is no group to describe.
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for ClineSource {
    fn name(&self) -> &'static str {
        SOURCE_CLINE
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(sessions);
        };
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            // `file_type` does not follow, so a symlinked session directory
            // is not descended into.
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {}
                Ok(_) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
            let Some(messages) = messages_file_for(&entry.path()) else {
                continue;
            };
            match std::fs::symlink_metadata(&messages) {
                Ok(m) if m.is_file() => {}
                _ => continue,
            }
            match session_ref_for(messages) {
                Some(r) => sessions.push(r),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped unreadable cline session entries during discovery");
        }
        Ok(sessions)
    }

    /// A changed messages file is its own session, on exactly the terms
    /// `discover` uses: `<root>/<id>/<id>.messages.json`, two components
    /// deep. The manifest is deliberately not mapped: it changing does not
    /// change the bytes the transcript hashes.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        let session_dir = path.parent()?;
        if session_dir.parent() != Some(self.root.as_path()) {
            return None;
        }
        (messages_file_for(session_dir)? == path).then_some(path)
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        Ok(session_ref_for(address))
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path)
    }
}

fn timestamp_rfc3339(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// `ts` is milliseconds since the epoch, as `Date.now()` writes it.
fn timestamp_millis(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_i64())
        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
}

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    // Task 3 replaces this body. Until then a load is a refusal, so the
    // discovery tests can run against a compiling adapter.
    let _ = path;
    Err(anyhow!("malformed_cline_session"))
}
```

Check `real_file_within_root`'s exact signature in `source/mod.rs` before using it; match how `gemini_cli.rs:203` calls it. Check whether `session_at` is a trait method with a default in `source/mod.rs`; Gemini implements it, so do the same.

- [x] **Step 4: Run tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib source::cline`
Expected: 5 passed.

- [x] **Step 5: fmt, clippy, commit**

```bash
cargo fmt --all
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
git add crates/trace-commons-contributor/src/source/cline.rs crates/trace-commons-contributor/src/source/mod.rs
git commit -m "Discover Cline sessions under the store its own paths.ts names"
```

---

### Task 3: Loading a session into events

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/cline.rs` (replace `load_session`, add block mapping and tests)

**Interfaces:**
- Produces: `SessionTranscript` with `source = "cline"`, `conversation_id = sessionId`, events in document order, `model` from the first `modelInfo.id` or the manifest, `cwd`/`project` from the manifest, `started_at` from the manifest or the first message `ts`.

- [x] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    fn load(name: &str) -> SessionTranscript {
        let s = source();
        let r = s
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains(name))
            .unwrap();
        s.load(&r).unwrap()
    }

    #[test]
    fn transcript_fields_come_from_the_document_and_its_manifest() {
        let t = load("k3x9q");
        assert_eq!(t.source, SOURCE_CLINE);
        assert_eq!(t.conversation_id.as_deref(), Some("1756900000000_k3x9q"));
        assert_eq!(t.model.as_deref(), Some("claude-sonnet-5"));
        assert_eq!(t.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(t.project.as_deref(), Some("alpha"));
        assert_eq!(
            t.started_at.map(|ts| ts.to_rfc3339()),
            Some("2026-09-03T11:20:00+00:00".to_string()),
            "the manifest's start, not the first message"
        );
        assert_eq!(t.agent_version, None);
        assert_eq!(t.subagent_count, 0);
        assert!(t.routing.is_empty());
        let bytes = std::fs::read(
            fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.messages.json"),
        )
        .unwrap();
        assert_eq!(t.session_hash, crate::source::session_hash(&bytes));
    }

    #[test]
    fn blocks_become_events_in_document_order() {
        let t = load("k3x9q");
        let kinds: Vec<&SessionEventKind> = t.events.iter().map(|e| &e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &SessionEventKind::User,
                &SessionEventKind::Reasoning,
                &SessionEventKind::Assistant,
                &SessionEventKind::ToolCall,
                &SessionEventKind::ToolResult,
                &SessionEventKind::Assistant,
            ]
        );
        let user = &t.events[0];
        assert_eq!(user.content.as_deref(), Some("List the files in src"));
        assert_eq!(
            user.timestamp.map(|ts| ts.to_rfc3339()),
            Some("2026-09-03T11:06:40+00:00".to_string()),
            "ts is epoch milliseconds"
        );
        let reasoning = &t.events[1];
        assert_eq!(
            reasoning.content.as_deref(),
            Some("The user wants a directory listing.")
        );
        let assistant = &t.events[2];
        assert_eq!(assistant.content.as_deref(), Some("I'll list the directory."));
        assert_eq!(assistant.token_counts, Some((1200, 85)));
        assert_eq!(
            assistant.served_by, None,
            "Cline does not split cache writes by duration, so the step is unpriced rather than underpriced"
        );
        let call = &t.events[3];
        assert_eq!(call.tool_name.as_deref(), Some("list_files"));
        assert_eq!(call.tool_call_id.as_deref(), Some("toolu_01"));
        assert_eq!(call.structured, serde_json::json!({ "path": "src", "recursive": false }));
        let result = &t.events[4];
        assert_eq!(result.tool_call_id.as_deref(), Some("toolu_01"));
        assert_eq!(result.content.as_deref(), Some("index.ts\nutil.ts"));
        assert_eq!(result.success, None, "no is_error field means no verdict, not success");
        let last = &t.events[5];
        assert_eq!(last.token_counts, Some((1300, 20)));
    }

    #[test]
    fn string_content_failed_results_and_unknown_blocks_are_handled() {
        let t = load("p2m7z");
        assert_eq!(t.model.as_deref(), Some("gpt-5.5"), "from modelInfo when there is no manifest");
        assert_eq!(t.cwd, None);
        assert_eq!(
            t.started_at.map(|ts| ts.to_rfc3339()),
            Some("2026-09-03T11:08:20+00:00".to_string()),
            "the first message's ts when there is no manifest"
        );
        let user = &t.events[0];
        assert_eq!(user.kind, SessionEventKind::User);
        assert_eq!(user.content.as_deref(), Some("Run the tests"));
        let call = &t.events[1];
        assert_eq!(call.kind, SessionEventKind::ToolCall);
        assert_eq!(call.tool_name.as_deref(), Some("execute_command"));
        assert_eq!(call.token_counts, None, "no metrics, no counts");
        let result = &t.events[2];
        assert_eq!(result.kind, SessionEventKind::ToolResult);
        assert_eq!(result.success, Some(false));
        assert_eq!(
            result.content.as_deref(),
            Some("error: no such command"),
            "text parts joined; the image part is dropped"
        );
        let image = &t.events[3];
        assert_eq!(image.kind, SessionEventKind::Opaque);
        assert_eq!(image.structured, serde_json::json!({ "record_type": "image" }));
        assert!(image.content.is_none());
        let unknown = &t.events[4];
        assert_eq!(unknown.kind, SessionEventKind::Opaque);
        assert_eq!(unknown.structured, serde_json::json!({ "record_type": "future_block" }));
        assert!(
            !t.events.iter().filter_map(|e| e.content.as_deref()).any(|c| c.contains("AAAA") || c.contains("BBBB")),
            "image data never reaches an event"
        );
    }

    #[test]
    fn a_document_with_no_messages_array_is_refused_with_a_label_only() {
        let s = source();
        let r = s
            .discover()
            .unwrap()
            .into_iter()
            .find(|r| r.path.to_string_lossy().contains("bad00"))
            .unwrap();
        let err = s.load(&r).unwrap_err().to_string();
        assert_eq!(err, "malformed_cline_session");
    }

    #[test]
    fn a_document_over_budget_is_declined_by_size_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("1756900300000_big00");
        std::fs::create_dir_all(&session).unwrap();
        let path = session.join("1756900300000_big00.messages.json");
        let mut body = String::from("{\"version\":1,\"sessionId\":\"1756900300000_big00\",\"messages\":[{\"role\":\"user\",\"content\":\"");
        body.push_str(&"x".repeat(CLINE_SESSION_BUDGET as usize + 16));
        body.push_str("\"}]}");
        std::fs::write(&path, body).unwrap();
        let s = ClineSource::new(dir.path().to_path_buf());
        let r = s.discover().unwrap().into_iter().next().unwrap();
        let err = s.load(&r).unwrap_err();
        let too_large = err.downcast_ref::<crate::source::SessionTooLarge>().expect("a size refusal");
        assert_eq!(too_large.label, "cline-session-too-large");
    }
```

Confirm `tempfile` is already a dev-dependency of the contributor crate (`grep tempfile crates/trace-commons-contributor/Cargo.toml`). If it is not, use `std::env::temp_dir()` with a unique subdirectory instead; do not add a dependency.

- [x] **Step 2: Run tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib source::cline`
Expected: the five new tests fail (`malformed_cline_session` from the stub load, or the size test failing to downcast).

- [x] **Step 3: Implement**

Replace the stub `load_session` and add the helpers:

```rust
/// Text from a `content` field that may be a bare string or a block list.
/// Only `text` blocks contribute; everything else in the list is mapped as
/// its own event by the caller.
fn text_parts(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let joined = parts
                .iter()
                .filter(|p| p.get("type").and_then(|v| v.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            (!joined.is_empty()).then_some(joined)
        }
        _ => None,
    }
}

fn opaque(record_type: &str, timestamp: Option<chrono::DateTime<chrono::Utc>>) -> SessionEvent {
    SessionEvent {
        served_by: None,
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

/// `metrics.inputTokens` and `metrics.outputTokens`, both or neither.
fn token_counts_of(message: &Value) -> Option<(u32, u32)> {
    let metrics = message.get("metrics")?;
    let input = metrics.get("inputTokens")?.as_u64()?;
    let output = metrics.get("outputTokens")?.as_u64()?;
    Some((u32::try_from(input).ok()?, u32::try_from(output).ok()?))
}

/// One message expands to its blocks, in order. A bare-string `content` is
/// one text block. The message's `ts` stamps every block: the SDK records
/// one time per message, not per block.
fn map_message(message: &Value, model: &mut Option<String>, events: &mut Vec<SessionEvent>) {
    let timestamp = timestamp_millis(message.get("ts"));
    let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
    if model.is_none() {
        if let Some(id) = message
            .get("modelInfo")
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
        {
            *model = Some(id.to_string());
        }
    }
    let text_kind = match role {
        "user" => SessionEventKind::User,
        "assistant" => SessionEventKind::Assistant,
        other => {
            events.push(opaque(other, timestamp));
            return;
        }
    };
    // Token counts belong to the assistant's step. They are attached to the
    // first text block of the message, which is what `token_counts` means on
    // every other adapter: the provider's count for the step that produced
    // this text. A user message never carries them.
    let mut token_counts = (text_kind == SessionEventKind::Assistant)
        .then(|| token_counts_of(message))
        .flatten();

    let Some(content) = message.get("content") else {
        return;
    };
    let blocks: Vec<Value> = match content {
        Value::String(s) => vec![json!({ "type": "text", "text": s })],
        Value::Array(parts) => parts.clone(),
        _ => return,
    };

    for block in &blocks {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                let Some(text) = block.get("text").and_then(|v| v.as_str()) else {
                    continue;
                };
                if text.is_empty() {
                    continue;
                }
                events.push(SessionEvent {
                    served_by: None,
                    kind: text_kind.clone(),
                    timestamp,
                    content: Some(text.to_string()),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: token_counts.take(),
                    tool_call_id: None,
                    success: None,
                });
            }
            "thinking" => {
                let Some(thinking) = block.get("thinking").and_then(|v| v.as_str()) else {
                    continue;
                };
                if thinking.is_empty() {
                    continue;
                }
                events.push(SessionEvent {
                    served_by: None,
                    kind: SessionEventKind::Reasoning,
                    timestamp,
                    content: Some(thinking.to_string()),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                    tool_call_id: None,
                    success: None,
                });
            }
            "tool_use" => events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::ToolCall,
                timestamp,
                content: None,
                structured: block.get("input").cloned().unwrap_or(Value::Null),
                tool_name: block
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                token_counts: None,
                tool_call_id: block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                success: None,
            }),
            "tool_result" => events.push(SessionEvent {
                served_by: None,
                kind: SessionEventKind::ToolResult,
                timestamp,
                content: block.get("content").and_then(text_parts),
                structured: Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: block
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                // Only an explicit `is_error` is a verdict. Absent means the
                // harness said nothing, which is not success.
                success: block.get("is_error").and_then(|v| v.as_bool()).map(|e| !e),
            }),
            other => events.push(opaque(other, timestamp)),
        }
    }
}

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    // Declined rather than truncated, and named rather than silent. The size
    // is the contributor's own file's and safe to state; the path is not.
    let declared = std::fs::metadata(path)?.len();
    if declared > CLINE_SESSION_BUDGET {
        return Err(super::SessionTooLarge {
            label: "cline-session-too-large",
            declared_bytes: declared,
            budget_bytes: CLINE_SESSION_BUDGET,
        }
        .into());
    }
    let bytes = std::fs::read(path)?;
    let hash = session_hash(&bytes);
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("malformed_cline_session"))?;
    // The tolerance is for block *types*, not for the document. A file with
    // no `messages` array is not a session document at all.
    let messages = document
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("malformed_cline_session"))?;

    let manifest = read_manifest(path);
    let mut events = Vec::new();
    let mut model: Option<String> = None;
    for message in messages {
        map_message(message, &mut model, &mut events);
    }

    let session_dir = path.parent();
    let project = session_dir.and_then(|dir| project_label(dir, manifest.cwd.as_deref()));
    let started_at = manifest
        .started_at
        .or_else(|| messages.first().and_then(|m| timestamp_millis(m.get("ts"))));
    // The document's own id, which is what the store addresses it by; the
    // directory name merely repeats it.
    let conversation_id = document
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            session_dir
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        });

    Ok(SessionTranscript {
        source: Cow::Borrowed(SOURCE_CLINE),
        // The manifest carries no extension version.
        agent_version: None,
        model: model.or(manifest.model),
        project,
        cwd: manifest.cwd,
        started_at,
        session_hash: hash,
        conversation_id,
        events,
        subagent_count: 0,
        subagents_dropped: 0,
        routing: Vec::new(),
    })
}
```

`SessionEventKind` must be `Clone` for `text_kind.clone()`; it derives `Clone` in `source/mod.rs`. If `ServedBy` or another field name differs from what is written here, match `source/mod.rs`, not this plan.

- [x] **Step 4: Run tests to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib source::cline`
Expected: 10 passed.

- [x] **Step 5: fmt, clippy, commit**

```bash
cargo fmt --all
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
git add crates/trace-commons-contributor/src/source/cline.rs
git commit -m "Load a Cline session into events"
```

---

### Task 4: Register the source, its settings key, and its discovery candidate

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:435-462` (`NATIVE_SOURCES`)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs:139` (field), `:561` (`source_settings_key`), `:707` (`apply_settings_object`), `:432` (`Default`)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs:2845-2876` (settings view)
- Modify: `crates/trace-commons-contributor/src/source/discovery.rs:83-118` (`probe`)
- Modify: `crates/trace-commons-contributor-ffi/tests/abi.rs:1845-1855`

**Interfaces:**
- Produces: settings key `cline_source` (same tri-state JSON as `gemini_source`), IPC field `cline_source_mode`, discovery candidate `source: "cline"` appended fourth.

- [x] **Step 1: Write the failing tests**

In `settings.rs` tests, next to `the_gemini_declaration_is_settable_and_type_checked`:

```rust
    #[test]
    fn the_cline_declaration_is_settable_and_type_checked() {
        let mut s = DaemonSettings::default();
        assert_eq!(s.cline_source, None, "never asked, and undeclared constructs nothing");
        apply_settings_object(&mut s, &serde_json::json!({"cline_source": {"mode": "off"}}))
            .unwrap();
        assert_eq!(s.cline_source, Some(SourceDeclaration::Off));
        apply_settings_object(
            &mut s,
            &serde_json::json!({"cline_source": {"mode": "watch", "path": "/declared/cline"}}),
        )
        .unwrap();
        assert_eq!(
            s.cline_source,
            Some(SourceDeclaration::Watch {
                path: PathBuf::from("/declared/cline")
            })
        );
        assert!(
            apply_settings_object(&mut s, &serde_json::json!({"cline_source": "/a/path"})).is_err(),
            "a bare path is not a declaration"
        );
        assert_eq!(source_settings_key(crate::source::SOURCE_CLINE), Some("cline_source"));
    }

    #[test]
    fn a_settings_file_written_before_cline_existed_loads_with_it_absent() {
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v.as_object_mut().unwrap().remove("cline_source");
        let loaded: DaemonSettings = serde_json::from_value(v).unwrap();
        assert_eq!(loaded.cline_source, None);
    }
```

Copy the exact call shapes from the neighbouring Gemini tests (`apply_settings_object`'s signature and how `PathBuf` is imported there) rather than trusting this snippet.

In `source/mod.rs` tests, add:

```rust
    #[test]
    fn an_undeclared_cline_source_constructs_nothing() {
        let roots = SourceRoots::new();
        let names: Vec<&str> = all_sources(&roots).iter().map(|s| s.name()).collect();
        assert!(!names.contains(&SOURCE_CLINE), "{names:?}");
        let roots = roots.declare(
            SOURCE_CLINE,
            Some(SourceDeclaration::Watch {
                path: PathBuf::from("/declared/cline"),
            }),
        );
        let names: Vec<&str> = all_sources(&roots).iter().map(|s| s.name()).collect();
        assert!(names.contains(&SOURCE_CLINE), "{names:?}");
    }
```

Check how existing tests in that module construct `SourceRoots` and import `SourceDeclaration`; match them.

In `ipc.rs`, extend the existing settings-view test near line 3647 (the one asserting `gemini_source_mode`) with:

```rust
        assert_eq!(v["cline_source_mode"], "unset");
        assert!(v.get("cline_source").is_none());
```

In `discovery.rs` tests, add:

```rust
    #[test]
    fn probes_the_cline_store_fourth_and_counts_its_sessions() {
        let home = Scratch::new("cline");
        let session = home.path().join(".cline/data/sessions/1756900000000_k3x9q");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("1756900000000_k3x9q.messages.json"), "{}").unwrap();
        std::fs::write(session.join("1756900000000_k3x9q.json"), "{}").unwrap();
        let found = probe(home.path(), |_| None);
        assert_eq!(found[3].source, SOURCE_CLINE, "appended, so older shells indexing by position are unaffected");
        assert_eq!(found[3].path, home.path().join(".cline/data/sessions"));
        assert!(found[3].exists);
        assert_eq!(found[3].session_count, 1, "the manifest is not a session");
        assert!(!found[3].relocated_by_env);
    }

    #[test]
    fn a_cline_environment_variable_relocates_the_store_and_says_so() {
        let home = Scratch::new("cline-env");
        let found = probe(home.path(), |k| {
            (k == CLINE_SESSION_DATA_DIR_ENV).then(|| "/elsewhere".to_string())
        });
        assert_eq!(found[3].path, PathBuf::from("/elsewhere"));
        assert!(found[3].relocated_by_env);
    }
```

Read how `describe` counts sessions (it counts files by extension under the path, recursively). A manifest is also `.json`, so `session_count` would be 2 with a `JSON_EXTENSION` count. Look at `describe`'s signature: if it takes only an extension, add a `MESSAGES_EXTENSION: &str = "messages.json"` constant and make the counting match on `file_name().ends_with(".messages.json")` for this candidate. Read `describe` and its counting helper in full before deciding; the test above pins `session_count == 1` and that is the requirement.

In `abi.rs:1845-1855`, change `assert_eq!(items.len(), 3, ...)` to `4` and the expected vector to `vec!["claude-code", "codex", "gemini-cli", "cline"]`.

- [x] **Step 2: Run tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib cline`
Expected: compile errors (`cline_source` field, `SOURCE_CLINE` in discovery).

- [x] **Step 3: Implement**

`source/mod.rs`, append to `NATIVE_SOURCES`:

```rust
    SourceSpec {
        name: SOURCE_CLINE,
        conventional_root: cline::conventional_root_this_machine,
        build: |path| Box::new(cline::ClineSource::new(path)),
        // Same reasoning as Gemini: every shipped shell declares claude and
        // codex and carries no cline field, so an absent declaration must
        // construct nothing rather than watch the contributor's real
        // `~/.cline`.
        undeclared: Undeclared::Nothing,
    },
```

`settings.rs`: add after `gemini_source`:

```rust
    /// Added after every desktop client had shipped; absent means "no cline
    /// adapter", never the conventional `~/.cline` -- see [`gemini_source`]
    /// for the reasoning, which is identical.
    #[serde(default)]
    pub cline_source: Option<SourceDeclaration>,
```

Add `cline_source: None,` to `Default`. Add `crate::source::SOURCE_CLINE => Some("cline_source"),` to `source_settings_key`. Add the arm `"cline_source" => { settings.cline_source = parse_source_declaration(value)?; }` beside the Gemini arm. Find where `gemini_source` is passed to `SourceRoots::declare` (line 519) and add `.declare(crate::source::SOURCE_CLINE, self.cline_source.clone())` beside it.

`ipc.rs` settings view: add `let cline_mode = mode_of(&s.cline_source);`, `obj.remove("cline_source");`, and

```rust
        obj.insert(
            "cline_source_mode".to_string(),
            serde_json::Value::String(cline_mode.to_string()),
        );
```

`discovery.rs`: import `super::cline::{CLINE_SESSION_DATA_DIR_ENV, CLINE_DATA_DIR_ENV, CLINE_DIR_ENV, conventional_root as cline_root}` (alias to avoid clashing with the Gemini import) and `SOURCE_CLINE`; compute

```rust
    let cline_relocated = [CLINE_SESSION_DATA_DIR_ENV, CLINE_DATA_DIR_ENV, CLINE_DIR_ENV]
        .iter()
        .any(|key| env(key).is_some_and(|v| !v.trim().is_empty()));
```

and append a fourth `describe(SOURCE_CLINE, cline_root(home, &env), cline_relocated, <messages-file matcher>)` with the comment "Appended: shells index the first rows by position." Adjust `describe`'s counting as decided in Step 1.

`abi.rs`: the two edits from Step 1.

- [x] **Step 4: Run tests to verify they pass**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test abi
```
Expected: all pass. Report the two summary lines verbatim.

- [x] **Step 5: Full crate verification, fmt, clippy, commit**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo fmt --all
git add crates/trace-commons-contributor/src/source/mod.rs crates/trace-commons-contributor/src/source/discovery.rs crates/trace-commons-contributor/src/daemon/settings.rs crates/trace-commons-contributor/src/daemon/ipc.rs crates/trace-commons-contributor-ffi/tests/abi.rs
git commit -m "Offer Cline as a declarable session source"
```

`cargo fmt --all` on this repo rewrites whole files that were not rustfmt-clean; run `git show --stat HEAD` after each commit and if a file you did not intend to touch appears, `git checkout HEAD~1 -- <file>` and amend. Note anything of the kind in the report.

---

### Out of scope, to state in the report

- No settings row in GTK, macOS or Windows shells. `SourceTool` in `source_copy.rs` still has three variants. A shell that lists discovery candidates generically will show the fourth; one that hard-codes three will not. Say which is the case for GTK after a grep of `crates/trace-commons-contributor-gtk/src` for `SOURCE_GEMINI_CLI` or `gemini`.
- The pre-SDK `tasks/<id>/api_conversation_history.json` layout is not read.
- Subagent sessions (`origin.parentThreadId`) ship standalone.
- Nothing has been run against a real Cline install.
