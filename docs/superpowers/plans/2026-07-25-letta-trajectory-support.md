# Letta Trajectory Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the contributor CLI submit sessions from any coding harness that the Letta Trajectory v1 format covers, and capture model reasoning as a first-class, distinguishable event type across every adapter.

**Architecture:** A new `TrajectorySource` implements the existing `TraceSource` trait and reads trajectory-v1 files (JSON array or JSONL) that the contributor produced with `npx @letta-ai/trajectory`. Reasoning gains its own `SessionEventKind` and `TraceContributionEventType`, replacing the current drop-on-the-floor handling in the claude-code and codex adapters. `SessionTranscript.source` widens to `Cow<'static, str>` so the real harness name survives into the receipt and `feature_flags["agent"]`.

**Tech Stack:** Rust 2024 edition, `serde_json`, `chrono`, `clap` 4. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-25-letta-trajectory-support-design.md`

## Global Constraints

- **No new dependencies.** Everything needed (`serde`, `serde_json`, `chrono`, `clap`, `sha2`, `hex`, `uuid`, `dirs`, `anyhow`) is already in `crates/trace-commons-contributor/Cargo.toml`. Do not add any.
- **PostgreSQL-only repo.** No libsql feature flags, no dual-backend verification.
- **Warnings are errors in CI.** Verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --all-targets` — plain `cargo check` does not catch what CI catches.
- **Clippy is CI-enforced.** Run `cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- **Hash-only logging.** Never put file contents, absolute paths, tool arguments, or record text into an error string, a log line, or a `tracing` field. Errors carry a reason label only.
- **`cwd` is never serialized.** `SessionTranscript.cwd` feeds the redactor's path-prefix stripping and must not appear in any output. Existing adapters document this; preserve it.
- **No emojis** in code, comments, commit messages, or docs.
- **Commit style:** short imperative subject, no `feat:`/`fix:` prefix (e.g. `Add trajectory record parser`).

---

### Task 1: Add the Reasoning event type end to end

Adds the `Reasoning` variant to both enums and maps between them. No adapter emits it yet — that is Tasks 2 and 3. This task exists on its own because it is the only change that leaves the contributor crate, and a reviewer may want to gate the protocol change independently.

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs:205-213` (enum), `:3072-3082` (`side_effect_for`)
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:25-32` (`SessionEventKind`)
- Modify: `crates/trace-commons-contributor/src/envelope.rs:377-408` (`raw_event_for`)
- Test: `crates/trace-commons-contributor/src/envelope.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `TraceContributionEventType::Reasoning` (serializes to `"reasoning"`), `SessionEventKind::Reasoning`. Tasks 2, 3, and 6 construct `SessionEvent { kind: SessionEventKind::Reasoning, .. }`.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `crates/trace-commons-contributor/src/envelope.rs`:

```rust
#[test]
fn reasoning_events_map_to_the_reasoning_event_type() {
    let event = crate::source::SessionEvent {
        kind: crate::source::SessionEventKind::Reasoning,
        timestamp: None,
        content: Some("weighing two approaches".to_string()),
        structured: serde_json::Value::Null,
        tool_name: None,
        token_counts: None,
    };
    let raw = super::raw_event_for(&event, chrono::Utc::now());
    assert_eq!(
        raw.event_type,
        trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning
    );
    assert_eq!(raw.content.as_deref(), Some("weighing two approaches"));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor reasoning_events_map -- --nocapture`

Expected: FAIL to compile with `no variant named 'Reasoning' found for enum 'SessionEventKind'`.

- [ ] **Step 3: Add the protocol enum variant**

In `crates/trace-commons-protocol/src/trace_contribution.rs`, change the enum at line 205:

```rust
pub enum TraceContributionEventType {
    UserMessage,
    AssistantMessage,
    Reasoning,
    ToolCall,
    ToolResult,
    RoutingDecision,
    Feedback,
    HttpExchange,
}
```

- [ ] **Step 4: Update the one exhaustive match**

`side_effect_for` at line 3072 is the only exhaustive match on this enum in the workspace (verified: the `match event_type` sites in `trace-commons-ingest.rs` all match `TraceCreditLedgerEventType`, a different enum). Reasoning is pure model deliberation and performs no action, so it classifies as `None`:

```rust
    match event_type {
        TraceContributionEventType::UserMessage
        | TraceContributionEventType::AssistantMessage
        | TraceContributionEventType::Reasoning
        | TraceContributionEventType::Feedback => SideEffectLevel::None,
```

- [ ] **Step 5: Add the contributor-side kind**

In `crates/trace-commons-contributor/src/source/mod.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind {
    User,
    Assistant,
    Reasoning,
    ToolCall,
    ToolResult,
    Opaque,
}
```

- [ ] **Step 6: Map between them**

In `crates/trace-commons-contributor/src/envelope.rs`, add an arm to `raw_event_for` after the `Assistant` arm:

```rust
        SessionEventKind::Reasoning => (
            TraceContributionEventType::Reasoning,
            e.content.clone(),
            e.structured.clone(),
        ),
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor reasoning_events_map`

Expected: PASS.

- [ ] **Step 8: Verify the whole workspace still builds**

Run: `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets`

Expected: clean. If the server crate fails on a non-exhaustive match, add a `Reasoning` arm there mirroring `AssistantMessage`; do not add a catch-all `_` arm, which would silently swallow future variants.

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs \
        crates/trace-commons-contributor/src/source/mod.rs \
        crates/trace-commons-contributor/src/envelope.rs
git commit -m "Add Reasoning event type to the trace contribution protocol"
```

---

### Task 2: Capture reasoning in the claude-code adapter

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/claude_code.rs:381-384` (the `thinking` arm), `:1-10` (module doc)
- Test: `crates/trace-commons-contributor/src/source/claude_code.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `SessionEventKind::Reasoning` from Task 1.

Background: `map_assistant_record` walks `message.content` blocks. Text blocks accumulate into `texts` and are inserted as one `Assistant` event at `text_insert_at`; `tool_use` blocks push immediately. The `thinking` arm currently does nothing. Claude Code thinking blocks carry the text in a `thinking` field, not `text`.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `crates/trace-commons-contributor/src/source/claude_code.rs`:

```rust
#[test]
fn thinking_blocks_become_reasoning_events() {
    let record = serde_json::json!({
        "message": {
            "content": [
                { "type": "thinking", "thinking": "the user wants X, so I should Y" },
                { "type": "text", "text": "Here is the answer." }
            ]
        }
    });
    let mut events = Vec::new();
    super::map_assistant_record(&record, None, &mut events);

    let kinds: Vec<_> = events.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![SessionEventKind::Reasoning, SessionEventKind::Assistant]
    );
    assert_eq!(
        events[0].content.as_deref(),
        Some("the user wants X, so I should Y")
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor thinking_blocks_become_reasoning`

Expected: FAIL — `assertion failed: left == right`, with only one event (the `Assistant` text) because the thinking block is dropped.

- [ ] **Step 3: Emit the reasoning event**

Replace the `Some("thinking")` arm at line 381:

```rust
            Some("thinking") => {
                // Reasoning is captured as a first-class event and redacted
                // through the same client-side pipeline as every other kind.
                if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        events.push(SessionEvent {
                            kind: SessionEventKind::Reasoning,
                            timestamp,
                            content: Some(t.to_string()),
                            structured: Value::Null,
                            tool_name: None,
                            token_counts: None,
                        });
                    }
                }
            }
```

- [ ] **Step 4: Update the module doc**

The header comment at line 6 claims `Opaque` events guarantee no reasoning leaves the machine. Replace that clause so the file does not contradict itself:

```rust
//! rules; `Opaque` events carry only a record-type marker and never a payload.
//! Reasoning (`thinking`) blocks are captured as `Reasoning` events and
//! redacted like any other content.
```

- [ ] **Step 5: Fix the existing contradicting assertion**

The existing test asserting `!all.contains("secret reasoning")` (near line 498) encodes the reversed posture. Change it to assert reasoning IS present and correctly typed:

```rust
        assert!(
            t.events
                .iter()
                .any(|e| e.kind == SessionEventKind::Reasoning
                    && e.content.as_deref() == Some("secret reasoning")),
            "reasoning is now captured as a first-class event"
        );
```

- [ ] **Step 6: Run the adapter tests**

Run: `cargo test -p trace-commons-contributor --lib source::claude_code`

Expected: PASS. If the `expected_kinds` vector near line 484 now mismatches, update it to include `SessionEventKind::Reasoning` at the position the fixture's thinking block occupies — do not delete the assertion.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/source/claude_code.rs
git commit -m "Capture claude-code thinking blocks as reasoning events"
```

---

### Task 3: Capture reasoning in the codex adapter

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/codex.rs:253-365` (`map_response_item`), `:1-10` (module doc)
- Test: `crates/trace-commons-contributor/src/source/codex.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `SessionEventKind::Reasoning` from Task 1.

Background: `map_response_item` matches on `payload.type`. A `reasoning` item currently falls through to the `other` arm and becomes `Opaque` with only a `record_type` marker. Codex reasoning items carry text in two possible shapes — `summary: [{ "type": "summary_text", "text": ... }]` and `content: [{ "type": "reasoning_text", "text": ... }]` — and either may be absent. Collect both, in that order, and join with a newline.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `crates/trace-commons-contributor/src/source/codex.rs`:

```rust
#[test]
fn reasoning_items_become_reasoning_events() {
    let payload = serde_json::json!({
        "type": "reasoning",
        "summary": [{ "type": "summary_text", "text": "planning the edit" }],
        "content": [{ "type": "reasoning_text", "text": "file A needs a guard" }]
    });
    let mut events = Vec::new();
    super::map_response_item(Some(&payload), None, &mut events);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, SessionEventKind::Reasoning);
    assert_eq!(
        events[0].content.as_deref(),
        Some("planning the edit\nfile A needs a guard")
    );
}

#[test]
fn reasoning_items_with_no_text_are_dropped() {
    let payload = serde_json::json!({ "type": "reasoning" });
    let mut events = Vec::new();
    super::map_response_item(Some(&payload), None, &mut events);
    assert!(events.is_empty());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor reasoning_items`

Expected: FAIL — `reasoning_items_become_reasoning_events` gets `SessionEventKind::Opaque`, and `reasoning_items_with_no_text_are_dropped` gets one `Opaque` event instead of zero.

- [ ] **Step 3: Add the reasoning arm**

Insert a new arm in `map_response_item`, before the final `other` arm at line 355:

```rust
        "reasoning" => {
            // Reasoning is captured as a first-class event and redacted
            // through the same client-side pipeline as every other kind.
            let mut parts = Vec::new();
            for key in ["summary", "content"] {
                if let Some(blocks) = payload.get(key).and_then(|v| v.as_array()) {
                    for block in blocks {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                parts.push(text.to_string());
                            }
                        }
                    }
                }
            }
            // A reasoning item with no recoverable text carries no signal;
            // emitting an empty event would only add noise to the transcript.
            if !parts.is_empty() {
                events.push(SessionEvent {
                    kind: SessionEventKind::Reasoning,
                    timestamp,
                    content: Some(parts.join("\n")),
                    structured: Value::Null,
                    tool_name: None,
                    token_counts: None,
                });
            }
        }
```

- [ ] **Step 4: Update the module doc**

The header comment at line 6-7 names `reasoning` as an `Opaque` category. Correct it:

```rust
//! rules; `Opaque` events (covering `event_msg`, `web_search_call`, and any
//! unknown payload/record type) carry only a record-type marker, never a
//! payload. `reasoning` items are captured as `Reasoning` events.
```

- [ ] **Step 5: Run the adapter tests**

Run: `cargo test -p trace-commons-contributor --lib source::codex`

Expected: PASS. If an existing fixture test asserted a `reasoning` item produced `Opaque`, update it to expect `Reasoning`.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/source/codex.rs
git commit -m "Capture codex reasoning items as reasoning events"
```

---

### Task 4: Widen SessionTranscript.source to Cow

Mechanical prerequisite for Task 6, which needs to carry a harness name known only at runtime.

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:44-54`
- Modify: `crates/trace-commons-contributor/src/source/claude_code.rs:245`
- Modify: `crates/trace-commons-contributor/src/source/codex.rs:242`

**Interfaces:**
- Produces: `SessionTranscript.source: Cow<'static, str>`. Task 6 sets it from a validated `meta.source`. `SessionRef.source` is unchanged and stays `&'static str` — it is the routing key `commands::source_for` matches on.

- [ ] **Step 1: Widen the field**

In `crates/trace-commons-contributor/src/source/mod.rs`, add the import and change the field:

```rust
use std::borrow::Cow;
```

```rust
pub struct SessionTranscript {
    /// Provenance: the harness that produced this session. For the native
    /// adapters this equals the adapter name; for trajectory files it is the
    /// file's own `meta.source`, so a session normalized from OpenHands is
    /// attributed to OpenHands rather than to the trajectory reader.
    /// Distinct from `SessionRef.source`, which is the adapter routing key.
    pub source: Cow<'static, str>,
    pub agent_version: Option<String>,
    pub model: Option<String>,
    pub project: Option<String>, // basename
    pub cwd: Option<String>, // full path; used for redactor prefixes + hashing, NEVER serialized
    pub started_at: Option<DateTime<Utc>>,
    pub session_hash: String, // "sha256:<hex>" of raw file bytes
    pub events: Vec<SessionEvent>,
}
```

- [ ] **Step 2: Run the build to see what breaks**

Run: `cargo check -p trace-commons-contributor --all-targets`

Expected: FAIL with `expected 'Cow<'static, str>', found '&str'` at the two adapter construction sites and in any test fixture that builds a `SessionTranscript` literal.

- [ ] **Step 3: Fix the construction sites**

In `claude_code.rs:245` and `codex.rs:242`, wrap the constant:

```rust
        source: Cow::Borrowed(SOURCE_CLAUDE_CODE),
```

```rust
        source: Cow::Borrowed(SOURCE_CODEX),
```

Add `use std::borrow::Cow;` to each file. Fix any test-fixture literals the compiler flags the same way. `Cow::Borrowed` of a `&'static str` allocates nothing.

- [ ] **Step 4: Verify the consumer still compiles**

`submit.rs:258` calls `transcript.source.to_string()`, which works unchanged on `Cow`. Confirm with:

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --all-targets`

Expected: clean.

- [ ] **Step 5: Run the full crate test suite**

Run: `cargo test -p trace-commons-contributor`

Expected: PASS, no behavior change.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/source/
git commit -m "Widen SessionTranscript source to Cow for runtime provenance"
```

---

### Task 5: Trajectory record model and parser

Pure parsing and validation, no filesystem and no `TraceSource` impl. Task 6 wires it to the trait.

**Files:**
- Create: `crates/trace-commons-contributor/src/source/trajectory.rs`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (add `pub mod trajectory;` and `pub const SOURCE_TRAJECTORY: &str = "trajectory";`)

**Interfaces:**
- Produces:
  - `pub(crate) fn parse_trajectory(bytes: &[u8]) -> anyhow::Result<ParsedTrajectory>`
  - `pub(crate) struct ParsedTrajectory { pub source: String, pub model: Option<String>, pub cwd: Option<String>, pub events: Vec<SessionEvent> }`
  - `pub(crate) fn validate_source_name(raw: &str) -> anyhow::Result<String>`
- Task 6 consumes all three.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/source/trajectory.rs` containing only the module doc and this test module:

```rust
//! Reader for Letta Trajectory v1 files (`https://letta.ai/schemas/trajectory/v1.json`).
//!
//! Accepts either a JSON array of records or JSONL (one record per line).
//! Contributors produce these with `npx @letta-ai/trajectory`; this reader
//! never shells out and never touches a harness's native session store.
//!
//! Fails closed: any malformed record, unparseable timestamp, orphaned
//! `tool_call_id`, or invalid `meta.source` rejects the entire file. Error
//! strings are reason labels only and never carry record content.

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
```

Register the module in `crates/trace-commons-contributor/src/source/mod.rs`:

```rust
pub mod claude_code;
pub mod codex;
pub mod trajectory;

pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
pub const SOURCE_CODEX: &str = "codex";
pub const SOURCE_TRAJECTORY: &str = "trajectory";
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor --lib source::trajectory`

Expected: FAIL to compile — `cannot find function 'parse_trajectory' in this scope`.

- [ ] **Step 3: Implement the parser**

Add above the test module in `trajectory.rs`:

```rust
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::source::{SessionEvent, SessionEventKind};

const MAX_SOURCE_LEN: usize = 64;

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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --lib source::trajectory`

Expected: all eight tests PASS.

- [ ] **Step 5: Verify no warnings**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --all-targets`

Expected: clean. `ParsedTrajectory` fields are unused until Task 6; if dead-code warnings fire, proceed to Task 6 rather than adding `#[allow(dead_code)]` — but if you must commit in between, the allow is acceptable and Task 6 removes it.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/source/trajectory.rs \
        crates/trace-commons-contributor/src/source/mod.rs
git commit -m "Add trajectory v1 record parser"
```

---

### Task 6: TrajectorySource implementing TraceSource

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/trajectory.rs`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:75-91` (`all_sources`)

**Interfaces:**
- Consumes: `parse_trajectory`, `ParsedTrajectory` (Task 5); `SessionTranscript.source: Cow<'static, str>` (Task 4).
- Produces: `pub struct TrajectorySource; pub fn new(path: PathBuf) -> Self`, and the new `all_sources` signature:
  `pub fn all_sources(claude_root: Option<PathBuf>, codex_root: Option<PathBuf>, trajectory_path: Option<PathBuf>) -> Vec<Box<dyn TraceSource>>`. Task 7 calls it.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `trajectory.rs`:

```rust
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
        write_temp(dir.path(), "b.jsonl", "{\"role\":\"meta\",\"source\":\"pi\"}\n");
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor --lib source::trajectory`

Expected: FAIL to compile — `cannot find type 'TrajectorySource'`.

- [ ] **Step 3: Implement the source**

Add to `trajectory.rs`, above the test module:

```rust
use std::borrow::Cow;
use std::path::{Path, PathBuf};

use crate::source::{
    SOURCE_TRAJECTORY, SessionRef, SessionTranscript, TraceSource, session_hash,
};

/// Reads trajectory-v1 files from an explicitly supplied path. Unlike the
/// native adapters there is no conventional local store to scan, so this
/// source is only constructed when the contributor passes `--trajectory`.
pub struct TrajectorySource {
    path: PathBuf,
}

impl TrajectorySource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

fn is_trajectory_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
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
        path,
        project: None,
        cwd: None,
        started_at,
        size_bytes: metadata.len(),
    })
}

impl TraceSource for TrajectorySource {
    fn name(&self) -> &'static str {
        SOURCE_TRAJECTORY
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        if self.path.is_file() {
            return Ok(session_ref_for(self.path.clone()).into_iter().collect());
        }
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return Ok(Vec::new());
        };
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            let path = entry.path();
            if !path.is_file() || !is_trajectory_file(&path) {
                continue;
            }
            match session_ref_for(path) {
                Some(r) => sessions.push(r),
                None => skipped += 1,
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

        Ok(SessionTranscript {
            source: Cow::Owned(parsed.source),
            // Trajectory carries no harness version field.
            agent_version: None,
            model: parsed.model,
            project,
            cwd: parsed.cwd,
            started_at,
            session_hash: hash,
            events: parsed.events,
        })
    }
}
```

- [ ] **Step 4: Extend all_sources**

In `crates/trace-commons-contributor/src/source/mod.rs`, replace `all_sources`:

```rust
/// Construct the set of available `TraceSource` adapters, defaulting roots to
/// `~/.claude/projects` and `~/.codex/sessions` when not overridden. The
/// trajectory source is included only when an explicit path is supplied,
/// because trajectory files have no conventional local store.
pub fn all_sources(
    claude_root: Option<PathBuf>,
    codex_root: Option<PathBuf>,
    trajectory_path: Option<PathBuf>,
) -> Vec<Box<dyn TraceSource>> {
    let claude_root = claude_root.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".claude/projects")
    });
    let codex_root =
        codex_root.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".codex/sessions"));
    let mut sources: Vec<Box<dyn TraceSource>> = vec![
        Box::new(claude_code::ClaudeCodeSource::new(claude_root)),
        Box::new(codex::CodexSource::new(codex_root)),
    ];
    if let Some(path) = trajectory_path {
        sources.push(Box::new(trajectory::TrajectorySource::new(path)));
    }
    sources
}
```

- [ ] **Step 5: Add tempfile as a dev-dependency check**

`tempfile = "3"` is already in `[dev-dependencies]` of `crates/trace-commons-contributor/Cargo.toml`. Confirm with `grep tempfile crates/trace-commons-contributor/Cargo.toml`; do not add it if present.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p trace-commons-contributor --lib source::trajectory`

Expected: PASS. The two `all_sources(None, None)` call sites in `commands.rs` will now fail to compile; that is Task 7. To keep this task independently verifiable, temporarily pass `None` as the third argument at both sites and let Task 7 replace them properly.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/source/ crates/trace-commons-contributor/src/commands.rs
git commit -m "Add TrajectorySource reading trajectory v1 files"
```

---

### Task 7: Plumb --trajectory through the CLI

**Files:**
- Modify: `crates/trace-commons-contributor/src/commands.rs:210-270` (`discover_filtered`, `source_for`), `:344` (`list`), `:363-374` (`SubmitSelection`), `:395`, `:411`, `:434`
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs:37`, `:39-63`, `:108`, `:109-130`

**Interfaces:**
- Consumes: `all_sources(claude, codex, trajectory)` from Task 6.
- Produces: `SubmitSelection.trajectory: Option<&'a Path>`; `pub fn list(trajectory: Option<&Path>) -> Result<()>`.

- [ ] **Step 1: Write the failing test**

Add to `crates/trace-commons-contributor/src/commands.rs`, inside the existing `mod project_filter_tests` (or a new `mod trajectory_selection_tests`):

```rust
#[test]
fn discover_filtered_includes_trajectory_only_when_a_path_is_given() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.json");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(
        br#"[{"role":"meta","source":"pi"},
             {"role":"user","content":"hi","timestamp":"2026-07-10T12:00:00Z"}]"#,
    )
    .unwrap();

    let without = super::discover_filtered(Some("trajectory"), None, None, None).unwrap();
    assert!(
        without.is_empty(),
        "trajectory files must never appear without an explicit path"
    );

    let with = super::discover_filtered(Some("trajectory"), None, None, Some(&p)).unwrap();
    assert_eq!(with.len(), 1);
    assert_eq!(with[0].source, crate::source::SOURCE_TRAJECTORY);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor discover_filtered_includes_trajectory`

Expected: FAIL to compile — `discover_filtered` takes 3 arguments, not 4.

- [ ] **Step 3: Thread the path through discovery**

In `commands.rs`, change both function signatures and their `all_sources` calls:

```rust
fn discover_filtered(
    source_filter: Option<&str>,
    project_filter: Option<&Path>,
    since: Option<chrono::Duration>,
    trajectory: Option<&Path>,
) -> Result<Vec<SessionRef>> {
```

```rust
    for source in all_sources(None, None, trajectory.map(|p| p.to_path_buf())) {
```

```rust
/// Build a fresh `TraceSource` instance for the adapter named `name` (used
/// to pair a previously discovered `SessionRef` with a loadable source).
fn source_for(name: &str, trajectory: Option<&Path>) -> Option<Box<dyn TraceSource>> {
    all_sources(None, None, trajectory.map(|p| p.to_path_buf()))
        .into_iter()
        .find(|s| s.name() == name)
}
```

- [ ] **Step 4: Update the call sites**

- `list` (line 344): `let sessions = discover_filtered(None, None, None, trajectory)?;` and change the signature to `pub fn list(trajectory: Option<&Path>) -> Result<()>`.
- `submit` (line 395): `let mut refs = discover_filtered(sel.source, sel.project, since, sel.trajectory)?;`
- Lines 411 and 434: `source_for(r.source, sel.trajectory)`.

Add the field to `SubmitSelection`:

```rust
pub struct SubmitSelection<'a> {
    pub all: bool,
    pub since: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub source: Option<&'a str>,
    pub yes: bool,
    pub dry_run: bool,
    pub pii_filter: Option<&'a str>,
    pub manifest: Option<&'a Path>,
    /// Path to a trajectory-v1 file or a directory of them. Trajectory
    /// sessions are only discoverable when this is set.
    pub trajectory: Option<&'a Path>,
}
```

- [ ] **Step 5: Update the CLI**

In `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`, add the argument to both subcommands:

```rust
    /// List discoverable local sessions
    List {
        /// Path to a trajectory-v1 file or directory of them (from `npx @letta-ai/trajectory`)
        #[arg(long)]
        trajectory: Option<PathBuf>,
    },
```

In `Submit`, update the `source` doc and add `trajectory`:

```rust
        /// Restrict to one source: claude-code | codex | trajectory
        #[arg(long)]
        source: Option<String>,
        /// Path to a trajectory-v1 file or directory of them (from `npx @letta-ai/trajectory`)
        #[arg(long)]
        trajectory: Option<PathBuf>,
```

Update the dispatch:

```rust
        Command::List { trajectory } => commands::list(trajectory.as_deref()),
        Command::Submit {
            all,
            since,
            project,
            source,
            yes,
            dry_run,
            pii_filter,
            manifest,
            trajectory,
        } => {
            let sel = commands::SubmitSelection {
                all,
                since: since.as_deref(),
                project: project.as_deref(),
                source: source.as_deref(),
                yes,
                dry_run,
                pii_filter: pii_filter.as_deref(),
                manifest: manifest.as_deref(),
                trajectory: trajectory.as_deref(),
            };
            commands::submit(&store, &sel).await
        }
```

- [ ] **Step 6: Run the test and the full suite**

Run: `cargo test -p trace-commons-contributor`

Expected: PASS. Fix any remaining `SubmitSelection` literals in tests by adding `trajectory: None`.

- [ ] **Step 7: Verify the binary works end to end**

```bash
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- list
cargo run -p trace-commons-contributor --bin trace-commons-contributor -- list --trajectory /nonexistent
```

Expected: both exit 0. The second prints `no sessions found` rather than erroring — a missing path yields empty discovery, not a crash.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-contributor/src/commands.rs \
        crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs
git commit -m "Plumb --trajectory through list and submit"
```

---

### Task 8: Add --no-reasoning opt-out

**Files:**
- Modify: `crates/trace-commons-contributor/src/commands.rs` (`SubmitSelection`, `submit`)
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`
- Test: `crates/trace-commons-contributor/src/commands.rs`

**Interfaces:**
- Consumes: `SessionEventKind::Reasoning` (Task 1), `SubmitSelection` (Task 7).
- Produces: `pub(crate) fn strip_reasoning(t: &mut SessionTranscript)`.

- [ ] **Step 1: Write the failing test**

Add to `commands.rs` tests:

```rust
#[test]
fn strip_reasoning_removes_only_reasoning_events() {
    use crate::source::{SessionEvent, SessionEventKind};
    let mk = |kind: SessionEventKind| SessionEvent {
        kind,
        timestamp: None,
        content: Some("x".to_string()),
        structured: serde_json::Value::Null,
        tool_name: None,
        token_counts: None,
    };
    let mut t = crate::source::SessionTranscript {
        source: std::borrow::Cow::Borrowed("claude-code"),
        agent_version: None,
        model: None,
        project: None,
        cwd: None,
        started_at: None,
        session_hash: "sha256:aa".to_string(),
        events: vec![
            mk(SessionEventKind::User),
            mk(SessionEventKind::Reasoning),
            mk(SessionEventKind::Assistant),
        ],
    };
    super::strip_reasoning(&mut t);
    let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![SessionEventKind::User, SessionEventKind::Assistant]
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor strip_reasoning_removes_only`

Expected: FAIL to compile — `cannot find function 'strip_reasoning'`.

- [ ] **Step 3: Implement the helper**

Add to `commands.rs`:

```rust
/// Drop reasoning events before envelope construction. Reasoning is captured
/// by default; this is the per-run opt-out behind `--no-reasoning`.
pub(crate) fn strip_reasoning(t: &mut SessionTranscript) {
    t.events
        .retain(|e| e.kind != crate::source::SessionEventKind::Reasoning);
}
```

Import `SessionTranscript` in the `use crate::source::{...}` list at line 23.

- [ ] **Step 4: Wire it into submit**

Add the field to `SubmitSelection`:

```rust
    /// Drop model reasoning from this run. Reasoning is included by default.
    pub no_reasoning: bool,
```

In `submit`, immediately after the transcript is loaded via `src.load(r)` (around line 434), call:

```rust
        if sel.no_reasoning {
            strip_reasoning(&mut transcript);
        }
```

Bind the loaded transcript as `mut` if it is not already.

- [ ] **Step 5: Add the CLI flag**

In the `Submit` subcommand:

```rust
        /// Exclude model reasoning from this submission (reasoning is included by default)
        #[arg(long)]
        no_reasoning: bool,
```

Add `no_reasoning,` to the destructuring and `no_reasoning,` to the `SubmitSelection` literal.

- [ ] **Step 6: Run the tests**

Run: `cargo test -p trace-commons-contributor`

Expected: PASS. Add `no_reasoning: false` to any other `SubmitSelection` literals the compiler flags.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/commands.rs \
        crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs
git commit -m "Add --no-reasoning opt-out to submit"
```

---

### Task 9: End-to-end test, docs, and full verification

**Files:**
- Test: `crates/trace-commons-contributor/src/envelope.rs` (inline tests)
- Create: `crates/trace-commons-contributor/fixtures/trajectory-openhands.json`
- Modify: `README.md`, `docs/trace-commons.md`

**Interfaces:**
- Consumes: everything from Tasks 1-8.

- [ ] **Step 1: Write the end-to-end test**

Add to `mod tests` in `crates/trace-commons-contributor/src/envelope.rs`:

```rust
#[test]
fn trajectory_session_builds_an_envelope_with_reasoning_and_no_cwd_leak() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.json");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(
        br#"[
          {"role":"meta","source":"openhands","cwd":"/home/dev/secretproj","model":"gpt-5"},
          {"role":"user","content":"fix the bug","timestamp":"2026-07-10T12:00:00Z"},
          {"role":"reasoning","content":"the guard clause is inverted","timestamp":"2026-07-10T12:00:01Z"},
          {"role":"assistant","content":"Fixed it.","timestamp":"2026-07-10T12:00:02Z"}
        ]"#,
    )
    .unwrap();

    let src = crate::source::trajectory::TrajectorySource::new(path);
    let r = &crate::source::TraceSource::discover(&src).unwrap()[0];
    let t = crate::source::TraceSource::load(&src, r).unwrap();

    assert_eq!(t.source.as_ref(), "openhands");

    let cfg = test_config();
    let raw = super::build_raw_contribution(&t, &cfg, chrono::Utc::now());
    assert!(
        raw.events.iter().any(|e| e.event_type
            == trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning),
        "reasoning must survive into the raw contribution"
    );

    let serialized = serde_json::to_string(&raw).unwrap();
    assert!(
        !serialized.contains("secretproj"),
        "cwd must never be serialized"
    );
}
```

`build_raw_contribution(t: &SessionTranscript, cfg: &ContributorConfig, now: DateTime<Utc>) -> RawTraceContribution` is defined at `envelope.rs:254`, and `test_config()` is the existing helper at `envelope.rs:441` that the other tests in this module use the same way (see `envelope.rs:461`).

- [ ] **Step 2: Run it**

Run: `cargo test -p trace-commons-contributor trajectory_session_builds_an_envelope`

Expected: PASS.

- [ ] **Step 3: Commit the test**

The trajectory JSON stays inline in the test. Do not create a separate fixture file: nothing else reads it, and a second copy of the same sanitized body would only drift.

```bash
git add crates/trace-commons-contributor/
git commit -m "Add end-to-end trajectory to envelope test"
```

- [ ] **Step 4: Document the contributor workflow**

Add to `README.md`, in the contributor CLI section:

```markdown
### Contributing sessions from other harnesses

The CLI reads Claude Code and Codex sessions natively. For any other harness
covered by [Letta Trajectory](https://github.com/letta-ai/trajectory) -- Hermes,
Letta Code, OpenClaw, OpenHands, Pi, or Deep Agents -- normalize the session
first, then point the CLI at the result:

```sh
npx @letta-ai/trajectory > session.json
trace-commons-contributor submit --trajectory session.json
```

`--trajectory` also accepts a directory, in which case every `*.json` and
`*.jsonl` file directly inside it is offered. Trajectory files are never
discovered implicitly; without `--trajectory` they are invisible to `list`
and `submit`.

Model reasoning is captured by default and redacted client-side like any
other content. Pass `--no-reasoning` to exclude it from a submission.
```

- [ ] **Step 5: Record the canonical-text cohort boundary**

Add to `docs/trace-commons.md`, near the canonical-representation discussion:

```markdown
Reasoning events (`TraceContributionEventType::Reasoning`) became part of the
envelope on 2026-07-25. `canonical_whole_trace_representation` truncates at the
first 12 events, so reasoning occupies slots that previously held other event
types. Traces submitted before that date are unchanged and were not re-scored,
but novelty and dedup comparisons spanning the boundary are not like-for-like.
Re-check `gate-calibrate` floors against a post-change sample before treating
scores from the two cohorts as comparable.
```

- [ ] **Step 6: Full CI-equivalent verification**

```bash
RUSTFLAGS="-D warnings" cargo check --workspace --all-targets
RUSTFLAGS="-D warnings" cargo test --workspace --no-run
cargo test -p trace-commons-contributor
cargo test -p trace-commons-protocol
cargo clippy -p trace-commons-contributor -p trace-commons-protocol --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: all clean. Do not claim completion until every command above has actually been run and its output confirmed.

- [ ] **Step 7: Commit**

```bash
git add README.md docs/trace-commons.md
git commit -m "Document trajectory contributor workflow and reasoning capture"
```

---

## Verification Checklist

Before opening a PR, confirm each:

- [ ] `RUSTFLAGS="-D warnings" cargo check --workspace --all-targets` is clean.
- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --no-run` is clean.
- [ ] Clippy passes with the repo's exact allow-list, unwidened.
- [ ] `scripts/operator/pilot-bootstrap-smoke.sh` still passes (CI gates on it).
- [ ] No new entries in any `Cargo.toml` `[dependencies]` section.
- [ ] No absolute path, file content, or tool argument appears in any error string or log line added by this work.
- [ ] `--trajectory` is required for trajectory sessions to appear in `list` and `submit`.
- [ ] Reasoning appears in submissions by default and is absent with `--no-reasoning`.
