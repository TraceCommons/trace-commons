# Insights: which model is best for what -- design

Date: 2026-10-20. Owner: Zaki. Status: approved in conversation, for spec review.

## Goal

The Patterns tab answers, from the contributor's own watched sessions on this
machine:

1. Which model is best for bug fixing?
2. Which model is best for refactoring?
3. Which model writes the best tests?
4. Which model is cheapest for documentation?
5. Which model creates the least rejected code?
6. Which model is fastest to finish? (The owner chose "fastest to finish" over
   "saves the most time": there is no recorded baseline for time saved.)

Success: each question shows a named model, a runner-up and the session counts
behind both. When the evidence is too thin it says so instead of naming a
winner.

## Constraints

- **Local only.** Like all of Insights, nothing here leaves the machine. Rows
  store counts and labels only: no prompt or response text, no paths, no file
  names, no commit hashes. These are the counter store's rules.
- **Read each file once.** Session facts are collected by the counter pass,
  which since #1499 reads every watched Claude Code and Codex session line by
  line, off the watcher tick. No second pass over the sessions.
- **Never zero for unknown.** A fact that could not be read is unknown and
  stays out of every answer. The counter store follows the same rule.
- **Honest comparisons.** An answer needs at least two models that each have
  enough sessions of that task type. Below that, it shows "Not enough sessions
  yet" with the counts.

## Decisions (owner, 2026-10-20)

- Task type comes from **local heuristics** first. A later NEAR AI private
  inference classifier plugs into the same interface. That is a separate
  decision, because it would be the first Insights data to leave the machine.
- "Rejected code" combines three signals: **denied edits**, **edits undone in
  the same session**, and **edits never committed**.
- The time question shows **fastest to finish**: median active time, plus the
  turns the contributor needed.
- Approach: **extend the counter pass** (option 1). Rejected alternatives:
  - a separate model-report pass, which reads every file twice;
  - reusing Analyze's comparison tasks, which is manual and covers only
    sessions the contributor picked.

## Design

### 1. Session facts (daemon, counter pass)

These are new fields on a counter row, all optional, with serde defaults. A
row written before this change reads as "facts unknown" and is re-read once,
the same way #1499's `streamed` flag works.

- `models`: per declared model label, the turns, tokens (uncached, cache read,
  cache write, output) and estimated cost. Cost comes from the existing pricing
  catalog (`pricing_catalog.rs`) and is unknown when the model is not priced.
  A turn is attributed to the model the transcript declares for it (the turn
  series already records it).
- `task`: one of `bug_fix`, `refactor`, `tests`, `docs`, `mixed`, `other`,
  `unknown`. It reuses the existing `TaskCategory` (`Debugging` is "bug fix"
  in copy). If the classifier ends up needing `Mixed`, it is added
  deliberately and the exhaustive-match test is updated. The row also stores
  `task_source: heuristic | contributor | private_ai`.
- `rejections`, each attributed to the model whose turn made the edit or call:
  - `denied`: a tool call or edit the contributor declined. Claude Code
    records the tool result "The user doesn't want to proceed with this tool
    use" / "tool use was rejected". Both appear in the owner's recent
    transcripts (19 and 20 in 60 recent files). The Codex approval-denial
    markers must be confirmed against a real Codex transcript before Codex
    denials are counted. 80 recent Codex sessions on the owner's machine show
    no denial events, likely because they auto-approve. Until confirmed,
    Codex `denied` is unknown, never zero.
  - `undone`: a later edit in the same session restores a file region the
    model changed. It is detected by comparing the old and new strings of edit
    tool calls on the same file, holding only hashes of those strings during
    the scan.
  - `interrupted`: "[Request interrupted by user" (56 in the same 60 files)
    followed by a redirecting prompt. It is reported beside rejections but
    kept separate, because an interruption is not always a rejection.
  - `edits`: the denominator, meaning edit tool calls attempted per model.
- `active_secs`: time from each contributor prompt to the model's last action
  for that prompt, summed. Gaps over `ACTIVE_GAP_SECS` (300) are dropped.
  Unknown when timestamps are missing; the existing `time_evidence.rs` rules
  apply.
- `user_turns`: prompts the contributor sent.

Memory stays bounded like the #1499 scans. Per-file and per-edit state is
capped, and a session past a cap marks its facts partial, never zero.

### 2. Task classifier (pure function, contributor-insights crate)

`classify(&SessionShape) -> (TaskCategory, Confidence)`. `SessionShape` holds
counts only, so the same input can later go to a private-inference backend
behind a `TaskClassifier` trait (one implementation now). The input counts
are: file kinds touched, by extension class (test, doc, source, config);
test-runner invocations; failing-then-passing command pairs; and renames,
moves and net lines.

The first rules, in order. The first match wins, and a session matching two
rules strongly is `mixed`:

- **docs**: edits touch only doc-class files (`.md`, `.mdx`, `.rst`, `.txt`,
  doc directories).
- **tests**: most edited files are test-class (test directories, `_test`,
  `.test.`, `tests/`), or new test functions dominate the edits.
- **bug_fix**: a command fails, then edits follow, then the same command
  passes; or the edit-fail-edit pattern fires on non-test files.
- **refactor**: renames or moves across 3 or more files, or many files edited
  with net lines added near zero.
- Otherwise `other`. Sessions with no edits are `other` and are excluded from
  the quality questions.

Tuning targets the owner's corpus. A test fixture set of synthetic sessions
pins each rule. Extension classes and thresholds live in
`analytics_constants.rs`.

### 3. Not committed (daemon, separate job, its own PR)

For each watched folder that is a local git repository, a low-priority job
checks whether files a model edited during a session were changed by a local
commit within `COMMIT_WINDOW` (24 h) after the session ended. It builds on
the read-only git inspection in `outcomes.rs`. That code inspects one named
commit object today, so walking the commits made after a session is new code,
held to the same read-only rule: it never runs anything that writes. Only hashes of the edited file paths are held during
the scan, and the row gains `committed` and `uncommitted` edit counts per
model. Folders that are not git repositories, and edits whose history cannot
be read, are unknown. Squash merges and rebased history make "not committed"
noisy, so it shows as its own figure beside the in-session signals rather
than mixed into one rate. It can be switched off in Insights settings.

### 4. Rollup and answers (contributor-insights crate)

The `model_answers(rows, window) -> ModelAnswers` function runs over the
13-week window.

- For each (task type, model), it computes:
  - sessions;
  - rejection rate: (denied + undone) / edits;
  - not-committed rate (separate);
  - median cost per session;
  - median active time;
  - median contributor turns.
- **Qualifying:** a model needs at least `MODEL_ANSWER_MIN_SESSIONS` (5)
  sessions of that task type. A question needs at least 2 qualifying models.
- **Answers:**
  - **Questions 1-3** (bug fix, refactor, tests): the lowest rejection rate
    within that task type. Fewer median contributor turns breaks ties.
  - **Question 4** (cheapest for docs): the lowest median cost per docs
    session.
  - **Question 5** (least rejected code): the lowest rejection rate across
    all task types with edits.
  - **Question 6** (fastest): the lowest median active time across all task
    types.
- Each answer carries the winner, the runner-up, both sample sizes and the
  figures. It is otherwise `not_enough_sessions` with the per-model counts.
- **Mixed-model sessions:** cost and time are split by turn share, while
  rejections and edits are attributed per turn. A session counts toward a
  model's sessions when that model served at least half its turns.

`insights_week` gains a `model_answers` part (or a new `insights_models`
method if the payload is large), documented in
`docs/contributor-daemon-ipc-v1_1.md`. Older clients ignore it.

### 5. Patterns "Models" card (macOS first)

The card is a list of the six questions. Each row shows the question, the
winning model and its figure, and the runner-up underneath ("Opus: 4%
rejected, 31 sessions. GPT-5: 9%, 12 sessions"). Rows below the floor say
"Not enough sessions yet" with the counts. An (i) on each row says how the
answer is measured, including that task types come from what the session did,
not what it said. Copy goes through the shared copy module and follows
`docs/ui-copy-guide.md`. Other shells adopt it later, as with the other
Patterns cards.

## Delivery

1. **PR 1, daemon facts and classifier:**
   - the session facts on counter rows, the classifier and `model_answers`;
   - the IPC part;
   - fixtures and tests;
   - re-reading existing rows once.

   Depends on #1499.
2. **PR 2, macOS Models card:** the card, its copy and Swift tests.
3. **PR 3, not committed:** the git job, its setting, and the separate figure
   on the card.
4. **Later:** a NEAR AI private-inference `TaskClassifier`, behind a
   contributor setting and its own privacy and copy review.

## Testing

- Synthetic transcript fixtures for each classifier rule and for each
  rejection marker. Claude Code markers come from the real formats above;
  Codex denial markers are added once confirmed.
- `model_answers` unit tests cover:
  - the floor (one model, too few sessions);
  - tie-breaks;
  - mixed-model splitting;
  - unknown facts left out, never counted as zero.
- An equivalence test: rows without facts keep today's Patterns output.
- Privacy tests: the stored row and the IPC payload contain no path, file
  name or text. These assert on serialized bytes, as the existing counter
  store tests do.
- Swift tests for the card's rows, the not-enough state and the VoiceOver
  labels.

## Not in scope

- Any data leaving the machine, which is the private-inference classifier,
  later.
- Cross-contributor or global model rankings.
- A "time saved" estimate.
