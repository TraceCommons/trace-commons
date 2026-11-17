# Insights, mission drafts and skill learning

Baseline `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. Current behavior finite reference projection. The owner selected P13 on 2026-11-06: “A tested skill is installed only after I confirm the matching installation plan.” The selected invariant and observer correspondence are documented in [skill-install notes](../proofs/skill-install-notes.md).

## Variables and bounds

Insights keeps screen lifetime/presentation, saved-snapshot existence, analysis enablement, counter feed, chosen feed, visible figures and week request generation separate. Two abstract weeks and three reusable generation names bound stale-result races while allowing repeated selection/close/reopen. Local snapshots and daemon week are alternative complete feeds, not combined figures. Mission drafts keep native mutation, saved draft existence, selected file/detail and presentation epoch separate. A native mutation may commit after closure; reopening waits and refreshes rather than interpreting cancellation as rollback.

Two submission-keyed skill records keep workflow phase, operation, owner lease/generation, package identity, reviewed/evaluated/planned identities, evaluation outcome, plan lifetime, marker identity, filesystem publication, installed-status knowledge, external edits and public run publication separate. Two owners (1→2), one package edit (0→1), one held evaluation/one in-flight request index, and two marker identities bound the model. Filesystem publication is distinct from status reconciliation and UI success.

## Exact source mapping

- Insights active/generation/cancel guards, saved-feed fallback and whole counter-week replacement: `macos/Sources/TraceCommonsApp/InsightsModel.swift:52-119`. Overview clears/reloads figures and session inputs: `:122-188`. Store/security-scope routing: `macos/Sources/TraceCommonsApp/InsightsServiceRouter.swift:17-55` (coarse native operation).
- Mission copy/list/detail/confirmation and detached mutation: `macos/Sources/TraceCommonsApp/MissionDraftsModel.swift:79-100`, `:122-198`, `:199-279`. In particular closing keeps the mutation wrapper alive, and a late result for a reopened screen triggers a fresh read rather than applying stale detail.
- Candidate/status/review/evaluation/planning guards: `macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:9-187`; `macos/Sources/TraceCommonsApp/SkillLearningStore.swift:152-177`, `:183-258`, `:263-325`. Exact package/review/evaluation lineage checks are represented by explicit identity fields, with payload mismatch/failure coarsened.
- Installed status first, install commit followed by independent status read: `macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:193-260`; UI uncertainty/recovery state `macos/Sources/TraceCommonsApp/SkillLearningStore.swift:328-376`.
- Rollback, install-unknown recovery and changed-file refusal: `macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:266-312`; `macos/Sources/TraceCommonsApp/SkillLearningStore.swift:379-402`; `crates/trace-commons-contributor/src/skill_loop/install.rs:375-393`.
- Account-owned daemon workflow, TTL and bounded eviction: `crates/trace-commons-contributor/src/daemon/skill_loop.rs:30-53`, `:132-185`. Fresh owner lease check: `crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:184-225`; candidate/review/evaluation/planning `:304-599`; filesystem publication, owner recheck/compensation and marker recovery `:602-735`. Native account reset `macos/Sources/TraceCommonsApp/AppModel.swift:3989` clears the UI store; this model retains filesystem facts while returning stale workflow knowledge to idle.
- Public run publication is a separate account/digest/version reviewed operation, not the skill evaluation or installation: `crates/trace-commons-contributor/src/daemon/public_run.rs:25-27`, `:269-349`.

## Frame/copy provenance

Frames are authored reference projections, not full native screenshots or exact message reproduction. Skill controls use literal shared labels from `crates/trace-commons-contributor/src/skill_loop/copy.rs:85-158` (Learn, Review skill, Approve & test, Edit skill, Review install, Retry install, Install skill, Roll back, Keep). Insights Open file/Refresh/Delete labels are shared in `crates/trace-commons-contributor-insights/src/insights/service.rs:297-318`; Mission Choose file/Import/Refresh/Delete are read through `macos/Sources/TraceCommonsApp/Views/MissionDraftsView.swift:48-52`, `:91-102`. Enable, Select week and Select trace are explicit input abstractions for the relevant enablement/row choices, not claims of literal buttons. Navigation/window labels are projection controls. Publish/Unpublish project the daemon public-run flow; complete native review copy and acknowledgement screens are not reproduced.

## Coarse detail and omitted behavior

Actual imports, parser/security-scoped permissions, local analysis algorithms, sample data, per-card drill-downs and their independently canceled tasks, subscriptions/credit forecast metrics, model output and mission package validation are coarse native interfaces. Enabled/unreadable/empty must remain distinct; an empty saved feed is not evidence of no local traces. This model does not reproduce complete Insights layout or every local service operation.

Mission import duplicates, selected draft identities and stale delete confirmation tokens are summarized as validated request outcomes, with one saved draft instead of a multi-file inbox. Skill gate scoring/fixtures/model budgets and catalogues are external outcomes, not verified model-quality claims. Full bounded workflow eviction (actual maximum 12), multiple evaluation concurrency, credential refresh during evaluation and install OS primitives are not exhaustively represented. The single request index prevents faithfully exploring two concurrent distinct skill operations; independent durable rows and owner identity are retained, but multi-request response interleavings require refinement. Native UI callbacks lack a screen-active guard for skill operations; closing the view therefore does not cancel a daemon mutation or erase installation facts. Owner change compensation failure/retained signed marker is coarse; unchanged installed bytes can be compensated, edited bytes remain. Public run publication is a coarse remote interface and is not skill installation publication.

## Composition hooks

`TheReadableCounterWeekReturns` / `TheCounterWeekIsDisabledOrUnreadable` expose read-back feeds; `TheMissionMutationCommits` is only a local draft effect. `TheSkillReviewBindsPackageBytes`, `TheSkillEvaluationPasses`, `TheSkillInstallPlanIsHeld`, `TheFilesystemPublishesTheSkill`, `TheInstallStatusConfirmsPublication`, `TheInstallStatusReadFailsAfterPublication`, and `TheUnchangedSkillIsRolledBack` separate approval, evaluation, publication and reconciliation. `TheLearningAccountChanges` invalidates in-memory owner leases while filesystem markers remain external durable facts. `TheRunPublicationReturns` is a separate server publication event.

No fairness assumptions or progress guarantees are encoded. `P13` records authorization at filesystem publication and requires an independent status observation before installed-status success. Its monitors preserve the original transition projection; they do not add product guards. The configuration selects `INVARIANT P13`; see the proof notes for check evidence and finite bounds.

No universal wait/stutter action is enabled. Timing and fairness remain unselected; no deadlock or progress claim follows from this model.

Account change resets both rows' workflow/plan/UI-install knowledge while preserving external filesystem facts. Outstanding operation identity remains only to model its old-owner rejection; a full native-store reset allowing a second new-owner request concurrently remains outside the one-request bound.

## Source-review repair: repeatable screen lifetime

`YouCloseLearning` remains available after every reopen; it chooses finite fresh view/mission/week generations distinct from their current and captured outstanding read identities. `YouSelectAnotherInsightWeek` similarly reuses fresh symbolic generations and toggles two abstract weeks. This preserves the actual UUID invalidation/equality behavior for retained outstanding requests without disabling valid UI actions at a numeric cap. Source: `macos/Sources/TraceCommonsApp/InsightsModel.swift:52-59`, `:81-85`, `:174-188`; `MissionDraftsModel.swift:86-100`. As elsewhere, the model retains a bounded set of request snapshots and does not claim full unbounded request history.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees. P13 was subsequently selected; the [proof notes](../proofs/skill-install-notes.md) distinguish focused observer tests from the selected invariant checks.
