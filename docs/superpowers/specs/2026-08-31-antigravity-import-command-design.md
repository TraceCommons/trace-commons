# Antigravity import via the local language server API

Date: 2026-08-31
Status: Approved design, pending implementation plan
Supersedes: `2026-08-29-antigravity-source-design.md` (file-reading approach, abandoned)

## Summary

Collect Google Antigravity conversations with a one-shot command,
`trace-commons-contributor import-antigravity`, which reads them from the
IDE's local language server API while Antigravity is running, materializes
them into a Trace-Commons-owned staging directory, and hands them to the
existing queue, preview, redaction and approval path.

This replaces an earlier design that read Antigravity's SQLite conversation
files directly. That approach worked and was fully implemented, but it could
not order multi-turn conversations, and it required scraping the
contributor's prompt out of a blob that also held a vendor system prompt and
the contents of every file the agent had read.

## Motivation

Hackathon contributors — the Devfolio cohort in particular — use Antigravity
because it has a free plan. Multi-turn conversations are a core case, not an
edge one: a trace whose turns are mis-ordered misrepresents how the work
actually happened, and this corpus is the product.

## Why the API and not the files

Both paths were built far enough to compare. The file path is implemented,
reviewed and passing; this is not a decision made from a distance.

**Ordering.** The API returns steps in conversation order with
`CORTEX_STEP_TYPE_USER_INPUT` as a first-class step carrying
`metadata.createdAt`. The file path has no ordering signal for user turns at
all — they live only inside the serialized model input — so it front-loaded
them, which is correct for single-turn conversations and silently wrong for
every other kind.

**Privacy.** The file path had to locate the contributor's prompt inside
`gen_metadata`, a blob that also carries the vendor's system identity
prompt, every tool's JSON schema, and the contents of files the agent read.
That extraction accumulated four distinct leaks during development and ended
with one residual no tag-based parser can close. The API response carries
none of that material: a search of the full 99 KB capture for the vendor
identity prompt, the tool schemas, the skills listing and the
`<USER_REQUEST>` wrapper returns zero occurrences. The problem is not fixed;
it ceases to exist.

**Schema.** The file path read protobuf field numbers derived from a single
capture of an unpublished schema. The API returns named fields and named
enum values.

**Coverage.** The API also serves the legacy `.pb` conversations, which are
sealed with the OS keychain and permanently unreadable from disk. An
Antigravity user's entire pre-AGY-2.0 history is invisible to the file path
and collectable through the API.

Considered and rejected: keeping the file reader as a fallback for
conversations the running instance has not indexed. It preserves the
multi-turn defect, keeps ~1,400 lines alive, and means two readers producing
different transcripts — and different `session_hash` values — for one
conversation, which would upload it twice and count it twice in the credit
pipeline.

## The RPC surface

Not documented by Google and not present in the shipped IDE bundles; read
from the language server binary's own symbols. The methods named by existing
community exporters are stale for current builds.

```
LanguageServerService/GetAllCascadeTrajectories       -> the listing
LanguageServerService/GetCascadeTrajectorySteps       -> the conversation
LanguageServerService/GetCascadeTrajectory
LanguageServerService/ConvertTrajectoryToMarkdown     -> vestigial; always "not found"
```

Two identifiers matter and are easy to confuse. A conversation's **file name
is its cascade id**; the `trajectory_id` recorded inside it is a different
UUID. `GetCascadeTrajectorySteps` takes `{"cascadeId": "..."}`. Passing the
trajectory id, or passing either as `trajectoryId`, returns a generic
`trajectory not found` — the same error an empty request produces, which is
why the wrong identifier is not self-diagnosing.

Transport is Connect over HTTP with JSON bodies. Authentication is a CSRF
token in the `x-codeium-csrf-token` header.

## Decisions and rationale

### A one-shot command, not a daemon source

The API exists only while Antigravity is running, its token rotates on every
language-server restart, and finding it requires reading another process's
command line. Those properties are disqualifying for a background service
whose premise is fail-closed consent, and acceptable in a command a person
types.

The daemon therefore gains no new capability: it never enumerates processes,
never opens a socket to Antigravity, and cannot import anything on its own.

### The endpoint is found by a bounded, positively-identified probe

`sysinfo` supplies process command lines, which is where `--csrf_token`
lives. It does not supply per-process listening sockets, and the API port is
not on the command line — `--extension_server_port` is a different port.
Observed offsets between them span +1 to +27 across three instances, so the
port cannot be computed.

The command therefore probes `extension_server_port + 1 ..= +64` on
loopback and identifies the API positively rather than by guessing: the
right port answers a Connect request with
`{"code":"unauthenticated","message":"missing CSRF token"}` and then
authenticates with *that process's own* token. The token match is what
proves the intended process was reached.

Refusal is the failure mode. A port outside the window yields
`antigravity-api-not-found`, never a request to something else.

Considered and rejected: a second crate for per-process socket enumeration
(exactness at the price of another dependency), and shelling out to `lsof`
(exactness at the price of a subprocess surface this crate has previously
refused to add).

### Staging, because the queue re-reads at upload time

The queue addresses a session by path and re-reads it when it uploads, which
may be long after the IDE has closed. Each imported conversation is
therefore written into a Trace-Commons-owned staging directory as
Trajectory-v1 JSON.

**The existing `trajectory` source reads that directory.** The command does
not enqueue directly and does not add a fourth native adapter: it produces
files in a format this crate already parses, validates and fails closed on,
and the daemon's existing trajectory reader discovers them. `meta.source` is
`"antigravity"`, which passes `validate_source_name` unchanged, so the
provenance travels correctly without a new source registration.

Two consequences worth stating, both simplifications:

- **No content-derived hash exception.** The staged artifact is an ordinary
  immutable JSON file, so `session_hash` is sha256 over its bytes exactly as
  every other adapter computes it. The SQLite design needed a documented
  departure from that contract because page reuse and WAL checkpointing move
  a file's bytes without changing a message. That departure is no longer
  needed and is not carried forward.
- **No registration, settings field, or roots-screen row.** Those exist so a
  contributor can consent to a *watched directory*. A command they type is
  the consent, and the staging directory is ours rather than theirs. The
  `SourceSpec` row, the `antigravity_source` settings field, the
  `antigravity_declared` predicate, the `discovery::probe` entry and the
  Swift and GTK rows all go with the file reader.

### `thinkingSignature` is never carried

`toolCalls[].thinkingSignature` is an opaque encrypted blob of model
internals. It reaches no event, no transcript field, and no hash preimage,
and a test asserts its absence.

### Dependency

`sysinfo = "0.36"`, pinned to the 0.36.1 already present in this workspace's
lockfile, so no new package version enters the tree. **Not** the current
0.39.6, which requires Rust 1.95 against this workspace's 1.92 floor. MIT,
192M downloads. It becomes a real runtime dependency of the shipped
contributor binary — it is currently reachable only through
`mistralrs-core` under a server feature — and brings `ntapi` and `windows`
on Windows and the `objc2-*` crates on macOS. Those land in the flatpak
offline vendor set, which must be regenerated in the same change.

`rusqlite` and `prost` are removed: nothing reads the files any more.

## Architecture

Four units, each testable alone. They live under `antigravity/` beside the
command rather than under `source/`, because this is not a `TraceSource`
implementation — the `trajectory` source is what the daemon sees.

- **`endpoint`** — the only unit that touches `sysinfo` or performs
  discovery. Enumerates `language_server_*` processes, reads
  `--csrf_token` and `--extension_server_port`, probes the bounded window,
  and returns `Endpoint { port, token }` or a typed refusal. Knows nothing
  about trajectories.
- **`client`** — one struct over an `Endpoint`; `list_trajectories()` and
  `fetch_steps(cascade_id)`. Speaks Connect JSON. A trait, so everything
  above it is testable against recorded responses with no IDE.
- **`convert`** — pure function from the API's JSON to `SessionTranscript`.
  No I/O. Holds the mapping and the refusal rules.
- **`commands::import_antigravity`** — orchestration only: discover, list,
  filter by project, fetch, convert, stage, hand off.

## Event mapping

| API | Becomes |
|---|---|
| `USER_INPUT` → `userInput.userResponse` | `User` |
| `PLANNER_RESPONSE` → `toolCalls[] {id, name, argumentsJson}` | `ToolCall` with `tool_call_id` |
| `PLANNER_RESPONSE` → `response` | `Assistant` |
| `PLANNER_RESPONSE` → `thinking` | `Reasoning` |
| `LIST_DIRECTORY`, `VIEW_FILE`, `RUN_COMMAND`, and siblings | `ToolResult` |
| `SYSTEM_MESSAGE`, `GENERIC`, `CHECKPOINT`, `CONVERSATION_HISTORY`, `KNOWLEDGE_ARTIFACTS`, and any step type not listed above | dropped |
| `metadata.createdAt` | `SessionEvent::timestamp` |
| `metadata.generatorModel` | transcript `model` |
| `trajectoryScope.workspaceUri` | `cwd`, `project` |
| cascade id | `conversation_id` |

The step-type enum is wider than any single capture shows: the single-turn
capture exposed seven values, and adding one turn revealed three more
(`RUN_COMMAND`, `SYSTEM_MESSAGE`, `GENERIC`). An unrecognised
`CORTEX_STEP_TYPE_*` must therefore never fail a conversation, because the
next capture will contain a value this one does not.

It is **dropped**, not carried as an opaque event. An earlier draft of this
spec said `Opaque`, which was carried over from the file-reading design that
mapped straight onto `SessionEvent`. Dropping applies only to step types
that carry no contributor-visible content: a step type later found to carry
real tool output is mapped, not dropped — `CORTEX_STEP_TYPE_GENERIC` turned
out to be the agent's own `manage_task` tool with real results, and is
mapped as a tool result rather than discarded. This design writes Trajectory-v1, whose
five roles — `meta`, `user`, `reasoning`, `assistant`, `tool` — have no way
to express an opaque one: a sixth role would be rejected by the reader, and
an empty `assistant` record would put a turn in the transcript that never
happened. Dropping keeps the property that matters, which is that a step
kind Google adds later costs one step rather than a session.

## Consent and project scoping

`GetAllCascadeTrajectories` returns a map keyed by CASCADE id — the same
identifier `GetCascadeTrajectorySteps` takes — carrying `summary`,
`stepCount`, `status`, `createdTime`, `lastModifiedTime`,
`lastUserInputStepIndex`, `trajectoryId`, and `workspaces[]` with
`workspaceFolderAbsoluteUri` and `gitRootAbsoluteUri`, for every
conversation *before* any is fetched.

An earlier draft named `GetUserTrajectoryDescriptions` as the listing. That
was wrong and it matters: that endpoint returns a *user* trajectory, a
different concept, whose `trajectoryId` cannot be used to fetch steps under
either field name. The listing that works is keyed by exactly the id the
fetch consumes, so no mapping step is needed at all. `--project` therefore filters at listing time, and the command
fetches only what the contributor scoped it to. This is strictly better than
the file path, which could not know a conversation's workspace without
opening it.

Imported conversations enter the queue as pending entries. The contributor
approves each one with the normal preview, so the existing consent gate is
unchanged.

## Testing

The fixture is the captured `GetCascadeTrajectorySteps` response, redacted
as the database fixture was: the operator username is scrubbed, and
`thinkingSignature` values are stripped. `client` being a trait means
`convert` runs against it in CI with no IDE. Only `endpoint` needs a live
instance; that test self-skips loudly, as the non-UTF-8 path test does.

Required properties:

- No `thinkingSignature`, and no vendor system-prompt marker, reaches any
  event, transcript field or hash preimage.
- An unrecognised `CORTEX_STEP_TYPE_*` becomes one `Opaque` event, never a
  failed conversation.
- A multi-turn capture produces `User` events interleaved in conversation
  order, each with a real timestamp.
- `--project` filters at listing time and fetches nothing outside it.
- The endpoint probe refuses rather than guessing when no port in the window
  authenticates with the process's own token.

## Non-goals

- Any reading of Antigravity's conversation files. The `.db` reader and the
  `.pb` decryption question are both out.
- A daemon-driven Antigravity source.
- The `agy` CLI as a separate source.
- Server or protocol changes.

## Multi-turn is a first-class requirement

Multi-turn conversations are the core case, not a variant to support later.
A trace whose turns are mis-ordered, deduplicated, or collapsed
misrepresents how the work happened, and this corpus is the product. The
whole reason this design replaced the file-reading one is that the file path
could not order them.

Concretely, the implementation must satisfy all of these, each pinned by a
test against a real multi-turn capture:

- Every `CORTEX_STEP_TYPE_USER_INPUT` step becomes its own `User` event.
  N user turns in, N `User` events out — never collapsed, never joined.
- `User` events appear **interleaved in conversation order**, each between
  the agent work that preceded and followed it. Not front-loaded, not
  reordered, not sorted by anything other than the API's own step order.
- Every `User` event carries the real `metadata.createdAt` timestamp of its
  step. No `None`, and nothing inherited from a neighbouring step.
- The event sequence round-trips: reading the staged Trajectory-v1 file back
  yields the same turn count and the same order as the API returned.
- A conversation that gains turns after being imported produces a different
  `session_hash`, so the extended conversation is offered again rather than
  suppressed as a duplicate.

**This is a merge gate, not a follow-up.** If a multi-turn capture shows the
API does not order or timestamp turns as this design assumes, the design
returns for revision rather than shipping with a documented limitation. The
previous design shipped exactly that limitation and it is why this one
exists.

### Verified 2026-08-31 against a real two-turn capture

A second turn was added to a live conversation and the API re-read. 48
steps, up from 23:

```
idx  0  CORTEX_STEP_TYPE_USER_INPUT  createdAt=2026-08-29T10:13:36Z
idx 23  CORTEX_STEP_TYPE_USER_INPUT  createdAt=2026-08-31T22:39:28Z
```

Two turns produce two `USER_INPUT` steps, interleaved at 0 and 23 rather
than grouped, with the first turn's 23 steps intact between them. Each
carries its own real timestamp, two days apart, matching when each message
was actually sent. All 48 steps are timestamped and the sequence is
monotonic. The step count moving 23 to 48 means an extended conversation
necessarily re-hashes.

The privacy result holds at the larger size: zero occurrences of the vendor
identity prompt, the skills listing, or the `<USER_REQUEST>` wrapper across
all 212 KB.

The gate is passed on evidence, not construction.

## Open questions
- ~~Whether tool results carry a link back to their call id.~~ **Answered
  2026-09-01: they do**, at `metadata.toolCall.id`, and every result in both
  captures resolves against an announced call. Positional matching is kept
  only as a fallback. `metadata.sourceTrajectoryStepInfo` decodes to
  `{trajectoryId, cascadeId, stepIndex, metadataIndex}` — the step's own
  address, not a call link.
- Whether `GetUserTrajectoryDescriptions` lists trajectories from workspaces
  that are not currently open. The observed listing returned only the
  current workspace, which would bound what a single import can reach.
