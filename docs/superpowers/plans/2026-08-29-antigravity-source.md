> **SUPERSEDED on 2026-08-31** by the `import-antigravity` command.
>
> This plan implements the abandoned file-reading approach: it decodes
> Antigravity's on-disk SQLite conversation store and registers a watched
> source. The design it implements
> (`docs/superpowers/specs/2026-08-29-antigravity-source-design.md`) carries
> the full account of why that approach was dropped -- no ordering signal
> for user turns, and a prompt blob that could not be separated from vendor
> system text and file contents. The shipped command reads the running
> IDE's local API instead, and none of the `rusqlite`/`prost` work below is
> in the tree.
>
> Kept as the written record of what was built and measured.

# Antigravity Trace Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collect Google Antigravity IDE conversations as a watched contributor trace source, so contributors who use Antigravity can contribute traces through the existing queue, preview, redaction and approval path.

**Architecture:** Antigravity stores each conversation as an unencrypted SQLite database of protobuf step payloads at `~/.gemini/antigravity-ide/conversations/<uuid>.db`. A new `source/antigravity/` module reads them behind the existing `TraceSource` trait and registers as one `SourceSpec` row. Four internal units — `decode` (protobuf wire walking), `store` (SQLite snapshot and read), `convert` (events and refusals), and the composing `AntigravitySource` — are built bottom-up so each is testable alone.

**Tech Stack:** Rust 2024, `rusqlite` 0.40 (`bundled`), `prost` 0.13 (wire-format helpers only, no generated code), existing `TraceSource` / `SourceRoots` / `discovery::probe` machinery.

**Spec:** `docs/superpowers/specs/2026-08-29-antigravity-source-design.md`

## Global Constraints

- Verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run`. Plain `cargo check` does not apply `-D warnings`; CI does.
- Run `cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- Run `cargo fmt --all` before every commit. The repo is not rustfmt-clean, so check `git show --stat` afterwards — a formatting hook can turn a one-line edit into a whole-file diff.
- No emojis in commits, PRs, code or comments. Commit subjects are short and imperative with no `feat:` / `fix:` prefix.
- Error strings are fixed, content-free reason labels. Never a path, a prompt, a tool argument, or any session content.
- Fail closed: a source that cannot be read is refused, never silently degraded.
- No server or protocol changes. `source` reaches the envelope as a free string.
- `rusqlite` and `prost` are already in `crates/trace-commons-contributor/Cargo.toml` (commit d269ff60). Add no further dependencies without explicit approval.
- The precedent to copy throughout is `source/gemini_cli.rs` and its registration — it is the most recently added source and uses every seam this one needs.

---

### Task 1: Record the wire format and commit the fixture

The investigation this task originally called for has been done; its result is recorded in the spec. User turns are **not** in `steps` — they are in `gen_metadata`, inside the serialized model input, wrapped as `<USER_REQUEST>\n...\n</USER_REQUEST>`. This task turns that finding into the two artifacts every later task consumes.

**Files:**
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/README.md`
- Create: `crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db`
- Create: `docs/superpowers/notes/2026-08-29-antigravity-wire-format.md`

**Interfaces:**
- Consumes: nothing.
- Produces: the fixture path later tasks read, and a wire-format note recording every `step_type` observed and every payload field number, so Tasks 2 and 4 name constants instead of bare numbers.

- [ ] **Step 1: Copy the capture**

The capture already exists on this machine. Do not create a new conversation.

```bash
mkdir -p crates/trace-commons-contributor/tests/fixtures/antigravity
cp ~/.gemini/antigravity-ide/conversations/39f32a85-508b-430a-98fb-a67e89b4e689.db \
   crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db
```

- [ ] **Step 2: Redact the prompt blob before committing anything**

`gen_metadata` row `idx=9` is 258,595 bytes holding the vendor's system identity prompt, every tool's JSON schema, the skills and plugins listing, and the contents of every file the agent read. **This repository is public.** Committing that verbatim would publish a third party's system prompt and inject unrelated file contents into a test asset.

Replace that one row's data with a truncated blob that preserves the protobuf framing and the `<USER_REQUEST>` span and drops the rest. Every other table and row stays byte-identical.

```python
import sqlite3
db = sqlite3.connect("crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db")
row = db.execute("select data from gen_metadata where idx=9").fetchone()[0]
start = row.index(b"<USER_REQUEST>")
end = row.index(b"</USER_REQUEST>") + len(b"</USER_REQUEST>")
# Keep a small run of bytes either side so the wrapper is found in situ,
# not at a synthetic offset, and add one marker the privacy test asserts
# never escapes.
kept = row[max(0, start - 64):end + 64].replace(b"\x00", b" ")
db.execute(
    "update gen_metadata set data = ?, size = ? where idx = 9",
    (b"SENTINEL-MUST-NOT-ESCAPE " + kept, len(kept) + 25),
)
db.commit()
db.execute("VACUUM")
```

- [ ] **Step 3: Verify nothing unwanted survived**

```bash
strings crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db | grep -c 'You are Antigravity'
strings crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db | grep -c 'USER_REQUEST'
sqlite3 crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db "select idx, length(data) from gen_metadata order by idx;"
```

Expected: `0` for the system prompt, non-zero for the wrapper, and row 9 now a few hundred bytes rather than 258 KB. If the system-prompt count is not zero, stop — do not commit.

- [ ] **Step 4: Write the wire-format note**

Create `docs/superpowers/notes/2026-08-29-antigravity-wire-format.md` with three tables, each row citing the step index it was observed on:

1. **Step types.** Observed in the capture: `8` and `9` (tool result / tool execution, both carrying the tool-call submessage), `14` (conversation start; ids and a status timeline only), `15` (model turn — carries the tool call, and on the final turn the assistant text and reasoning), `23` (title, field 4), `98` and `99` (small, ids only).
2. **Step payload fields.** Field `1` is the step type; field `5` is the body. Within the body: `4` is the tool-call submessage (`1` = call id such as `call_304828`, `2` = tool name such as `list_dir`, `3` = arguments as a JSON string, `9` = tool name again), `5` is the repeated argument-key list, `1`/`6`/`7`/`8` are timestamps. On an assistant turn the body carries field `1` (assistant text), `3` (reasoning) and `8` (rendered text).
3. **Metadata blob fields** (`trajectory_metadata_blob.data`): `1` and `2` are the workspace URI, `3` is a submessage with `1` = `owner/repo` and `2` = remote URL, `4` is the branch.

State plainly that field numbers were derived from one capture on one Antigravity build, that they are a vendor's unpublished schema, and that the reader skips unknown fields precisely because they will change.

- [ ] **Step 5: Write the fixture README**

Record: which Antigravity build produced it, the platform, that it is a real capture rather than an authored file, and — explicitly — that `gen_metadata` row 9 was truncated to strip the vendor system prompt and injected file contents, with every other byte left as captured. A future reader must not mistake the truncation for the real format.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/tests/fixtures/antigravity/ docs/superpowers/notes/2026-08-29-antigravity-wire-format.md
git commit -m "Record the Antigravity conversation wire format from a capture"
```

---

### Task 2: Decode step payloads

**Files:**
- Create: `crates/trace-commons-contributor/src/source/antigravity/mod.rs`
- Create: `crates/trace-commons-contributor/src/source/antigravity/decode.rs`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (add `pub mod antigravity;` beside the existing `pub mod` lines)

**Interfaces:**
- Consumes: the field numbers recorded in Task 1's wire-format note.
- Produces:

```rust
pub(crate) struct DecodedStep {
    pub step_type: u32,
    pub timestamp: Option<chrono::DateTime<chrono::Utc>>,
    pub text: Option<String>,
    pub reasoning: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_args_json: Option<String>,
}

pub(crate) fn decode_step(step_type: u32, payload: &[u8]) -> anyhow::Result<DecodedStep>;

/// Workspace URI, git remote, git branch — read from
/// `trajectory_metadata_blob`, which is protobuf rather than columns.
pub(crate) fn decode_metadata(
    bytes: &[u8],
) -> anyhow::Result<(Option<String>, Option<String>, Option<String>)>;

/// The contributor's own turn, and NOTHING else, out of a `gen_metadata`
/// row.
///
/// That row carries the whole assembled model input: the vendor's system
/// prompt, every tool schema, and the contents of every file the agent
/// read. Only the `<USER_REQUEST>` span may leave this function. Returns
/// `Ok(None)` for a row with no wrapper at all (the small generation-config
/// rows), and `Err("antigravity-user-turn-unreadable")` for a row that has
/// an opening wrapper it cannot close — a renamed or truncated wrapper must
/// fail loudly rather than yield a session with no human turn.
pub(crate) fn extract_user_request(bytes: &[u8]) -> anyhow::Result<Option<String>>;
```

- [ ] **Step 1: Write the failing test**

In `decode.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// An unrecognised field number must be skipped, not fail the parse.
    ///
    /// This is the whole reason the reader walks the wire format instead of
    /// generating from a reverse-engineered schema: Antigravity adds fields
    /// without notice, and a parse failure would discard the session.
    #[test]
    fn an_unknown_field_is_skipped_rather_than_fatal() {
        // field 4095, wire type 2 (length-delimited), three bytes of body.
        let mut payload = Vec::new();
        prost::encoding::encode_key(4095, prost::encoding::WireType::LengthDelimited, &mut payload);
        prost::encoding::encode_varint(3, &mut payload);
        payload.extend_from_slice(b"abc");

        let decoded = decode_step(15, &payload).expect("unknown fields must not be fatal");
        assert_eq!(decoded.step_type, 15);
        assert_eq!(decoded.text, None);
    }
}
```

- [ ] **Step 2: Run it and confirm it fails**

Run: `cargo test -p trace-commons-contributor decode::tests::an_unknown_field -- --nocapture`
Expected: FAIL — `decode_step` does not exist.

- [ ] **Step 3: Implement the walker**

`Buf` comes from `prost::bytes`, which prost re-exports — do not add a
direct `bytes` dependency for it.

```rust
use prost::bytes::Buf;
use prost::encoding::{DecodeContext, WireType, decode_key, skip_field};

pub(crate) fn decode_step(step_type: u32, mut payload: &[u8]) -> anyhow::Result<DecodedStep> {
    let mut out = DecodedStep::new(step_type);
    let buf = &mut payload;
    while buf.has_remaining() {
        let (field, wire) = decode_key(buf).map_err(|_| anyhow::anyhow!("antigravity-malformed-step"))?;
        match (field, wire) {
            // Field numbers come from the Task 1 wire-format note; each arm
            // reads one known field and every other field falls through to
            // skip_field below.
            (FIELD_BODY, WireType::LengthDelimited) => read_body(buf, &mut out)?,
            _ => skip_field(wire, field, buf, DecodeContext::default())
                .map_err(|_| anyhow::anyhow!("antigravity-malformed-step"))?,
        }
    }
    Ok(out)
}
```

Define one `const FIELD_*: u32` per field the note records, beside this function, each with a comment naming the evidence. Do not inline bare numbers.

- [ ] **Step 4: Run the test and confirm it passes**

Run: `cargo test -p trace-commons-contributor decode::`
Expected: PASS.

- [ ] **Step 5: Add the fixture-backed test**

```rust
#[test]
fn the_captured_conversation_decodes_to_recognisable_steps() {
    let conv = super::store::read_conversation(&fixture_path()).expect("fixture must read");
    let decoded: Vec<_> = conv
        .steps
        .iter()
        .map(|s| decode_step(s.step_type, &s.payload).expect("fixture must decode"))
        .collect();
    assert!(
        decoded.iter().any(|d| d.tool_name.is_some()),
        "the capture contains tool calls; the decoder must find them"
    );
    assert!(
        decoded.iter().any(|d| d.text.is_some()),
        "the capture contains assistant text; the decoder must find it"
    );
}
```

This test depends on Task 3's reader. Write it now and leave it failing to compile only if Task 3 has not landed; otherwise run it here.

- [ ] **Step 6: Add the user-request extractor and its tests**

```rust
const OPEN: &[u8] = b"<USER_REQUEST>";
const CLOSE: &[u8] = b"</USER_REQUEST>";

#[test]
fn only_the_tagged_span_escapes_the_prompt_blob() {
    let blob = b"SENTINEL-MUST-NOT-ESCAPE ...<USER_REQUEST>\nTell me about this repo\n</USER_REQUEST>... more";
    let got = extract_user_request(blob).unwrap().unwrap();
    assert_eq!(got, "Tell me about this repo");
    assert!(!got.contains("SENTINEL"), "nothing outside the wrapper may escape");
}

#[test]
fn a_row_with_no_wrapper_is_not_a_user_turn_and_not_an_error() {
    assert!(extract_user_request(b"generation config, no wrapper").unwrap().is_none());
}

#[test]
fn an_unclosed_wrapper_is_refused_rather_than_silently_empty() {
    let err = extract_user_request(b"x<USER_REQUEST>\nhello").unwrap_err().to_string();
    assert_eq!(err, "antigravity-user-turn-unreadable");
}
```

**Search backwards from the close tag. Searching forwards from the first
`OPEN` is a privacy bug, not a style preference.** Task 1's wire-format note
established why: the vendor system prompt *describes* the `<USER_REQUEST>`
tag in prose before any wrapped turn appears, and repeats that description
later. A naive `index(OPEN)` finds the description, and pairing it with a
later `CLOSE` returns everything in between — in the real capture, the
entire system prompt and tool-schema listing. That is precisely the material
this function exists to keep out of a trace.

Correct algorithm:

1. Find `CLOSE`. If absent and `OPEN` is also absent, return `Ok(None)`.
   If absent while `OPEN` is present, return the error label.
2. Take the **last** `OPEN` at or before that `CLOSE`.
3. Return the bytes between, trimmed, as lossy UTF-8.
4. Repeat from after `CLOSE` if further pairs follow, so a multi-turn
   conversation yields every turn.

Never return, log, or retain any other part of the input.

Add this test, which fails under the naive algorithm and passes under the
correct one:

```rust
#[test]
fn a_tag_described_in_surrounding_prose_is_not_mistaken_for_the_turn() {
    // Shape taken from the real capture: the system prompt explains the
    // <USER_REQUEST> tag long before the wrapped turn appears.
    let blob = b"The user's request appears inside <USER_REQUEST> tags. \
                 SENTINEL-MUST-NOT-ESCAPE lots more system prompt here. \
                 <USER_REQUEST>\nTell me about this repo\n</USER_REQUEST>";
    let got = extract_user_request(blob).unwrap().unwrap();
    assert_eq!(got, "Tell me about this repo");
    assert!(!got.contains("SENTINEL"), "prose between the description and the real turn must not escape");
}
```

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/antigravity/ crates/trace-commons-contributor/src/source/mod.rs
git commit -m "Decode Antigravity step payloads by walking the wire format"
```

---

### Task 3: Read the conversation database

**Files:**
- Create: `crates/trace-commons-contributor/src/source/antigravity/store.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:

```rust
pub(crate) struct RawStep { pub idx: i64, pub step_type: u32, pub status: i64, pub payload: Vec<u8> }
pub(crate) struct RawConversation {
    pub trajectory_id: Option<String>,
    pub workspace_uri: Option<String>,
    pub git_remote: Option<String>,
    pub git_branch: Option<String>,
    pub steps: Vec<RawStep>,
    /// User turns, extracted span-only from `gen_metadata` in row order.
    /// The rows themselves are read, scanned and dropped — no other part of
    /// them is retained anywhere in this struct.
    pub user_requests: Vec<String>,
}
pub(crate) fn read_conversation(db: &Path) -> anyhow::Result<RawConversation>;
```

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn reading_the_fixture_yields_its_steps_in_order() {
    let conv = read_conversation(&fixture_path()).expect("fixture must read");
    assert!(!conv.steps.is_empty(), "the capture has steps");
    let indices: Vec<i64> = conv.steps.iter().map(|s| s.idx).collect();
    let mut sorted = indices.clone();
    sorted.sort_unstable();
    assert_eq!(indices, sorted, "steps must be returned in idx order");
}

#[test]
fn a_file_that_is_not_sqlite_is_refused_without_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not-a-db.db");
    std::fs::write(&path, b"this is not a database").unwrap();
    let err = read_conversation(&path).unwrap_err().to_string();
    assert_eq!(err, "antigravity-unreadable");
    assert!(!err.contains("not-a-db"), "a reason label must not carry the path");
}
```

- [ ] **Step 2: Run and confirm both fail**

Run: `cargo test -p trace-commons-contributor antigravity::store`
Expected: FAIL — `read_conversation` does not exist.

- [ ] **Step 3: Implement the snapshot-and-read**

Copy the database and any `-wal` / `-shm` sidecars into a `tempfile::TempDir`, then open the copy with `OpenFlags::SQLITE_OPEN_READ_ONLY`. The daemon must never write to a contributor's live Antigravity store, and read-only WAL access against the original would still want to touch the shared-memory file.

```rust
pub(crate) fn read_conversation(db: &Path) -> anyhow::Result<RawConversation> {
    let scratch = tempfile::tempdir().map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
    let copy = scratch.path().join("conversation.db");
    std::fs::copy(db, &copy).map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{}", db.display(), suffix));
        if side.exists() {
            let dest = PathBuf::from(format!("{}{}", copy.display(), suffix));
            std::fs::copy(&side, dest).map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
        }
    }
    let conn = rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;

    let mut stmt = conn
        .prepare("SELECT idx, step_type, status, step_payload FROM steps ORDER BY idx")
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
    let steps = stmt
        .query_map([], |row| {
            Ok(RawStep {
                idx: row.get(0)?,
                step_type: row.get::<_, i64>(1)? as u32,
                status: row.get(2)?,
                payload: row.get(3)?,
            })
        })
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;

    // trajectory_meta gives the id; trajectory_metadata_blob is a protobuf
    // holding the workspace URI, git remote and branch, so it goes through
    // decode rather than a column read. Both are optional: a conversation
    // missing them is still a conversation.
    let trajectory_id = conn
        .query_row("SELECT trajectory_id FROM trajectory_meta LIMIT 1", [], |r| r.get(0))
        .ok();
    let meta_blob: Option<Vec<u8>> = conn
        .query_row("SELECT data FROM trajectory_metadata_blob LIMIT 1", [], |r| r.get(0))
        .ok();
    let (workspace_uri, git_remote, git_branch) = match meta_blob {
        Some(bytes) => super::decode::decode_metadata(&bytes)?,
        None => (None, None, None),
    };

    // gen_metadata rows carry the assembled model input. Each is scanned
    // for the user-turn span and then dropped; nothing else from them is
    // kept, hashed, or returned.
    let mut user_requests = Vec::new();
    let mut gen = conn
        .prepare("SELECT data FROM gen_metadata ORDER BY idx")
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
    let blobs = gen
        .query_map([], |row| row.get::<_, Vec<u8>>(0))
        .map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
    for blob in blobs {
        let blob = blob.map_err(|_| anyhow::anyhow!("antigravity-unreadable"))?;
        if let Some(turn) = super::decode::extract_user_request(&blob)? {
            user_requests.push(turn);
        }
    }

    Ok(RawConversation {
        trajectory_id,
        workspace_uri,
        git_remote,
        git_branch,
        steps,
        user_requests,
    })
}
```

Note the `?` on `extract_user_request`: an unclosed wrapper propagates
`antigravity-user-turn-unreadable` and refuses the whole conversation. That
is deliberate — see the spec.

Add this test alongside the two above:

```rust
#[test]
fn the_fixture_yields_its_user_turn_and_nothing_around_it() {
    let conv = read_conversation(&fixture_path()).expect("fixture must read");
    assert_eq!(conv.user_requests, vec!["Tell me about this repo".to_string()]);
    let joined = conv.user_requests.join(" ");
    assert!(
        !joined.contains("SENTINEL"),
        "only the tagged span may leave the prompt blob"
    );
}
```

`decode_metadata` is a second entry point on Task 2's walker with the
signature `pub(crate) fn decode_metadata(bytes: &[u8]) -> anyhow::Result<(Option<String>, Option<String>, Option<String>)>`,
reading the workspace URI, git remote and branch field numbers recorded in
Task 1's note. Add it in Task 2 alongside `decode_step`.

Every failure maps to `antigravity-unreadable`. A rusqlite error string can contain the file path, so it must never reach the label.

- [ ] **Step 4: Run and confirm both pass**

Run: `cargo test -p trace-commons-contributor antigravity::store`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/antigravity/store.rs
git commit -m "Read an Antigravity conversation from a snapshot of its database"
```

---

### Task 4: Convert steps to session events

**Files:**
- Create: `crates/trace-commons-contributor/src/source/antigravity/convert.rs`

**Interfaces:**
- Consumes: `RawConversation` (Task 3), `decode_step` / `DecodedStep` (Task 2), the `step_type` table from Task 1.
- Produces: `pub(crate) fn to_transcript(conv: RawConversation, session_hash: String) -> anyhow::Result<SessionTranscript>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn an_unknown_step_type_becomes_one_opaque_event_not_a_lost_session() {
    let conv = fixture_conversation_with_extra_step(60000);
    let t = to_transcript(conv, "sha256:aa".into()).expect("one unknown step must not refuse");
    assert_eq!(
        t.events.iter().filter(|e| e.kind == SessionEventKind::Opaque).count(),
        1
    );
}

#[test]
fn a_conversation_with_no_user_or_assistant_content_is_refused() {
    let conv = RawConversation { steps: vec![], ..fixture_conversation() };
    let err = to_transcript(conv, "sha256:aa".into()).unwrap_err().to_string();
    assert_eq!(err, "antigravity-no-content");
}

#[test]
fn tool_results_pair_with_their_calls_by_id() {
    let t = to_transcript(fixture_conversation(), "sha256:aa".into()).unwrap();
    let calls: Vec<_> = t.events.iter().filter(|e| e.kind == SessionEventKind::ToolCall).collect();
    for call in calls {
        let id = call.tool_call_id.as_ref().expect("a call carries its id");
        assert!(
            t.events.iter().any(|e| e.kind == SessionEventKind::ToolResult
                && e.tool_call_id.as_ref() == Some(id)),
            "every call in the capture is answered"
        );
    }
}
```

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor antigravity::convert`
Expected: FAIL — `to_transcript` does not exist.

- [ ] **Step 3: Implement the mapping**

Map each decoded step by `step_type` using the table from Task 1: tool-call types to `SessionEventKind::ToolCall` carrying `tool_call_id`, `tool_name` and `structured` (the arguments parsed as JSON, or `Value::Null` if they do not parse); result types to `ToolResult` with the same `tool_call_id`; assistant-text types to `Assistant`; reasoning to `Reasoning`; user turns to `User`. Any `step_type` not in the table becomes `Opaque` with `content: None`.

Emit one `User` event per entry in `conv.user_requests`, in order, before the step-derived events. They carry no timestamp of their own — the prompt blob records none — so leave `timestamp: None` rather than inventing one from a neighbouring step.

Set `source` to `Cow::Borrowed(SOURCE_ANTIGRAVITY)`, `conversation_id` from `trajectory_id`, `cwd` from the workspace URI with the `file://` scheme stripped, and `project` to that path's basename. Refuse with `antigravity-no-content` when no `User` or `Assistant` event was produced.

Add the privacy test, which is the one that keeps a vendor system prompt and injected file contents out of a contributed trace:

```rust
#[test]
fn nothing_from_the_prompt_blob_reaches_the_transcript() {
    let t = to_transcript(read_conversation(&fixture_path()).unwrap(), "sha256:aa".into()).unwrap();
    let everything = format!("{:?}", t.events);
    assert!(
        !everything.contains("SENTINEL"),
        "the fixture's gen_metadata carries a sentinel outside the wrapper; \
         no event may carry it"
    );
    assert!(
        t.events.iter().any(|e| e.kind == SessionEventKind::User),
        "the capture has a user turn and it must survive"
    );
}
```

- [ ] **Step 4: Run and confirm they pass**

Run: `cargo test -p trace-commons-contributor antigravity::convert`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/antigravity/convert.rs
git commit -m "Map Antigravity steps onto session events"
```

---

### Task 5: The content-derived session hash

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/antigravity/convert.rs`

**Interfaces:**
- Produces: `pub(crate) fn content_hash(conv: &RawConversation) -> String` returning `sha256:<hex>`.

- [ ] **Step 1: Write the failing test**

This is the test the deviation exists for. Every other adapter hashes raw file bytes; SQLite page reuse and WAL checkpointing move those bytes without changing a message, which would re-offer sessions already uploaded and defeat dedup.

```rust
#[test]
fn vacuuming_the_database_does_not_move_the_session_hash() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("c.db");
    std::fs::copy(fixture_path(), &copy).unwrap();

    let before = content_hash(&read_conversation(&copy).unwrap());
    rusqlite::Connection::open(&copy).unwrap().execute_batch("VACUUM;").unwrap();
    let after = content_hash(&read_conversation(&copy).unwrap());

    assert_eq!(before, after, "storage layout must not change the session id");
    assert_ne!(
        crate::source::session_hash(&std::fs::read(&copy).unwrap()),
        crate::source::session_hash(&std::fs::read(fixture_path()).unwrap()),
        "the raw bytes DID move; this is why the hash is content-derived"
    );
}
```

- [ ] **Step 2: Run and confirm it fails**

Run: `cargo test -p trace-commons-contributor vacuuming_the_database`
Expected: FAIL — `content_hash` does not exist.

- [ ] **Step 3: Implement the canonical serialization**

Feed a `SessionHasher` a domain separator, then for each step in `idx` order: the `idx`, the `step_type`, and the payload bytes, each length-prefixed so no two different step sequences can produce the same byte stream. Include `trajectory_id` and the workspace URI. Do not include file size, mtime, or page-level state.

```rust
pub(crate) fn content_hash(conv: &RawConversation) -> String {
    let mut hasher = crate::source::SessionHasher::new();
    hasher.update(b"trace-commons:antigravity:v1\0");
    for turn in &conv.user_requests {
        hasher.update(&(turn.len() as u64).to_le_bytes());
        hasher.update(turn.as_bytes());
    }
    for step in &conv.steps {
        hasher.update(&step.idx.to_le_bytes());
        hasher.update(&step.step_type.to_le_bytes());
        hasher.update(&(step.payload.len() as u64).to_le_bytes());
        hasher.update(&step.payload);
    }
    hasher.finish()
}
```

The user turns are part of the session's identity — two conversations with
identical agent steps and different prompts are different sessions — so
they are in the preimage. Only the extracted spans; the blob they came from
never reaches the hasher.

- [ ] **Step 4: Run and confirm it passes**

Run: `cargo test -p trace-commons-contributor vacuuming_the_database`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/antigravity/convert.rs
git commit -m "Key the Antigravity session hash to content, not storage layout"
```

---

### Task 6: The TraceSource implementation

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/antigravity/mod.rs`

**Interfaces:**
- Produces: `pub struct AntigravitySource` with `pub fn new(root: PathBuf) -> Self`, implementing `TraceSource`.

Mirror `source/gemini_cli.rs` throughout: one ref-construction function shared by `discover` and `session_at`, as the trait requires, and `real_file_within_root` for containment.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn discovery_finds_databases_and_nothing_else() {
    let root = Scratch::new("ag");
    std::fs::write(root.path().join("a.db"), b"x").unwrap();
    std::fs::write(root.path().join("a.db-wal"), b"x").unwrap();
    std::fs::write(root.path().join("legacy.pb"), b"x").unwrap();

    let found = AntigravitySource::new(root.path().into()).discover().unwrap();
    let names: Vec<_> = found.iter().filter_map(|r| r.path.file_name()).collect();
    assert_eq!(names, vec!["a.db"], "sidecars and legacy .pb are not sessions");
}

#[test]
fn the_ref_covers_the_sidecars_so_quiescence_is_judged_on_the_group() {
    let root = Scratch::new("ag");
    std::fs::write(root.path().join("a.db"), vec![0u8; 100]).unwrap();
    std::fs::write(root.path().join("a.db-wal"), vec![0u8; 400]).unwrap();

    let found = AntigravitySource::new(root.path().into()).discover().unwrap();
    assert_eq!(found[0].size_bytes, 500, "a session still writing its WAL is not settled");
}

#[test]
fn session_at_describes_a_session_exactly_as_discover_does() {
    let root = Scratch::new("ag");
    let db = root.path().join("a.db");
    std::fs::write(&db, vec![0u8; 100]).unwrap();
    let source = AntigravitySource::new(root.path().into());
    let from_scan = source.discover().unwrap().remove(0);
    let from_path = source.session_at(&db).unwrap().unwrap();
    assert_eq!(from_scan.path, from_path.path);
    assert_eq!(from_scan.size_bytes, from_path.size_bytes);
}

#[test]
fn a_path_outside_the_root_is_refused() {
    let root = Scratch::new("ag");
    let source = AntigravitySource::new(root.path().into());
    let escape = root.path().join("../elsewhere/a.db");
    assert_eq!(source.session_for_path(&escape), None);
}
```

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor antigravity::tests`
Expected: FAIL — `AntigravitySource` does not exist.

- [ ] **Step 3: Implement**

`discover` walks the root for `*.db` entries, skipping symlinks with `symlink_metadata` as the other adapters do. The shared ref builder sums the database and any `-wal` / `-shm` sizes into `size_bytes`, takes the newest mtime among them into `group_modified_at`, and leaves `group_member_count` at 0 — the sidecars are not separate transcripts. `load` calls `store::read_conversation`, then `convert::content_hash`, then `convert::to_transcript`.

- [ ] **Step 4: Run and confirm they pass**

Run: `cargo test -p trace-commons-contributor antigravity::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/antigravity/mod.rs
git commit -m "Read Antigravity conversations behind the TraceSource trait"
```

---

### Task 7: Register the source

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (name constant, `NATIVE_SOURCES` row)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (`antigravity_source` field, `source_settings_key` arm, `antigravity_declared`, `source_roots`, `apply_settings_object` arm)

**Interfaces:**
- Produces: `pub const SOURCE_ANTIGRAVITY: &str = "antigravity";` and `pub fn antigravity_declared(settings: &DaemonSettings) -> bool`.

- [ ] **Step 1: Write the failing test**

The property that matters is the one `Undeclared::Nothing` exists for. Copy the shape of the equivalent `gemini-cli` test in `source/mod.rs`.

```rust
#[test]
fn an_undeclared_antigravity_source_constructs_nothing() {
    let sources = all_sources(&SourceRoots::conventional());
    assert!(
        !sources.iter().any(|s| s.name() == SOURCE_ANTIGRAVITY),
        "every shipped desktop client carries no antigravity field; an absent \
         declaration must construct no adapter, least of all one rooted at \
         the contributor's real ~/.gemini"
    );
}

#[test]
fn a_declared_antigravity_source_is_constructed() {
    let sources = all_sources(
        &SourceRoots::new().declare(
            SOURCE_ANTIGRAVITY,
            Some(SourceDeclaration::Watch { path: "/declared/ag".into() }),
        ),
    );
    assert!(sources.iter().any(|s| s.name() == SOURCE_ANTIGRAVITY));
}
```

Check the exact constructor `SourceRoots` exposes before writing this — read the `gemini-cli` tests at the bottom of `source/mod.rs` and match them rather than assuming `conventional()`.

- [ ] **Step 2: Run and confirm they fail**

Run: `cargo test -p trace-commons-contributor source::tests::an_undeclared_antigravity`
Expected: FAIL — `SOURCE_ANTIGRAVITY` does not exist.

- [ ] **Step 3: Add the constant and the registration row**

```rust
pub const SOURCE_ANTIGRAVITY: &str = "antigravity";
```

Append to `NATIVE_SOURCES` — appended, not inserted, because a shell written before this source existed indexes earlier rows by position:

```rust
SourceSpec {
    name: SOURCE_ANTIGRAVITY,
    conventional_root: antigravity::conventional_root_this_machine,
    build: |path| Box::new(antigravity::AntigravitySource::new(path)),
    // Same reason as gemini-cli: every shipped desktop client declares
    // claude and codex and carries no antigravity field, so an absent
    // declaration must construct nothing rather than fall back to the
    // contributor's real ~/.gemini.
    undeclared: Undeclared::Nothing,
},
```

- [ ] **Step 4: Add the settings field and its predicate**

In `settings.rs`, add `#[serde(default)] pub antigravity_source: Option<SourceDeclaration>` with a comment matching the `gemini_source` one, add the `SOURCE_ANTIGRAVITY => Some("antigravity_source")` arm to `source_settings_key`, add `antigravity_declared`, extend `source_roots()` with a fourth `.declare(...)`, and add the `"antigravity_source"` arm to `apply_settings_object` — which rejects unknown keys, so without this arm a shell writing the field gets `settings-unknown-field`.

- [ ] **Step 5: Run the full crate tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS, including the pre-existing settings round-trip tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/mod.rs crates/trace-commons-contributor/src/daemon/settings.rs
git commit -m "Register Antigravity as a native source"
```

---

### Task 8: Describe the store on the roots screen

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/discovery.rs`
- Modify: `macos/Sources/TCShellCore/SourceCandidate.swift`
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/roots.rs`

**Interfaces:**
- Consumes: `SOURCE_ANTIGRAVITY` (Task 7).

- [ ] **Step 1: Write the failing discovery test**

```rust
#[test]
fn probes_the_antigravity_store_and_counts_its_databases() {
    let home = Scratch::new("antigravity");
    let convs = home.path().join(".gemini/antigravity-ide/conversations");
    std::fs::create_dir_all(&convs).unwrap();
    std::fs::write(convs.join("a.db"), b"x").unwrap();
    std::fs::write(convs.join("legacy.pb"), b"x").unwrap();

    let found = probe(home.path(), |_| None);
    let ag = found.iter().find(|c| c.source == SOURCE_ANTIGRAVITY).unwrap();
    assert!(ag.exists);
    assert_eq!(ag.session_count, 1, "legacy .pb conversations are not offered");
}
```

- [ ] **Step 2: Run and confirm it fails**

Run: `cargo test -p trace-commons-contributor discovery::tests::probes_the_antigravity`
Expected: FAIL — no antigravity candidate.

- [ ] **Step 3: Add the probe row**

Append a fourth `describe(...)` call using a `DB_EXTENSION` constant, rooted at `home.join(".gemini/antigravity-ide/conversations")`, with `relocated_by_env: false`. Append rather than insert, for the same positional reason as the registration row. Record beside it that whether `GEMINI_CLI_HOME` relocates this store is unverified, per the spec's open question.

- [ ] **Step 4: Add the shell display strings**

Swift: add `case antigravity = "antigravity"` to `SourceKind` and `"Antigravity"` to `displayName`. The enum already drops unrecognized slugs, so an older app against a newer daemon shows one fewer row rather than failing to render.

GTK: add the `ROOTS_ANTIGRAVITY` copy string and the `SOURCE_ANTIGRAVITY` arm in `ui/roots.rs` beside the existing three.

- [ ] **Step 5: Run every affected suite**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cd macos && swift test
```

`swift test` gates PRs through the `macOS app tests` job on `macos-26`, so a Swift failure here is a real CI failure. It needs `cargo build -p trace-commons-contributor-ffi` first, because the Swift package links that dylib.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/source/discovery.rs macos/ crates/trace-commons-contributor-gtk/
git commit -m "Offer the Antigravity store on the roots screen"
```

---

### Task 9: Final verification

- [ ] **Step 1: Run everything CI runs**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

Paste the actual output. Do not claim green without it.

- [ ] **Step 2: Confirm no session content reaches an error label**

```bash
grep -rn 'antigravity-' crates/trace-commons-contributor/src/source/antigravity/
```

Every label must be a fixed `&'static str`. Any `format!` that interpolates a path, a prompt or a tool argument is a defect.

- [ ] **Step 3: Confirm the fixture carries nothing it should not**

```bash
strings crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db | grep -i 'file:///\|github.com' | sort -u
```

Only the scratch repository may appear.

- [ ] **Step 4: Update the contributor README**

Add Antigravity to the sources table in `crates/trace-commons-contributor/README.md`, stating the store path, that only the current `.db` format is readable, and that legacy `.pb` conversations are encrypted and cannot be collected.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/README.md
git commit -m "Document Antigravity as a contributor source"
```
