# Private personal refactor comparisons: delivery plan

Date: 2026-09-12
Parent: [Trace insights program](2026-09-11-trace-insights-program.md).
Status: Revised delivery plan; implementation and pilot qualification remain outstanding.

## Implementation checkpoint

The task lifecycle is implemented in [PR #932](https://github.com/TraceCommons/trace-commons/pull/932), with the macOS task workflow stacked in [PR #933](https://github.com/TraceCommons/trace-commons/pull/933). Both remain open. The earlier daemon-preview stack overflow was corrected; #932 at `e6c65868` has passed all reported CI checks, including Windows contributor tests. That verifies this implementation slice, not the private pilot.

Draft [#934](https://github.com/TraceCommons/trace-commons/pull/934) implements immutable saved specifications, descriptive evaluation, readable CLI results, and C-interface operations; [#936](https://github.com/TraceCommons/trace-commons/pull/936) adds macOS specification review and evaluation. The adapter for the pinned released Codex source format in [#935](https://github.com/TraceCommons/trace-commons/pull/935) supplies a bounded structural attribution rule for direct messages, reasoning, and sequential command records; see the [source profile](../specs/2026-09-12-codex-comparison-source-profile.md). A synthetic CLI workflow verifies nonempty two-cohort results and retained session-overlap exclusions. This does not establish real task independence, provider identity, or pilot readiness; other source shapes remain explicitly unavailable.

Native Claude import and declaration inspection are in drafts [#939](https://github.com/TraceCommons/trace-commons/pull/939) and [#940](https://github.com/TraceCommons/trace-commons/pull/940). Linux CI exposed a stale source-picker selection in the lifecycle fixture and an unhandled Claude declaration variant in GTK; both were fixed, and #939/#940 passed their reported CI checks at the reviewed heads. Draft [#941](https://github.com/TraceCommons/trace-commons/pull/941) implements attribution for identified agent branches and context-bearing attachments. Astra cleared the implementation; real-file, malformed-input, copied-store, Rust, Clippy, GTK, and macOS local checks passed. All 26 remote checks passed at head `8d018dc19b618967e599ba159bca960b12a63a3d`; #941 remains draft. The selected branches for #616 and #606 exercise source attribution, while #632 remains unavailable. All three retain shared-root overlap; this does not establish independent tasks or two model cohorts. The reviewed [Claude agent-branch source contract](../specs/2026-09-12-claude-comparison-source-profile.md) targets observed writer version `2.1.260`, preserves legacy task digests and shared-root overlap, and separates asynchronous launch evidence from process completion. Native segment termination, user-assessed acceptance, complete task boundaries, and independent tasks remain distinct evidence requirements.

Remaining exits retain the original scope: qualified nonempty two-cohort evidence, the estimator and calibration, complete macOS evaluation and a private pilot, Windows/GTK parity, then controlled mission participation and the wider program. Passing a lifecycle test or opening another stacked PR does not satisfy those exits.

## First result and delivery order

Ship one complete private, local refactor comparison through the shared protocol/contributor backend, CLI, service/FFI, and macOS. A result may say that user-confirmed refactor tasks in an exact repository/language/harness/configuration context have specified user-reported outcome distributions for declared-model cohorts. It may report an observed difference, insufficient precision, an indeterminate boundary, or no eligible evidence. It must not turn snapshots or attempts into trials, assign mixed work to a final model, call uncertainty a tie, call a task rejection “rejected code,” infer full-task tokens from observed intervals, or estimate time saved.

First consolidate the existing Insights PRs and qualify their actual evidence with canonical Codex and trajectory fixtures. Existing episodes, assessments, declarations, Git/test drilldown, persisted Codex usage intervals, cards, and native views are candidate inputs. Qualifying the declaration extractor is a milestone with measured valid/missing/invalid/omitted coverage; an empty eligible cohort is not a completed comparison feature.

The delivery milestone is the complete user workflow: create a task, set context and outcome, inspect attempts/evidence, confirm independence, resolve stale state, save a retrospective specification, evaluate it, understand coverage/uncertainty, and delete the draft task. Outcome comparisons do not require cost evidence. Cost remains unavailable until pricing and identity mapping are separately qualified.

Run a private macOS pilot before Windows and GTK parity. After native parity, qualify one bounded mission path with one source type, one structured task proposal, and one declared evaluator. Retain the full program scope for later qualified task categories, adapters, missions, providers, and public findings.

## Task evidence and authority

Add `LocalComparisonTaskV1` above episodes. An episode is a selected group of whole snapshots; a comparison task is one work item whose repeated/resumed attempts, context, declaration evidence, and delayed outcome are reviewed together. It contains:

- stable UUID, revision, created/updated times, and local user-confirmed provenance;
- frozen episode ID/revision/membership revision/members digest plus every member snapshot ID/source digest;
- category, user-confirmed task date, local project UUID, optional qualified checkout/source-tree provenance, language, harness/version, and an allowlisted configuration fingerprint;
- declaration state derived from every bound snapshot: one retained label with complete supported-candidate coverage, multiple labels, or unavailable with typed reasons;
- outcome `pending`, `accepted`, `partial`, `rejected`, or explicit `unknown`, bound to the current material-evidence revision/digest with user-report provenance;
- an independence confirmation that this is one work item and the episodes contain all known attempts, bound to the material-evidence revision and complete frozen-input digest.

Keep the task's optimistic-concurrency revision separate from its material-evidence revision/digest. Every mutation increments the CAS revision. Episode selection/membership, category, task date, project UUID, checkout provenance, language, harness, configuration, or bound evidence increments the material-evidence revision and stales the independence confirmation and outcome binding. Setting/clearing an outcome or reconfirming independence changes the CAS revision but does not change material evidence, so a newly written outcome/confirmation does not invalidate itself. The user must review changed attempts/context and explicitly reconfirm; a generic save never carries confirmation forward. Independence remains a user assertion.

The [context v1 contract](../specs/2026-09-12-comparison-context-v1.md) defines the configuration allowlist, harness/version authority, project identity, and optional checkout provenance. Before persisting DTOs, publish the exact canonical encoding and golden bytes; any admitted checkout/source-tree digest also needs a qualified construction scheme. It must show which changes alter a stratum and must not derive context from paths, branch names, or arbitrary environment blobs.

Use a user-confirmed local project UUID as the project matching identity across worktrees. Checkout/source-tree digests are provenance, not required exact stratum keys; do not require identical changing code trees across independent tasks. Any extra matching rule needs explicit qualification. Use this project UUID for cross-worktree grouping; existing Git repository-path digests identify checkouts and must not be silently treated as project identity. The initial configuration allowlist is harness identifier/version, reasoning-effort enum, tool-policy profile ID/version, and prompt-template digest, each with explicit unknown state. Missing required comparable settings suppress comparison. Do not store repository paths, trace bodies, test output, free-text descriptions, or arbitrary environment data. Git and test links support drilldown only. They do not prove acceptance, merge, revert, execution against a bound revision, model identity, or independence.

Model metadata is a declaration, not proof that a model/version exclusively performed the work. Supported-candidate coverage alone cannot qualify task attribution. Milestone B must implement and test a source-specific attribution rule covering the admitted task boundaries and attempts, with explicit limits for resumed exports and model switches. Until that rule is qualified, declared-label summaries may be inspected but model-comparison eligibility remains unavailable. Do not substitute those summaries for the comparison milestone. Exclude mixed, switched, omitted, missing, invalid, unsupported, and user-only model identities. Version claims require a separately qualified adapter version field; do not parse versions from aliases.

## Overlap and retained stale state

The overlap graph uses canonical snapshot identities—snapshot ID plus source digest—from frozen episode membership. Task IDs are graph vertices; episode IDs are provenance/display references. A whole snapshot containing several work items cannot be split into independent trials by repeated confirmation. Different-byte overlapping exports need an additional qualified source/session rule or explicit exclusion; snapshot hashing alone does not detect them. Connect two tasks when their frozen attempts contain the same canonical snapshot. Exclude every task in a connected component larger than one until the user edits or consolidates and reconfirms; never select a representative automatically.

Use one retain-stale rule for all upstream changes:

| Event | Retained task state | User action |
|---|---|---|
| Episode assessment-only revision | stale: episode revision changed | Review and reconfirm |
| Episode membership edit | stale: membership changed | Replace binding and reconfirm |
| Identical-content alias change | current if canonical identity is exact | None |
| Snapshot replacement | stale: snapshot replaced/missing | Select current evidence and reconfirm |
| Snapshot deletion | stale: snapshot deleted/missing | Select evidence or delete task |
| Episode deletion | stale: episode deleted/missing | Select episode/evidence or delete task |

Material evidence/context changes also make the prior outcome stale; retain it as history, excluded from current results until explicit review. Assessment-only episode changes preserve task adjudication and require review only of the changed binding. Task outcome changes invalidate comparison results; context edits require reconfirmation. Creating a new overlapping task invalidates previously eligible results. Never silently shrink, rebind, or delete tasks after an upstream mutation. Preserve only frozen IDs/digests needed to explain stale state. Mutation effects list affected task IDs/reasons. Dependent results are invalid when their complete input digest no longer resolves; tasks/specifications remain reviewable. Explicit task deletion alone removes a task.

## Eligibility, outcomes, and usage

A task contributes at most once; episodes and snapshots are attempts/evidence, never trials. Eligibility requires exact bindings, a current digest/revision-bound independence confirmation, refactor category, task date, project UUID, language, harness/version, configuration fingerprint, qualified task attribution under a named extractor/rule version, and no overlap component.

Pending, explicit unknown, and unassessed outcomes are distinct and excluded from assessed-outcome denominators. `partial` remains categorical. Report accepted, partial, rejected, pending, explicit unknown, and unassessed task counts by declared-model cohort and exact stratum; do not collapse them into a score.

Usage is optional secondary evidence. Persisted Codex usage describes observed attributable intervals, potentially omitting earlier or ambiguous activity. First form the union of canonical snapshot identities across every episode in the task, then accumulate each snapshot's complete interval at most once. Label the result “observed attributed tokens,” never full task tokens or cost. Missing usage excludes a task from token estimands but not outcome estimands. The initial release qualifies refactor; other categories return `category_not_yet_qualified` until piloted.

## Retrospective specification and estimator

`ComparisonSpecificationV1` fixes selected declaration labels, refactor category, date window, exact context fields, task/attempt/outcome rules, estimands, interval method, precision target, multiplicity policy, and rubric version. Results bind its digest, an audit digest of every included/excluded task and reason, and a separate estimation-input digest containing only included eligible task facts consumed by the estimator. Excluded-task/provenance-only changes may change the audit digest but never the estimation input or seed.

A specification created after tasks/outcomes exist is `retrospective_user_specification`, with creation time and evidence cutoff. Do not call it predeclared or preregistered. A future prospective label requires a verified immutable specification that predates the bound evidence.

Cutoff chronology uses a task's local `material_recorded_at` and the bound outcome's `recorded_at`, not the optimistic-concurrency `updated_at`. Creation and substantive context/evidence changes advance material chronology; outcome edits use their own timestamp, and reconfirmation does not advance either. Legacy tasks without material chronology receive `cutoff_time_unavailable`. Neither task dates nor upstream execution timestamps substitute for the missing local chronology.

Saving captures material/outcome bindings recorded by the cutoff under the store lock. A separate substantive material digest excludes assessment-only episode revisions while retaining episode identity, membership, member evidence, and context. Current full evidence bindings and independence review are still required during evaluation. This lets an explicitly reviewed assessment-only refresh preserve cutoff membership without admitting changed substantive evidence or reviving stale outcomes. A delayed outcome after the cutoff produces a per-task exclusion rather than failing the entire specification.

The analytical `specification_digest` binds cohort/context/date selection and analytical rules. The `saved_record_digest` additionally binds record identity, creation/cutoff chronology, and frozen cutoff evidence. Audit-only record changes must not perturb analytical inputs or the future estimator seed. Results bind both specification identities and the current audit digest; explaining a result after relevant evidence changes returns an explicit stale-result error.

The initial candidate was a deterministic task-level stratified bootstrap. Its percentile intervals failed exploratory coverage checks in [#938](https://github.com/TraceCommons/trace-commons/pull/938), so this is not an admitted production method. The frozen exact simultaneous binomial-component candidate has now passed its numerical protocol, independent raw/derived artifact review, and the bounded runtime matrix. Production integration and validation are the next step; these results alone do not activate new specifications or establish pilot usefulness. Version saved analytical rules when the method changes; never silently replace the method of an existing specification.

The approved bounded integration freezes `QualifiedExactCategoricalV1` in each newly admitted specification: method/version, protocol SHA-256, canonical cohort orientation (second minus first), accepted/partial/rejected outcome order, the six-component Bonferroni rule, at least two assessed tasks per cohort, the existing 256-total-facts gate and separate 256-assessed-task evaluator bound, and a 500,000-millionth contrast-width threshold. Width above the threshold suppresses for precision; equality or an endpoint exactly zero is indeterminate. Do not turn an interval containing zero into equivalence.

Existing `NotYetCalibrated` specifications and schema-1 result bytes/digests remain unchanged and uncalibrated. Newly qualified results use schema 2; older clients explicitly reject them rather than being claimed compatible through additive fields. A new domain-separated output digest binds the frozen method state, canonical cohort order, assessed counts/denominators, six component intervals, and three ordered contrasts/statuses. Preserve existing audit/input digest semantics. Validate qualified results against the saved state and recomputed counts/intervals. Complete CLI, FFI, and macOS evaluation and cross-state rejection tests before the separate admission change enables new qualified specifications.


Implementation checkpoint: the production integration remains local and is not yet admitted or published. Sol reports 13/13 specification tests and 6/6 arithmetic/oracle tests passing against the production primitive, including actual schema-1 fixture re-projection and unchanged bytes/digests. CLI and macOS presentation separate signed observed percentage-point differences from intervals and statuses using shared Rust copy. Complete lifecycle, adversarial decoding, native wording, routing integration, and Astra final review remain required before the separate admission change. Direct native rendering has independent review evidence, but neither rendering nor synthetic fixtures satisfy the two-cohort human pilot.

For any resampling candidate, define and serialize the actual estimator inputs before deriving its seed:

1. Canonically order task IDs, cohorts, strata, outcomes, and fixed weights.
2. Serialize rubric/specification and estimation-input digests, selected contrast, replicate count, interval method, and weights. Do not seed from the audit digest, timestamps, provenance labels, or excluded-task metadata.
3. Domain-separate and hash those bytes for the PRNG seed.
4. Resample tasks within model-by-stratum cells; never resample attempts/snapshots.
5. Apply fixed, outcome-independent stratum weights to each replicate.

Show every stratum beside any aggregate. Calibration fixes common-support rules, actual weights, replicate count, interval construction, precision width, sparse-cell suppression, boundary policy, and multiplicity behavior before advantage copy is enabled. Homogeneous small samples can yield falsely certain zero-width bootstrap intervals: require a calibrated boundary suppression rule or a justified alternative interval method. Report assessed-outcome proportions as conditional on observed outcomes; sampling intervals do not account for missing-outcome selection. Test known distributions, exact no-difference cohorts, sparse cells, imbalance, repeated attempts, overlap, and values exactly on decision boundaries.

Keep result states distinct:

- `observed_difference`: interval strictly excludes zero and passes precision;
- `no_observed_difference`: only if an explicit calibrated equivalence rule passes;
- `uncertain_difference`: a finite sufficiently narrow interval includes zero and no equivalence rule passes;
- `insufficient_precision`: an interval exists but is too wide;
- `indeterminate_boundary`: rounding, discrete mass, or equality at a rule boundary prevents a strict decision;
- `insufficient_resampling_support`: required cells cannot support a resampling estimator; a non-resampling method must name its own minimum-support policy rather than implying a mathematical resampling requirement.

Never map an interval containing zero, boundary equality, or uncertainty to `tie`. Decide with full precision and round only display. Calibration validates implementation and comprehension, not causal superiority.

## Implementation sequence

### 0. Consolidate and qualify

Consolidate the existing local stack, run full warning-denied checks, and qualify declarations and observed usage on canonical fixtures. Record candidate/valid/missing/invalid/omitted declarations and attributable/unattributable usage intervals. Resolve gaps before admitting cohorts, including missing attribution and single-work-item boundary extraction rules. Require a nonempty supported two-cohort end-to-end fixture and an authorized pilot path; measuring missing fields alone does not satisfy qualification.

### 1. Shared task lifecycle and CLI

Add permissive DTOs and contributor storage using existing locks, restrictive permissions, and atomic replacement. Support create, list, explain, replace episodes, set context, set/clear outcome, reconfirm independence, and delete with expected revisions. Create may yield an incomplete draft.

CLI:

```text
insights comparison-task create|list|explain|replace-episodes
insights comparison-task set-context|set-outcome|clear-outcome
insights comparison-task reconfirm|delete
insights comparison eligibility --category refactor ...
```

Reconfirm requires expected revision and displayed input digest. Eligibility returns every included/excluded task with stable reasons, canonical snapshot-overlap components, current/frozen bindings, and confirmation state. Reads create no state; mutations return fixed errors and stale-task effects.

### 2. Specification and descriptive result

Add immutable retrospective specifications, exact strata, categorical distributions, observed-attributed-token coverage, canonical digests, and suppression states. Preview is ephemeral; save creates a new specification; evaluate uses a saved specification and current reviewed tasks. CLI adds `preview-spec`, `save-spec`, `list-specs`, `evaluate`, and `explain-result`.

### 3. Estimator and calibration

The frozen exact-binomial component candidate completed 252 synthetic settings of 10,000 trials each at source `9d41d5b4a7f4e0ca74a0264f63124e5c2b030bcc`. All settings passed the frozen numerical noncoverage criterion; 36 settings emitted no intervals because support was insufficient. Astra independently verified the raw artifact, setting grid, counters, and confidence bounds. This is numerical calibration evidence, not product admission or a guarantee of useful precision. The immutable raw artifact SHA-256 is `68926edabd94d317c381640d712787077e7001d8357cf8f9128d577af8a01d70`.

The reviewed report implementation separates frozen all-trial diagnostics from conditional eligible-interval diagnostics, exposes suppression, validates the complete setting set, and refuses output overwrite. The derivation completed, and Astra independently verified all 252 derived denominators, rates, and confidence bounds (report SHA-256 `06da84afe0da9ecd949d4ef93470986f1c1843cbe39bcad51b3065202e0b9724`). Astra also verified all 32 debug/release runtime runs and their provenance. On the tested Apple M4 Max, the three supported release cases had median candidate times of approximately 5.3, 11.0, and 20.3 ms; parsing, eligibility, service, and UI are excluded, and no worst-case bound is established. All 26 checks passed at #942 `a01efd88` and #943 `d7636fcf`; this includes integrated Clippy. The candidate remains test-only and saved comparisons remain unqualified. Do not retune frozen thresholds to make the small pilot look decisive.

Implement the estimator/seed above with existing dependencies and checked arithmetic. Pin byte fixtures. Property-test that duplicating attempts cannot add a task, excluded/stale tasks cannot change estimates, import order cannot change bytes, and suppressed results contain no winner language.

### 4. Complete macOS workflow

The current UI has explicit task/context/outcome/reconfirmation controls. Draft [#944](https://github.com/TraceCommons/trace-commons/pull/944) corrects local Gregorian task dates and binds post-save reconciliation to the saved specification ID and both digests. Draft [#945](https://github.com/TraceCommons/trace-commons/pull/945), from reviewed commit `f44e7bc3`, implements immutable `--insights-store` launch selection shared by snapshots, tasks, specifications, and drilldown, with visible custom location and no default-store fallback on malformed selection. It preserves source/repository security-scoped access and does not create missing directories during selection. Astra cleared both slices, and focused Swift tests passed 25/25. The real-FFI routing follow-up passed 27/27 focused tests after a reviewed actor gate fixed concurrent store-read contention. The actor fix and stateless Rust refusal/location copy are published in #945 at `15f66a62`, with Astra clearance and passing focused copy/header/ABI and Swift wording checks. The wording baseline was not widened; stateless copy performs no store/path/default resolution. #944 passed all 26 checks at `f1a339f3`. Updated #945 CI and the actual isolated-store app pilot remain outstanding.

Use shared copy and typed service/FFI responses. Support task create, structured context, outcome, frozen/current evidence and overlap review, explicit digest-bound reconfirmation, stale resolution, deletion, specification selection/save, eligibility review, evaluation, and drilldown to task/episode/snapshot/declaration/outcome/usage/Git/test evidence.

Bind callbacks/drafts to presentation generation, task ID/revision/input digest, and specification digest. Reject stale callbacks. Invalidate results after mutations and retain committed success/stale notices if reconciliation fails. Never retry conflicts or reconfirm automatically.

### 5. Private pilot, then platform parity

Use the [private pilot runbook](2026-09-12-private-refactor-pilot.md) for the selected Trace Commons candidates, local commands, user review, and observation record. Original trace inputs and outcomes remain to be qualified. Pilot real user-reviewed refactor tasks locally. Measure extractor coverage, context completion, stale/reconfirmation behavior, overlap resolution, delayed outcomes, declaration ambiguity, interval stability, and comprehension of declared cohorts and observed intervals. Descriptive or suppressed output is a valid pilot result.

Apply pilot corrections, then port to Windows and GTK with actual WinUI runtime/build and GTK display CI. macOS-hosted managed tests do not qualify WinUI; headless GTK tests do not qualify display behavior.

### 6. One bounded mission path

Connect the local mission inbox to one qualified source adapter and one structured task/evaluator shape. Preserve source digest/provenance, unverified identities, curator review, and explicit import. The bounded mission milestone continues through explicit user-authorized controlled participation, a qualified execution boundary, declared evaluation, and evidence-linked results; draft review alone is not completion. Keep publication authority absent in this bounded delivery. User rewards for missions and insights are outside scope and owned by Abhishek; execution budgets are not reward funding. Qualify this one path end to end before adding providers or variants.

## Verification and claim boundary

Test legacy reads, canonical serialization, revision/digest conflicts, every retain-stale matrix row including deletion, overlap by snapshot identity, absent-store reads, limits, fixed errors, and unchanged episode/card behavior. Test complete task create/context/outcome/reconfirm/delete in shared Rust, CLI, service/FFI, and macOS. Test declaration missingness, observed-interval coverage, exact strata, categorical outcomes, bootstrap boundaries, sparse support, and exact seed inputs.

Run warning-denied protocol/contributor/FFI tests, repository-allowlist Clippy, license boundary, CLI fixtures, Swift build/tests, and actual local FFI lifecycle. Run platform-native checks as later layers land. Add no dependency.

User reward systems for missions and insights, including reward eligibility, credits, funding, redemption, and payouts, are outside this program and owned by Abhishek. They are not later delivery requirements for this team. Keep enrollment, remote providers, hosted analytics, pricing, public comparisons, exclusive model attribution, causal claims, code-rejection inference, and time-saved estimation out of the first delivery; the full program retains those separate capabilities behind their evidence contracts and qualification gates.
