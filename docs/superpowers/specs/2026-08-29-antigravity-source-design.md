> **SUPERSEDED on 2026-08-31** by
> `2026-08-31-antigravity-import-command-design.md`.
>
> This design read Antigravity's SQLite conversation files directly. It was
> fully implemented and reviewed, and it works for single-turn
> conversations. It was abandoned for two reasons the implementation made
> concrete rather than theoretical: it has no ordering signal for user turns,
> so multi-turn conversations are silently mis-ordered; and extracting the
> contributor's prompt required reading a blob that also holds a vendor
> system prompt and the contents of every file the agent read, which
> accumulated four separate leaks and one residual no tag-based parser can
> close.
>
> Kept because its findings about the on-disk format remain the only written
> record of it, and because the reasoning above is worth preserving rather
> than deleting.

# Antigravity as a contributor trace source

Date: 2026-08-29
Status: Approved design, pending implementation plan

## Summary

Add Google Antigravity as a watched trace source in the contributor daemon,
alongside Claude Code and Codex. Antigravity's current build stores each
conversation as an unencrypted SQLite database holding protobuf step
payloads, one file per conversation, so it fits the existing `TraceSource`
model without a new collection mechanism.

It lands as one slice. The source registry an earlier draft of this design
called for already exists on `main`, and `gemini-cli` is already registered
through it, so Antigravity is a module plus a row rather than a refactor
plus a source.

## Motivation

Hackathon contributors — the Devfolio population in particular — use
Antigravity because it has a free plan. They are a trace source we currently
cannot collect from at all, and they are exactly the cohort the pilot needs.

## What is actually on disk

Verified on a macOS install on 2026-08-29, against a conversation created for
the purpose rather than from documentation.

The live store is `~/.gemini/antigravity-ide/`. The IDE's language-server
processes select it with `--app_data_dir antigravity-ide`; the older
`~/.gemini/antigravity/` path is a previous location and is not written by
current builds.

```
~/.gemini/antigravity-ide/conversations/<uuid>.db     SQLite 3.x
  tables: steps, trajectory_meta, trajectory_metadata_blob,
          gen_metadata, executor_metadata, parent_references,
          battle_mode_infos
  steps:  idx, step_type, status, has_subtrajectory, metadata,
          error_details, permissions, task_details, render_info,
          step_payload (blob), step_format
```

`step_payload` is plaintext protobuf. Decoding a captured session yielded
assistant prose, a separate reasoning field, tool calls carrying call id,
tool name and JSON arguments, per-step timestamps, and — from
`trajectory_metadata_blob` — the workspace URI, git repository, remote URL
and branch. A small number of fields hold high-entropy opaque bytes
(consistent with model thought signatures); they are not needed and are
skipped.

Two things are **not** readable and are treated as out of scope:

- Conversations in the previous `.pb` format remain sealed with Electron
  `safeStorage` against the OS keychain. A storage-format change left them
  in place unmigrated; both formats were observed side by side in the same
  directory.
- The opaque per-step signature blobs described above.

## Decisions and rationale

### A watched file source, not the local API

Antigravity's language server exposes a Connect/JSON API on localhost
(`exa.language_server_pb.LanguageServerService`) which community exporters
use, authenticating with a CSRF token taken from the process command line.
It was the design assumption before the on-disk format was examined.

Considered and rejected. It requires the IDE to be running, the token
rotates on every language-server restart and is readable by any same-user
process, the port must be discovered by probing, and no part of it is
documented or committed to by Google — the changelog through August 2026
lists no export feature, local API, or extension API for conversations.
Adopting it would also put process enumeration and local port probing inside
a background service whose entire premise is fail-closed consent.

Reading files the contributor already has needs none of that, works with the
IDE closed, and reuses the discovery, queue, preview, redaction and approval
path that already exists.

### `session_hash` is derived from content, not raw file bytes

Every existing adapter hashes raw file bytes. That contract does not survive
SQLite: page reuse, `VACUUM` and WAL checkpointing all change the bytes
while the conversation is unchanged, so the same session would hash
differently between reads — re-offering sessions already uploaded and
defeating dedup.

This adapter therefore hashes a canonical serialization of the extracted
steps. The `sha256:<hex>` shape and the determinism guarantee are unchanged;
only the preimage differs. This is a deliberate, documented exception to the
storage contract and is pinned by a test that vacuums a database and asserts
the hash is unmoved.

### Read a copy, never the contributor's live database

`load` copies the database and any `-wal` / `-shm` sidecars to a temp
directory and opens the copy read-only. The daemon must never write to a
contributor's Antigravity store, and read-only WAL access against the
original would still want to touch the shared-memory file. The copy also
provides a stable snapshot.

Quiescence is judged over the group: `SessionRef::size_bytes` sums the
database and its sidecars, and `group_modified_at` is the newest mtime among
them. A conversation whose latest turns are still in an uncheckpointed WAL
is not settled, and a ref measuring only the main file would say it was.
This is the same group treatment `claude-code` already applies to subagent
transcripts.

### Two-tier failure model

The repo's convention is fail-closed, and the trajectory reader rejects a
whole file on any malformed record. Applying that verbatim to a vendor
format that changes without notice would discard whole sessions the first
time Antigravity adds a step kind.

- Unrecognized **fields** are skipped. This is the reason the reader walks
  the wire format with `prost::encoding` rather than generating from a
  reverse-engineered `.proto`: a schema pinned by guesswork turns an
  upstream field addition into a parse failure.
- Unrecognized **step types** become `Opaque` events, retained with no
  content, so a new step kind costs one event rather than a session.
- A database that cannot be parsed at all, or a session yielding no
  identifiable user or assistant content, is **refused** with a reason
  label. Reason labels carry no session content, per the hash-only rule.

### The registry already exists; this is one row in it

An earlier draft of this design called for a registry refactor first, on the
belief that the source set was two named fields threaded through settings,
IPC, the C ABI and three shells, and that a third declaration would bounce
every onboarded contributor off the roots screen. That was written against a
stale branch. On `main` the refactor has already landed, and `gemini-cli`
is already a third source using it.

What exists:

```rust
struct SourceSpec { name, conventional_root, build, undeclared }
static NATIVE_SOURCES: &[SourceSpec] = &[ claude-code, codex, gemini-cli ];
pub fn all_sources(roots: &SourceRoots) -> Vec<Box<dyn TraceSource>>
enum Undeclared { Conventional, Nothing }
```

The problem the earlier draft worried about is solved, and solved the way
that draft proposed: a source added after the desktop shells shipped takes
`Undeclared::Nothing`, so an absent declaration constructs no adapter and
scans nothing, and `roots_declared()` is deliberately left as
claude-and-codex with a separate `gemini_declared()` predicate answering
"should a shell offer to ask" rather than "may the daemon start".

Antigravity therefore takes `Undeclared::Nothing` for the same reason, and
`gemini-cli` is the precedent to copy throughout — module shape, settings
field, `source_settings_key` arm, declared-predicate, discovery row.

Not addressed here: the C ABI header still exists in two hand-synced copies
that nothing checks. Adding a settings key does not change the ABI itself,
so fixing that is unrelated cleanup and stays out of this slice.

### Dependencies

`rusqlite` 0.40.2 with `bundled`, and `prost` 0.13.5 (already on the
approved list and already in the lockfile). Added in their own commit with
the rationale recorded there. `bundled` compiles SQLite from vendored C
because Windows has no dependable system `libsqlite3` and the named-pipe ACL
job is the only control behind the daemon's IPC there; the cost is borne by
the flatpak offline vendor set and the GTK workspace lockfile, both updated
in that commit.

## Architecture

Four units, each testable alone, in `source/antigravity/`:

- **`discover`** — enumerate `<uuid>.db` under the declared root, build
  `SessionRef`s including sidecar group size and mtime. Skips `.pb`,
  `-wal`, `-shm`. Shares one ref-construction function with `session_at`,
  as the trait requires.

  **Discovery and addressing are different questions.** `discover` refuses
  sidecars as sessions, but `session_for_path` must MAP `<name>.db-wal` and
  `<name>.db-shm` to `<name>.db`. Antigravity's newest turns land in the
  WAL, so the filesystem event fires on the sidecar; a `session_for_path`
  that refused it would return `None`, no scoped scan would run, and the
  change would wait for the slow reconciliation sweep. This is the same
  shape as claude-code mapping a transcript under `<uuid>/subagents/` to
  its parent, which the trait's doc comment names as the precedent. Every
  containment rule still applies to the mapped parent.
- **`store`** — snapshot to temp, open read-only, read `steps`,
  `trajectory_meta` and `trajectory_metadata_blob`. Knows SQLite, not
  protobuf.
- **`decode`** — wire-format walking over a step payload. Pure, no I/O.
- **`convert`** — decoded fields to `SessionEvent` / `SessionTranscript`.
  Pure, table-testable, holds the mapping and the refusal rules.

`AntigravitySource` composes them behind `TraceSource` and registers as one
`SourceSpec` row in `NATIVE_SOURCES` with `Undeclared::Nothing`. Nothing
downstream changes: the queue, preview, redaction, approval hold, uploader
and envelope are untouched, and `"antigravity"` passes
`validate_source_name` as-is.

## Event mapping

| Source | Becomes |
|---|---|
| the model turn's tool-call submessage (call id, tool name, JSON arguments, no output) | `ToolCall` with `tool_call_id`, `tool_name`, `structured` |
| the following step's submessage (same call id and arguments, plus the tool's output) | `ToolResult`, paired to its call by call id |
| payload assistant-text field | `Assistant` |
| payload reasoning field | `Reasoning` |
| per-step timestamps | `SessionEvent::timestamp` |
| `trajectory_metadata_blob` workspace URI | `cwd` (never serialized) |
| `trajectory_metadata_blob` repo / remote / branch | `project`, `--project` scoping |
| `trajectory_meta.trajectory_id` | `conversation_id` (attribution only) |
| `gen_metadata` `<USER_REQUEST>` span, and nothing else from that row | `User` |

Step-type numbers are deliberately not enumerated here. They are an
unpublished vendor enum observed from one capture; the plan resolves them
against the fixture and records them in code beside the mapping, where a
drift is visible.

## Consent and onboarding

New installs see a third card on the roots screen, described by
`discovery::probe` the way the existing two are — path, whether it exists,
session count, most recent mtime — so the contributor agrees to something
specific.

Existing installs see nothing new at startup. Settings gains a row, and
where discovery finds a non-empty Antigravity store the shell may surface it
once as an offer, never as a block.

**The two Gemini stores share a parent, and that is an open discovery
question.** `gemini-cli` watches `<gemini home>/tmp`, where the home is
`~/.gemini` unless `GEMINI_CLI_HOME` overrides it. Antigravity's store is
`~/.gemini/antigravity-ide/conversations` — a sibling subtree, so the two
adapters never collect the same file. But Antigravity selects its store
with `--app_data_dir` *relative to that same home*, so `GEMINI_CLI_HOME`
plausibly relocates both. This has not been verified. Until it is,
Antigravity's conventional root is computed from the home directory alone;
if a check against the real binary confirms the override applies, the root
should be derived from `gemini_cli::conventional_root`'s home resolution so
the two cannot disagree about where the Gemini home is.

Beyond that, discovery ships **no environment-variable override** for
Antigravity.
`CLAUDE_CONFIG_DIR` and `CODEX_HOME` were each verified against the real
binaries. Antigravity was observed selecting its store with an
`--app_data_dir` process flag; no environment override has been verified to
exist, and this module's rule is to fall back to the conventional location
rather than invent a second guess. If one is later confirmed against the
real binary, it can be added the same way the other two were.

## Testing

The fixture is a real Antigravity-produced database captured from a
throwaway conversation in a scratch repository — not a synthesized file.
A fixture authored alongside the reader proves only that they agree.
It lives beside the existing `tests/fixtures/letta-conformance/` precedent.

- Hash stability: read, `VACUUM` a copy, read again, assert the
  `session_hash` is identical. Fails under the raw-bytes contract, which is
  the point.
- Wire-walker tolerance: unknown field number still parses; unknown
  `step_type` yields one `Opaque` event, not a lost session.
- Refusals: unparseable database; a session with no identifiable user or
  assistant content; and a `gen_metadata` row carrying no
  `<USER_REQUEST>` wrapper. Reason labels carry no content.
- **Nothing but the tagged span escapes the prompt blob**: given a
  `gen_metadata` row holding a distinctive marker outside the
  `<USER_REQUEST>` wrapper, no event, no transcript field and no hash
  preimage contains that marker. This is the test that keeps a vendor
  system prompt and injected file contents out of a contributed trace.
- Path safety: `session_for_path` refuses `.pb`, `-wal`, `-shm`, symlinked
  members and `..` traversal, through `real_file_within_root`.
- Quiescence: a database with a large uncheckpointed `-wal` is not eligible.
- **Undeclared constructs nothing**: `all_sources` over a `SourceRoots` with
  no Antigravity declaration yields no Antigravity adapter, and in
  particular none rooted at the contributor's real `~/.gemini`. This is the
  `Undeclared::Nothing` property, pinned the way the equivalent
  `gemini-cli` test pins it; if it fails, an upgrade silently starts
  scanning a directory nobody agreed to.

Swift `SourceKind` changes are gated: `swift test` runs in CI under the
`macOS app tests` job on `macos-26`. The GTK roots screen is covered by that
crate's own manifest, which CI checks explicitly.

## Non-goals

- Decrypting legacy `.pb` conversations.
- The local HTTP API, process enumeration, or CSRF-token discovery.
- The `agy` CLI as a source. Different surface, different capture model,
  its own decision.
- GitHub Copilot, in any of its surfaces.
- Server or protocol changes. `source` reaches the envelope as a free
  string.
- Recovering conversations Antigravity's own index has lost.

### User turns come from the prompt blob, and only the tagged span

Resolved 2026-08-29 against a capture, replacing the open question this
section previously carried.

User turns are not in `steps`. They are in `gen_metadata`, inside the
serialized model input, wrapped as
`<USER_REQUEST>\n...\n</USER_REQUEST>`. In a 23-step capture that table
held nine ~1 KB generation configs and one row of 258,595 bytes carrying
the whole assembled input: the vendor's system identity prompt, every
tool's JSON schema, the skills and plugins listing, and the contents of
every file the agent read, injected as context.

The reader therefore:

- extracts **only** the `<USER_REQUEST>` span, and retains nothing else
  from that row;
- never feeds the surrounding blob to the content hash, the transcript, or
  the redactor — it is read, scanned for the span, and dropped;
- **refuses the session** with `antigravity-user-turn-unreadable` when a
  `gen_metadata` row exists but carries no `<USER_REQUEST>` wrapper.

That last rule is the important one. A renamed wrapper would otherwise
yield sessions that parse cleanly, look complete, and silently contain no
human turns — the failure fail-closed design exists to prevent. An absent
wrapper is a refusal, never an empty result.

Considered and rejected: collecting Antigravity traces without user turns
at all. It avoids the blob entirely, but a transcript of agent output with
no prompt is materially weaker evidence of contribution, and the
scorecard work already established what unbounded partial traces do to
credit.

Note for the reader's author: the injected file contents in that blob can
include files the contributor never opened in the conversation. Nothing
downstream should ever see them.

### User turns are front-loaded, and multi-turn ordering is unresolved

`User` events are emitted in row order before every step-derived event,
because the prompt blob records no timestamp and inventing one from a
neighbouring step would fabricate ordering data the source does not carry.

For the single-turn conversations this design was validated against, that
is correct. For a multi-turn conversation it is not: the transcript would
read as every user turn followed by every assistant and tool event, losing
the interleaving that makes a trace a conversation. Nothing in the current
fixture detects this, because it holds one turn.

The likely fix is to interleave by `gen_metadata` row index — row N is
generation N, so its turn precedes the steps that generation produced —
but the correspondence between a row and a step range has not been
observed, and guessing it would be the same fabrication the `timestamp:
None` rule avoids.

**A multi-turn capture is therefore a prerequisite for claiming multi-turn
support.** Until one exists, this adapter is honest about single-turn
conversations and silently wrong about the ordering of multi-turn ones.
That is a real limitation, not a nicety: it is the kind of distortion that
would make a contributed trace misrepresent how the work actually
happened.

## Open questions

- Whether `GEMINI_CLI_HOME` relocates Antigravity's store as well as the
  CLI's. See the consent section; unverified, and the conventional root is
  computed from the home directory alone until it is.
- Whether the `<USER_REQUEST>` wrapper is stable across Antigravity
  releases. It is a prompt template, not a schema, so it is the most
  likely thing in this design to break. The refusal rule is what makes
  that break loud.

## Slice

One deliverable, because the groundwork it would have stacked on is already
merged: the four units above, the registration row, the settings field and
its predicate, the discovery row, the shell display strings, and the
fixture.

The gating question that shaped this design — where user turns live — was
settled before implementation and is recorded above.
