# Insights: which model did what in your sessions -- design

Date: 2026-10-20. Owner: Zaki. Status: revised after review (PR #1501,
poldsam), owner decisions recorded 2026-10-20.

## Goal

The Patterns tab answers six questions from the contributor's own watched
sessions on this machine:

1. Bug fixing: which model had the fewest rejected edits?
2. Refactoring: the same.
3. Tests: the same.
4. Documentation: which model cost least per session?
5. Across task types: which model had the fewest rejected edits?
6. Across task types: which model finished sessions in the least active
   time? (The owner chose "fastest to finish" over "saves the most time":
   there is no recorded baseline for time saved.)

Each answer is a scoped fact about this contributor's sessions, never a
benchmark: "Fewest rejected edits in your bug-fix sessions: claude-opus-4-7
(4%, 31 sessions)". It shows the leading model, the next one and the
session counts behind both. When the evidence is too thin, unpriced or
within noise, it says which.

## Constraints

- **Local only, with one opt-in exception.** Nothing in this design leaves
  the machine. The exception is the opt-in private judgements of
  [`2026-10-20-insights-private-judgements-addendum.md`](2026-10-20-insights-private-judgements-addendum.md):
  off by default, with its own consent, and sending only what its open owner
  decision O1 settles. Rows store counts, keyed digests and fixed labels only:
  no prompt or response text, no paths, no file names, no commit hashes, no
  prices. That holds with judgements on too. A judged row gains labels, never
  text.
- **Read each file once, for counting.** Session facts are collected by the
  counter pass, which since #1499 streams every watched Claude Code and Codex
  session line by line on its own worker, off the watcher tick. The counter
  pass makes no second pass. The one other read is the addendum's: when
  private judgements are on, a session's file is read once more, at send time
  and in memory, to build its digest. That is never during the counter pass,
  and nothing it reads is stored (addendum, "When the digest is built").
- **Never zero for unknown.** A fact that could not be read is unknown and
  stays out of every figure it would change. Each answer's (i) gives the
  number of sessions left out and why.
- **Scoped wording.** No answer says "best", "better model", "fastest
  model" or "benchmark" (section 8).

## Owner decisions (2026-10-20)

| # | Decision |
|---|---|
| M1 | Task type comes from **local heuristics**. A NEAR AI private-inference classifier may later plug into the same trait. That is a separate decision, because it would be the first Insights data to leave the machine. |
| M2 | "Rejected" means **denied edits plus undone edits**. Questions 1-3 and 5 rank on that. **Not committed** is shown as its own figure beside the rejection rate and never feeds the ranking, because squash merges and rebases make it noisy. (Revises the first draft, which listed not-committed as a third part of "rejected".) |
| M3 | The time question shows **fastest to finish**: median active time of finished sessions, plus the contributor turns they took. |
| M4 | **Extend the counter pass.** Rejected: a separate model-report pass (reads every file twice); reusing Analyze's comparison tasks (manual, and covers only sessions the contributor picked). |
| M5 | **D5 settled: Insights shows money, labelled.** The NEAR AI ledger's `cost_usd` is shown as a charge. Everything else is priced "at API prices" from reviewed list-price data shipped in the app; no runtime fetch. Rows store tokens only; the answer layer prices them. An unpriced model shows "Not priced", never "Not enough sessions" and never $0. |
| M6 | **D11 settled: a Codex per-turn reader ships in v1** (section 2). `CODEX_PER_TURN_SERIES` becomes `true`. |
| M7 | **The git "not committed" signal runs with the counter pass**, gated by `insights_counter_pass` and each folder's rule, with no opt-in of its own. The review proposed a default-off setting; the owner chose this instead, so that a contributor who turned on the counter pass gets the whole Models card. This widens the outcomes contract (section 4). |
| M8 | Session costs on the **Sessions tab** use the same pricing (section 3) and ship as their own PR. |

Defaults taken in this spec, for the owner to confirm, are marked "default
taken".

## Design

### 1. Session facts (daemon, counter pass)

New optional fields on a counter row, with serde defaults. A row written
before this change reads as "facts unknown" and is read once more, the way
#1499's `streamed` flag works.

**Not duplicated.** Turns and tokens per model are not stored again. For
Claude they are derived at answer time from the row's `series` and
`model_labels`; for Codex, from the per-turn series of section 2. Only the
facts below are new.

**Per model** (keyed by index into `model_labels`), each count optional,
absent meaning unknown:

- `edits`: edit tool calls the model made. Edit tools are Claude's `Edit`,
  `Write`, `MultiEdit` and `NotebookEdit` (the `ToolKind` edit set) and
  Codex's `apply_patch`. Edits made through a shell (`Bash`, Codex `exec`,
  `exec_command`, `shell`) are not seen. A Codex session with shell calls
  but no `apply_patch` call has `edits` unknown, not zero.
- `denied`: edit tool calls the contributor declined (below).
- `undone`: edits a later edit restored (below).
- `interrupted`: interrupts that ended one of this model's turns (below).
- `committed`, `uncommitted`, `commit_unknown`: section 4.

**Per session:**

- `shape` (the `SessionShape` counts of section 5), `task` (the inferred
  task) and `classifier_version`.
- `active_secs`, `finished` and `user_turns` (below).
- `edit_mode`: `prompting` when every edit call was made where the
  contributor is asked, `not_prompting` when none was, `unknown`
  otherwise (below).

#### Denied

Claude Code returns a declined call as a `tool_result` whose text contains
"The user doesn't want to proceed with this tool use", with the same text
repeated in the line's `toolUseResult`. The reader counts **one denial per
`tool_result` block, keyed by `tool_use_id`**, and only when that id belongs
to an edit tool call. A declined `Bash`, `AskUserQuestion` or `SendMessage`
call is not a denied edit. Measured on the owner's 80 most recent Claude
sessions: 30 declined calls, 27 of them `Bash`, none on an edit tool, and
every line in `permissionMode: auto`.

`denied` is known only when the contributor could have been asked:

- **Claude**: `permissionMode` is read per line. `denied` is known for a
  model when every one of its edit calls was made under `default`. Under
  `auto`, `acceptEdits`, `bypassPermissions`, `dontAsk` or any value the
  reader does not know, edits are not put to the contributor, so `denied`
  is unknown, never zero. (`plan` makes no edits.)
- **Codex**: `turn_context.approval_policy` and `approvals_reviewer` are
  read per turn. `denied` can be known only when the policy is not `never`
  and the reviewer is the contributor. With `approvals_reviewer:
  auto_review` (every one of the owner's 6,084 recent turn contexts) or an
  unknown value, it is unknown.
- **Codex markers are not confirmed.** A search of the owner's 600 most
  recent Codex sessions found no contributor denial. The only near-matches
  were "aborted by user after Ns" (an exec abort, counted as an interrupt
  below) and strings inside command output. Until a denial marker, and the
  `approvals_reviewer` value that names the contributor, are seen in a real
  Codex transcript and pinned by a fixture made from it, Codex `denied` is
  unknown in every mode. Confirming them is a PR 1 task; if no such
  transcript is found, v1 ships with Codex `denied` unknown.

#### Undone

An edit counts as undone when a later edit in the same session restores the
region it changed **and** a contributor prompt, a denial or an interrupt
falls between the two. A restore with none of those between is the model's
own try-and-revert or debugging, and is not counted.

Covered: `Edit`, each edit inside `MultiEdit`, and each update hunk of
Codex `apply_patch`. A later edit on the same path whose old text equals the
earlier new text, and whose new text equals the earlier old text, is a
restore. Only keyed digests of those strings are held, and only during the
scan. Not covered: `Write` overwrites, `NotebookEdit`, shell commands
(`git checkout -- f`, `git restore`, `sed`) and reverts in an editor. The
(i) says "undone edits we could see".

#### Interrupted

Claude: a user line whose text starts with "[Request interrupted by user]"
or "[Request interrupted by user for tool use]", with or without a trailing
newline. Codex: an `event_msg` of type `turn_aborted` with reason
`interrupted`, or an exec output starting "aborted by user". The "for tool
use" interrupt that immediately follows a denied call is part of that
denial and is not counted again. Interrupts are shown beside rejections,
never added to them: an interrupt is not always a rejection.

#### Active time, turns and "finished"

`user_turns` counts contributor prompts: user lines that carry prompt text,
not tool results and not interrupt markers. For Codex, user messages that
open a `task_started`.

A prompt's span runs from the prompt's timestamp to the last model event
(assistant line, tool result, Codex `task_complete` or `turn_aborted`)
before the next prompt. Long builds and test runs inside a span count in
full. Time after the last model event, while the contributor reads or is
away, is in no span. From each span, subtract the time from a tool call to
its result when the call waits on the contributor: `AskUserQuestion`,
Codex `request_user_input` and `request_user_input_async`, and any call
that ended in a denial. `active_secs` is the sum. A session with a missing
timestamp in any span has `active_secs` unknown.

Waits to approve a call that was then allowed are recorded in neither
transcript, so a `prompting` session may include them. Question 6
therefore compares sessions of one `edit_mode` only (section 6). This rule
replaces the first draft's 300 s gap drop; `time_evidence.rs` does not
compute active time and is not used.

A session is **finished** when it has at least one edit, its last turn ends
in model text with no unpaired tool call (Claude) or in `task_complete`
(Codex), and no denial, undo or interrupt comes after its last contributor
prompt. Only finished sessions enter question 6.

#### Partial rows

| Cause | Rejection (Q1-3, Q5) | Cost (Q4) | Time and turns (Q6) |
|---|---|---|---|
| Delegated (subagent) transcripts not read | left out: their edits are unseen | left out: their tokens are unseen | kept: the parent's timeline covers the delegated work |
| Series caps hit (`MAX_SERIES_TURNS`, `MAX_SERIES_TOOL_CALLS`) | left out | left out | left out |
| A line over `COUNTER_PASS_MAX_LINE_BYTES` | the row is unknown | unknown | unknown |

The (i) on each answer gives the count left out per cause. Tests pin each
cell.

#### Store compatibility

`CounterRow` is `deny_unknown_fields` (`insights_week.rs:178`). A daemon
rolled back past this change cannot read the new rows, so it starts the
store again (`insights_week.rs:794-797`) and re-reads every session within
its per-poll budget. Nothing is lost but time. Serde defaults protect
upgrades only. This is accepted, not fixed.

### 2. The Codex per-turn reader (v1)

With D11 settled, the Codex scan produces the same shape as Claude's.
`RowBody::Codex` gains a `UsageSeries` and `model_labels`; `observed`
stays for rows the reader cannot place per turn.

- **Model per turn**: `turn_context.model`. Each `turn_context` opens a
  turn, labelled with that model.
- **Tokens per turn**: `event_msg` of type `token_count`, field
  `info.last_token_usage` (present on 51,649 of the 51,650 such events in
  the owner's 600 recent sessions).
- **Tool calls**: `response_item` of type `custom_tool_call` or
  `function_call`. `apply_patch` is an edit: its hunk paths give the path
  digests and its update hunks feed `undone`. `exec`, `exec_command` and
  `shell` are shell calls. Others are `Other`.
- **Times**: the top-level `timestamp` on every line; turn boundaries from
  `task_started`, `task_complete` and `turn_aborted`.
- **Modes**: `turn_context.approval_policy` and `approvals_reviewer`
  (section 1, Denied).
- **Reviewer sessions**: sessions whose turns all declare
  `codex-auto-review` are Codex's automatic reviewer, not the contributor's
  work. They keep their token totals and are left out of every model
  answer. On the owner's machine they never share a file with other models.

On the owner's machine, 12 of 21,848 recent custom tool calls are
`apply_patch`; most edits go through `exec`. So most Codex sessions will
have `edits` unknown, and Codex rows will mostly answer the cost and time
questions, not the rejection ones. The example in section 8 names no Codex
figure for that reason.

### 3. Pricing (shared with the Sessions tab cost PR)

- **Data.** The production catalog in `pricing_catalog.rs` is empty today
  (line 3, pinned by the test at lines 266-268). A pricing-data PR adds
  reviewed list-price tables, one table per source page, with
  `provenance.source_url` and `retrieved_at` set from that page. No runtime
  fetch. The production-catalog test changes to pin the shipped tables.
- **Cache writes.** `BillableTokenCategory` has a single
  `CacheCreationInput`. The pricing-data PR splits it into a 5-minute and a
  1-hour write rate. `CacheWrite::Split { m5, h1 }` prices each part at its
  own rate. `CacheWrite::TotalOnly` (the proxy ledger; lifetime unknown) is
  never list-priced. The (i) names the rates used.
- **When.** Rows store tokens. An answer prices each turn with
  `select_newest_applicable` for its model and date, when the answer is
  built. A turn whose model has no applicable entry is unpriced, and so is
  its session.
- **Labels.** List-priced figures read "at API prices". The (i) says this
  is what the API would charge, not what a subscription plan costs.
- **Ledger charge.** A session with calls through the proxy shows the
  ledger's `cost_usd` as "charged". Default taken, owner to confirm: the
  charge is not kept in the counter store, so it is shown only while the
  proxy ledger still holds the call (24 hours); after that the session
  reads "charge no longer recorded". The alternative is one stored
  per-session charge on the route tally, which would be the store's only
  price. Question 4 leaves out every session with a route tally, because
  its charge does not last the 13-week window and API list prices do not
  describe it. Settled 2026-10-20 (owner): the alternative was taken. The
  route tally stores each session's NEAR AI charge per model, in integer
  USD micros, as the ledger is folded, and Question 4 shows the median
  charge per session of its routed sessions as its own figure
  (`near_ai_charge`) beside the API-price one, never summed with it.
- **Unpriced.** A model whose sessions meet the session floor but are not
  priced shows "Not priced", never "Not enough sessions" and never $0.
- **What changes in code and tests:**
  - `LEDGER_MONEY_IN_INSIGHTS` (`analytics_constants.rs:145`) becomes
    `true`, its const assert (`:292`) flips, and its doc says D5 is
    settled.
  - The store still holds no price. The store test that forbids "cost"
    (`insights_week.rs:4153`) and the module doc's "never ... a price"
    (`insights_week.rs:71`) stay.
  - Model answers go in a new IPC method (section 7), so `insights_week`
    still carries no price: its doc (`insights_week.rs:2008`) and its
    forbidden-string test (`insights_week.rs:3488`) stay.
  - The new method's doc and tests say that it carries prices and list
    exactly which fields do.
  - `insights_route_tally.rs` and `insights_glance.rs` say "D5 open:
    `cost_usd` is never read". The Sessions cost PR changes the reader and
    that wording; the model-answers PRs do not.

The Sessions tab cost PR uses this pricing module unchanged: a session's
API-price cost from its stored tokens, and the ledger charge while the
ledger holds it.

### 4. Not committed (daemon, counter worker, its own PR)

Owner decision M7: this runs whenever the counter pass runs, with no opt-in
of its own. **This widens the outcomes contract.** `outcomes.rs:1` says
"never inferred acceptance", and the evidence contract
(`docs/superpowers/plans/2026-09-11-insights-evidence-contracts.md`, lines
34-40) limits git inspection to one named commit in an explicitly selected
repository. This job reads recent commit history in the repository of
every watched session, on a timer, and reports whether the files a model
edited were committed. PR 3 amends that document to say so: the figure is
an observation that a file appeared in a local commit, not acceptance, and
it never feeds a ranking (M2).

**What is held.** Claude's series already records each edit call's
`ToolCallRecord.path_key`, a keyed digest of the transcript's path under
the counter key (`insights_usage_series.rs:175`, `:397`); the Codex reader
records the same. For the git check the scan also records, per edit, a
keyed digest of the **repo-relative** path, in a new digest domain under
the same key. The edited path is normalized with `normalize_tool_path`,
joined to the session's working directory (Claude `cwd`, Codex
`turn_context.cwd`) when relative, and stripped of that directory as a
prefix. A path that does not start with it is outside the repository and
counts as `commit_unknown`. These digests sit in the row, inside the
counter store, and are never a path.

**The join.** `git log` and `git ls-files` print repo-relative paths. Each
goes through the same normalization and keyed digest and is compared with
the row's digests. No path is stored or logged. When git reports the
repository as case-insensitive (`core.ignorecase`), both sides are
case-folded before digesting.

**When.** A row with edits is pending until `COMMIT_WINDOW` (24 h) after
its session's last event. On the first poll after that, the counter worker
checks it. The repository is the session's working directory, resolved
through the job's `project_of` and the watcher's cwd cache; the row never
holds it. A row the per-poll caps defer is retried on later polls until
`COMMIT_CHECK_DEADLINE` (48 h after the session's last event); then its
edits become `commit_unknown`. Once the check runs or the deadline passes,
the row keeps only the three counts and its repo-relative digests are
dropped.

**The check**: three read-only calls through `read_git` and its 10 s
`GIT_TIMEOUT`, each output capped at `COMMIT_CHECK_MAX_BYTES` (1 MiB):

1. `git log --branches --since=<session start> --until=<last event + 24 h>
   --name-only --no-renames --format= -z -n <COMMIT_CHECK_MAX_COMMITS>`
   (500): files changed by a local commit in the window.
2. `git ls-files -z`: tracked files.
3. `git ls-files -z --others --ignored --exclude-standard`: ignored files.

An edit is `committed` when its digest is in (1); `commit_unknown` when it
is in (3); otherwise `uncommitted`. Hitting a cap or a timeout makes all of
that row's edits `commit_unknown`.

**Unknown, never uncommitted:**

- the working directory is not a repository's top level, including a
  session started in a subdirectory of a repository (`read_git` refuses it
  and does no parent discovery);
- the working directory cannot be resolved, or the folder is Never (a
  Never folder is not read at all, as today);
- the file is outside the repository, or ignored;
- any git call fails, times out or hits a cap;
- the deadline passes first.

**Bounds.** It runs on the counter worker, after the job's file reads,
never on the watcher tick. Per poll: at most `COMMIT_CHECK_MAX_REPOS` (4)
repositories and `COMMIT_CHECK_MAX_GIT_CALLS` (12) git calls. `clear()`
abandons it between calls, as it abandons a pass.

**Row replacement.** A session file that grows replaces its row. The new
row's digests come from the new read and its window restarts from its new
last event; the old counts are dropped and recomputed. A row evicted by the
store caps, and the whole store on unenroll (`clear()`), take their digests
with them.

**Tests over serialized bytes**: the store and the IPC payload contain no
path, repo-relative path, file name or commit hash; after the check runs no
repo-relative digest remains in the row; after `clear()` no file remains.

### 5. Task classifier (pure function, contributor-insights crate)

`classify(&SessionShape) -> InferredTask`, behind a `TaskClassifier` trait
with one implementation now. `InferredTask` is a new enum: `BugFix`,
`Refactor`, `Tests`, `Docs`, `Other`, `NoEdits`. `TaskCategory` does not
change: it is "a user label on a saved session proxy, not an inferred task
boundary" (`insights.rs:115`), and it feeds annotations, comparison specs
and `EpisodeCategory` (`card_store.rs:343-351`). Where a shared label is
needed, `InferredTask` maps to it: `BugFix` to `Debugging`, `NoEdits` to
`Other`, the rest by name. There is no `Mixed`.

`SessionShape` holds counts only: edited files per extension class (test,
doc, source, config); files renamed or moved by `git mv` or `mv` commands,
counted during the scan without keeping the command; lines added and
removed by edit calls; and, per command `args_key`, whether it failed
before the session's first edit and whether it later passed. The shape is
stored on the row with `classifier_version`. A version bump that changes
only rules or thresholds re-classifies from the stored shape; one that
changes the shape itself re-reads each row once.

Rules, in order. **The first match wins.**

1. **Docs**: every edited file is doc-class: `.md`, `.mdx`, `.rst`,
   `.adoc`, or `.txt` inside a `docs/` or `doc/` directory. `.txt`
   elsewhere is config-class (`requirements.txt`, `CMakeLists.txt`).
2. **Tests**: more than half of the edited files are test-class (a `tests/`,
   `test/`, `__tests__/` or `spec/` directory, or `_test.`, `.test.`,
   `.spec.` or a `test_` prefix in the name).
3. **BugFix**: a command that failed **before the session's first edit**
   passes after the session's edits, and at least one edited file is not
   test-class. A test written in the session and then made to pass does not
   qualify.
4. **Refactor**: 3 or more files renamed or moved, or 5 or more files
   edited with net lines within 10% of lines changed.
5. **Other** when there are edits; **NoEdits** otherwise. NoEdits sessions
   are left out of every question.

Extension classes and thresholds live in `analytics_constants.rs`. A
synthetic fixture set pins each rule, including the TDD case of rule 3 and
the `.txt` case of rule 1. Tuning targets the owner's corpus.

### 6. Answers (contributor-insights crate)

**Input.** `SessionInput` (`week_rollup.rs:146`) gains `facts:
Option<SessionFacts>`. The daemon fills it from counter rows; saved
snapshots set `None`, which is unknown, so they are left out.
`model_answers(&[SessionInput], now) -> ModelAnswers` covers the 13 weeks
ending at `now`, the counter store's retention
(`COUNTER_STORE_KEEP_WEEKS`), whatever week is on screen.

**Model labels.** `<synthetic>` turns are dropped. Labels are lower-cased,
and a trailing `-YYYYMMDD` date stamp or `-latest` is removed. A context
suffix such as `[1m]` is kept, because it may be priced differently.
Reviewer sessions (section 2) are left out.

**Routed sessions.** A label in a session sent through the proxy may not
name the model that served it (`model_notice`, `service.rs:337`). Default
taken, using the route tally's categories (`insights_route_tally.rs`):
sessions in `unobserved`, `outside` and `routed_verified` are compared;
sessions in `unrecorded`, `routed_unverified`, `mixed` and `check_failed`
are left out, with their count in the (i). While the ledger feed is off,
every session is `unobserved`; "labels as recorded" covers that case.

**Attribution.** A session counts for model *m* only when *m* served a
strict majority of its turns; a session with no majority counts for no
model. Rates for *m* use only *m*'s own edits in the sessions counted for
*m*. A session's cost is each model's tokens priced at that model's own
rates, summed; nothing is split by turn share.

**Cells.** For each (task type, model):

- sessions;
- rejection rate (below);
- not-committed rate: uncommitted / (committed + uncommitted), separate;
- interrupts per session, separate;
- median API-price cost per session, over priced sessions;
- median active time and median contributor turns, over finished sessions.

A cell **qualifies** with at least `MODEL_ANSWER_MIN_SESSIONS` (5) sessions
and, for rejection figures, at least `MODEL_ANSWER_MIN_EDITS` (30) edits. A
question needs two qualifying models.

**Rejection rate.** When `denied` is known in every session of the cells a
question compares: (denied + undone) / edits, labelled "rejected".
Otherwise every model in that question uses undone / edits, labelled
"undone". One question, one basis. A session with `edits` unknown is out of
the rejection figures.

**Questions 1-3** compare the cells of one task type. **Question 4**
compares median cost per Docs session over priced sessions. **Questions 5
and 6** compare across task types without letting the task mix decide:

- For each model, take the task types where its cell qualifies.
- Weight each type by the contributor's own session count in it, across all
  compared models.
- A model's figure is the weighted mean of its per-type figures over its
  qualifying types, with the weights renormalized.
- A model is compared only when its qualifying types carry at least
  `MODEL_ANSWER_MIN_MIX_COVERAGE` (80%) of the total weight. Otherwise the
  (i) says its task mix is too different to compare.
- Question 6 compares sessions of one `edit_mode` only. It uses
  `not_prompting` when two models qualify there, and `prompting` otherwise.

**Noise.** A leader is named only when the gap holds across resamples: a
bootstrap over sessions within each cell, `MODEL_ANSWER_BOOTSTRAP_DRAWS`
(1,000) draws from a fixed seed, so the same rows always give the same
answer. The leader is named when it leads the next model in at least 95% of
draws. Otherwise the row reads "No clear difference" and shows both
figures.

**Order.** Models are ordered by the figure (lower first), then more edits
(priced sessions for Q4, finished sessions for Q6), then more sessions,
then label byte order. The order is total, so iteration order never decides
an answer.

**Row states**, checked in this order:

1. "Not enough sessions yet": fewer than two qualifying models. Shows the
   per-model counts.
2. "Not priced" (Q4 only): two models have enough sessions, but fewer than
   two have enough priced ones.
3. "No clear difference": the bootstrap test fails.
4. The answer: the leader, the next model, both figures and sample sizes.

### 7. IPC

A new method, `insights_model_answers`, with no parameters, gated like
`insights_week` on `insights_counter_pass`. It returns the six rows: state,
basis (`rejected` or `undone`), up to two models as declared labels with
their figures and sample sizes, the not-committed and interrupt figures,
and the counts left out per reason (partial, routed, reviewer, unpriced,
too few edits, mix too different). Money is integer USD micros with a
`pricing` tag (`api_list`) and the pricing table id and version. Never an
id, digest, path, file name, backend or commit hash. Documented in
`docs/contributor-daemon-ipc-v1_1.md`. It is a new method rather than a
part of `insights_week`, so `insights_week` keeps its no-price contract and
tests. Older clients never call it.

### 8. Patterns "Models" card (macOS first)

The card's copy follows the "trace" wording ruling (2026-10-11): it says
"traces" where this section says "sessions" (owner decision, 2026-10-20).

A list of the six questions. Each answered row reads as a scoped fact, the
leading model first and the next one underneath:

> Fewest rejected edits in your bug-fix sessions: claude-opus-4-7 (4%, 31
> sessions). Next: claude-sonnet-4-6 (9%, 14 sessions).

Every row shows "in your sessions", "labels as recorded" and "task type
inferred" on the row itself, not only in the (i), so a screenshot keeps
them. A row on the `undone` basis says "undone edits" instead of "rejected
edits". The not-committed figure and interrupts sit beside the rejection
figure. Q4 rows say "at API prices". The (i) says how each figure is
measured, what was left out and why, which cache-write rates applied, and
that undone edits are the ones we could see.

A test pins forbidden phrases in the card copy and the IPC labels, as
`cli/insights.rs:1393` pins "better model": "best", "better model",
"fastest model", "benchmark", "winner". Copy goes through the shared copy
module and follows `docs/ui-copy-guide.md`. Other shells follow later, as
with the other Patterns cards.

## Delivery

0. **PR 0, pricing data:** reviewed list-price tables with provenance, the
   5-minute and 1-hour cache-write split in `BillableTokenCategory`, the
   pricing module of section 3 and its tests. Shared with the Sessions cost
   PR.
1. **PR 1, daemon facts, Codex reader and answers:**
   - the session facts on counter rows (section 1);
   - the Codex per-turn reader, `CODEX_PER_TURN_SERIES = true`, and the
     Codex marker confirmation (section 2);
   - the classifier (section 5) and `model_answers` (section 6);
   - `LEDGER_MONEY_IN_INSIGHTS = true` and the doc and test changes of
     section 3;
   - the `insights_model_answers` method;
   - fixtures and tests, and re-reading existing rows once.

   Depends on #1499 (merged) and PR 0.
2. **PR 2, macOS Models card:** the card, its copy, the forbidden-phrase
   test and Swift tests. Depends on PR 1.
3. **PR 3, not committed:** the git check on the counter worker (section
   4), the amendment to the evidence-contracts document, and the figure on
   the card. Depends on PR 1.
4. **Sessions tab cost:** API-price cost and ledger charge per session, on
   PR 0's pricing module. Its own PR, any time after PR 0.
5. **After v1:** a NEAR AI private-inference `TaskClassifier`, behind a
   contributor setting and its own privacy and copy review.

## Testing

- Synthetic transcript fixtures for each classifier rule, each Claude
  denial and interrupt spelling, each permission mode (`denied` known and
  unknown), each Codex approval setting, and the reviewer-session rule.
  Codex denial fixtures come only from a confirmed real transcript.
- Denial: one per `tool_result` though the text appears twice; a declined
  `Bash` or `AskUserQuestion` is not counted; the interrupt after a denial
  is not counted.
- Undone: a restore after a contributor prompt counts; the same restore
  with no prompt, denial or interrupt between does not.
- Active time: a long build inside a span counts; the idle tail does not;
  `AskUserQuestion` waits are subtracted.
- `model_answers` unit tests: the session and edit floors; the `undone`
  basis when any compared session has `denied` unknown; strict majority
  (50/50 and three-way sessions count for no model); per-model pricing;
  `<synthetic>` and dated labels; each partial-row cell of section 1; each
  route category; mix coverage; the fixed bootstrap seed (same rows, same
  answer) and "No clear difference"; the total order; "Not priced" versus
  "Not enough sessions yet"; unknown facts never counted as zero.
- An equivalence test: rows without facts keep today's Patterns output.
- Privacy tests on serialized bytes, as the counter store tests do: the
  stored row holds no path, file name, text, commit hash or price; the
  `insights_model_answers` payload holds no path, file name, id, digest,
  backend or commit hash, and its money fields are exactly the ones listed.
- Pricing tests: shipped tables carry a source URL and date; a `Split`
  cache write prices each part at its rate; a `TotalOnly` write is never
  list-priced; an unpriced turn makes its session unpriced.
- Not-committed tests per section 4, including a subdirectory working
  directory, an ignored file, a file outside the repository, a deferred row
  passing its deadline, and row replacement.
- Swift tests for the card's rows, each row state, the scope line and the
  VoiceOver labels.

## Not in scope

- Any data leaving the machine; that is the private-inference classifier.
- Cross-contributor or global model rankings.
- A "time saved" estimate.
- Contributor-set task labels (`task_source: contributor`). Counter rows
  are read-only to shells and are rebuilt when their file grows, so there
  is no write path that would keep one.
