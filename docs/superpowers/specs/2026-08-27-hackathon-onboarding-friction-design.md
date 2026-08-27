# Hackathon onboarding friction: Gemini CLI, one-step submit, attestation return

Date: 2026-08-27
Status: Approved design, pending implementation plan

## Summary

Six slices answering Devfolio's third round of hackathon feedback. Hackers
submit under deadline pressure; every extra step is a drop-off.

| Slice | Change | Feedback item |
| --- | --- | --- |
| A | Native Gemini CLI adapter, behind a registration seam | 1 |
| B | Bounded auto-discovery of Letta Trajectory files | 1 |
| C | `submit` collapsed to one step | 2 |
| D | The attestation reaches the collector without the user carrying it | 3 |
| E | An attestation that says what it does not know | 2, 3 |
| F | A one-time script for a single submission | 2 |

Each slice is independent and gets its own implementation plan and PR.

### What this is measured against

The goal is not fewer commands; it is less work for one hacker at a deadline.
Counting honestly, including the steps earlier drafts of this spec left out:

| | Commands typed | Interactive answers | Carries a file by hand |
| --- | --- | --- | --- |
| Today | 6 | 5 | yes |
| After A-F | 3 | 1 | no |
| After A-F, via Slice F | 1 | 1 | no |

"Today" is `curl -o`, `sh install.sh`, `login --invite`, `submit --dry-run`,
`submit`, `attest --out` — the sequence `docs/collector-integration.md`
prescribes — then uploading the resulting JWS to Devfolio by hand. The five
answers are four consent prompts (`consent.rs:108-134`) and one index
selection (`commands.rs:845-867`).

Two caveats this table must carry rather than hide:

- **The single interactive answer assumes `--default`.** Without it, first-run
  enrollment still asks all four consent questions, so the honest first-run
  count is five. Consent is not friction to be optimised away; the quickstart
  documents `--default`, and the number in this table is the number a hacker
  gets when they follow it.
- **"Install" is two commands, not one**, and on a default macOS shell
  `~/.local/bin` is often not on `PATH` (`install.sh:162-169`), which can make
  it three. Homebrew is worse: `brew tap`, `brew trust`, `brew install`
  (`README.md:197-203`). Slice F sidesteps install entirely; nothing here
  shortens it for someone who wants the CLI on their machine, and Devfolio
  praised Paxel's *installation* flow specifically. That is a known,
  unaddressed part of item 2.

An earlier draft of this design claimed two commands and one answer by
counting install as one command and omitting the consent prompts it had
already documented elsewhere. Any slice that does not move a number in this
table is not doing the job, and any number in it that needs a footnote gets
one.

### Build order

**C, then E, then D, then A, then B, then F.**

C first because E and D both extend `submit`. D after E because the artifact D
returns is the one E produces. A and B are independent and can land in
parallel with any of it. F last: it wraps the finished behaviour.

An earlier draft put D first, reasoning that C's auto-enroll needed the invite
to arrive from somewhere. That premise was wrong — see Slice D.

## Motivation

### The Letta on-ramp has never worked

Hackers asked to use Gemini CLI and were pointed at the Letta Trajectory
integration docs. They were following instructions that could not have worked,
though not for the reason an earlier draft of this spec gave.

That draft said Trajectory has no Gemini CLI adapter, citing the eight-adapter
list in `2026-07-25-letta-trajectory-support-design.md`. **That is stale.**
Upstream `@letta-ai/trajectory` 0.3.0, published 2026-08-20, carries a
`gemini-cli` adapter among fourteen.

The real defect is worse and applies to every harness. `README.md:346-350`
tells contributors to run `npx @letta-ai/trajectory > session.json`. **No
published version of that package has ever had a `bin` entry** — verified
against the registry packument for every version from 0.1.0 to 0.3.0, where
`bin` is absent throughout. `npx` cannot execute it; it fails with "could not
determine executable to run". The package is a library exposing
`normalizeTranscript()`, requiring Node >= 20.

So the actual workflow behind our one-line instruction is: install Node,
install the library, locate your harness's session store yourself, and write a
script against its API. That is precisely the "too many additional steps"
Devfolio reported, and the "documented two-step" premise of the 2026-07-25
design was never true.

Slice A survives this correction, on different grounds. A native adapter reads
the local store directly with zero conversion step, which is strictly better
than any Letta path even once that path works.

### "The more agents your CLI can support" is a capability ask

Devfolio's phrasing is general. Gemini is the instance they hit, not the
requirement. Adding one adapter and leaving the next one just as expensive
answers the instance and not the ask, which is why Slice A includes a
registration seam and not only a parser.

### Attestation is a mandatory step nobody counted

`docs/collector-integration.md:8-16` is unambiguous: a submission id is not
proof of authorship, and a collector must score a **signed attestation**. A
Devfolio submission has therefore always been `submit`, then `attest`, then a
manual upload of the resulting file. Every previous framing of "simplify the
CLI" counted only `submit`.

`attest` (`commands.rs:1797-1803`) is also a single `fetch_score_attestation`
call with no polling, over traces the server has already scored. Scoring is
asynchronous, so a hacker who submits at 23:58 and attests at 23:59 receives an
attestation that does not cover what they just submitted.

### What Devfolio is building is not the forbidden pattern

Their feedback says a Devfolio API "will verify the token and only store the
submission IDs from a valid token." Read alongside
`collector-integration.md:8-16` that looks like the anti-pattern the contract
forbids. It is not. The rule is against scoring an **unverified id list handed
over by a participant**; extracting ids from a *verified* attestation and
storing those is the sanctioned flow, and the route that closes it already
exists: `POST /v1/admin/scores-by-submission` (`ingest.rs:7521`), behind the
scoped `CompetitionReadWorker` credential (`ingest.rs:3088`), built for this
integration in the `2026-07-17-devfolio-score-readback` slice.

The defect is documentation. `docs/collector-integration.md` never mentions
that route. The one collector we have was left to infer the second half of its
own integration. Slice D fixes that.

## Decisions and rationale

### Native Gemini adapter, and auto-discover Trajectory files

Considered and rejected: contributing a Gemini adapter upstream to Letta. That
is the right long-term home, but it puts the fix behind another project's
release cycle for the one harness hackers asked for.

Considered and rejected: shelling out to `npx @letta-ai/trajectory` when
available. The Trajectory design rejected this already — it adds a subprocess
surface to a privacy-sensitive CLI for a soft dependency — and nothing here
changes that reasoning.

**This leaves item 1b only partly answered, and the spec should say so.** Slice
B removes the `--trajectory` flag; it does not shorten the Letta-side
conversion workflow, which is the friction Devfolio actually described, and
which is worse than documented — see "The Letta on-ramp has never worked".
Closing it properly is planned separately; the registration seam in Slice A is
what makes native per-harness adapters affordable as the escalation path.

### Tolerant parsing for Gemini, not fail-closed

`gemini_cli.rs` maps unknown message types to `Opaque` with a type marker,
following `claude_code.rs:1044`, rather than rejecting the whole file the way
`trajectory.rs` does.

Trajectory is a versioned schema with a published conformance corpus, so an
unrecognised record means the file is not what it claims. Gemini CLI's session
format is unversioned and evolving; a strict parser would reject every session
the moment upstream adds a message type, costing the corpus exactly the traces
this slice exists to collect. The departure is scoped to message-type dispatch.
Path containment, byte budgets, and anything a gate depends on stay
fail-closed.

### Subagent sessions ship standalone

Gemini writes `kind: "subagent"` sessions as separate files with no
back-reference to a parent — verified: the sole subagent session's `sessionId`
appears in no other file. Claude Code's merge strategy
(`claude_code.rs:446-531`) is unavailable, and dropping them would discard real
work.

### `submit` itself becomes one-step; no new verb

Considered and rejected: a new `contribute` verb pair mirroring Paxel. It adds
a second command that does what `submit` already does, and every doc and
collector integration would have to explain which to use.

`--json` is frozen. `docs/collector-integration.md` scripts against it, and a
collector driving this CLI programmatically must not acquire an interactive
prompt or an auto-enroll path in a point release.

### The attestation schema does not bump

Devfolio is building a verifier against `trace_commons.score_attestation.v2`
right now. `collector-integration.md:187-213` documents v1 to v2 as a
deliberate breaking change and tells verifiers to pin the version string, so a
v3 would break the one integration item 3 is about, mid-build.

Slice E therefore adds `pending` and `unknown` **only to scoped responses**,
under an unchanged `v2`. A collector that never sends a submission set sees an
attestation byte-identical to today's. New fields reach a verifier only once it
opts in by asking a scoped question, which is the one moment it is certainly
ready for the answer.

## Design

### Slice A: `source/gemini_cli.rs`, and a seam for the next one

**Store.** `~/.gemini/tmp/<project>/chats/session-*.json`, root overridden by
`GEMINI_CLI_HOME` (confirmed in the shipped binary). One file is one session:
a single JSON document, not JSONL.

**Discovery.** Non-recursive `read_dir` of `<root>/*/chats/` matching
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
relativised path (`@../../.gemini/skills/...`) while `displayContent` carries
the absolute one (`/Users/<name>/.gemini/skills/...`).

**Transcript fields.** `conversation_id` from `sessionId`; `started_at` from
`startTime`; `model` from the first `gemini` message; `token_counts` from
`tokens.input` and `tokens.output` (the object is `{input, output, cached,
thoughts, tool, total}`); `session_hash` = sha256 over raw file bytes, matching
every other adapter so `submission_id_for` stays deterministic.

**`cwd`** comes from the sibling `<project>/.project_root` when present. Older
hash-named directories lack it, in which case `cwd` is `None` and `project`
falls back to the directory name. `cwd` feeds the redactor's path-prefix
stripping and is never serialised.

**Containment and budget.** `session_for_path` routes through
`real_file_within_root` (`source/mod.rs:243`). Sessions over the existing 64 MB
budget raise `SessionTooLarge` with label `gemini-session-too-large`.
`discover` and `session_at` share one ref-builder, per the contract at
`source/mod.rs:161-207`.

#### A1. The registration seam

`all_sources` (`source/mod.rs:350-383`) takes one positional
`Option<SourceDeclaration>` per source, so every new adapter changes its
signature and all seven call sites, plus a `DaemonSettings` field,
`parse_source_declaration`, a `discovery::probe` entry, and per-shell consent
UI. Adapter N+1 pays it again with eight call sites, then nine. Item 1 was a
capability ask; answering it with one adapter and the same wiring answers the
instance only.

`all_sources` therefore takes **one `SourceRoots` map** keyed by source name
instead of a positional parameter per source, and `DaemonSettings` carries the
declarations as a map beside its named fields. Adding an adapter becomes: a new
module, a name constant, one table row. No call site changes.

The named `claude_source` and `codex_source` fields stay as the serialised
shape, so existing `daemon-settings.json` files parse unchanged; the map is
built from them on load.

#### A2. `roots_declared` must not gain a third conjunct

It is `claude_source.is_some() && codex_source.is_some()`
(`settings.rs:310-312`), and all three shells refuse to start the daemon when
it is false — macOS and Windows through the C ABI
(`contributor-ffi/src/lib.rs:328`), GTK directly
(`contributor-gtk/src/backend.rs:89,142`). Every installed desktop client has
claude and codex declared and no gemini field. A third conjunct would stop the
daemon starting on every one of them.

**An absent Gemini declaration is not disqualifying.** `roots_declared` keeps
its two-conjunct form. A shell that wants to prompt asks whether the Gemini
declaration is absent, which is a question about what to show, not a gate on
starting.

This is a deliberate asymmetry with `2026-08-19-fail-closed-roots-parity-design.md`,
and safe for the reason that rule turns on: an undeclared root there meant the
daemon would scan the real `~/.claude` unasked. Here an absent declaration
constructs no adapter, so nothing is scanned.

### Slice B: bounded Trajectory auto-discovery

Without `--trajectory`, `TrajectorySource` is constructed over exactly two
locations:

- The current working directory, non-recursive, matching `*.trajectory.json`
  and `*.trajectory.jsonl`.
- `<state-dir>/trajectories/`, matching any `*.json` or `*.jsonl`, where
  `<state-dir>` is whatever `ConfigStore::resolve` already picked
  (`config.rs:206-220`).

Nothing else. Never `$HOME` at large, never a recursive walk.

The suffix requirement in the working directory keeps a stray `session.json`
out of a submission; the conventional directory needs no suffix because placing
a file there is the opt-in. Using the existing state directory rather than a
new dot-dir keeps one location per platform, inherits its `0700` mode
(`config.rs:228-234`), honours `TRACE_COMMONS_CONTRIBUTOR_DIR`, and means
`logout` clears staged trajectories.

**This changes what the README tells people to type.** `npx
@letta-ai/trajectory > session.json` (`README.md:346-350`) produces a file the
suffix rule will not auto-discover. The README changes to
`> session.trajectory.json`; explicit `--trajectory` keeps working for anyone
following the old line, including its hard error on a missing path
(`commands.rs:586-591`).

Under auto-discovery a file that fails to parse is skipped, because it never
claimed to be a trajectory. Under explicit `--trajectory` it still hard-rejects
with its reason label, because the contributor named it.

### Slice C: one-step `submit`, scoped by where you are

Paxel scopes by **directory subtree**: run it from the folder holding your
repos and it takes all of them; run it from one project and it takes that one.
The two commands Devfolio described are one command run from two places, and
the working directory is the only variable.

We adopt that model. It is also much closer to shipped than an earlier draft of
this spec assumed: `cwd_matches_project` is already
`Path::new(cwd).starts_with(project)` (`commands.rs:556-570`), so `--project`
has always matched a subtree rather than a single directory. The change is what
happens when nobody passes it.

- **Bare `submit` defaults the project filter to the current working
  directory** instead of `None`. That is the whole mechanism.
  `discover_filtered` (`commands.rs:578-638`) is untouched; it simply stops
  being handed `None`.
- **`--project <path>`** keeps working, for scoping somewhere you are not.
- **`--all`** ignores the working directory and means everything, everywhere.
- **A y/N summary confirm** — count, projects, date range, granted consent
  scopes — replaces the index picker (`commands.rs:845-867`). `--yes` skips it.
  The picker stays reachable for per-session selection.
- **Auto-enroll** when no config exists *and* an invite is available via
  `--invite` or `TRACE_COMMONS_INVITE`. With no invite the error is today's
  `not logged in; run login first`, unchanged.
- **`--json`** — frozen. No auto-enroll, no prompt, no changed default.

#### No positional, and no default time window

An earlier draft proposed `submit .` as a positional. It is redundant under
subtree scoping: running bare `submit` from that directory already means the
same thing, and Paxel demonstrates the working directory alone is enough. A
second spelling for one behaviour is a concept to document, not a feature.

That draft also gave bare `submit` a 7-day default window, because without a
filter it discovered every session ever recorded, and select-all would have
offered a contributor's entire history behind one keystroke. Subtree scoping
solves that problem better and on the right axis. A time window is the wrong
rail anyway: a hackathon project started ten days ago would have been silently
excluded from a submission its contributor believed covered the project.
`--since` remains available and opt-in.

#### The one case subtree scoping does not bound

Running from `$HOME`, a filesystem root, or any ancestor of every session store
makes the subtree "everything" — exactly the sweep the window was meant to
prevent. So `submit` **refuses** when the working directory is `$HOME` or a
filesystem root, and names `--all` as the way to say it deliberately.

A refusal, not a warning. The distinction matters at a deadline: a warning
above a y/N prompt reads as noise to someone in a hurry, and the answer to that
prompt is a full-history upload.

**Consent prompts are not removed.** `prompt_consent_answers`
(`consent.rs:108-134`) asks four y/N questions on a TTY and auto-enroll
inherits all four. That is the consent model; pre-seeding scopes from an invite
would be a dark pattern. The quickstart documents `--default`, which already
exists and takes the most restrictive answer for every optional scope.

### Slice D: the attestation reaches the collector

The half of item 3 that is ours. After Slice E a hacker holds a JWS file at
23:59 and must find it and upload it — a credential-shaped blob, handled by
hand, which is what item 3 asks us to remove.

#### D1. `submit --attest-post <url>`

`submit` POSTs the attestation body to a collector endpoint instead of leaving
a file. The host must pass the existing `HostAllowlist`
(`config.rs:109-116`) — the same pin already shared by `login`, issuer minting,
and ingest uploads, so a mistyped or hostile `--attest-post` cannot exfiltrate
to an arbitrary host. Not allowlisted, no post; the attestation is still
written locally so nothing is lost.

`--attest-out` and `--attest-post` compose. Neither is default.

#### D2. Deep-link enrollment is already built; document it honestly

An earlier draft proposed "OS registration of the `tracecommons://` scheme for
the CLI" and made it the first slice. Both were wrong.

The registration exists: the macOS app ships `CFBundleURLTypes`, guarded by a
test (`tests/release_pipeline.rs:125-126`), the flatpak declares
`MimeType=x-scheme-handler/tracecommons`
(`contributor-gtk/flatpak/ai.tracecommons.Contributor.desktop:9`), and
`invite_from_deep_link` parses the link (`commands.rs:1894`).

And it cannot be extended to a bare CLI on macOS, where a URL handler must be
an app bundle registered with LaunchServices — not a binary curl-installed into
`~/.local/bin`, still less the cache directory Slice F uses. So a CLI-only
hacker never receives a deep link, and the CLI path stays
`TRACE_COMMONS_INVITE` or the TTY prompt.

This slice therefore documents which enrollment path applies to which install,
and does not pretend the deep link covers the flow that matters most at a
hackathon.

#### D3. Document the collector's second half

`docs/collector-integration.md` gains:

- `POST /v1/admin/scores-by-submission` and the `CompetitionReadWorker`
  credential: how a collector reads scores for ids it already holds. This is
  how Devfolio resolves `pending` entries after the participant has gone home,
  and it is the missing half of their stated architecture.
- A correction at `:36`. "The CLI ships as source; build with cargo" contradicts
  the README's `install.sh`, Homebrew, and winget paths.
- A forward-compatibility rule the contract currently lacks: verifiers must
  ignore unrecognised **top-level** fields within a schema version. Without it,
  Slice E's scoped additions are a contract change; with it they are what the
  contract already anticipated.

### Slice E: an attestation that says what it does not know

Three verified findings:

**The attestation cannot be scoped, and truncates arbitrarily.** `GET
/v1/contributors/me/score-attestation` (`ingest.rs:14037`) takes no parameters,
enumerates every submission owned by `(tenant_id, auth_principal_ref)` with a
gate decision, `LIMIT 500` — ordered `BY d.submission_id, d.decided_at DESC,
d.decision_id DESC` (`db/postgres.rs:4928-4929`). Truncation is by submission
**UUID**, not recency. A contributor past 500 scored submissions gets an
arbitrary subset. This is a correctness defect reachable today, independent of
this design, and worth filing separately.

**There is no "not yet scored" signal.** The query inner-joins
`trace_gate_decisions` (`db/postgres.rs:4890-4933`), so an unscored submission
is silently absent — indistinguishable from never submitted.
`submission-status` (`ingest.rs:13956`) reports the corpus lifecycle
(`trace_corpus_storage.rs:29-42`), a different state machine with no
gate-decision field.

**Waiting cannot be bounded honestly by the client.** Scoring runs on a
45-second tick with a batch of 5 (`ingest.rs:995-999`), FIFO by `received_at`.
It is off unless `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` is truthy
(`ingest.rs:6283`), and a submission exhausting `max_attempts` leaves the
enumeration permanently (`ingest.rs:50152`).

#### E1. Scope the attestation to a submission set

The route accepts an optional set of submission ids and attests to exactly
those, capped by request size rather than a global `LIMIT`. Unscoped behaviour
is unchanged, except that its ordering becomes recency-based so truncation
drops the oldest rather than an arbitrary UUID range.

#### E2. Report pending and unknown, in scoped responses only

`ScoreAttestationClaims` (`trace_score_attestation.rs:308-320`) gains two
optional fields, present only when the request was scoped:

- `pending` — owned by this principal, no gate decision yet.
- `unknown` — asked about, not owned or not present.

`schema_version` stays `trace_commons.score_attestation.v2`. An unscoped
request returns exactly today's document, so Devfolio's in-flight verifier is
untouched until it opts in.

`submissions` keeps its meaning. The signature now covers "these are scored,
these are waiting, these I do not have" — so a hacker who submits at 23:58
hands Devfolio a signed statement that three traces are pending, and Devfolio
resolves them later through D3's read-back route instead of treating the hacker
as having submitted nothing.

`unknown` never distinguishes "does not exist" from "belongs to another
tenant", so it cannot be used to probe for submission ids.

#### E3. `submit` emits the attestation

`submit --attest-out <path>` requests an attestation scoped to the submission
ids it just wrote to receipts, waits a bounded time for `pending` to empty, and
writes it either way. On timeout it prints `N of M traces scored, K pending`
and does not fail: the traces are uploaded and the receipts written, and with
E2 the artifact is truthful about what it does not cover. `--json` gains
`attestation`, `scored`, and `pending`.

A failed attestation fetch is a warning, not a submit failure.

### Slice F: the one-time script, and the one thing it leaves behind

Devfolio's third question under item 2: "Can we also make the CLI runnable as a
one-time script, for a single traces submission?" The flow they praised is
Paxel's, which is `curl -fsSL .../upload.sh | bash` — no binary on PATH, no
persistent state, nothing that reads as installation.

We can match the first half of that exactly. We cannot match the second half,
and this section is mostly about why, and what the smallest honest amount of
persistence is.

#### F1. The binary is ephemeral

`scripts/contribute.sh` (and `contribute.ps1`) fetches the verified binary into
a cache directory, enrolls if needed, submits, attests, and exits. No PATH
entry, no daemon, no autostart, no login item.

It reuses `install.sh` rather than reimplementing verification. That script
already takes `--dir` / `TC_INSTALL_DIR` (`install.sh:33,42`) and refuses any
binary whose published SHA-256 does not match, or on macOS whose signature does
not name our Developer ID, with no `--force` and no `--skip-verify`
(`install.sh:15-27`). A second download path with its own verification would be
a second chance to get verification wrong.

The script itself may be advertised in its piped form for hackathon audiences.
The security boundary is the binary's signature, not the script's delivery: a
tampered script cannot make `install.sh` accept an unsigned binary. The
two-step download-then-read form stays documented first in the README, per
`install.sh:4-8`. The script body is wrapped in a function invoked on the last
line, so a truncated download executes nothing — table stakes once we advertise
piping.

#### F2. Statelessness is not available, and the reason is withdrawal

A fully stateless run — discard everything on exit — is correct for exactly one
operation: submit. Everything downstream breaks, because **the device key is
the identity**, not merely a credential for it.

On the CLI path the actor is always namespaced under the device key: `actor =
device_key_id`, or `instance:{tenant}:{device_key_id}:user:{subject}` when a
subject is present (`trace_upload_claim_issuer.rs:1808-1821`). The invite flow
derives the subject from the key itself — `let user_subject =
device_key_id_from_public_key_bytes(...)` (`:2072`). A fresh key is therefore a
fresh person, with four consequences:

- A new `auth_principal_ref`, and under `InviteTenantMode::Derived` a new
  tenant (`db/postgres.rs:760-765`).
- One invite use spent per run. Idempotency is keyed on `device_key_id`
  (`postgres.rs:2664-2690`); there is no subject-level or human-level dedup.
- A fresh per-contributor cap budget every run. Caps key on
  `auth_principal_ref` (`postgres.rs:4791-4832`), so wipe-and-rerun resets the
  concave cap. Content dedup (`dedup_assign.rs`) is the only remaining brake.
- **No withdrawal, ever.** The account is minted *from* the device principal
  (`create_or_reuse_account`, called at `ingest.rs:15317`), so no key means no
  account, means no way to revoke. Traces sit on the server that the
  contributor cannot take back.

The last one settles it. A one-time script that leaves a contributor unable to
withdraw is not a convenience feature; it is a consent failure. Uploading
something a person cannot retract is the one outcome this project's threat
model exists to prevent.

#### F3. The keep

The script therefore leaves exactly one thing behind: a **keep** — the device
private key plus the coordinates needed to use it.

Concretely it is a normal `ConfigStore` state directory
(`config.rs:206-234`, mode `0700`) at a clearly named path, holding the device
key, `contributor.json`, and the run's receipts. It is deliberately **not** a
new single-file format. The key alone is insufficient — reaching the server
needs the issuer URL, ingest URL, tenant, and instance id, none of which can be
re-derived locally — and inventing a combined blob would mean a new parser for
a private-key-bearing file, which is a poor place to add a parser. Reusing the
existing store is less code and already correct.

The keep is its own directory rather than the platform state directory an
installed CLI uses. A script fetched over the network should not silently adopt
an existing enrollment and submit under an identity the contributor did not
pick for it, and the printed delete instruction has to be safe to follow --
`rm -rf` against an installed CLI's state would not be.

The cost is a second identity: a contributor who later installs the CLI enrolls
again, spending another invite use, and per-contributor credit does not
aggregate across the two device keys. `TRACE_COMMONS_KEEP_DIR` points the keep
at an existing state directory for anyone who would rather they were one.

What the keep buys:

- **Withdrawal.** `account login` mints a browser session through the
  device-authenticated `POST /v1/account/login-links`
  (`account_auth.rs:325`), and `withdraw` revokes from there.
- **Idempotent re-runs.** The same key re-enrolling returns
  `OnboardDeviceKeyStatus::Idempotent` and spends no invite use
  (`postgres.rs:2664-2690`).
- **`status` and credit**, since the receipts are there.

The script prints, in plain language and every run: where the keep is, that it
is the only way to withdraw these traces, and the command to delete it.

There is no `--no-keep`. Deleting the keep later is the same act with better
timing — the contributor has by then seen what it is and what it is for —
whereas a flag that silently produces unrevocable traces is a footgun aimed at
the person holding it.

#### F4. What the keep does not buy, and should

`daemon/withdraw.rs:10-24` states withdrawal is "meant to survive losing the
device that submitted a trace." As wired it does not: surviving device loss
requires a passkey or NEAR identity registered on the account beforehand, and
**the CLI has no command to register one** — no caller of
`/v1/account/passkeys/register/*` exists in the contributor crate. Losing the
keep is losing the traces' revocability.

That gap predates this design and is not fixed here, but Slice F is the first
thing that makes it routine rather than exceptional, so it should be closed
before the script ships to an event. Two candidates, in order of cost:
teach the CLI to register a passkey; or teach it to rebuild receipts from `GET
/v1/contributors/me/credit` (`ingest.rs:13892-13931`), which returns the
caller's own records with no ids supplied and would at least make the keep
reconstructible from the key alone.

Shape — one command, and the working directory is the only variable, exactly
as Paxel has it:

```sh
# from a project directory: that project
# from a parent of several repos: all of them
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/contribute.sh | sh
```

Scoping comes from Slice C, so the script needs no scoping flags of its own and
inherits the `$HOME`-and-root refusal with them.

## Error handling

Gemini adapter failures are label-only: `unreadable-gemini-session`,
`malformed-gemini-json`, `gemini-session-too-large`. No content, no paths.

Auto-enroll failure aborts before discovery or upload, so a failed enrollment
never leaves a partially-submitted run.

The scoped attestation route keeps the existing fail-closed 503 when the
signing key or gate-driver pool is unconfigured (`ingest.rs:14037`).

## Testing

TDD, against sanitised fixtures under
`crates/trace-commons-contributor/fixtures/gemini-cli/`.

- One fixture per Gemini message type, asserting the mapping table.
- `thoughts[]` becomes `Reasoning`; `--no-reasoning` drops it.
- `content` as string and as part-array both parse; `displayContent` never
  appears in any output.
- An unknown message type maps to `Opaque` rather than rejecting the file.
- `.project_root` present and absent; `cwd` never serialised either way.
- Symlink escape from the Gemini root is refused.
- `session_hash` and `submission_id_for` determinism; `discover` and
  `session_at` produce identical refs.
- An existing `daemon-settings.json` with only `claude_source` and
  `codex_source` loads into the new `SourceRoots` map unchanged.
- `roots_declared` stays true for claude + codex with no Gemini declaration, so
  an upgraded desktop install still starts.
- Trajectory auto-discovery finds `*.trajectory.json` in cwd and files in the
  conventional directory, and nothing else — including a plain `session.json`.
- Bare `submit` from a project directory selects that project's sessions and no
  others; from a parent of several repos it selects all of them.
- Bare `submit` from `$HOME` or `/` refuses and names `--all`, rather than
  offering a full-history upload behind one keystroke.
- `--all` still reaches everything regardless of the working directory.
- `--json` output is byte-identical to today for every invocation in
  `docs/collector-integration.md`.
- An **unscoped** attestation request returns a document with no `pending` or
  `unknown` key at all — the regression test protecting Devfolio's verifier.
- A scoped request returns exactly the asked-for entries; a submission with no
  gate decision lands in `pending`; an unowned id lands in `unknown` without
  revealing whether it exists elsewhere.
- Unscoped truncation drops the oldest, not an arbitrary UUID range.
- `--attest-post` refuses a host outside the allowlist and still writes the
  attestation locally.
- `submit --attest-out` on timeout writes the attestation and reports the
  pending count rather than failing.
- The one-time script run twice enrolls once and does not spend a second invite
  use: the second run finds the keep and the server returns
  `OnboardDeviceKeyStatus::Idempotent`.
- The script never writes outside its cache directory and the keep, and puts
  nothing on `PATH` — the contract Slice F's "nothing that reads as
  installation" claim rests on.
- The keep is created mode `0700` and the device key within it `0600`.
- After a run, `account login` against the keep reaches an account whose
  principal set contains the run's `auth_principal_ref`, and `withdraw`
  revokes a submission from that run. This is the regression test for the
  consent property in F2: if it fails, the script is producing traces its
  contributor cannot retract.
- A truncated `contribute.sh` executes nothing (the `main` wrapper).

## Consequences

### Canonical representation

Gemini sessions are new traces, so no existing scored trace changes and no
re-score is required. `gate-calibrate` floors should be re-checked against a
post-change sample once Gemini traces are in the corpus.

### Gemini session retention

Gemini CLI prunes on its own schedule (`general.sessionRetention`), defaulting
to `maxAge: "30d"` on the install checked here. That covers a hackathon, so it
is not a blocker for this design, but a contributor installing months later
recovers far less history than the Claude Code or Codex stores would give
them — another argument for the daemon watching the Gemini root.

### Scoring throughput is the real deadline risk, and this design does not fix it

The driver processes 5 submissions every 45 seconds (`ingest.rs:995-999`), FIFO
by `received_at` — roughly 6.7 per minute for the whole tenant. A hackathon
whose participants all submit in the final hour presents a queue that drains
far slower than it fills, and every one of those contributors gets an
attestation dominated by `pending`.

Slice E makes that honest rather than invisible, and D3 gives Devfolio a way to
collect the scores once they land. Neither drains the queue. Deadline drop-off
is Devfolio's actual complaint, so the operator settings — batch size,
interval, and whether the driver is enabled — need an owner and a decision
before the next event. That decision is not in this design's scope, but it
should not be left to be discovered during the event.

The contributor-side mitigation is to submit throughout rather than at the end,
which is what the daemon is for, and worth saying plainly in the quickstart.

### A live bug this work uncovered, not fixed here

Upstream's trajectory-v1 schema now carries `system` and `observation` roles
alongside `meta`, `user`, `reasoning`, `assistant`, and `tool` — confirmed in
the 0.3.0 tarball's `schema/trajectory-v1.schema.json`. Our reader matches a
fixed set of roles and ends `_ => bail!("unknown_record")`
(`trajectory.rs:218`), which rejects the **entire file**.

A contributor who does everything correctly can therefore have a valid,
schema-conforming trajectory refused. This predates and is independent of every
slice here. It should be fixed on its own, ahead of them, and is tracked in the
separate Letta plan.

### Out of scope

- A Gemini adapter contributed upstream to Letta Trajectory.
- Server-side ingest of any new trace format. Slices D and E do change a server
  route and the collector document; that is the only server-side work here.
- Devfolio's verifier API.
- Recursive or `$HOME`-wide trajectory discovery.
- Native adapters for the six harnesses Trajectory already covers. Slice A1
  makes the next one cheap; it does not write it.
- Shortening the Letta-side conversion workflow (item 1b). See "Native Gemini
  adapter" above for why, and what it would take.
- Shortening installation itself, which item 2 also gestures at.
- Raising gate-scoring throughput.
