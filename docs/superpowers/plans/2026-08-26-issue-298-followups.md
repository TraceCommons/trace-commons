# Issue #298 Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the four findings issue #298 left open after 0.5.0, plus the
redactor over-aggression found while designing them.

**Architecture:** Three independent slices (timestamps, `conversation_id`,
payload profiles) and one sequential chain (corrections: consent flag →
redaction path → shells → shadow scoring).

**Tech Stack:** Rust (protocol, server, contributor, GTK), Swift (macOS), C#
(Windows).

**Spec:** `docs/superpowers/specs/2026-08-26-issue-298-followups-design.md` --
read the section for your task before starting.

## Global Constraints

- PostgreSQL-only. No libsql feature flags.
- No emojis in commits, PRs, code, or docs.
- Hash-only / label-only logging: never a raw URL, token, trace body,
  contributor identity, or correction text in a log line or audit row.
- Fail-closed by default. Never silently downgrade a control.
- Envelope tenant fields and `conversation_id` are attribution only, never
  authorization.
- Verify with `RUSTFLAGS="-D warnings"`. Plain `cargo check` does not apply it;
  CI does.
- Clippy allow-list, verbatim: `-A clippy::type_complexity
  -A clippy::collapsible_if -A clippy::manual_option_as_slice
  -A clippy::useless_vec -A clippy::redundant_pattern_matching`
- `cargo fmt --all` before committing. A post-edit formatter hook has
  previously rewritten whole files; check `git show --stat` after each commit.
- **No loosening of secret, credential, cookie or auth-header redaction
  anywhere**, including in corrections and in the re-scoped profiles.
- Tests must be non-vacuous and CI-runnable: correctly annotated `#[test]`,
  not `#[ignore]`, not behind a feature CI does not build. This repo has
  shipped test functions silently missing their attribute.

---

## Slice 1 -- independent, run in parallel

### Task A: recorded-trace timestamps (S3)

**Branch:** `s3-recorded-trace-timestamps`

**Files:**
- Modify: `crates/trace-commons-protocol/src/llm/recording.rs` (`TraceStep`)
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (`from_recorded_trace`)
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/` (loader passthrough)

**Interfaces produced:** `TraceStep.timestamp: Option<DateTime<Utc>>`.

- [ ] **Step 1: Write the failing test**

In `trace_contribution.rs`'s test module:

```rust
#[test]
fn a_recorded_step_keeps_its_own_timestamp() {
    // A recorded trace whose steps carry times must not collapse to one
    // instant. Every event sharing `created_at` is the identical-timestamp
    // finding in issue #298.
    let t0 = Utc::now();
    let t1 = t0 + chrono::Duration::seconds(5);
    let trace = recorded_trace_with_step_times(&[t0, t1]);

    let raw = RawTraceContribution::from_recorded_trace(&trace, &options());

    let stamps: Vec<_> = raw.events.iter().map(|e| e.timestamp).collect();
    assert!(
        stamps.windows(2).any(|w| w[0] != w[1]),
        "steps with distinct times must not collapse to one instant"
    );
}

/// A source with no times behaves exactly as before. Nothing is invented.
#[test]
fn a_recorded_step_without_a_timestamp_falls_back() {
    let trace = recorded_trace_with_step_times(&[]);
    let raw = RawTraceContribution::from_recorded_trace(&trace, &options());
    let stamps: Vec<_> = raw.events.iter().map(|e| e.timestamp).collect();
    assert!(stamps.windows(2).all(|w| w[0] == w[1]));
}
```

Build `recorded_trace_with_step_times` from the existing recorded-trace test
fixtures in this module; do not invent a new fixture shape if one exists.

- [ ] **Step 2: Run to verify it fails**

`cargo test -p trace-commons-protocol a_recorded_step_keeps_its_own_timestamp`
Expected: FAIL to compile -- `TraceStep` has no `timestamp` field.

- [ ] **Step 3: Add the field**

In `recording.rs`, on `TraceStep`:

```rust
    /// When this step happened, where the recording knows.
    ///
    /// `None` on every recording written before this field existed, and on
    /// sources that do not carry per-step times. `from_recorded_trace` falls
    /// back to the envelope's `created_at` in that case rather than
    /// synthesising an offset: a plausible-looking invented timestamp is
    /// worse than an absent one, because a consumer cannot tell it apart
    /// from a real one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
```

- [ ] **Step 4: Use it**

In `from_recorded_trace`, replace each of the five `timestamp: created_at`
with `timestamp: step.timestamp.unwrap_or(created_at)`. Locate them by
surrounding text; there are five, on `UserMessage`, `AssistantMessage`,
`ToolCall`, `ToolResult` and `HttpExchange`.

- [ ] **Step 5: Pass it through the bootstrap loader**

`pilot_bootstrap` builds the `RecordedTrace` it hands to
`from_recorded_trace`. Where the source JSONL record carries a timestamp,
populate `TraceStep.timestamp`. Where it does not, leave `None`.

- [ ] **Step 6: Measure and report**

Determine how many of the datasets pilot-bootstrap actually loads carry
per-record timestamps. Inspect the real JSONL, not the loader. **Put the
number in your report.** If it is near zero, say so plainly -- the field is
cheap but the benefit would then be theoretical, and whoever closes S3 needs
to know that.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add -A crates/ && git commit -m "Let a recorded step carry its own timestamp"
git show --stat HEAD
```

---

### Task B: conversation_id (S4a)

**Branch:** `s4a-conversation-id`

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (envelope + raw)
- Modify: `crates/trace-commons-contributor/src/envelope.rs` and `src/source/` adapters
- Modify: `docs/trace-spec.md`

**Interfaces produced:** `conversation_id: Option<String>` on the envelope.

- [ ] **Step 1: Write the failing test**

```rust
/// 23 traces in the reported corpus open with an assistant_message and are
/// indistinguishable from one another: a greeting, a triggered turn, and a
/// resumed thread are the same bytes. A conversation id separates them.
#[test]
fn an_envelope_carries_its_conversation_id() {
    let mut envelope = bare_envelope();
    envelope.conversation_id = Some("conv-1".to_string());
    let round_tripped: TraceContributionEnvelope =
        serde_json::from_str(&serde_json::to_string(&envelope).unwrap()).unwrap();
    assert_eq!(round_tripped.conversation_id.as_deref(), Some("conv-1"));
}

/// An envelope written before this field existed still parses.
#[test]
fn an_envelope_without_a_conversation_id_still_parses() {
    let mut value = serde_json::to_value(bare_envelope()).unwrap();
    value.as_object_mut().unwrap().remove("conversation_id");
    let parsed: TraceContributionEnvelope = serde_json::from_value(value).unwrap();
    assert_eq!(parsed.conversation_id, None);
}
```

- [ ] **Step 2: Run to verify it fails**

Expected: FAIL to compile -- no such field.

- [ ] **Step 3: Add the field**

```rust
    /// The source session this trace belongs to, so a consumer can tell a
    /// resumed thread from a fresh one. A trace that opens with an
    /// assistant message is otherwise indistinguishable from a greeting or
    /// a triggered turn (issue #298).
    ///
    /// ATTRIBUTION ONLY. Like every other envelope-declared identifier, this
    /// is what the emitter says, not something the server verified. It must
    /// never reach a gate, a scoring input, or a tenant-scoping decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
```

Mirror it on `RawTraceContribution` and carry it through redaction unchanged.

- [ ] **Step 4: Populate it from the adapters**

Each source adapter sets it from the session identifier it already has.
Read `claude_code.rs`, `codex.rs` and `trajectory.rs` -- all three already
resolve a session id. Do not invent one where the source has none; leave
`None`.

- [ ] **Step 5: Write the attribution-only guard test**

```rust
/// A guard, not a formality: an emitter-declared id that reached a gate
/// would be a spoofable input to admission.
#[test]
fn a_conversation_id_does_not_move_any_score() {
    let mut with = bare_envelope();
    with.conversation_id = Some("conv-1".to_string());
    let without = bare_envelope();
    assert_eq!(
        compute_value_scorecard(&with).composite,
        compute_value_scorecard(&without).composite
    );
}
```

Use whatever the scorecard entry point is actually called; read it first.

- [ ] **Step 6: Document it**

Add a row to `docs/trace-spec.md` stating it is attribution-only.

- [ ] **Step 7: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add -A crates/ docs/ && git commit -m "Let an envelope name the conversation it came from"
git show --stat HEAD
```

---

### Task C: re-scope the tool payload profiles (S6)

**Branch:** `s6-payload-profiles`

Read the spec's S6 section in full before starting. Two passes are explicitly
NOT to be changed: the cue-gated entropy sweep and `redact_known_paths`.

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`
  (`FILESYSTEM_RULES`, `BROWSER_RULES`, `field_matches`)

- [ ] **Step 1: Build the before/after fixture first**

Before changing any rule, build a fixture from a realistic coding session --
a shell command, a file read, an edit with a diff, a failing command with
stderr -- run it through redaction with payloads ENABLED, and record what
survives. Put the before-and-after in your report. This is the measurement
the spec says is missing; do not skip it.

- [ ] **Step 2: Write the failing tests**

```rust
/// The fields that make a coding trace replayable must survive with
/// payloads enabled. Replacing them wholesale is why enabling the payload
/// tier would hand a consumer markers instead of usable traces.
#[test]
fn a_shell_command_survives_payload_redaction() {
    let payload = serde_json::json!({"command": "cargo test -p foo --lib"});
    let out = redact_tool_payload_for("bash", &payload);
    assert!(
        out.to_string().contains("cargo test"),
        "the command is the replayable part: {out}"
    );
}

/// Narrowed matching: a field is not a path merely because its name
/// contains the letters "file".
#[test]
fn a_field_named_profile_is_not_treated_as_a_path() {
    assert!(!field_matches("profile", FILESYSTEM_PATH_MATCHER));
    assert!(field_matches("file_path", FILESYSTEM_PATH_MATCHER));
}

/// Unchanged: credentials and auth headers are still replaced wholesale.
#[test]
fn credentials_are_still_replaced() {
    let payload = serde_json::json!({"headers": {"authorization": "Bearer abc123"}});
    let out = redact_tool_payload_for("browser_fetch", &payload);
    assert!(!out.to_string().contains("abc123"));
}
```

Name the helper to match whatever the module actually exposes; read it first.

- [ ] **Step 3: Re-scope the rules**

Change content-bearing fields (`command`, `diff`, `patch`, `content`,
`contents`, `stdout`, `stderr`) from `Replace` to running the general
redaction passes over the value, which already handle paths, emails and
secrets well.

Keep `Replace` only where the field is inherently sensitive regardless of
content: credentials, cookies, auth headers.

Narrow the path matcher from substring `Contains` to exact names plus an
explicit suffix list, so `profile` and `file_count` stop matching.

- [ ] **Step 4: Verify the fixture again**

Re-run Step 1's fixture and put the after-state beside the before-state in
your report.

- [ ] **Step 5: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol
cargo clippy -p trace-commons-protocol --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
git add -A crates/ && git commit -m "Preserve what makes a coding trace replayable"
git show --stat HEAD
```

---

## Slice 2 -- the corrections chain, strictly sequential

Tasks D through G depend on each other in order. Do not start one before its
predecessor has landed.

### Task D: the correction consent flag

**Branch:** `s5-correction-consent-flag`

Read the spec's "A third consent flag" and "The policy pin does not cover
this, and should" sections in full.

Covers: `ConsentMetadata.correction_included`;
`derive_envelope_content_presence` reporting it; `residual_risk` flooring at
Medium for it; `corpus_status_with_pii_backstop_hold` becoming "any content
flag"; extending `consent_policy_pin.rs` to cover content flags with
compiler-enforced exhaustiveness.

**Hard gate:** the policy text at <https://tracecommons.ai/legal/> must be
published and `TRACE_CONTRIBUTION_POLICY_VERSION` bumped with it. If the text
is not ready, this task stops after the pin extension and does not add the
flag. Do not edit the pin to assert whatever the code does.

### Task E: corrections are not scrubbed

**Branch:** `s5-correction-redaction`
Depends on: D.

Corrections skip the semantic passes; secret detection still runs and still
blocks on High/Critical. Read the spec's "A correction is not scrubbed".

### Task F: the correction control in three shells

**Branch:** `s5-correction-ui`
Depends on: D, E.

Text input shown only when the verdict is `failed` or `partly`, beside the
0.5.0 verdict control, in GTK, macOS and Windows. macOS builds locally
(`cargo build -p trace-commons-contributor-ffi` then `swift test`); Windows
needs `tc-win-dev` via `windows/scripts/win-exec.sh` and the box must be
started and stopped deliberately.

### Task G: shadow-mode correction value

**Branch:** `s5-correction-value`
Depends on: D.

Embed the correction, score novelty against existing corrections, apply
`dup_pen` and the per-contributor cap. **Compute and store; credit nothing.**

---

## Not in this plan

**S4b, the payload tier, is deliberately absent.** It needs no code. It needs
an owner for the 48-trace quarantine queue, and it is additionally gated on
Task C -- enabling it before the profiles are re-scoped would hand consumers
markers rather than usable traces. Do not open a pull request enabling
`include_tool_payloads`.
