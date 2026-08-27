# Hackathon onboarding friction: Gemini CLI, one-step submit, deep-link enrollment

Date: 2026-08-27
Status: Approved design, pending implementation plan

## Summary

Four slices answering Devfolio's third round of hackathon feedback. Hackers
submit under deadline pressure; every extra step is a drop-off. The feedback
names three problems: unsupported agent CLIs, a multi-command submit sequence,
and token handling in the Devfolio integration.

1. A native Gemini CLI source adapter.
2. Bounded auto-discovery of Letta Trajectory files.
3. `submit` collapsed to one step.
4. `tracecommons://` deep-link enrollment, plus the documented seam Devfolio's
   verifier API needs.

Each slice is independent and gets its own implementation plan and PR.

## Motivation

Devfolio's feedback, in their words: hackers "submit projects under time
pressure at the end, and if they feel the process is complex, they will drop
off." Two concrete asks, and one item they own.

### The Gemini CLI advice was a dead end

Hackers asked to use Gemini CLI and were pointed at the Letta Trajectory
integration docs. Trajectory has no Gemini CLI adapter — it covers Claude Code,
Codex, Hermes, Letta Code, OpenClaw, OpenHands, Pi, and Deep Agents
(`2026-07-25-letta-trajectory-support-design.md`). Those hackers were not
facing "too many additional steps"; they were following instructions that could
not have worked. Smoothing the Trajectory step does not fix Gemini CLI. Only a
native adapter does.

### The submit sequence is four commands

Install, `login --invite <url>`, `submit --dry-run --since 7d`, `submit --since
7d` — and bare `submit` then prints a table and waits for an index selection
(`commands.rs:845-867`). Devfolio contrasts this with Paxel's two commands: one
for all repos, one for the project in the current directory.

## Decisions and rationale

### Native Gemini adapter, and auto-discover Trajectory files

Considered and rejected: contributing a Gemini adapter upstream to Letta. That
is the right long-term home, but it puts the fix behind another project's
release cycle for the one harness hackers are asking for today.

Trajectory auto-discovery is a deliberate reversal of the v1 posture that
trajectory files are invisible without an explicit `--trajectory`. That posture
exists so a stray `session.json` never joins a submission. The reversal is
bounded rather than total: see "Where auto-discovery looks".

Considered and rejected: shelling out to `npx @letta-ai/trajectory` when
available. The Trajectory design rejected this already — it adds a subprocess
surface to a privacy-sensitive CLI for a soft dependency — and nothing here
changes that reasoning.

### Tolerant parsing for Gemini, not fail-closed

`gemini_cli.rs` maps unknown message types to `Opaque` with a type marker,
following `claude_code.rs:1044`, rather than rejecting the whole file the way
`trajectory.rs` does.

The two postures are not in tension. Trajectory is a versioned schema with a
published conformance corpus, so an unrecognized record means the file is not
what it claims. Gemini CLI's session format is unversioned and evolving; a
strict parser would reject every session the moment upstream adds a message
type, silently costing the corpus exactly the traces this slice exists to
collect.

This is a departure from the repo's fail-closed default, and is scoped to
message-type dispatch only. It does not extend to path containment, byte
budgets, or anything a gate depends on — those stay fail-closed.

### Subagent sessions ship standalone

Gemini writes `kind: "subagent"` sessions as separate files carrying no
back-reference to a parent. Verified against real data: the sole subagent
session's `sessionId` appears in no other session file. Claude Code's merge
strategy (`claude_code.rs:446-531`) is therefore not available, and dropping
them would discard real work. They are offered as their own sessions.

### `submit` itself becomes one-step; no new verb

Considered and rejected: a new `contribute` verb pair mirroring Paxel. It reads
well but adds a second command that does what `submit` already does, and every
existing doc, script, and collector integration would have to explain which to
use.

`--json` is frozen. `docs/collector-integration.md` scripts against it, and a
collector driving this CLI programmatically must not acquire an interactive
prompt or an auto-enroll path in a point release.

## Design

### Slice A: `source/gemini_cli.rs`

New `TraceSource` implementation. `SOURCE_GEMINI_CLI = "gemini-cli"` beside the
existing constants at `source/mod.rs:46-48`.

**Store.** `~/.gemini/tmp/<project>/chats/session-*.json`, with the root
overridden by `GEMINI_CLI_HOME` (confirmed present in the shipped binary).
One file is one session: a single JSON document, not JSONL.

**Discovery.** Non-recursive `read_dir` of `<root>/*/chats/`, matching
`session-*.json`. Symlinks refused, as in `discovery.rs:131-169`.

**Record mapping**, verified against 26 real sessions:

| Gemini record | `SessionEvent` |
| --- | --- |
| `type: "user"` | `User`. `content` is a string or a `[{text}]` part array; parts join with newline. |
| `type: "gemini"`, `thoughts[]` | One `Reasoning` per thought: `subject`, then `description`, with the thought's own timestamp. |
| `type: "gemini"`, `content` | `Assistant`. |
| `type: "gemini"`, `toolCalls[]` | One `ToolCall` (`name`, `args` to `structured`, `tool_call_id` from `id`) plus one `ToolResult` (`success` = `status == "success"`). |
| `type: "info"` \| `"error"` \| unknown | `Opaque`, carrying only a type marker. |

**`displayContent` is never read.** Real data shows `content` carries the
relativized path (`@../../.gemini/skills/...`) while `displayContent` carries
the absolute one (`/Users/<name>/.gemini/skills/...`). Reading `displayContent`
would put home-directory paths into the transcript.

**Transcript fields.** `conversation_id` from `sessionId`; `started_at` from
`startTime`; `model` from the first `gemini` message's `model`; `token_counts`
from `tokens.input` and `tokens.output` (the object is `{input, output, cached,
thoughts, tool, total}`); `session_hash` = sha256 over raw file bytes, matching
every other adapter so `submission_id_for` stays deterministic.

**`cwd`** comes from the sibling `<project>/.project_root` file when present.
Newer session directories carry it; older hash-named ones do not, in which case
`cwd` is `None` and `project` falls back to the directory name. `cwd` feeds the
redactor's path-prefix stripping and is never serialized — the existing
invariant is preserved unchanged.

**Containment and budget.** `session_for_path` routes through
`real_file_within_root` (`source/mod.rs:243`). Sessions over the existing
64 MB budget raise `SessionTooLarge` with label `gemini-session-too-large`.
`discover` and `session_at` share one ref-builder, per the contract at
`source/mod.rs:161-207`.

**Wiring.** `all_sources` gains a `gemini: Option<SourceDeclaration>`
parameter, changing its signature and all seven call sites. Plus
`DaemonSettings.gemini_source` (`daemon/settings.rs:129-134`),
`roots_declared` (`:310`), `parse_source_declaration` (`:455`), and a
`discovery::probe` entry so the store appears in the consent UI.

### Slice B: bounded Trajectory auto-discovery

Without `--trajectory`, `TrajectorySource` is constructed over exactly two
locations:

- The current working directory, non-recursive, matching
  `*.trajectory.json` and `*.trajectory.jsonl`.
- `~/.trace-commons/trajectories/`, matching any `*.json` or `*.jsonl`.

Nothing else. Never `$HOME` at large, never a recursive walk.

The suffix requirement in the working directory is what keeps a stray
`session.json` out of a submission. The conventional directory needs no suffix
because placing a file there is itself the opt-in.

Explicit `--trajectory` is unchanged, including the hard error on a missing
path (`commands.rs:586-591`).

One deliberate asymmetry: under auto-discovery a file that fails to parse is
skipped, because it never claimed to be a trajectory. Under explicit
`--trajectory` it still hard-rejects with its reason label, because the
contributor named it.

### Slice C: one-step `submit`

- **`submit .`** — new positional, equivalent to `--project .`. Conflicts with
  `--project`. This is the "single project in the current directory" half of
  the Paxel shape.
- **Bare `submit`** — defaults to recent-everything and shows a y/N summary
  confirm (count, projects, date range, granted consent scopes) in place of the
  index picker. `--yes` skips it, as today.
- **Auto-enroll** — when no config exists *and* an invite is available via
  `--invite` or `TRACE_COMMONS_INVITE`, run the login flow, then submit. With
  no invite the error is today's `not logged in; run \`login\` first`,
  unchanged.
- **`--json`** — frozen. No auto-enroll, no prompt, no positional handling.

The index picker is not removed; it remains reachable for contributors who want
per-session selection.

### Slice D: deep-link enrollment and the Devfolio seam

`invite_from_deep_link` already parses `tracecommons://enroll?invite=…`
(`commands.rs:1908`). Missing is OS registration of the scheme for the CLI, so
Devfolio can publish a clickable link that enrolls without the contributor
copying a credential-shaped string.

Scope: scheme registration, `login --invite` accepting the deep-link form, and
a documented statement of what Devfolio's verifier API can rely on — the
compact `attest` JWS (EdDSA, `kid` resolved from
`/.well-known/trace-commons-attestation-keyset.json`), and `--project`
scoping. Per `docs/collector-integration.md:105-130`, a submission-id list is
explicitly not suitable for scoring; the JWS is.

We build no verifier-side code. Devfolio owns that API and has it in progress.

## Error handling

Gemini adapter failures are label-only, per the repo's hash-only logging rule:
`unreadable-gemini-session`, `malformed-gemini-json`, `gemini-session-too-large`.
No file content, no paths, in any error string.

Auto-enroll failure during `submit` aborts before any discovery or upload, so a
failed enrollment never leaves a partially-submitted run.

## Testing

TDD throughout, against sanitized fixtures committed under
`crates/trace-commons-contributor/fixtures/gemini-cli/`.

- One fixture per message type, asserting the mapping table.
- `thoughts[]` becomes `Reasoning`, and `--no-reasoning` drops it.
- `content` as string and as part-array both parse; `displayContent` never
  appears in any output.
- Unknown message type maps to `Opaque` rather than rejecting the file.
- `.project_root` present and absent; `cwd` never serialized in either case.
- Symlink escape from the Gemini root is refused.
- `session_hash` and `submission_id_for` determinism; `discover` and
  `session_at` produce identical refs.
- Trajectory auto-discovery finds `*.trajectory.json` in cwd and files in the
  conventional directory, and finds nothing else — including a plain
  `session.json` in cwd.
- `submit .` equals `--project .`; `--json` output is byte-identical to today
  for every invocation in `docs/collector-integration.md`.

## Consequences

### Canonical representation

Gemini sessions are new traces, so no existing scored trace changes and no
re-score is required. As with the Trajectory slice, `gate-calibrate` floors
should be re-checked against a post-change sample once Gemini traces are in the
corpus, since the novelty inputs gain a harness with different prose
characteristics.

### Gemini session retention

Gemini CLI prunes old sessions on its own schedule (`general.sessionRetention`
in `~/.gemini/settings.json`). Traces not submitted before pruning are gone.
This is a property of the store, not something this adapter can fix, but it
argues for the daemon watching the Gemini root rather than relying on a
one-shot sweep at hackathon deadline.

### Out of scope

- A Gemini adapter contributed upstream to Letta Trajectory.
- Server-side ingest of any new format.
- Devfolio's verifier API.
- Recursive or `$HOME`-wide trajectory discovery.
- Native adapters for the six harnesses Trajectory already covers.
