# Deterministic Insights question cards

## Goal and release boundary

Add a shared, versioned projection that answers a small set of useful questions from locally saved snapshots and user-selected episodes. The first cards describe observed evidence. They do not rank models, infer task boundaries, estimate active work or time saved, infer rejection from tool failures, or turn overlapping episode groups into independent trials.

The first release should answer:

1. How much recorded activity is represented?
2. What user-reported outcomes exist for the selected episode groups?
3. What model labels are observed, and with what coverage?
4. Which facts remain unknown?

Cost and pricing remain unavailable until native usage and versioned pricing evidence are persisted. The card surface must display that state rather than synthesize a number.

## Existing contracts this extends

`trace-commons-protocol::insights` currently exposes schema-1 descriptive metrics, a local-only provider manifest, evidence references, and observed/total coverage. `LocalInsight` persists one exact source digest, source format, a session-proxy boundary, the report, optional user annotation/model observations/outcome links, and import time. The local store accepts versions 1 through 4, validates every report against the first-party manifest and saved digest, and promotes the index version on the next mutation.

Episodes are schema-1 user-selected groups of 1–64 whole saved snapshots. Their canonical membership digest and `membership_revision` bind an optional user-reported assessment to the exact member set. Groups may overlap; the episode detail already identifies shared snapshots. Snapshot deletion can invalidate entire groups. These properties are inputs to cards, not details for shells to reconstruct.

The planned `insights/provider.rs` dispatch should land first. Its object-safe `LocalInsightProvider` accepts a schema-1 `ProviderRequest` for `private_descriptive`, the installed manifest, one exact source reference, and bounded `EventObservation` values that contain classification/tool-outcome facts only. It returns the existing schema-1 `InsightReport`; the host validates before dispatch and binds the result to the installed manifest and exact evidence afterward. Typed public errors stay safe, and the local trusted boundary grants no plugin sandbox or permission elevation. It preserves existing first-party output. Question cards add a separate versioned protocol and provider capability after that seam exists.

The planned `insights/time_evidence.rs` extractor should also land before persistence work. Its schema-1 contract counts eligible source records, distinguishes absent/null timestamps from invalid non-string or malformed values, normalizes RFC 3339 offsets to UTC, treats epoch zero as valid, and retains deterministic record-coordinate references for earliest/latest ties. Codex `session_meta` and `turn_context` rows and trajectory metadata are excluded by the adapter rules; other adapter-supported source records remain eligible, including Codex rows normalized as opaque. This plan consumes those facts; it does not introduce a second parser.

## Shared contracts

Keep serialized data contracts in `trace-commons-protocol` and the executable provider trait, host policy, persistence, and first-party implementation in `trace-commons-contributor`. Do not add dependencies.

Introduce a card-specific schema rather than widening `InsightReport` schema 1. Suggested names are illustrative until the prerequisite provider interface fixes its established naming:

```rust
pub const INSIGHT_CARD_SCHEMA_VERSION: u32 = 1;

pub enum InsightQuestionId {
    RecordedActivity,
    EpisodeOutcomes,
    ObservedModels,
    EstimatedCost,
}

pub struct InsightCardRequest {
    pub schema_version: u32,
    pub questions: Vec<InsightQuestionId>,
    pub evidence: Vec<EvidenceRef>,
    pub episodes: Vec<EpisodeCardInput>,
    pub expected_provider: ProviderManifest,
}

pub struct EpisodeCardInput {
    pub id: String,
    pub revision: u64,
    pub membership_revision: u64,
    pub members_digest: String,
    pub member_evidence_ids: Vec<String>,
    pub assessment: Option<EpisodeAssessmentInput>,
}

pub struct InsightCardResult {
    pub schema_version: u32,
    pub provider: ProviderManifest,
    pub input_digest: String,
    pub cards: Vec<InsightCard>,
}
```

The protocol must not depend on contributor episode types. `EpisodeAssessmentInput` therefore carries only the validated category, outcome, user-reported provenance, and membership binding required by the evaluator. The host builds it from one locked store read. Request validation requires unique supported questions, exact allowed evidence, local execution, bounded counts and strings, canonical episode/member ordering, unique episode IDs, known members, matching membership digests, and assessment revisions that equal the episode membership revision.

The card provider capability remains deterministic and local. Extend the provider interface through a distinct method or capability whose typed result is `InsightCardResult`; do not reinterpret the existing report evaluator result as presentation data. Dispatch must reject unsupported schema/rubric, provider mismatch, duplicate cards, missing requested cards, unknown evidence, forged digests, unrequested evidence, invalid coverage, and an incorrect input digest. Public errors are fixed labels and never forward raw provider failures.

## Common card projection

Every shell receives the same ordered, presentation-ready projection from `LocalInsightsResponse::QuestionCards`. Shells may format numbers and dates for locale and lay out native controls; they must not count outcomes, combine time ranges, calculate overlap, choose denominators, or invent unavailable reasons.

Each `InsightCard` contains:

- stable `question_id`, `metric_version`, and shared title/body copy keys;
- `state`: `observed`, `partial`, or `unavailable`;
- typed rows with stable labels and integer, timestamp, duration, or categorical values;
- a denominator block with `eligible`, `assessed`, `unassessed`, and `explicit_unknown` where applicable;
- `coverage` with an explicit unit and missing count;
- evidence references limited to saved snapshot IDs and episode IDs resolved by the host;
- limitation keys, including overlap and timestamp semantics;
- the validated provider manifest and input digest on the enclosing result.

Rows contain typed values, not preformatted English. Shared `ui_copy` owns every label and limitation sentence. Ordering is fixed by question ID and row ID, regardless of map iteration or import order. Unknown is never encoded as zero.

### Recorded activity

Show saved snapshot count, normalized events with coverage, tool calls, and tool failures among tool results with explicit outcomes. Tool failures remain tool-result observations and never become task rejection.

Add timestamp rows only from persisted extractor output:

- timestamp-eligible records;
- records with valid timestamps, records with missing/null timestamps, and records with invalid timestamps;
- earliest and latest observed record timestamps;
- bounded earliest/latest record-coordinate references and omitted-tie counts;
- record span in milliseconds (`latest - earliest`, with checked arithmetic) when at least two valid timestamps exist.

Call this `record_span`, never duration, active time, elapsed work, or time saved. Do not sum snapshot spans: snapshots may overlap or contain idle gaps. For an episode or multi-snapshot selection, use the minimum observed record timestamp and maximum observed record timestamp and show record coverage. If timestamps are partial, the card is partial. If fewer than two are observed, span is unavailable with a fixed reason. `analyzed_at`, episode `created_at`, and episode `updated_at` are lifecycle metadata and cannot substitute for source event time.

### Episode outcomes

Count only current episode assessments whose membership binding validates. Present integers:

- eligible episode groups;
- assessed groups;
- accepted, partial, and rejected user-reported groups;
- explicit `unknown` assessments;
- unassessed groups;
- groups that overlap at least one other selected group;
- distinct saved snapshots represented.

The primary wording is, for example, “2 rejected among 7 assessed user-selected episode groups; 3 unassessed; 1 explicitly unknown.” Do not emit a rejection rate, confidence interval, comparative score, or “trials” label in this slice. `unknown` is an explicit user assessment inside the assessed denominator; `unassessed` is absence of an assessment. The card must not combine them or omit either denominator.

If any selected groups share a snapshot, include the overlap limitation and the overlap count. Overlapping groups are dependent descriptions. Even when no overlap is observed, the card does not establish independent tasks because episode boundaries are user-selected rather than inferred.

### Observed models

Project only persisted adapter model observations. For every model label, show observed snapshot count and the eligible/observed snapshot denominator. Preserve mixed, ambiguous, and unavailable adapter states as typed unknown reasons. Do not call the most frequent label fastest, cheapest, preferred, or best. Do not associate an episode outcome with one model when its members contain multiple or unknown models.

### Estimated cost

For this slice, always return `unavailable` with the persisted usage/pricing prerequisite reason. Do not reuse the current ephemeral file usage command, infer tokens from text, or treat `estimated_cost_usd: None` as zero.

## Timestamp persistence and store migration

After the extractor API is fixed, add an optional `time_evidence` field to `LocalInsight` that retains the extractor's exact accepted schema-1 structure: source-record eligible/valid/missing/invalid counts, optional earliest/latest UTC extrema, up to 16 deterministic record-coordinate references for each endpoint, and omitted-tie counts. Bind it to the saved source digest and source format alongside the field rather than adding a second representation inside it. Equal timestamps from duplicate or resumed records remain separate coordinates. Validation enforces count partitions, chronological extrema, bounded references, and internal consistency. Record span and unavailable/partial presentation are derived by the card projector, not persisted. Span uses a checked millisecond difference so RFC 3339 subsecond precision is not silently reduced. No code deduplicates timestamps or substitutes import time.

Bump the local index to version 5. Versions 1–4 deserialize the missing field as `None`, which means unknown until explicit reimport; opening a legacy store must not invent timestamps or reread source files. The next successful mutation persists version 5 through the existing atomic write. Reject future versions and malformed version-5 evidence. Add round-trip and migration fixtures for versions 1–4, including annotations, model observations, outcome links, and episodes, and prove they remain unchanged.

Cards are derived on each request from one validated locked store snapshot and are not persisted in this slice. Their `input_digest` covers the card schema/rubric, canonical evidence IDs and source digests, episode IDs/revisions/membership revisions/member digests, and relevant assessment/timestamp/model facts. A relevant input change therefore changes the digest; an exact same-byte reimport or other semantic no-op remains stable. Snapshot deletion removes affected reports and invalidates affected episodes through the existing mutation lifecycle; a later card read cannot return evidence from either. Do not cache a provider result without keying it by the complete digest, and do not add such a cache in these PRs.

## Service and platform wiring

Add `LocalInsightsOperation::QuestionCards { questions, snapshot_ids, episode_ids }`. Validate selection limits before acquiring the store lock. Resolve snapshots, episodes, assessments, overlap, and optional timestamp/model evidence in one store transaction, dispatch once, validate the result, and return a typed `LocalInsightsResponse::QuestionCards`. Missing or changed inputs return fixed conflict/missing-evidence errors; the service does not silently drop them.

Add a CLI command that prints the shared cards and evidence IDs without recomputation. Extend the FFI size/copy bridge with the existing bounded-response and panic/error rules. Add typed decoders in macOS, Windows, and GTK. Each unified Insights view gets one compact “Questions from saved evidence” section, loading and error state, evidence navigation, and shared copy. Card reads use the existing generation/presentation guards. A stale completion cannot replace cards for a newer saved-snapshot or episode selection. Mutations that change evidence clear the prior card presentation before reconciliation; a failed reconciliation visibly leaves cards unavailable rather than editable as current.

Native shells render the shared rows and limitations. Platform tests verify decoding, stale completion rejection, unknown display, overlap copy, evidence navigation, and invalidation after snapshot or episode changes. Windows-native WinUI compilation remains a Windows CI gate; macOS-hosted .NET tests validate only managed decoding/view-model and native FFI behavior. GTK, Windows, and macOS release checks remain separate.

## Follow-on: persisted usage and pricing

Cost becomes eligible only after a separate design and migration stores native input/output/cache/reasoning usage with source digest, model attribution, adapter provenance, observed/eligible coverage, and usage schema version. A versioned local pricing table must record provider, model/version applicability, currency, unit rates, effective interval, and table provenance. Keep actual billed cost distinct from a deterministic estimate. Mixed models, missing usage, unknown prices, and price-window mismatch remain partial or unavailable. Historical cards must be reproducible against their price-table version. No provider network call or billing credential is authorized by this follow-on description.

Time-saved and active-time cards remain out of scope even after timestamp coverage exists. They require a separately approved measurement design and baseline.

## Testable PR sequence

### PR 1 — Provider dispatch prerequisite

Land the contributor-owned object-safe provider and host dispatch described above. Route `analyze_file` through `FirstPartyProvider` while preserving byte-for-byte compatible schema-1 JSON. Test request validation before invocation, the `private_descriptive` purpose, one-source binding, exact manifest/rubric matching, bounded normalized observations, provider failure sanitization, forged/extra evidence rejection, deterministic output, and unchanged adapter fixtures.

### PR 2 — Timestamp extractor prerequisite

Land `insights/time_evidence.rs` with adapter fixtures covering the exact eligible-record rules, missing/null versus invalid values, mixed offsets, epoch zero, out-of-order records, duplicate/resumed equal timestamps, 16-reference tie bounds, and omitted-tie counts. The extractor returns facts only and makes no elapsed-work claim. It does not touch persistence.

### PR 3 — Timestamp persistence and store v5

Add optional `LocalInsight::time_evidence`, populate it on new imports, validate its source binding, and implement the version-1-through-4 compatibility path. Test legacy reads, mutation promotion, future-version rejection, malformed evidence rejection, exact reimport replacement, dedup aliases, deletion, and preservation of episode/annotation/model/outcome data.

### PR 4 — Card protocol and first-party projection

Add the versioned request/result/card types, validation, executable metric dictionary, input digest, first-party deterministic projector, and exhaustive fixtures. Tests cover stable ordering/digest, unknown versus zero, assessed/unassessed/explicit-unknown denominators, overlapping groups, duplicate memberships across groups, mixed models, timestamp partiality, lack of rate output, lack of active/time-saved claims, and unavailable cost. Property-style table tests can use existing test tools; add no dependency.

### PR 5 — Store service, CLI, and FFI

Add the atomic store projection, `QuestionCards` service operation/response, fixed public errors, CLI output, and bounded FFI path. Test concurrent mutation/read consistency, replacement and deletion invalidation, episode revision changes, missing evidence, response-size limits, provider failure sanitization, CLI golden output, and actual native bridge behavior on a fresh shared library.

### PR 6 — macOS card section

Add typed bridge decoding and the shared card section to the unified view. Test generation guards, mutation invalidation, partial/unknown display, overlap limitation, evidence navigation, and malformed responses. Run Swift package tests and the macOS app build.

### PR 7 — Windows card section

Add typed interop decoding, view-model state, and WinUI controls. Test stale reads, failed reconciliation, partial/unknown display, overlap limitation, and evidence navigation in managed/native bridge tests; require Windows CI for WinUI compilation and control lifecycle.

### PR 8 — GTK card section and release matrix

Add typed response handling and compact GTK rows with the same generation and invalidation rules. Test reducer/state transitions and rendering helpers, run warning-denied GTK checks, and record the three-shell release matrix. The matrix reports each platform gate independently and does not convert green CI into product-quality or live-provider evidence.

## Required gates for the slice

- No new crate, Swift package, NuGet package, or system dependency.
- No source paths, trace bodies, user identity, or raw provider errors in cards, logs, or public errors. Digests may bind selected evidence bytes and typed projection inputs, but raw content is never logged or embedded in their serialized preimage.
- No remote execution, account, enrollment, contribution, discovery, upload, or publication path.
- No model ranking, cost number, rejection inference, active-time value, or time-saved claim.
- Every displayed count has a named unit, evidence reference, and explicit denominator or unavailable reason.
- Episode overlap is visible and never modeled as independent trials.
- Deletion/reimport/version changes invalidate derived presentation through the source digest and revision lifecycle.
- Protocol/contributor/FFI/desktop code remains permissive and gains no dependency on an AGPL crate.
- Apply repository formatting, warning-denied checks, focused tests, workspace tests for shared lifecycle/FFI changes, and the unchanged license-boundary test. Dependency-license sweeps are unnecessary because this slice adds no dependency.
