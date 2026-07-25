# Letta Trajectory support (contributor side)

Date: 2026-07-25
Status: Approved design, pending implementation plan

## Summary

Add support for the [Letta Trajectory](https://www.letta.com/blog/trajectory)
v1 format to the contributor CLI, so sessions from any coding harness Letta
adapts can be contributed to Trace Commons. As a cross-cutting change, reverse
the v1 posture that dropped model reasoning: reasoning becomes a first-class,
distinguishable event type across every adapter.

Trajectory is Apache-2.0, published as `@letta-ai/trajectory`, and normalizes
transcripts from Claude Code, Codex, Hermes, Letta Code, OpenClaw, OpenHands,
Pi, and Deep Agents into one validated record format. The upstream schema is
`schema/trajectory-v1.schema.json` (`https://letta.ai/schemas/trajectory/v1.json`).

## Motivation

The corpus is currently limited to the two harnesses we hand-wrote adapters
for. Trajectory's eight adapters are the actual asset: consuming a versioned
standard makes every one of those harnesses a potential trace source without
this repo taking on the maintenance of tracking eight upstream session formats.

## Decisions and rationale

### Contributor side, not server side

Considered and rejected: a server ingest endpoint accepting Trajectory arrays.

The envelope is not the friction in integrating with this server. A third party
still needs enrollment, an upload claim, a signing key, consent scopes, and a
revocation handle; Trajectory supplies none of those. Server-side ingest would
require the server to synthesize consent and attribution the contributor never
signed, and would put un-redacted harness text on the server, inverting the
client-side-redaction threat model. The PII backstop (PR #166) is
defense-in-depth and ships disabled; it is not a substitute for the primary
control.

A separate, still-open question was explicitly deferred: expressing the event
list *inside* an already-signed envelope as Trajectory records. That keeps the
trust boundary intact but perturbs canonical representation, so it is not part
of this slice.

### Read trajectory files in Rust; do not bridge to Node

The contributor CLI stays a standalone Rust binary with no external runtime
dependency. Trajectory v1 is small — five record types, roughly 100 lines of
JSON schema — so a native reader is cheap. Contributors using the six harnesses
we lack native adapters for run `npx @letta-ai/trajectory` themselves and hand
us the output: a documented two-step, not a runtime dependency.

The format is stable and versioned; the *adapters* are what churn. Consuming
the format and letting Letta absorb adapter churn is the whole point.

Considered and rejected: shelling out to `npx` when available (adds a
subprocess surface to a privacy-sensitive CLI for a soft dependency), and
porting six adapters to Rust (permanently owns exactly the maintenance burden
this change is meant to avoid).

### Keep the existing native adapters

`claude_code.rs` and `codex.rs` stay as-is and remain the zero-friction default
for those two harnesses. Trajectory is the universal on-ramp for everything
else. Routing claude-code and codex through Trajectory instead would change
event extraction for the two sources the pilot already uses, perturbing
canonical text and dedup comparisons against already-scored traces for no
corresponding gain.

### Capture reasoning, and keep it distinguishable

Reverses the stated v1 posture in `claude_code.rs` ("v1 privacy posture
excludes model reasoning traces from the transcript entirely") and the
equivalent `Opaque` mapping in `codex.rs`.

Reasoning is the highest-value part of a trace for training. It is also the
least sanitized: thinking blocks routinely quote file contents verbatim,
restate secrets the model just read, and speculate about the user — content
that never reaches the assistant's final message. The client-side redactor runs
over reasoning exactly as it does over other text, but it was tuned against
user/assistant/tool text, not chain-of-thought. This risk was raised and
accepted.

Reasoning gets its own `TraceContributionEventType::Reasoning` rather than
being folded into `AssistantMessage` with a marker. The value of capturing
reasoning is that downstream consumers — including the corpus map — can tell it
apart from assistant prose.

## Design

### Components

Three changes in the contributor crate, plus one protocol addition:

1. `crates/trace-commons-contributor/src/source/trajectory.rs` — new
   `TrajectorySource` implementing the existing `TraceSource` trait.
2. `SessionEventKind::Reasoning` (contributor) and
   `TraceContributionEventType::Reasoning` (protocol), wired through both
   existing adapters.
3. `SessionRef.source` and `SessionTranscript.source` widened from
   `&'static str` to `Cow<'static, str>`.

### Record mapping

| Trajectory record | `SessionEvent` |
| --- | --- |
| `meta` | Not an event. Populates `SessionTranscript`: `source`, `model`, `cwd`, and `project` (basename of `cwd`). |
| `user` | `User` — `content`, `timestamp`. |
| `reasoning` | `Reasoning` — `content`, `timestamp`. |
| `assistant` with `content` | `Assistant` — `content`, `timestamp`. |
| `assistant` with `tool_calls` | One `ToolCall` per entry: `name` to `tool_name`, `args` parsed to `structured`, `timestamp` from the record. |
| `tool` | `ToolResult` — `content`, `timestamp`, correlated by `tool_call_id`. |

Notes:

- `meta.git_branch` is dropped. It has no home in `SessionTranscript` and is
  identity-adjacent.
- `meta.cwd` maps to `SessionTranscript.cwd`, which feeds the redactor's
  path-prefix stripping and is **never serialized**. This invariant is
  preserved unchanged.
- `token_counts` is always `None`; Trajectory carries no usage data.
- `started_at` is the timestamp of the first conversational record.
- `session_hash` is sha256 over the raw file bytes, identical to every other
  source, so `submission_id_for()` stays deterministic.
- The schema guarantees `assistant` records carry either `content` or
  `tool_calls`, never both.

### Provenance

`Cow<'static, str>` lets the native adapters keep their `&'static` constants at
zero cost while Trajectory carries the file's own `meta.source`, preserving
per-harness attribution for exactly the harnesses this change unlocks.

Verified safe end to end: `transcript.source` flows only into
`feature_flags["agent"]` and the local receipt, both free-form strings. Ingest
performs no enum validation, so unknown harness names are not rejected.

Because `meta.source` is untrusted file input landing in a provenance field, it
is validated on read: non-empty, `[a-z0-9-]` only, maximum 64 characters.
Violations reject the file.

### CLI surface

- `--trajectory <file|dir>` names the input. A directory enumerates `*.json`
  and `*.jsonl` non-recursively.
- `discover()` returns empty without an explicit path, so trajectory files
  never appear unbidden in the interactive session picker.
- `--no-reasoning` drops reasoning events before envelope construction, for
  per-run opt-out. Reasoning is included by default.

### Error handling

Fails closed, label-only, per the repo's hash-only logging rule. Any of the
following rejects the **whole file** with a reason label — never a partial
submission, and never the offending content in the error string:

- Malformed JSON, or a record matching no schema variant.
- A timestamp failing the schema's ISO-8601 pattern.
- A `tool` record whose `tool_call_id` matches no preceding `tool_call`.
- A `meta.source` failing charset/length validation.
- A missing or non-leading `meta` record.

### Testing

TDD, against sanitized fixtures committed to the repo:

- One fixture per record type, asserting the mapping table above.
- Full round-trip: trajectory file to signed envelope.
- Orphaned `tool_call_id` rejection.
- `meta.source` injection and over-length rejection.
- `session_hash` and `submission_id_for` determinism.
- Reasoning survives redaction end to end in all three adapters.
- `cwd` never appears in any serialized output.

## Consequences

### Canonical representation shifts for new traces

`canonical_whole_trace_representation` renders `{:?}` of `event_type` and
truncates at the first 12 events. Reasoning events will occupy slots in that
window, so canonical text for traces submitted after this change is not
strictly comparable to traces scored before it.

Existing scored traces are unaffected — their stored events do not change, and
no re-score is required. But this creates a cohort boundary: novelty and
dedup comparisons that span it are not like-for-like. Two follow-ups:

- Document the boundary wherever pilot scores are analyzed.
- Re-check `gate-calibrate` floors against a post-change sample, since the
  shape of novelty inputs changes.

### Out of scope

- Server-side Trajectory ingest.
- Trajectory records as the in-envelope event payload.
- Corpus export in Trajectory format.
- Adopting `trajectory-canonical-v1` hashing internally.
- Native Rust adapters for the six harnesses Trajectory covers.

## References

- Upstream repo: `letta-ai/trajectory` (Apache-2.0)
- `schema/trajectory-v1.schema.json`
- `schema/trajectory-canonical-v1.schema.json` (ingestion-side; not used here)
