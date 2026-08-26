# Closing the issue #298 follow-ups

Design for the four findings left open by issue #298 after the 0.5.0 work.
Targeted at 0.5.1.

Issue #298 is a downstream report from the maintainer of `nearai-bench`,
measured on a 330-trace export: the scorecard rated all 330 perfectly
replayable while none could be rebuilt into a runnable regression task. Most
of it is closed. This covers what is not.

## What is already closed, for context

- Replayability now measures sufficiency rather than field presence
  (`replayability = sufficiency.score()`), and `quality` multiplies in
  `content_share` rather than rewarding length.
- `parent_event_id`, `tool_call_id`, `success`, `failure_modes` and
  `model_name` are reachable from emitters and populated.
- `Reasoning` and `ToolResult` are emitted.
- `task_success` is collected from the contributor as of 0.5.0.

## Scope

Four items. One of them is deliberately not code.

| Item | Nature |
| --- | --- |
| S5 corrections | Feature, and the largest piece |
| S3 recorded-trace timestamps | Format change |
| S4a `conversation_id` | Wire addition |
| S4b payload tier | Operational prerequisite, no code |
| S6 payload profile over-redaction | Found during review; gates S4b |

---

## S5: human corrections

### The gap this must not reopen

`human_correction` is scrubbed by redaction (18 references in
`trace_contribution.rs`) but is invisible to both
`derive_envelope_content_presence` and `residual_risk` -- zero references in
either.

Shipping a correction box without addressing that recreates exactly the bug
class #418 and #419 just closed. An envelope carrying contributor prose would
declare no content, take the Low-risk acceptance path, and skip the PII
backstop hold entirely, because #418's hold predicate reads the consent
flags. The redactor would scrub it; the backstop that exists to catch what
the regexes miss would never be enrolled.

This is the same asymmetry as the object-key gap: the component that would
catch it never runs, because enrolment is decided elsewhere.

### A third consent flag

`ConsentMetadata` gains `correction_included`.

A correction is a third content class. It is contributor-authored prose about
a session, not session message text and not a tool payload, so folding it into
`message_text_included` would make that flag mean two different things and
would misreport what the envelope carries. The flags are a FACTUAL
DECLARATION of content (`docs/trace-spec.md`), so a new content class needs a
new declaration.

Three things must learn about it, and all three are pre-existing seams:

- `derive_envelope_content_presence` -- must report the correction's presence.
- `residual_risk` -- must floor at Medium for it, as it already does for
  either existing flag.
- `corpus_status_with_pii_backstop_hold` -- its predicate is currently
  `message_text_included || tool_payloads_included`. It becomes "any content
  flag". #418's doc comment explicitly anticipated this: it takes the whole
  `ConsentMetadata` rather than a bool per flag precisely so a third flag
  could not reintroduce the gap. This is that third flag; the design it
  planned for is the design being used.

`reconcile_consent_declarations` corrects upward only, and that stays true for
the new flag.

### The policy pin does not cover this, and should

`crates/trace-commons-protocol/tests/consent_policy_pin.rs` (#390) exists so
that a change to the consent surface cannot land without a decision about the
published policy at <https://tracecommons.ai/legal/>.

It does not cover content flags. It pins the `ConsentScope` variants and
`TRACE_CONTRIBUTION_POLICY_VERSION`, and has no reference to
`message_text_included` or `tool_payloads_included`. So adding
`correction_included` would pass it silently -- even though a new content
class is exactly the kind of consent-surface change the pin was written to
catch.

That is a gap in the guard, not a reason to skip the policy work. This slice
must:

1. Extend the pin to cover the content flags, with the same
   compiler-enforced exhaustiveness the scopes get -- a `match` over a
   struct-literal destructure of `ConsentMetadata`, so a fourth flag added
   later fails to compile until someone writes the policy paragraph.
2. Publish the policy text describing what a correction is and what it is
   not, and bump `TRACE_CONTRIBUTION_POLICY_VERSION` with it.

Bumping the version without publishing the text, or extending the pin to
assert whatever the code already does, both defeat the guard. If the policy
text is not ready, the correction flag does not ship.

### Collection

A text input that appears only when the verdict is `failed` or `partly`, in
all three shells, beside the 0.5.0 verdict control.

Gating on a non-success verdict is a guard, not just semantics. You cannot
correct a run you have just called successful, so the surface for
correction-shaped credit farming is halved, and the field only appears where a
correction is meaningful.

Absent a correction, nothing changes: the flag is false and behaviour matches
0.5.0 exactly.

### A correction is not scrubbed

Redaction would destroy the thing a correction exists to carry. "The agent
used `/Users/zaki/proj/config.toml` instead of the staging one" is useless
once the path is a placeholder, and a correction is written to be read.

A correction is also categorically different from session content. Session
content is captured incidentally; a correction is composed deliberately, for
submission, by someone who chooses every word knowing where it goes.

So corrections skip the semantic passes -- path, email and identifier
replacement -- and their text is stored as written.

**Secret detection still runs, and still blocks.** A High or Critical match
sets `blocked_secret_detected` and refuses the submission, exactly as
elsewhere. The asymmetry is deliberate: a path in a correction is the point,
and a live credential in a correction is never the point. The contributor is
asked to remove it rather than having it silently masked, because a masked
credential still leaked to whoever saw it in transit.

This means `human_correction` reaches the corpus unredacted apart from
credentials, which is precisely why it needs its own consent flag and its own
declaration -- see above.

### Value: novelty-scored, in shadow

A correction earns through the machinery a trace already faces rather than a
new one:

- embed the correction,
- score novelty against corrections already in the corpus,
- let cross-trace dedup (`dup_pen = 1/cluster_size`, #169) collapse
  near-duplicates,
- let the per-contributor concave cap (#171) bound repeat abuse.

A contributor pasting the same correction fifty times earns roughly once.

**Ships in shadow mode**: compute and store the value, credit nothing. This is
how #168, #169 and #171 all shipped, and it is the only way to calibrate
against real corrections rather than guessing.

The known weakness, recorded rather than solved: novelty rewards unusual text,
not accurate text, so novel nonsense scores well. The verdict gate and shadow
mode bound the damage; a relevance check is the escalation if the shadow data
shows it is needed. Do not add a model-graded value without a further
decision -- a model deciding what a contributor earns is a trust and appeals
problem, not just an engineering one.

---

## S3: recorded-trace timestamps

`from_recorded_trace` gives every event one `created_at`. That is the
identical-timestamp finding, and it is real.

Two things narrow it, and both should be stated when closing the issue:

- The contributor CLI path is unaffected. `claude_code.rs`, `codex.rs` and
  `trajectory.rs` all parse a per-record timestamp from the source file;
  `trajectory.rs` fails closed on an unparseable one.
- The capture path was fixed in #322 on 2026-08-18: `from_capture_turns`
  spends `turn.started_at` on the user message, `turn.completed_at` on the
  assistant message, and derives `latency_ms` from the difference.

So the broken builder serves corpus building -- `pilot_bootstrap/submitter.rs`
and the smoke tooling -- not live contributor traffic. It still matters,
because corpus quality is exactly what a benchmark harness consumes.

### The change

`TraceStep` gains `timestamp: Option<DateTime<Utc>>`, with
`#[serde(default, skip_serializing_if = "Option::is_none")]`.
`from_recorded_trace` uses `step.timestamp.unwrap_or(created_at)`.

A source carrying timestamps produces real per-event ordering. A source
carrying none behaves exactly as today. Nothing is invented -- synthesising
plausible offsets would be fabricating data, which is worse than absence.

**Measure during implementation**: how many of the datasets actually loaded by
pilot-bootstrap carry per-record timestamps. Report the number. If it is near
zero, the field is cheap but the benefit is theoretical, and that is worth
knowing before anyone claims S3 is closed.

---

## S4a: conversation_id

23 traces in the reported corpus open with an `assistant_message` before any
`user_message`. Their shape is canonical, so they are well-formed, but with
`redacted_content` absent a consumer cannot distinguish a product greeting, a
proactive or triggered turn, and a resumed or windowed thread. They are the
same bytes. Those traces need prior conversation state seeded, not a prompt
re-issued.

The envelope gains an optional `conversation_id`, populated by the emitters
from the source session identifier.

- Optional, `#[serde(default)]`, so old envelopes parse unchanged.
- Metadata, not user content: no consent decision, no new flag.
- Attribution only, never authorization -- the repo's standing envelope rule.
  It must not reach any gate, scoring input, or tenant-scoping decision.
- A wire addition, so the server deploys before any client that emits it.

---

## S4b: the payload tier

**No code. Do not open a pull request for this.**

#418 and #419 already made enabling it safe: a marker is no longer counted as
a payload, and a payload-bearing trace with no prose now enrols for the PII
backstop. `include_tool_payloads: true` appears only in tests.

Enabling it is one configuration change with an operational consequence:
`residual_risk` returns Medium for any content flag, so on a deployment that
does not accept medium risk those traces release to `Quarantined`. That queue
holds 48 traces and has never been reviewed.

The prerequisite is an owner for that queue -- drain the backlog, then enable
behind a flag for one tenant. This is the item standing between the reporter
and the 109 traces they identified as one field from usable, and it will not
be moved by a release.

---

---

## S6: the tool payload profiles are too aggressive

Found while reviewing the redactor for over-aggression. This is not in issue
#298 by name, but it decides whether closing S4b would actually deliver
anything.

### What is fine

Two passes were checked and are well-designed; do not "fix" them.

- The contextual-entropy sweep is **cue-gated**: it requires a
  credential-shaped cue before measuring entropy, and carries an identifier
  allowlist. Hashes, UUIDs and git SHAs survive it.
- `redact_known_paths` replaces only the known **prefix** with a stable
  placeholder, so `/Users/zaki/proj/src/main.rs` keeps `/src/main.rs`.
  Structure and repeat-identity both survive.

### What is not

`FILESYSTEM_RULES` (and `BROWSER_RULES` similarly):

```
Contains["path","file","filename","cwd","directory"]  -> Replace("local_path")
Exact["content","contents","command","stdout",
      "stderr","diff","patch"]                        -> Replace("workspace_content")
```

`Replace` is wholesale: the entire value becomes a marker. So a shell call's
`command`, an edit's `diff` or `patch`, a read's `content`, and any tool's
`stdout`/`stderr` are replaced outright.

Note the inconsistency with the general pass above. `redact_known_paths`
carefully preserves everything after the prefix; a filesystem tool's
`file_path` field is blanked entirely. Same data, two treatments, and the
aggressive one wins.

Secondary: `field_matches` uses a plain substring test on the lowercased field
name, so `Contains` over-matches. `profile` contains `file`; `file_count` is a
count, not a path.

### Why this decides S4b

Enabling `include_tool_payloads` would not deliver the 109 traces the reporter
identified as one field from usable, because the fields that make a coding
trace replayable are exactly the ones these rules replace. Contributors would
consent to sharing payloads and the corpus would receive markers.

These rules are latent today only because payloads are never on. Enabling the
tier makes them live. **S4b is therefore gated on this, not only on a
quarantine-queue owner.**

### The change

Re-scope the profiles from **replace by default** to **preserve by default,
redact narrowly**.

- Keep wholesale replacement only where the field is inherently sensitive
  regardless of content: credentials, cookies, auth headers.
- For content-bearing fields (`command`, `diff`, `patch`, `content`,
  `stdout`, `stderr`), run the general redaction passes over the value --
  which already handle paths, emails and secrets well -- instead of
  discarding it.
- Narrow `Contains` to exact names plus an explicit suffix list, so `profile`
  and `file_count` stop matching.

**No measurement backs this section.** No trace has ever been submitted with
payloads enabled, so there is no corpus to check the rules against. This is
read from the rules themselves. Anyone implementing it should build a fixture
from a real coding session, run it through with payloads on, and look at what
survives -- before and after.

---

## Non-goals

- No model-graded correction value. Requires a separate decision.
- No crediting of corrections in 0.5.1. Shadow only.
- No correction box on a successful verdict.
- No synthesised timestamps anywhere.
- No `conversation_id` reaching a gate or scoring input.
- No loosening of secret, credential, cookie or auth-header redaction
  anywhere, including in corrections and in the re-scoped profiles.
