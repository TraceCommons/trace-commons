# Private personal refactor comparisons: delivery plan

Date: 2026-09-12
Parent: [Trace insights program](2026-09-11-trace-insights-program.md).
Status: Revised delivery plan; implementation and pilot qualification remain outstanding.

## First result and delivery order

Ship one complete private, local refactor comparison through the shared protocol/contributor backend, CLI, service/FFI, and macOS. A result may say that user-confirmed refactor tasks in an exact repository/language/harness/configuration context have specified user-reported outcome distributions for declared-model cohorts. It may report an observed difference, insufficient precision, an indeterminate boundary, or no eligible evidence. It must not turn snapshots or attempts into trials, assign mixed work to a final model, call uncertainty a tie, call a task rejection “rejected code,” infer full-task tokens from observed intervals, or estimate time saved.

First consolidate the existing Insights PRs and qualify their actual evidence with canonical Codex and trajectory fixtures. Existing episodes, assessments, declarations, Git/test drilldown, persisted Codex usage intervals, cards, and native views are candidate inputs. Qualifying the declaration extractor is a milestone with measured valid/missing/invalid/omitted coverage; an empty eligible cohort is not a completed comparison feature.

The delivery milestone is the complete user workflow: create a task, set context and outcome, inspect attempts/evidence, confirm independence, resolve stale state, save a retrospective specification, evaluate it, understand coverage/uncertainty, and delete the draft task. Outcome comparisons do not require cost evidence. Cost remains unavailable until pricing and identity mapping are separately qualified.

Run a private macOS pilot before Windows and GTK parity. After native parity, qualify one bounded mission path with one source type, one structured task proposal, and one declared evaluator. Retain the full program scope for later qualified task categories, adapters, missions, providers, and public findings.

## Task evidence and authority

Add `LocalComparisonTaskV1` above episodes. An episode is a selected group of whole snapshots; a comparison task is one work item whose repeated/resumed attempts, context, declaration evidence, and delayed outcome are reviewed together. It contains:

- stable UUID, revision, created/updated times, and local user-confirmed provenance;
- frozen episode ID/revision/membership revision/members digest plus every member snapshot ID/source digest;
- category, user-confirmed task date, repository digest, checkout/source-tree digest, language, harness/version, and an allowlisted configuration fingerprint;
- declaration state derived from every bound snapshot: one retained label with complete supported-candidate coverage, multiple labels, or unavailable with typed reasons;
- outcome `pending`, `accepted`, `partial`, `rejected`, or explicit `unknown`, bound to the current material-evidence revision/digest with user-report provenance;
- an independence confirmation that this is one work item and the episodes contain all known attempts, bound to the material-evidence revision and complete frozen-input digest.

Keep the task's optimistic-concurrency revision separate from its material-evidence revision/digest. Every mutation increments the CAS revision. Episode selection/membership, category, task date, repository/checkout digest, language, harness, configuration, or bound evidence increments the material-evidence revision and stales the independence confirmation and outcome binding. Setting/clearing an outcome or reconfirming independence changes the CAS revision but does not change material evidence, so a newly written outcome/confirmation does not invalidate itself. The user must review changed attempts/context and explicitly reconfirm; a generic save never carries confirmation forward. Independence remains a user assertion.

Before freezing DTOs, publish a context-qualification artifact that names the exact configuration allowlist, canonical encoding, harness/version sources, repository digest meaning, and checkout/source-tree digest construction. It must show which changes alter a stratum and must not derive context from paths, branch names, or arbitrary environment blobs.

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

A task contributes at most once; episodes and snapshots are attempts/evidence, never trials. Eligibility requires exact bindings, a current digest/revision-bound independence confirmation, refactor category, task date, repository digest, language, harness/version, configuration fingerprint, qualified task attribution under a named extractor/rule version, and no overlap component.

Pending, explicit unknown, and unassessed outcomes are distinct and excluded from assessed-outcome denominators. `partial` remains categorical. Report accepted, partial, rejected, pending, explicit unknown, and unassessed task counts by declared-model cohort and exact stratum; do not collapse them into a score.

Usage is optional secondary evidence. Persisted Codex usage describes observed attributable intervals, potentially omitting earlier or ambiguous activity. First form the union of canonical snapshot identities across every episode in the task, then accumulate each snapshot's complete interval at most once. Label the result “observed attributed tokens,” never full task tokens or cost. Missing usage excludes a task from token estimands but not outcome estimands. The initial release qualifies refactor; other categories return `category_not_yet_qualified` until piloted.

## Retrospective specification and estimator

`ComparisonSpecificationV1` fixes selected declaration labels, refactor category, date window, exact context fields, task/attempt/outcome rules, estimands, interval method, precision target, multiplicity policy, and rubric version. Results bind its digest, an audit digest of every included/excluded task and reason, and a separate estimation-input digest containing only included eligible task facts consumed by the estimator. Excluded-task/provenance-only changes may change the audit digest but never the estimation input or seed.

A specification created after tasks/outcomes exist is `retrospective_user_specification`, with creation time and evidence cutoff. Do not call it predeclared or preregistered. A future prospective label requires a verified immutable specification that predates the bound evidence.

Use a deterministic task-level stratified bootstrap. Define and serialize the actual estimator inputs before deriving its seed:

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
- `insufficient_resampling_support`: required cells cannot support the estimator.

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

Implement the estimator/seed above with existing dependencies and checked arithmetic. Pin byte fixtures. Property-test that duplicating attempts cannot add a task, excluded/stale tasks cannot change estimates, import order cannot change bytes, and suppressed results contain no winner language.

### 4. Complete macOS workflow

Use shared copy and typed service/FFI responses. Support task create, structured context, outcome, frozen/current evidence and overlap review, explicit digest-bound reconfirmation, stale resolution, deletion, specification selection/save, eligibility review, evaluation, and drilldown to task/episode/snapshot/declaration/outcome/usage/Git/test evidence.

Bind callbacks/drafts to presentation generation, task ID/revision/input digest, and specification digest. Reject stale callbacks. Invalidate results after mutations and retain committed success/stale notices if reconciliation fails. Never retry conflicts or reconfirm automatically.

### 5. Private pilot, then platform parity

Pilot real user-reviewed refactor tasks locally. Measure extractor coverage, context completion, stale/reconfirmation behavior, overlap resolution, delayed outcomes, declaration ambiguity, interval stability, and comprehension of declared cohorts and observed intervals. Descriptive or suppressed output is a valid pilot result.

Apply pilot corrections, then port to Windows and GTK with actual WinUI runtime/build and GTK display CI. macOS-hosted managed tests do not qualify WinUI; headless GTK tests do not qualify display behavior.

### 6. One bounded mission path

Connect the local mission inbox to one qualified source adapter and one structured task/evaluator shape. Preserve source digest/provenance, unverified identities, curator review, and explicit import. The bounded mission milestone continues through explicit user-authorized controlled participation, a qualified execution boundary, declared evaluation, and evidence-linked results; draft review alone is not completion. Keep publication and funding authority separate and absent. Qualify this one path end to end before adding providers or variants.

## Verification and claim boundary

Test legacy reads, canonical serialization, revision/digest conflicts, every retain-stale matrix row including deletion, overlap by snapshot identity, absent-store reads, limits, fixed errors, and unchanged episode/card behavior. Test complete task create/context/outcome/reconfirm/delete in shared Rust, CLI, service/FFI, and macOS. Test declaration missingness, observed-interval coverage, exact strata, categorical outcomes, bootstrap boundaries, sparse support, and exact seed inputs.

Run warning-denied protocol/contributor/FFI tests, repository-allowlist Clippy, license boundary, CLI fixtures, Swift build/tests, and actual local FFI lifecycle. Run platform-native checks as later layers land. Add no dependency.

Keep enrollment, contribution credit, remote providers, hosted analytics, pricing, rewards, public comparisons, exclusive model attribution, causal claims, code-rejection inference, and time-saved estimation out of the first delivery. The full program retains them only behind separately approved evidence contracts and qualification gates.
