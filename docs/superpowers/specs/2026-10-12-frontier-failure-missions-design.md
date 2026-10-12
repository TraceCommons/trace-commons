# Frontier failure missions

Status: draft for owner review, 2026-10-12.

## Goal

The most valuable traces in the commons are the ones where a frontier model
was given a well-posed task and failed at it, especially when the trace also
shows what the right answer was. This spec builds a flow that finds those
traces on the device, has the contributor confirm them, verifies them on the
server, and turns the verified ones into evaluation items that stay current
as models improve.

The hard part is not plumbing. Most of the fields exist. The hard part is
that paying for failures invites fabricated failures, and that the model
behind most frontier traces cannot be attested. The design is organised
around those two facts.

## Non-goals

- Inferring failure on the server from trace content alone. The server
  verifies a claim; it does not mint one.
- Sending contributor traces to a third-party model provider for replay.
  Replay uses attested NEAR AI inference only (see "Verification").
- Choosing reward rates. Credit amounts are an owner decision outside this
  spec; this spec fixes only when credit is held, settled or refused.

## What exists today

Read from `main` at 7009d82ae.

- **Outcome fields with nothing writing them.** `OutcomeMetadata`
  (`crates/trace-commons-protocol/src/trace_contribution.rs:472`) carries
  `task_success`, `error_taxonomy`, `failure_modes: Vec<TraceFailureMode>` and
  `human_correction`. `TraceFailureMode` (`:433`) has fourteen values,
  including `PrematureTermination`, `LoopingOrRepetition`, `ContextLoss`,
  `UserIntentMisread`, `MissingVerification` and `UnrecoverableToolFailure`.
  The contributor writes `failure_modes: Vec::new()` at every construction
  site in `envelope.rs`.
- **The one outcome label is asked, not inferred.** `ContributorVerdict`
  (worked / partly / failed) maps to `TaskSuccess` in
  `crates/trace-commons-contributor/src/envelope.rs:407-457`, applied by
  `apply_verdict` in `daemon/uploader.rs`.
- **Struggle signals live in Insights only, on the device.**
  `session_patterns` (`contributor/src/insights/patterns.rs`) finds retried
  calls (same tool and arguments digest), edit-fail-edit loops (an edit, a
  failed Bash call, an edit to the same file) and repeated reads. Markers in
  `insights/markers.rs` add `ContextShrank` and `CrossedLongContext`.
  `insights/outcomes.rs` imports test reports (`TestReportEvidence`). None of
  this reaches the envelope.
- **Tool success is Claude Code only.** `success: Option<bool>` on events is
  set from Claude Code's `is_error` (`source/claude_code.rs`). Other adapters
  leave it `None`. No adapter records an exit code.
- **The model is one string per session.** `ironclaw.model_name`, taken in
  Claude Code from the first `message.model` seen. A session that switches
  models mid-way is attributed to its first model.
- **Missions match tool, folder language and source only.**
  `MissionPredicateV1` (`protocol/src/activity_missions.rs:101`) has `tools`,
  `tool_families`, `languages` and `min_sessions`. Matching is on-device
  (`daemon/mission_matching.rs`, rules M1/M2); the server never evaluates a
  predicate.
- **The server side is partly ready.** `coverage_tags_for_envelope` emits
  `outcome:<task_success>` and `failure:<mode>` tags that reviewers can filter
  on. Benchmark conversion routes exist (`/v1/benchmarks/convert`,
  `/v1/workers/benchmark-convert`, `/v1/workers/benchmark-evaluations/run`)
  behind the `BenchmarkWorker` token, and `ConsentScope::BenchmarkOnly`
  exists. `compute_value_scorecard` already rates Failure and Partial traces
  as harder (difficulty 0.65 against 0.35).

## Definitions

**Frontier model.** A model on a server-published list,
`frontier_models` in the activity policy, matched against the per-call model
label (F1). The list is versioned with the policy so it can change without a
client release. A trace qualifies when the failing calls were made by a listed
model.

**Failure.** The model was asked to do something it could have done, and the
session ended without it done, done wrong, or done only after the person
stepped in. Three things are not failures for this mission: the person
changing their mind, an environment fault the model could not see (network
down, credentials missing), and a task that was impossible as posed.

**Evidence tiers.** Each candidate carries the highest tier it reaches. Tiers
are computed, never claimed.

| Tier | Name | Requires |
|---|---|---|
| E0 | claimed | the contributor said "failed" or "partly", nothing else |
| E1 | corroborated | E0 and at least one on-device detector signal consistent with the claim |
| E2 | checked | E1 and a hard signal: a failing test report or non-zero exit as the last verification in the session, a revert of the model's changes, or a `human_correction` the contributor wrote |
| E3 | verified | E2 and the server's Review-stage verification passed (see "Verification") |

Only E3 settles credit. E0 and E1 are accepted as ordinary contributions but
earn nothing from this mission.

## Threat model

The reward is for failure, so the attacker wants to manufacture one cheaply.

1. **Claimed failure that did not happen.** Mark a successful session
   "failed". Countered by tiers: a bare claim never passes E1 without a
   detector signal, and never passes E3 without the server finding the
   failure in the events.
2. **Ill-posed task.** Ask for something impossible, contradictory or
   under-specified, then record the model's failure. Countered by the
   well-posedness check at Review: a judge model must be able to state what a
   correct answer would be, from the trace.
3. **Sabotaged context.** Feed the model wrong files, misleading
   instructions or a broken environment. Countered by the environment-fault
   exclusion and the judge's sabotage check (instructions that contradict the
   stated goal, injected misleading content).
4. **Mislabelled model.** Claim a frontier model when a weaker one ran. For
   non-attested sessions the model label comes from the transcript and cannot
   be proven. Countered partly by consistency checks (the transcript's
   per-call model labels, the tool's own format) and partly by weighting:
   attested sessions (NEAR AI `provider_tee` receipt bound to a listed model)
   settle at full weight; unattested sessions settle at a reduced weight the
   owner chooses (open decision D2).
5. **Farming one failure.** Submit the same failure many times, or many
   trivial variants. Countered by clustering at Review: one settlement per
   failure cluster per account, and diminishing settlement across accounts
   for a cluster already verified.
6. **Sybil accounts.** Countered by the existing account admission and
   earned-trust tiers (#1373, #1379). This mission's settlement is gated on a
   minimum trust tier (open decision D4).

## Design

Six slices, in build order. F1 and F2 are useful without the rest: they
improve every trace's outcome metadata.

### F1. Signals on the device

- **Per-call model label.** Add an optional `model` to model-turn events in
  the envelope, from the adapter's per-message model field. Keep
  `ironclaw.model_name` as the session's first model for compatibility. This
  is an envelope change and moves the pinned envelope digests; land it with
  the pin updates in the same PR.
- **Exit status.** Where a tool result carries an exit code (Bash in Claude
  Code and Codex), record `exit_code: Option<i32>` on the `ToolResult` event.
  Set `success` from it for adapters that leave `success` empty today.
- **Detectors** in a new module beside `insights/patterns.rs`, each mapping
  to a `TraceFailureMode`:
  - retried identical calls, three or more: `LoopingOrRepetition`;
  - edit-fail-edit, three or more rounds on one file: `LoopingOrRepetition`;
  - last verification in the session failed (test report or non-zero exit
    from a test or build command): `MissingVerification` when the model
    claimed done, otherwise `UnrecoverableToolFailure`;
  - the person interrupted a model turn (Claude Code's interrupt marker) and
    then rewrote the instruction: `UserIntentMisread`;
  - the model's file changes reverted within the session (a `git checkout`,
    `git restore` or `git reset` over files the model edited):
    a revert signal, no mode of its own;
  - the person made the final edits to files the model was working on:
    a takeover signal, no mode of its own;
  - the session crossed long context and the model re-read files it had
    already read: `ContextLoss`.
- Detector output goes to `failure_modes` only when the contributor confirms
  (F2). Unconfirmed, it stays on the device. Detectors run on the device
  only; nothing new is sent before consent.

### F2. Confirmation by the contributor

- A session becomes a **failure candidate** when a detector fires or a hard
  signal is present, and the session's model is on the frontier list.
- At review time, a candidate shows one question in place of the existing
  verdict prompt, offering worked / partly / failed. On partly or failed it
  offers the detected failure modes as choices, and one optional text field
  for the right answer or how it was fixed, which becomes
  `human_correction`.
- `human_correction` goes through the same redaction pass as every other
  text field before upload.
- Copy comes from the core, as for every other string. The wording is not
  in this spec; it needs owner approval before any shell ships it.

### F3. Mission predicates that know outcomes

`MissionPredicateV2` adds, to the V1 fields:

```json
{"version":2,"tools":["claude-code","codex"],"outcomes":["failure","partial"],
 "failure_modes":["looping_or_repetition","missing_verification"],
 "models":"frontier","min_tier":"e2","with_correction":true,"min_sessions":1}
```

- `outcomes`: any of the `TaskSuccess` values.
- `failure_modes`: any of the `TraceFailureMode` values.
- `models`: `"frontier"` (the policy's list) or an explicit list.
- `min_tier`: the lowest evidence tier the client counts, at most `e2`
  (E3 is server-side).
- `with_correction`: the contributor wrote a correction.

The V1 rules carry over: lists are "any of", fields combine with AND, at
least one restricting field, unknown fields refused, matching stays on the
device. A V1-only client ignores V2 missions rather than mis-matching them.

### F4. Verification on the server, at Review

For a submission that claims a frontier failure at E2, the Review stage runs
four checks and records each as a label-only outcome:

1. **Consistency.** The claimed failure modes are present in the events
   (the server re-runs the detectors on the uploaded envelope). The claimed
   model appears on the failing calls.
2. **Environment.** The failure is not explained by an environment fault the
   model could not see (network, credentials, missing binary), using a fixed
   list of tool-result error patterns.
3. **Well-posedness and sabotage.** A judge model reads the trace and must
   produce: what the task was, what a correct result would look like, and
   whether any instruction or context contradicts the goal. A task the judge
   cannot state, or one with contradictory or injected instructions, fails
   this check.
4. **Correction check.** When a `human_correction` is present, the judge
   decides whether it resolves the task as stated. A correct correction
   raises the item's value; an incorrect one is dropped, not the trace.

The judge runs on attested NEAR AI inference only, under the scrubbed
envelope, and only for traces whose consent includes the benchmark scope.
Its verdict, model and receipt hash are stored; its reasoning text is not.

Passing all four makes the submission E3. Failures are clustered on the
judge's task statement embedding plus the failure modes, with the existing
dedup machinery; settlement is once per cluster per account.

### F5. Evaluation items that stay current

- An E3 trace with a correction, or with a test the session ran, converts to
  a benchmark item through the existing conversion route, with a new
  outcome filter (`task_success`, `failure_modes`, `min_tier`).
- The benchmark evaluation worker re-runs items against new frontier models
  as they are listed. Each run records solved or failed per model. An item
  every listed model now solves is retired; the retirement is kept, since
  "which model first solved this" is itself useful data.
- Re-runs go to third-party providers only for items whose contributor gave
  explicit consent to that (open decision D3). By default re-runs use the
  NEAR AI hosted models only.

### F6. Credit

- A frontier-failure contribution is accepted like any other and earns the
  ordinary contribution credit on acceptance.
- The mission's own credit is **pending** from E2 and settles only at E3.
  It is refused, with a label-only reason, if any F4 check fails.
- Weights: a correction the judge accepts, and a test in the trace, each
  raise the settlement. An unattested model lowers it (D2). A cluster
  already verified lowers it for later accounts.
- The client shows pending mission credit labelled as projected, as for
  other missions.

## Privacy

Failure traces carry more sensitive context than most: error messages,
stack traces, file paths, internal host names and credentials that broke.
The tool-output detectors from #1366 apply to every field, including
`human_correction`. Missions in this family require the benchmark consent
scope, and the review screen says so. The judge and re-runs see only the
scrubbed envelope.

## Open decisions for the owner

- **D1. The frontier list.** Which models are on it at launch, and who
  updates it. Proposed: the operator publishes it in the activity policy.
- **D2. Unattested weight.** Most frontier traces come from Anthropic and
  OpenAI directly, where the model label cannot be proven. Proposed: settle
  them at a reduced weight rather than refuse them, since refusing them would
  leave almost nothing to find.
- **D3. Third-party re-runs.** Whether a contributor can opt in to having
  their item re-run against third-party frontier APIs. Proposed: no at
  launch; NEAR AI hosted models only.
- **D4. Trust gate.** The minimum earned-trust tier for settlement.
- **D5. The judge model.** Which NEAR AI hosted model judges, and whether one
  judge or two must agree. Proposed: two must agree, from different model
  families.

## Testing

- F1: adapter fixtures for per-call model labels and exit codes, including a
  session that switches models; detector unit tests with positive and
  negative fixtures for each mode; a test that no detector output reaches an
  envelope without confirmation.
- F3: predicate validation tests mirroring V1's; a V1 client ignores a V2
  mission.
- F4: each fabrication mode in the threat model has a fixture trace that
  fails the check meant to catch it, and a genuine-failure fixture that
  passes all four.
- F6: settlement only at E3; refusal reasons are labels; cluster
  deduplication across accounts.

## Build order

1. F1 per-call model and exit code, with the envelope pin update.
2. F1 detectors and F2 confirmation (unlocks real `failure_modes` data).
3. F3 predicate V2, client and server validation.
4. F4 Review checks behind a switch, run in shadow first: record verdicts,
   settle nothing, and compare against a hand-labelled sample.
5. F6 settlement, after the shadow sample agrees.
6. F5 evaluation items and re-runs.
