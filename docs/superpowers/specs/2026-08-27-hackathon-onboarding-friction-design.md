# Hackathon onboarding friction: Gemini CLI, one-step submit, attestation, enrollment

Date: 2026-08-27
Status: Approved design, pending implementation plan

## Summary

Six slices answering Devfolio's third round of hackathon feedback. Hackers
submit under deadline pressure; every extra step is a drop-off. The feedback
names three problems: unsupported agent CLIs, a multi-command submit sequence,
and token handling in the Devfolio integration.

| Slice | Change | Feedback item |
| --- | --- | --- |
| A | Native Gemini CLI source adapter | 1 |
| B | Bounded auto-discovery of Letta Trajectory files | 1 |
| C | `submit` collapsed to one step | 2 |
| D | `tracecommons://` deep-link enrollment, and the documented Devfolio seam | 3 |
| E | Attestation folded into `submit`, without the async-scoring race | 2, 3 |
| F | A one-time script for a single submission | 2 |

Each slice is independent and gets its own implementation plan and PR.

### What this is measured against

The goal is not fewer commands; it is less work for one hacker at a deadline.
The measure:

| | Commands | Interactive answers |
| --- | --- | --- |
| Today | 4 (`install`, `login`, `submit`, `attest`) | 5 (four consent y/N, one index pick) |
| After all six slices | 2 (`install`, `submit`) | 1 |

An earlier draft of this design cut one command and no interactions at all, by
renaming work rather than removing it. Any slice that does not move a number in
that table is not doing the job.

### Build order

**D, then C, then E, then A, then B, then F.**

D comes first because C's auto-enroll needs the invite to arrive from
somewhere, and without a deep link that somewhere is the contributor pasting a
credential-shaped string into an environment variable -- the exact handling
feedback item 3 asks us to remove. C before E because E extends `submit`. A and
B are independent of that chain and can land in parallel. F comes last: it
wraps the finished behaviour and has nothing to wrap until then.

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

### Attestation is a mandatory step nobody counted

`docs/collector-integration.md:8-16` is unambiguous: a submission id is not
proof of authorship, and a collector must score a **signed attestation**. So a
Devfolio submission has always been `submit` *and then* `attest`, plus a manual
handoff of the resulting file. Every previous framing of "simplify the CLI"
counted only `submit`.

Worse, `attest` (`commands.rs:1797-1803`) is a single `fetch_score_attestation`
call with no polling, and it attests to traces the server has already scored.
Scoring is asynchronous. A hacker who submits at 23:58 and attests at 23:59
receives an attestation that does not cover the traces they just submitted --
not slow, wrong. Collapsing `submit` does not touch this, which is why Slice E
exists.

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
`parse_source_declaration` (`:455`), and a `discovery::probe` entry so the
store appears in the consent UI.

**`roots_declared` must not gain a third conjunct.** It is
`claude_source.is_some() && codex_source.is_some()` (`settings.rs:310-312`),
and all three application shells refuse to start the daemon when it is false
-- macOS and Windows through the C ABI (`contributor-ffi/src/lib.rs:328`),
GTK directly (`contributor-gtk/src/backend.rs:89,142`). Every installed
desktop client today has claude and codex declared and no gemini field at
all. Adding `&& gemini_source.is_some()` would make the predicate false for
every one of them, and the daemon would stop starting on upgrade.

The rule is therefore: **an absent `gemini_source` is not disqualifying.**
`roots_declared` keeps its current two-conjunct form, and the Gemini root
follows the same tri-state as the others at the *adapter* layer -- `None`
means "never asked", which for the CLI falls back to the conventional path
and for the shells means the Gemini store is simply not watched until the
user answers. A shell that wants to prompt for it asks whether
`gemini_source.is_none()`, which is a question about what to show, not a
gate on starting.

This is a deliberate asymmetry with the fail-closed-roots parity rule
(`2026-08-19-fail-closed-roots-parity-design.md`), and it is safe for the
reason that rule turns on: an undeclared root there meant the daemon would
scan the *real* `~/.claude` unasked. Here an absent field means no Gemini
adapter is constructed at all, so nothing is scanned. Half a declaration
buys no protection; a missing third declaration costs no protection.

### Slice B: bounded Trajectory auto-discovery

Without `--trajectory`, `TrajectorySource` is constructed over exactly two
locations:

- The current working directory, non-recursive, matching
  `*.trajectory.json` and `*.trajectory.jsonl`.
- `<state-dir>/trajectories/`, matching any `*.json` or `*.jsonl`, where
  `<state-dir>` is whatever `ConfigStore::resolve` already picked
  (`config.rs:206-220`): `--config-dir`, else
  `TRACE_COMMONS_CONTRIBUTOR_DIR`, else the platform default.

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
- **Bare `submit` gains a default window of 7 days.** Today `sel.since` is
  `None` when unset and `discover_filtered` applies no time filter, so bare
  `submit` discovers *every session ever recorded*, across every project. That
  is safe only because the index picker forces a choice. Replacing the picker
  with select-all would otherwise offer a contributor's entire history --
  including unrelated client work -- behind one keystroke. `--since` overrides
  it; `--all` means all of time and is how the old behaviour is reached.
- **Bare `submit`** shows a y/N summary confirm (count, projects, date range,
  granted consent scopes) in place of the index picker. `--yes` skips it, as
  today.
- **Auto-enroll** — when no config exists *and* an invite is available via
  `--invite` or `TRACE_COMMONS_INVITE`, run the login flow, then submit. With
  no invite the error is today's `not logged in; run \`login\` first`,
  unchanged.
- **`--json`** — frozen. No auto-enroll, no prompt, no positional handling.

The index picker is not removed; it remains reachable for contributors who want
per-session selection.

**Consent prompts are not removed.** `prompt_consent_answers`
(`consent.rs:108-134`) asks four y/N questions on a TTY, and auto-enroll
inherits all four in the middle of a submit. That is the consent model and it
stays; pre-seeding scopes from an invite or deep link would be a dark pattern.
The hackathon quickstart should instead document `--default`, which already
exists, takes the most restrictive answer for every optional scope, and is
sufficient for scoring. Contributors broaden later if they choose.

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

### Slice E: an attestation that says what it does not know

The premise this slice started from -- "poll until the traces are scored, then
attest" -- does not survive contact with the code. Three findings, each
verified:

**The attestation cannot be scoped, and truncates arbitrarily.** `GET
/v1/contributors/me/score-attestation` (`ingest.rs:14037`) takes no parameters.
It enumerates every submission owned by `(tenant_id, auth_principal_ref)` that
has a gate decision, `LIMIT 500` -- and the ordering is `ORDER BY
d.submission_id, d.decided_at DESC, d.decision_id DESC`
(`db/postgres.rs:4928-4929`). Truncation is therefore by submission **UUID**,
not recency. A contributor with more than 500 scored submissions gets an
arbitrary subset, and their hackathon traces may be absent for no reason a
human could predict. This is a correctness defect, not a latency problem, and
it is reachable today by any prolific contributor.

**There is no "not yet scored" signal on any contributor surface.** The
attestation query is an INNER JOIN against `trace_gate_decisions`
(`db/postgres.rs:4890-4933`), so an unscored submission is silently *absent* --
indistinguishable from one that was never submitted. `submission-status`
(`ingest.rs:13956`) reports the corpus lifecycle -- `received`, `accepted`,
`quarantined`, `awaiting_pii_backstop`, `rejected`, `revoked`, `expired`,
`purged` (`trace_corpus_storage.rs:29-42`) -- which is a different state
machine from gate scoring and carries no gate-decision field.
`credit_points_final` is set by the review path (`ingest.rs:37507`), not by
scoring, so it is not a proxy either.

**Waiting cannot be bounded honestly by the client.** Scoring is an in-process
driver on a 45-second tick with a batch of 5 (`ingest.rs:995-999`) -- roughly
6.7 submissions per minute, FIFO by `received_at`. It is **off unless
`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` is truthy** (`ingest.rs:6283`), and a
submission that exhausts `max_attempts` leaves the enumeration permanently
(`gate_scoring_exhausted`, `ingest.rs:50152`). A client that waits for its
traces to appear can wait forever, through no fault of its own.

So the fix is mostly server-side. A client-only `--wait` would be a spinner in
front of a question the server declines to answer.

#### E1. Scope the attestation to a submission set

The route accepts an optional set of submission ids and attests to exactly
those, capped by request size rather than by a global `LIMIT`. Unscoped
behaviour is unchanged for existing collectors, except that its ordering
becomes recency-based so truncation drops the oldest rather than an arbitrary
UUID range.

#### E2. Report pending and unknown explicitly

`ScoreAttestationClaims` (`trace_score_attestation.rs:308-320`) gains, at
`schema_version` `trace_commons.score_attestation.v3`:

- `pending: Vec<SubmissionId>` — owned by this principal, no gate decision yet.
- `unknown: Vec<SubmissionId>` — asked about, not owned or not present.

`submissions` keeps its current meaning. The signature now covers the statement
"these are scored, these are waiting, these I do not have", which is the claim a
collector actually needs. Absence stops being ambiguous.

This is what makes the whole slice work: a hacker who submits at 23:58 hands
Devfolio an attestation that *says* three traces are pending, signed. Devfolio
can score them when the decisions land instead of treating the hacker as having
submitted nothing.

#### E3. `submit` emits the attestation

`submit` gains `--attest-out <path>`. After upload it requests an attestation
scoped (E1) to the submission ids it just wrote to receipts, waits a bounded
time for `pending` to empty, and writes the attestation either way.

- Default wait is short and honest rather than optimistic. On timeout it writes
  the attestation as it stands and prints `N of M traces scored, K pending`.
  It does not fail: a contributor at a deadline needs the artifact, and with E2
  the artifact is truthful about what it does not yet cover.
- `--attest-out` is additive. Bare `submit` behaviour and the standalone
  `attest` command are unchanged.
- `--json` gains `attestation`, `scored`, `pending` fields. No prompt, no
  waiting beyond the same bound.

#### E4. Correct the collector documentation

`docs/collector-integration.md:220-222` currently reads: "Attestations cover
what is claimed, not completeness — a participant may withhold traces." That
frames absence as *deliberate withholding* only. A collector following this
document today would read a not-yet-scored trace as a hacker who withheld it,
and could mark down a participant for the server's queue depth.

The text gains the second reason for absence, and the failure-mode table
(`:238-243`) gains a "submission missing from attestation" row distinguishing
pending from withheld from truncated.

### Slice F: the one-time script

Devfolio's third question under item 2: "Can we also make the CLI runnable as a
one-time script, for a single traces submission?"

`scripts/contribute.sh` (and `contribute.ps1`): fetch the verified binary into
a cache directory, enroll if needed, submit, attest, print where the
attestation landed. No PATH entry, no daemon, no autostart, no login item.

**It reuses `install.sh` rather than reimplementing verification.** That script
already takes `--dir` / `TC_INSTALL_DIR` (`install.sh:33,42`) and refuses any
binary whose published SHA-256 does not match, or on macOS whose signature does
not name our Developer ID, with no `--force` and no `--skip-verify`
(`install.sh:15-27`). `contribute.sh` calls it with a cache directory and
inherits every one of those refusals. A second download path with its own
verification would be a second chance to get verification wrong.

**"One-time" means the binary, not the state.** A genuinely stateless run would
be actively harmful, for two reasons:

- Enrollment is **not idempotent** — each success spends one use of the invite
  (`collector-integration.md:31-34`). A script that discarded state would burn
  a fresh invite use on every run, and a hacker who ran it twice would be
  refused.
- Withdrawal needs the account session. "Uninstalling is not withdrawal"
  (`README.md:266-270`); traces already submitted stay on the server, and
  `daemon withdraw` is what removes them. Wiping state would leave a
  contributor with traces on the server and no way to withdraw them. Consent
  that cannot be revoked is not consent.

So the state directory is the normal one, resolved exactly as always
(`config.rs:206-220`). Re-running the script is idempotent: already enrolled,
so it skips straight to submitting.

**The invite never appears in argv.** `contribute.sh` reads it from
`TRACE_COMMONS_INVITE`, from a `tracecommons://` deep link (Slice D), or by
prompting on a TTY — never as a positional argument. An invite passed on a
command line lands in shell history and in every `ps` listing on the machine,
which is precisely the credential handling feedback item 3 asks us to remove.

**The two-step form stays documented first.** `install.sh` deliberately
documents `curl -o` then `sh` ahead of the piped form so the script can be read
before it runs (`install.sh:4-8`). `contribute.sh` follows that, for the same
reason: this tool reads coding transcripts.

Shape:

```sh
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/contribute.sh -o contribute.sh
sh contribute.sh --project .
```

`--project .` scopes to the current directory; omitting it submits the last
seven days across all projects — the same two shapes as Slice C, and the same
two Paxel offers.

## Error handling

Gemini adapter failures are label-only, per the repo's hash-only logging rule:
`unreadable-gemini-session`, `malformed-gemini-json`, `gemini-session-too-large`.
No file content, no paths, in any error string.

Auto-enroll failure during `submit` aborts before any discovery or upload, so a
failed enrollment never leaves a partially-submitted run.

Slice E fails closed on the server side as the route already does: 503 when
the attestation signing key or the gate-driver pool is unconfigured
(`ingest.rs:14037`). An `unknown` entry never distinguishes "does not exist"
from "belongs to another tenant", so the list cannot be used to probe for
submission ids.

`submit --attest-out` treats a failed attestation fetch as a warning, not a
submit failure. The traces are already uploaded and the receipts already
written; refusing to exit zero at that point would tell a contributor their
submission failed when it did not.

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
- Bare `submit` selects only sessions inside the 7-day default window, and
  `--all` still reaches everything.
- `roots_declared` stays true for a settings object holding claude and codex
  declarations and no `gemini_source`, so an upgraded desktop install still
  starts. This is the regression test for the migration rule in Slice A.
- Attestation scoped to a submission id set returns exactly those entries, and
  a submission with no gate decision appears in `pending`, not omitted.
- An id the principal does not own appears in `unknown`, and never leaks
  whether it exists for another tenant.
- Unscoped attestation truncation drops the oldest, not an arbitrary UUID
  range — the regression test for `ORDER BY d.submission_id`.
- `submit --attest-out` on a timeout writes the attestation and reports the
  pending count rather than failing.
- The one-time script run twice enrolls once: the second run finds an existing
  state directory and does not spend a second invite use.

## Consequences

### Canonical representation

Gemini sessions are new traces, so no existing scored trace changes and no
re-score is required. As with the Trajectory slice, `gate-calibrate` floors
should be re-checked against a post-change sample once Gemini traces are in the
corpus, since the novelty inputs gain a harness with different prose
characteristics.

### Gemini session retention

Gemini CLI prunes old sessions on its own schedule (`general.sessionRetention`
in `~/.gemini/settings.json`), defaulting to `maxAge: "30d"` on the install
checked here. Traces older than that are gone before the CLI ever sees them.

Thirty days comfortably covers a hackathon, so this is not a blocker for the
case this design serves. It does mean a contributor who installs months later
recovers far less history than the Claude Code or Codex stores would give them,
and it is another argument for the daemon watching the Gemini root rather than
relying on a one-shot sweep. It is a property of the store; the adapter cannot
fix it.

### Scoring throughput is the real deadline risk

The scoring driver processes a batch of 5 every 45 seconds
(`ingest.rs:995-999`), FIFO by `received_at` — roughly 6.7 submissions per
minute for the whole tenant. A hackathon whose participants all submit in the
final hour presents a queue that drains far slower than it fills, and every
one of those contributors gets an attestation dominated by `pending`.

Slice E makes this honest rather than invisible, which is a real improvement:
Devfolio receives a signed statement that the traces exist and are queued,
instead of an attestation that silently omits them. But it does not make the
queue drain faster, and the operator-side answer — driver batch size, interval,
and whether the driver is enabled at all — has to be settled before the next
hackathon, not during it.

The contributor-side mitigation is to submit throughout the event rather than
at the end. That is what the daemon is for, and it is worth saying plainly in
the hackathon quickstart.

### Out of scope

- A Gemini adapter contributed upstream to Letta Trajectory.
- Server-side ingest of any new trace format. (Slice E does change a server
  route and the attestation schema; that is deliberate and is the only
  server-side work here.)
- Devfolio's verifier API.
- Recursive or `$HOME`-wide trajectory discovery.
- Native adapters for the six harnesses Trajectory already covers.
- Raising gate-scoring throughput. Slice E makes the queue's depth *visible and
  signed*; it does not make it shorter. See "Scoring throughput" below.
