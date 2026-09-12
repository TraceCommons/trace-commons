# Trace insights implementation program

Date: 2026-09-11
Updated: 2026-09-12
Status: Delivery sequence revised around a complete personal-comparison workflow. Implementation remains in open PRs; this document does not authorize merging, deployment, new dependencies, hosted processing, or public publication.

## Canonical plan and implementation status

[PR #870](https://github.com/TraceCommons/trace-commons/pull/870) is the single review entry point and source of truth for this program. Update its program plan, linked delivery plan, and pilot runbook when scope or sequence changes; implementation PRs link here rather than becoming competing plans. Historical branch snapshots are not the current authority.

- [Personal refactor delivery plan](2026-09-12-personal-refactor-comparison.md): task evidence, comparison rules, qualification, native workflow, and bounded mission sequence.
- [Private pilot runbook](2026-09-12-private-refactor-pilot.md): selected tasks, local commands, explicit user review, and coverage/usability observations.
- [Context contract](../specs/2026-09-12-comparison-context-v1.md), [canonical configuration encoding](../specs/2026-09-12-comparison-configuration-encoding-v1.md), and [Codex source profile](../specs/2026-09-12-codex-comparison-source-profile.md): current bounded evidence contracts.

Current implementation checkpoints are draft PRs, not releases:

| Work | PRs and remaining evidence |
| --- | --- |
| Shared task lifecycle, macOS task editing, saved specifications | [#932](https://github.com/TraceCommons/trace-commons/pull/932), [#933](https://github.com/TraceCommons/trace-commons/pull/933), [#934](https://github.com/TraceCommons/trace-commons/pull/934). |
| Bounded released-Codex admission and macOS saved-comparison workflow | [#935](https://github.com/TraceCommons/trace-commons/pull/935), [#936](https://github.com/TraceCommons/trace-commons/pull/936). Synthetic two-cohort wiring is verified; it does not qualify model advantage. |
| Pilot preparation and overall-plan review follow-through | [#937](https://github.com/TraceCommons/trace-commons/pull/937); its plan updates are consolidated here. |
| Estimator experiments and evidence | [#938](https://github.com/TraceCommons/trace-commons/pull/938), [#942](https://github.com/TraceCommons/trace-commons/pull/942), and [#943](https://github.com/TraceCommons/trace-commons/pull/943). The exact candidate passed the frozen numerical criterion; Astra independently verified the raw and derived 252-setting reports and all 32 runtime runs. Runtime evidence covers only the measured matrix, machine, and builds. Production integration, admission for new specifications, useful precision, and a real two-cohort pilot remain outstanding. Existing saved specifications remain unqualified. |
| Native Claude local import | [#939](https://github.com/TraceCommons/trace-commons/pull/939). Three authorized real-file drafts imported and user outcomes recorded as accepted. Task boundaries, complete attempt linkage, model attribution, and cohort eligibility remain unqualified; one recorded model label does not supply two cohorts. |
| Native Claude agent-branch attribution | [#941](https://github.com/TraceCommons/trace-commons/pull/941), stacked on #940. Astra cleared commit `1744f822`; real-source, malformed-input, and copied-store acceptance checks passed. Local Rust, Clippy, GTK, and macOS checks passed; all 26 remote checks passed at follow-up `8d018dc1`. Two selected branches supply source attribution; the third stays unavailable, and shared-session overlap prevents independent-task admission. |
| macOS comparison corrections and pilot store | [#944](https://github.com/TraceCommons/trace-commons/pull/944) fixes local Gregorian dates and saved-record identity/digest reconciliation. Explicit immutable store routing is in [#945](https://github.com/TraceCommons/trace-commons/pull/945), from reviewed commit `f44e7bc3`. Astra cleared the changes and routing follow-ups; the combined focused Swift suite passed 27/27, including real-FFI routing. Actual app pilot remains required; see the CI and native-render checkpoints below. |

CI checkpoint: #942 at `a01efd88`, #943 at `d7636fcf`, #944 at `f1a339f3`, and #945 at `9ac49116` each passed all 26 checks. #945 includes reviewed serialization of concurrent native requests, stateless shared routing copy, and direct native rendering coverage. Warning-denied contributor/FFI Clippy and the unchanged shell-wording baseline passed. These checks do not replace the real two-cohort human pilot.

Native rendering checkpoint: #945's reviewed synthetic-store render test shows the selected-store location and populated task/specification rows without app startup services. Against the newer exact-comparison FFI, OCR failed to recognize an attribution sentence that remained visible. The reviewed follow-up in #947 uses a stable populated-task outcome marker and separately checks the pending backend state; its render test passed 1/1. This is rendering evidence, not interaction coverage or a human pilot.

Production integration is published in draft [#947](https://github.com/TraceCommons/trace-commons/pull/947) at `d870dcbd`, stacked on #945, with Astra clearance for the non-activating implementation and follow-ups. The shared production arithmetic/oracle and legacy schema-1 re-projection checks passed. Strict Rust/Swift decoding rejects ambiguous nulls and unknown nested fields; targeted tests isolate the task cap. The immutable FFI built from source `30b960cf` passed 31 focused Swift tests on integrated Swift source `3fcc5c60`, including bridge/model, routing, and shell-wording coverage, plus the separately reviewed rendering check. Actual CLI and FFI saved-spec evaluation now passes both insufficient-support and supported nonempty paths. Astra cleared source `a6c13813` and its synthetic four-task store: distinct root sessions, two tasks per cohort, matching context, and current outcome/independence bindings. The full CLI suite passed 16/16 and FFI Insights suite 15/15. Actual supported evaluation through the macOS router/model now passes: source `02d5816f` has Astra clearance, the new test passed 1/1, and its full real-FFI routing class passed 4/4. Swift checks structural specification binding and digest shape; Rust performs digest recomputation. Current-head CI remains outstanding. Enabling new qualified specifications remains a separate change; no human pilot or model advantage is established.

Recorded Claude declaration inspection is now in [#940](https://github.com/TraceCommons/trace-commons/pull/940): source-bound physical-line references, missing/invalid coverage, and native schema compatibility. The three selected files produced 438 declaration candidates (437 valid and one invalid synthetic label); accepted outcomes survived explicit reimport. These are declaration observations, not task or serving-model attribution. Original/final local Git object links are available for the two patch-matched cases; complete attempt linkage remains unresolved.

The pilot exposed a missing native Claude importer and incremental records sharing message IDs; those findings changed implementation and regression coverage. Further pilot work measures whether users can complete the flow and make a decision, including excluded and abandoned cases. Runtime/platform CI is tracked on each PR at its actual head; these checkpoint links are not a broad readiness claim. Rewards for missions and insights are excluded and owned by Abhishek, as specified below.

## Outcome

Give developers evidence-backed answers to three questions: which models work for their tasks, how they can improve their prompting, and how their approaches perform on shared missions.

Start with a private Insights view in the contributor product that works without enrollment or contribution. Advance from descriptive observations to comparisons only as outcome coverage and evaluation quality justify them. Vendor-wide claims such as “Claude is best for refactors” are not initial deliverables.

Trace Commons owns a unified product with Insights, Coaching, and Missions. Multiple participants may supply analytics, specialist coaching, missions, and evaluations through shared interfaces. Users receive a useful default experience without selecting a provider or creating provider-specific accounts. Local use remains available without a Trace Commons account; hosted features use one Trace Commons account and permission surface.

## Rewards ownership and scope

Abhishek owns the system for rewarding users for missions and insights. Reward design and implementation are outside this program's scope, not deferred work for this agent team. Do not add reward eligibility, points/credits, reward terms, funding, wallets, redemption, settlement, payouts, or reward-specific UI/API/schema requirements. Analytics evidence and mission results remain within scope; they do not determine or promise compensation. Any future integration with Abhishek's work needs a separately agreed interface and authorization and is not a completion dependency here. Execution-resource budgets remain in scope and are distinct from user rewards.

## Current delivery priority

Current position: between milestones B and C. The next engineering step is reviewed production integration of the exact candidate while preserving historical specification bytes and digests, followed by the isolated-store macOS workflow. The selected three accepted tasks share a root session and one model cohort, so they cannot qualify an independent two-model comparison. Obtain authorized evidence for two cohorts and complete human task/context/outcome/independence review; do not manufacture independence or treat passing synthetic checks as the pilot.

The architecture and full user-story scope remain intact. The sequence below supersedes the numerical order of the phase and packet inventory. Progress is measured by user questions answered with inspectable evidence, not by contracts, test counts, or platform implementations completed.

The next product milestone is: **a user can compare two models on their own refactor tasks, understand the evidence and limitations, and make a better choice.** A justified uncertain result is useful; a workflow that always abstains because required evidence cannot be collected is not completion of this milestone.

The existing stack supplies local analysis and saved history, episode grouping and user assessments, model declarations and Git/test evidence links, descriptive cards, usage observations, pricing contracts, and local mission draft inboxes across desktop shells. These are implemented foundations in open PRs, not a released comparison product. Price tables remain unqualified for automatic real-cost claims; model declarations and partial token intervals do not establish exclusive model use or full-task cost.

| Milestone | Work and exit evidence |
| --- | --- |
| A — Consolidate the existing stack | Resolve review findings and dependency conflicts, verify CI at the actual PR heads, and record platform qualification and remaining release gates. Keep commit/PR provenance while reducing simultaneous integration work. Prepare the existing private Insights experience for release; merge and release remain separate authorized actions. |
| B — Qualify comparison evidence | Audit representative synthetic and explicitly authorized traces against pinned source versions. Establish supported task boundaries, repeated/resumed attempts, model attribution and identity scope, and outcome capture. Implement and qualify any missing extraction rule needed for the refactor workflow. Publish supported cases, exclusions, and evidence fixtures before adding ranking machinery. |
| C — Deliver one complete comparison | Build the shared engine and CLI, then macOS as the first desktop pilot. A user groups attempts into tasks, confirms context, records outcomes, reviews stale or overlapping evidence, selects two models, and inspects comparable outcome distributions and justified uncertainty. Complete an authorized human pilot for linkage correctness and comprehension. Scope conclusions to the observed cohort; do not claim causal superiority. |
| D — Expand the validated experience | Correct problems found in the pilot, then implement Windows and GTK comparison parity using the same contract and qualify each platform. Preserve existing shell features throughout. Extend categories to tests and docs only when their outcome definitions and evidence are qualified. |
| E — Complete one mission loop | After the personal-comparison pilot, qualify one read-only source, one reproducible task, and one evaluator. Take a sourced draft through curator review, controlled participation, evaluation, and evidence-linked results in the same product. Validate this loop before broad discovery or more adapters. |
| Later program work | Continue calibrated coaching, independent provider conformance, hosted team analytics, and qualified community comparisons under the phases below. They remain part of the program; they are not prerequisites for the first personal comparison. |

Use Sol for bounded implementation and Astra for independent review, including re-review of fixes. A review is not a substitute for tests, platform checks, or a human evidence pilot. Limit concurrent work to the next milestone and genuinely independent consolidation tasks. Do not start another round of generic framework expansion or three-platform parity before the first complete flow is validated.

The detailed comparison plan is [Personal refactor comparison delivery](2026-09-12-personal-refactor-comparison.md). A separate evidence design is still required for billed cost, exact code rejection, and time saved. Keep those user stories visible: observed intervals are not full-task cost, rejected tasks are not rejected code, and elapsed trace span is not time saved.

## Unified product and participant model

| Participant | Contribution | Trace Commons responsibility |
| --- | --- | --- |
| Analytics or coaching provider | Structured findings and specialist methods | Common evidence standards, presentation, qualification, and default selection. |
| Mission scout | Discover external developments and turn them into evidence-linked mission drafts | Source provenance, deduplication, draft validation, publication policy, and a unified discovery feed. |
| Mission author or sponsor | Versioned tasks, execution-resource budgets, and success criteria | Mission discovery, participation flow, attribution, and published rules. |
| Evaluator | Assessments tied to evidence and rubric versions | Result validation, conflict disclosures, challenges, and reproducibility requirements. |
| Contributor | Authorized evidence or mission attempts | Clear permissions, evidence access, and lifecycle controls. |

An organization may hold several roles. Record and display sponsor/evaluator relationships, including evaluation of a provider's own models. Analytics scores and mission results do not grant compensation. Reward systems for both missions and insights belong to Abhishek and are outside this plan.

Trace Commons supplies shared cards, terminology, comparison views, evidence drilldowns, and permission controls. Providers return structured results rather than arbitrary UI or competing dashboards. Provider attribution is available on each result and prominent where sponsorship or conflicts affect interpretation. Specialist capabilities appear in task context; a provider catalog is deferred until independent implementations prove useful within the shared experience.

The default provider-selection policy is versioned and inspectable. Users can see which provider analyzed their work and change optional capabilities in one settings surface. Selection cannot broaden access, switch execution mode, or incur additional spend beyond the authorized budget. Provider failure produces a clear unavailable or insufficient-evidence state; fallback is limited to already-authorized providers and never weakens privacy requirements.

Result reconciliation belongs to Trace Commons: deduplicate equivalent findings, group only compatible metrics and populations, preserve provider attribution, and explain substantive disagreement. Do not average incompatible scores or silently promote a disputed finding into a definitive recommendation. Qualification assesses evidence support, calibration, privacy, reliability, latency, and cost. Signed provenance establishes authorship, not correctness.

## Existing foundations and design reconciliation

- `crates/trace-commons-protocol/src/trace_contribution.rs`: event usage/cost, outcome metadata, feedback, failure modes, process evaluation labels, and allowed uses.
- `docs/trace-commons-storage.md`: versioned derived records, source artifact linkage, process evaluation, export purposes, and invalidation lifecycle.
- `docs/superpowers/specs/2026-08-07-private-contributor-insight-design.md`: private analysis independent of contribution, local scrubbing, verified remote analysis, no server-side personal profile persistence.
- `docs/superpowers/specs/2026-08-04-community-surface-gate-split-design.md`: community analytics has publication controls distinct from public attribution. Inspect current implementation before extending it; the document alone does not establish deployed behavior.
- `README.md`: evaluator adapters are operator-owned; interface availability does not prove a configured or qualified evaluator.

This plan refines the earlier private-insight proposal with task outcomes and a staged delivery sequence. Its historical deployment statements must be rechecked. Start with deterministic local analysis; remote semantic evaluation is optional and separately qualified. Do not reuse corpus novelty or contribution credit as model-quality scores.

The initial architecture recommendation placed analytics in server-side derived records. That remains appropriate for an explicitly authorized hosted team product, but is not a prerequisite for private insights. Private analysis must not silently create corpus records or server-side profiles.

## Proposed architecture

```text
Local agent adapters + optional local Git/test evidence
    -> normalized observations and coverage report
    -> task episodes and outcome evidence
    -> permission-aware provider dispatcher
    -> local metrics + optional scrubbed, attested provider evaluation
    -> versioned local insight store
    -> validated, reconciled findings in Trace Commons CLI / desktop

Separate, explicit hosted-sharing path (later):
    authorized episode evidence -> tenant-scoped evaluation workers
    -> versioned derived records -> team Insights API

Separate community path (later):
    eligible sources -> approved privacy mechanism -> community comparisons

External sources -> mission scouts -> validated drafts -> publication review
    -> versioned mission definitions -> controlled task runs
    -> same evidence contracts -> findings in Insights and mission results
```

Use one versioned provider interface for the first-party implementation and later independent implementations. Requests carry bounded evidence references/projections, permitted purpose, requested capability, execution policy, deadline, and cost limit. Responses carry structured evaluations, evidence references, coverage/abstention states, and provenance. Enforce schema validation, authorization, cancellation, idempotency, resource limits, and lifecycle invalidation at the host boundary. Signing and key rotation are required for externally supplied remote results; local results identify the installed implementation and version.

Provider execution modes are explicit: local, attested remote, and separately designed authorized hosted processing. Private remote analysis initially supports only the qualified attested path. The shared interface does not authorize ordinary external plaintext processing. Installing a provider grants no corpus access, training rights, publication rights, or indefinite retention. Revocation blocks future access and invalidates controlled derivatives; it cannot recall plaintext already delivered to an external service. Any later hosted mode must explain and qualify that boundary before activation.

Shared serializable contracts belong in the permissive protocol crate. Local extraction and analysis belong in the contributor crate or a deliberately scoped permissive module. Server orchestration belongs in the AGPL server; use gate-api contracts only for implementations on that side. Never introduce an AGPL dependency into a client crate. Prefer existing libraries and local storage facilities; new dependencies require explicit human approval.

## Data and measurement contract

Proposed concepts, not existing type or table names:

| Concept | Required meaning |
| --- | --- |
| Observation | Source adapter/version, source event reference, evidence hash, timestamp, exact model/configuration where known, value, and missingness reason. |
| Task episode | A piece of work with start/end boundaries, task category, source sessions, project context, model segments, and boundary confidence; editable by its owner. |
| Outcome evidence | Typed result with source authority and observation time: user acceptance, test result, review decision, merge, revert, or explicit correction. Pending and unknown remain distinct from failure. |
| Provider manifest | Stable identity, implementation version, capabilities, execution mode, input needs, output schema, retention policy, budget model, and qualification status. |
| Evaluation | Input digest, provider identity, evaluator/rubric version, execution mode, structured judgments, supporting event references, confidence, coverage, and verifiable provenance where remote. |
| Insight | Metric and denominator, filters/cohort, date window, missingness, uncertainty, supporting evaluations, validity state, and human-readable explanation. |
| Mission | Author/sponsor identity, immutable definition version, starting artifact, allowed tools/models, budget, success rubric, declared evaluators/conflicts, evidence rules, and challenge process. Reward terms are outside this contract’s scope. |
| Mission proposal | Scout identity/version, source URL and retrieval time in an authorized content artifact, source/content digest, attributed claim, reproducible task proposal, evaluator requirements, duplicate lineage, and review/publication state. Operational logs retain hashes and safe labels only. |

One session can contain several episodes; one episode can span sessions and models. Preserve those relationships. Prefer explicit task identifiers and user confirmation; semantic boundary detection is a labeled inference. Do not attribute all work to the final model. Report mixed-model episodes separately until a defensible attribution policy exists.

Distinguish observed facts, user reports, evaluator judgments, and estimates in both storage and UI. A model saying “tests passed” is not a test-run record. Record evidence freshness and delayed outcomes; later reviews and reverts can revise insights. Preserve native usage categories and pricing version. Missing usage is never zero, and list-price estimates are not billed spend.

| Question | Initial measurement | Stronger claim requires |
| --- | --- | --- |
| Best for refactors | Accepted task rate, regression results, correction rounds | Comparable task difficulty and configuration; sufficiently precise estimates. |
| Best for tests | Accepted tests and verified behavior checks | Independent test usefulness, such as known-bug detection or qualified mutation testing. |
| Cheapest for docs | Estimated cost per accepted documentation episode, all attempts included | Usage completeness and price provenance; actual billing data for billed-cost claims. |
| Saves time | Observed completion duration; separately defined active effort if available | Comparable baseline or controlled experiment; elapsed session time alone is insufficient. |
| Rejected code | Explicit rejection/replacement outcomes per eligible episode | Reliable change linkage and adjudication; deletion, churn, and an unmerged PR are insufficient. |

## Phase 0 — Coverage audit and executable contracts

Deliver a source-capability matrix for the supported adapters: model identity/version, tokens/cache/reasoning usage, time, tool outcomes, task boundaries, feedback, and Git linkage. Inspect actual adapter code and synthetic or explicitly authorized traces; do not assume optional fields are populated. Prioritize two adapters based on measured coverage and current user usage.

Define versioned contracts and a metric dictionary. Trace permitted uses through the actual authorization paths: private analysis, hosted analysis, corpus contribution, public attribution, and community aggregation are different actions. Inventory existing local storage and attestation support. Record unknowns and proposed defaults rather than expanding scope to repair every adapter.

Define the initial provider manifest/request/result contract and common rendering vocabulary alongside the evidence contracts. The first-party analyzer uses this interface from the start. Specify provider qualification and result-reconciliation rules, including disagreement and unavailability states, without building a public marketplace.

Acceptance:

- Capability matrix identifies native, inferred, missing, and unsupported fields for each source.
- Fixtures include mixed models, resumed sessions, incomplete usage, absent outcomes, idle time, and duplicate imports.
- Each launch metric has a unit, denominator, missing-data rule, evidence requirement, and example of a prohibited claim.
- Current runtime prerequisites are distinguished from historical design assumptions.
- Contract fixtures cover two provider identities, incompatible rubrics, invalid evidence references, denied permissions, and provider failures.

## Phase 1 — Private descriptive insights: first useful release

Implement idempotent local observation import, source digests, conservative episode grouping, manual category/outcome corrections, and a versioned local derived store. Reuse existing storage where suitable. Keep contribution optional and analysis usable without enrollment.

Expose a CLI command family, with names finalized against existing CLI conventions, for analyze, list, explain, and delete. Deliver a desktop Insights view using the shared backend: task mix, usage/estimated cost coverage, retries and observed tool failures, known outcomes, and a small number of deterministic insight cards. Show source evidence and “insufficient evidence” states. Explicitly assess macOS, GTK, and Windows integration; any platform deferred from release must be identified in the release scope.

Acceptance:

- A user can analyze local sessions without enrollment, upload, or contribution.
- Reimport does not double-count events, resumed sessions, or costs.
- Unknown outcomes and costs stay unknown; mixed-model tasks are visible.
- Every card drills down to its inputs and calculation; explanations initially use templates.
- Deleting a source removes or invalidates affected local episodes, cards, and cached exports; stale results are not served.
- Synthetic golden datasets reproduce the displayed totals and edge-case states.
- First-party results pass through the provider interface and common renderer. No provider selection is required for first use; attribution and evidence remain inspectable.

This phase delivers descriptive value, not a model leaderboard, time-saved estimate, or automatic prompting advice.

## Phase 2 — Outcome linkage and personal model comparisons

Add opt-in local Git/change linkage and structured test-run evidence. Support many-to-many links between episodes and commits; branch proximity alone is only a candidate match. Add a lightweight accepted/partial/rejected/unknown user outcome control with provenance. Scope any Git-host or CI integration explicitly and implement read access first; it must not install sharing hooks or publish trace URLs implicitly.

Start with the complete refactor workflow in milestones B–C; expand task categories after the pilot. Produce personal comparisons for matched task categories, project/language context, qualified model identity scopes, and harness configurations. Include failed and abandoned attempts in task coverage and any qualified task-usage or cost estimand. Report outcome/usage coverage and separate pending outcomes; missing cost does not block an otherwise qualified outcome comparison. Avoid bias from counting repeated attempts as independent tasks. Use predeclared comparison rules, confidence intervals, and suppression for inadequate precision. The calibration packet must choose minimum evidence rules before evaluating ranking results; no universal sample count proves a winner.

Acceptance:

- Test cases cover merged-but-later-reverted work, abandoned tasks, multiple contributing models, and uncertain commit attribution.
- Rankings cannot be emitted from raw transcript sentiment, code churn, or unverified success assertions.
- A comparison includes counts, date range, configuration, missingness, and uncertainty. Uncertain or insufficient evidence is distinct from demonstrated equivalence; any tie claim needs a defined equivalence criterion.
- A manually reviewed, authorized pilot sample validates linkage accuracy before release.

## Phase 3 — Semantic evaluation and prompting coach

Build a bounded evaluator for task classification, clarification/correction loops, missing acceptance criteria, and verification behavior. Start with structured labels and evidence spans; narrative suggestions are generated from validated labels. Treat trace text as untrusted data: the evaluator has no action tools, uses a strict output schema, and cannot accept instructions embedded in a trace.

For remote evaluation, scrub locally, verify attestation before content transmission, bound input/output and spend, and refuse the remote path when verification fails. Maintain the working deterministic local experience. Return results to the contributor without persisting personal profiles or bodies server-side. Audit provider retention, response handling, and transient processing; transport reuse alone does not establish the privacy claim. Token-level log-probability capture is not a prerequisite.

Create a versioned human-labeled evaluation set, split by project and task rather than adjacent events. Measure classification accuracy, evidence support, false advice rate, abstention, and consistency across agent sources. Register acceptance thresholds before the held-out run and retain disagreements for rubric revision.

Integrate a second independently implemented evaluator through the same interface, with its own qualification evidence. It must work through the common Insights/Coaching surfaces without core-specific privileges, provider-specific screens, or a separate account flow. Exercise complementary findings and deliberate disagreement before considering provider discovery features.

Acceptance:

- Every suggestion identifies supporting evidence, acknowledges uncertainty, and proposes a testable behavior change.
- Unsupported explanations, evidence references, and injected instructions are rejected.
- No causal “this prompt saves time” claim without an experiment.
- Privacy, attestation, evaluation quality, latency, and per-episode cost gates pass independently.
- Two implementations render through the same product surfaces; conflicting results retain their provenance and do not produce a fabricated consensus.
- Provider timeout, disablement, invalid signature/schema, and revoked access have tested failure behavior. No fallback transmits evidence without authorization.

## Phase 4 — Optional hosted team insights

Write a dedicated storage/consent design before implementation: roles, content visibility, authorized purposes, retention, deletion, account removal, and source-to-derived provenance. Do not automatically synchronize private profiles. Prefer recomputing from explicitly authorized episode evidence.

Reuse PostgreSQL, forced RLS, authenticated tenant/actor context, existing worker authorization patterns, and versioned derived artifacts. Introduce a dedicated analytics worker scope where existing scopes do not fit. Build bounded read APIs for summaries, comparisons, and evidence, followed by read-only agent tools that retrieve relevant windows. Do not expose content through operational logs or audit fields.

Bind provider grants to tenant, actor, capability, source selection, purpose, expiry, and execution mode. Keep provider credentials separate from user identity and prevent providers from enumerating other tenants or browsing the corpus. Present hosted and provider permissions in the same Trace Commons settings, including authorized budgets and retention terms.

Acceptance:

- Cross-tenant and role-denial tests cover ingestion, evaluation, aggregation, evidence reads, and export.
- Revocation, rescrubbing, retention expiry, and policy changes invalidate derived results and cached views before they can be read.
- Backup/restore drills prove tombstones and consent changes cannot resurrect insights.
- Team owners and contributors can see exactly what is shared and delete it under the documented lifecycle.

## Phase 5 — World missions and community comparisons

Version mission definitions: task, starting artifact, permitted models/tools, budget, time rules, independent success rubric, and submission evidence. Isolate any code execution in a qualified sandbox; replaying arbitrary traces is not authorized by this plan. Evaluate mission success independently from contribution credits and keep named participation consent separate from analytics permissions.

Allow distinct authors, sponsors, and evaluators to participate through the common mission contract. Publish attribution, conflicts, evaluator versions, and challenge rules before an attempt starts; bind attempts to that immutable version. Results and participation stay in Trace Commons Missions. Qualification and moderation can suspend a provider or mission while retaining audit provenance and a clear participant status.

Start with a bounded mission that has objectively testable outcomes. Control or record harness/settings and use randomized or counterbalanced assignments where feasible. Handle duplicate submissions, repeated attempts, leakage, and evaluator gaming. Predeclare analysis rules before collecting the comparison set.

For community analytics, preserve existing fail-closed gates. Minimum cell sizes alone do not protect repeated or overlapping queries. Require a reviewed privacy mechanism, bounded releases/query policy, contribution limits, and a defined revocation policy before computing or storing public aggregate releases. Never approve a mechanism by changing an allowlist to match a placeholder.

Acceptance:

- Independent evaluators reproduce mission outcomes from bounded evidence.
- A mission from a separate author and assessments from distinct evaluators complete the same discovery, submission, result, and challenge flow without a provider-specific UI.
- Public claims identify the mission population, configuration, budget, and uncertainty.
- Privacy qualification, consent, abuse controls, and release/revocation behavior are demonstrated before public publication.
- If these gates fail, private insights remain available and community analytics stays withheld.

### Mission scouts and discovery

Support agents that discover candidate missions from sources such as X, Hugging Face, papers and associated code, and Hacker News. These are candidate source categories, not confirmed integrations. Audit each source's current API/access rules and retrieval capabilities before choosing adapters. Start with manual URL intake and one qualified read-only source adapter; do not make the mission program depend on broad scraping or paid source access.

Scouts translate an external development into a testable proposal. For example, a model release claiming improved tool use could become a mission comparing that model with a declared baseline on a reproducible task. Preserve the distinction between the source's claim, the scout's interpretation, and the eventual experimental finding.

The lifecycle is discover -> draft -> validate -> review -> publish -> collect attempts -> evaluate -> report. Proposals can also be rejected, superseded, or withdrawn with reasons. Each draft must include source attribution, the claim being tested, an obtainable starting artifact, a concrete task, allowed tools/models, execution and cost budgets, success criteria, an evaluator, and required evidence. A discussion thread alone is not a runnable task.

Before publication, check reproducibility, artifact availability and rights, evaluator availability, duplicate/related missions, budget limits, and mission safety. External text is untrusted input: it cannot grant permissions, execute code, alter the rubric, or authorize spending, rewards, or publication. Fetching a source is distinct from downloading and executing its code; any execution uses the mission sandbox and its qualification rules. Inaccessible or changing sources remain explicitly unresolved or superseded rather than silently supporting a published claim.

Initially, scouts create drafts for an authorized curator to review inside Trace Commons. Record the reviewed proposal digest and bind the published mission to that version. Later automatic publication requires a separately qualified policy limited to approved scout identities, task templates, sources, evaluators, budgets, and rate limits, with suspension and audit controls. That policy does not authorize automatic funding or payouts.

Present published proposals in a shared Discover missions feed. Cards show why the task matters, source attribution, expected effort/cost, required capabilities, sponsor/scout/evaluator identities, and mission status. Users can follow topics or scouts within the same product. Keep unpublished drafts in curator views; apply feed eligibility, deduplication, expiration, and disclosed ranking rules so volume or sponsorship does not become evidence quality.

Completed attempts feed the existing evaluation and Insights contracts. Link findings back to the originating claim and mission version, including negative and inconclusive results. Personal recommendations require the same evidence rules as other insights; public findings still pass community publication gates. Discovery popularity does not improve a model's measured score.

Acceptance:

- An authorized curator can take a sourced scout draft through review and publish it in the existing mission experience, without a separate provider account or dashboard.
- Fixtures cover duplicate proposals, source edits/deletion, inaccessible artifacts, unsupported claims, prompt injection, unavailable evaluators, and unauthorized budget/publication requests.
- Publication fails when the task, rubric, required evidence, or reviewed version is missing; retries cannot publish duplicate missions.
- A bounded pilot produces one reproducible mission from an external source and reports its evaluated findings with source/attempt provenance and uncertainty.
- Users can follow a topic or scout, understand why a mission appears, and navigate from mission results to supporting evidence and eligible Insights findings.
- Scout disablement blocks new drafts/publication; published mission history retains provenance and exposes withdrawn or superseded status.

## Work packets and sequencing

The packets below are a scope/dependency inventory, not an instruction to finish each layer on every platform before testing a user workflow. Follow milestones A–E above. Each implementation slice should become a focused PR with fixtures, Astra review evidence, and rollback notes. Consolidate existing PRs before growing another long stack; use isolated worktrees and preserve unrelated dirty files.

| Packet | Scope | Depends on |
| --- | --- | --- |
| 1 | Adapter capability audit, metric dictionary, contract fixtures | None |
| 2 | Shared observation/episode/evidence and provider contracts, reconciliation fixtures | 1 |
| 3 | Local import, deduplication, derivation, deletion lifecycle | 2 |
| 4 | First-party provider, descriptive aggregation, common result validation, explainable CLI | 3 |
| 5 | Unified desktop Insights view, attribution/settings, platform release matrix | 4 |
| 6 | Git/test/user outcome linkage | 3 |
| 7 | Personal comparisons and statistical suppression | 4, 6 |
| 8 | Calibrated semantic evaluator, private remote path, second independent provider conformance | 2, 3, 4; independent privacy qualification |
| 9 | Prompting coach UI and feedback experiments | 5, 8 |
| 10 | Hosted consent/storage design, scoped provider grants, APIs, lifecycle qualification | 7; explicit hosted product decision |
| 11 | Controlled mission pilot with separate author/evaluator roles and unified participation | 6, 7; sandbox and rubric qualification |
| 12 | Community privacy qualification and public comparisons | 11; independent publication gates |
| 13 | Mission scout contracts, manual intake, one source adapter, draft validation, curator review, unified discovery feed | 2, 11; source-access qualification |
| 14 | Bounded scout pilot and mission-to-Insights feedback | 7, 13; public findings additionally require 12 |

The descriptive release boundary remains packets 1–5 and needs consolidation and rollout qualification. Much of outcome linkage has been implemented in the open stack. The next development milestone combines the necessary parts of packets 1, 6, and 7 with a single macOS pilot flow; packet completion alone does not prove that milestone. No schedule is committed before evidence qualification identifies the remaining extraction and pilot gaps.

Provider extensibility is an internal architectural requirement from packet 2. The first release remains a curated first-party experience. User rewards and reward settlement are excluded and owned by Abhishek. A public provider catalog, self-service onboarding, and provider billing are deferred; two independent evaluators must first pass conformance and product usability checks.

Scouts extend the mission program after its core participation and evaluation flow works. Packets 13–14 do not expand the first private-insights release. Unattended scout publication and additional live source adapters are follow-on decisions based on the bounded pilot, not prerequisites.

## Review follow-through

[Kristi’s review of the original plan](https://github.com/TraceCommons/trace-commons/pull/870#issuecomment-5645229086) supports the direction and identifies evidence and product risks. These requirements attach to existing milestones:

- **Current pilot:** follow the [private pilot runbook](2026-09-12-private-refactor-pilot.md). Report the full reviewed-task denominator, mixed-model/delegated/unsupported/missing-context exclusions, boundary split/merge and omitted-attempt corrections, time and manual effort to a reviewed result, abandonment, and decision usefulness. Report overlapping exclusion reasons separately without double-counting the total excluded tasks. User confirmation is an assertion to assess, not validation by itself.
- **Provider recommendations:** disclosure alone is insufficient for a provider's self-model evaluation. Require independent, unaffiliated evaluation before promoting such a finding into a model recommendation. Attributed descriptive findings can remain available within their evidence limits.
- **Phase 3 / packet 8:** publish the semantic execution decision before implementation: rules, local model, or attested remote; supported hardware and licensing; history/input caps; CPU/GPU/RAM/time and monetary budgets; cancellation; and cache invalidation. Choose the method from qualification evidence.
- **Bounded mission path:** distinguish technical access, permitted collection, retained excerpts, attribution, artifact redistribution, and mission publication rights. Treat starting artifacts and dependencies as untrusted. Bind execution to reviewed digests and test substitution plus prohibited filesystem/network access. Reward accounting, mission funding for user rewards, and redemption are outside this work and owned by Abhishek; no reward contract or counsel checklist is a delivery requirement here.
- **Phase 5 / packet 12:** require a reviewed privacy design with an accountable owner, privacy unit, named mechanism, contribution bounds, repeated-query composition/accounting, cumulative release budget, and revocation limits before public analytics implementation. This remains a later gate.

External lessons are patterns to evaluate, not permission to import another product's schemas or text. These requirements strengthen the existing sequence; they do not make hosted analytics or public missions prerequisites for the local pilot.

## Verification and rollout

- Use synthetic fixtures for routine development; real trace inspection requires the relevant authorization and permitted purpose.
- Run focused metric, adapter, lifecycle, and authorization tests as each slice changes. Verify licensing boundaries without changing their expected sets.
- Apply `RUSTFLAGS=-D warnings` for applicable Rust checks/tests, repository Clippy allowances, and formatting checks. Shared lifecycle/FFI changes require workspace tests; client changes require standalone permissive-crate checks and relevant native-shell checks.
- Run all four dependency-license configurations if dependencies change, after obtaining approval for additions.
- Qualify a local release with duplicate imports, incomplete data, deletion, and a small authorized human review. Record evidence coverage, incorrect-card rate, explanation traceability, latency, and cost where applicable.
- Add separate rollout gates for remote inference, hosted persistence, missions, and public analytics. CI passing does not establish those gates.
- Version metrics and evaluators so a faulty version can be disabled, its derived outputs invalidated, and authorized sources recomputed. Keep user annotations distinct from regenerable evaluations.
- Run provider conformance checks for schema/version compatibility, evidence integrity, restricted access, resource limits, failures, provenance, and revocation. Validate that users can understand findings and complete a mission without learning the provider architecture.

## Product choices to revisit with evidence

Product direction is settled for this plan: one Trace Commons experience across Insights, Coaching, and Missions, with curated defaults and attributable providers underneath. Private local use remains independent of enrollment and contribution. Packet 1 determines initial source coverage. Packet 5 records desktop release scope. Remote evaluation needs a budget and qualified attestation policy before activation. Hosted team storage and community publication need their own concrete designs; neither is necessary to deliver the first release.

## External lessons

Reviewed during the preceding architecture discussion on 2026-09-11:

- [Traces Git hooks](https://traces.com/docs/sharing/git-hooks): many-to-many trace/commit linkage is a useful pattern. Adopt evidence linkage without automatically adopting background sharing.
- [Traces message API](https://traces.com/docs/api-reference/messages): native token categories differ by source; normalize accounting explicitly.
- [Traces MCP](https://traces.com/docs/mcp): separate discovery from bounded evidence reads when returning value to agents.
- [Traces organization privacy](https://traces.com/docs/organizations/privacy): enforce permissions centrally for hosted features.

These docs provide integration patterns, not validation of model-quality rankings.
