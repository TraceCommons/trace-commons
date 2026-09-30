# Persisted native usage and versioned pricing for Insights

Date: 2026-09-11
Status: Implementation plan. No price, dependency, external call, deployment, or billing access is authorized by this document.

## Outcome and boundary

This follow-on makes the deterministic `estimated_cost` question eligible for a number only when a saved snapshot contains complete source-native usage, that usage has defensible model attribution, and an immutable reviewed price table applies to the usage interval. The result is a deterministic estimate in integer monetary units. It is not an invoice, billed spend, a budget debit, or evidence that the declared model served the request.

This extends the question-card follow-on and the private Phase 1 foundation in the unified Insights program. It supplies a prerequisite for later outcome-linked comparisons, but does not implement or change the authorization of Phase 2 model comparisons. Cost must not be used as a model ranking or joined to episode outcomes until the separate comparison population, mixed-model suppression, and uncertainty gates are implemented.

The implementation remains local and reads only explicitly selected source files. It does not discover sessions, fetch prices, contact providers, use billing credentials, persist source bodies, alter enrollment, or publish anything. Active time and time saved remain out of scope.

## Current facts that constrain the design

`insights::usage::extract_usage` is currently ephemeral. The service and CLI accept one explicit JSONL file and return `UsageSummary`; the FFI transports the same operation. Nothing attaches that result to a saved snapshot.

The two native accounting schemes are intentionally different:

- Codex `total_token_usage` is a cumulative snapshot. The latest monotonic value is the total; snapshots are not summed. `cached_input_tokens` is a subset of input, and `reasoning_output_tokens` is a subset of output. A billable decomposition can subtract cached input from input with checked arithmetic, but must not add cached input or reasoning output again.
- Claude Code usage is per assistant message. Repeated `message.id` values are latest monotonic snapshots of the same message and are deduplicated before totals are summed. Input, cache-read input, cache-creation input, and output are separate accounting categories.

Codex source qualification is pinned to the upstream schema and runtime that
define these rollout records. [`SessionMeta` has `model_provider` but no model
field](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/protocol/src/protocol.rs#L3055-L3131),
and an assistant [`ResponseItem::Message` also has no model
field](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/protocol/src/models.rs#L997-L1023).
The authoritative declaration is the required [`TurnContextItem.model`, whose
record is documented as a durable replay
baseline](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/protocol/src/protocol.rs#L3227-L3267),
and [the runtime fills it from the effective model
slug](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/core/src/session/turn_context.rs#L654-L677).
Consequently, absence of `model` on session metadata or assistant messages is
normal and is not a declaration gap.

Codex [constructs `total_token_usage` by adding the latest response usage and
replacing `last_token_usage`](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/protocol/src/protocol.rs#L2267-L2305),
but the upstream contract also says a [`TokenCountEvent` may be accumulated,
estimated, or replayed](https://github.com/openai/codex/blob/c4017a87aacc7558002b7cb510025e967c1d765e/codex-rs/protocol/src/protocol.rs#L1920-L1927).
A context-window failure can synthesize a full-window count. Complete category accounting,
monotonicity, and checked subtraction therefore qualify these values as
observed rollout counters for a deterministic estimate; they do not establish
provider billing or an invoice.

The current extractor retains bounded model labels but deliberately does not allocate counters to models. For Codex, a final or most recent label cannot own a cumulative total that spans a model switch. Claude records carry a model on each assistant message and can eventually support per-model grouping after duplicate resolution.

Saved store schema 5 binds optional model and timestamp observations to the exact report evidence digest and source format. Versions 1–4 read those fields as unknown and upgrade on the next mutation. `LocalInsight.estimated_cost_usd` is required to remain `None`, and `cost_unavailable_reason` is required to remain `adapter_usage_unavailable`. These deprecated placeholders remain unchanged for decoder compatibility; new cost results use the typed integer contract below.

Saved analysis supports Codex and trajectory. Native usage inspection supports Codex and Claude Code. Therefore:

- Codex usage can be extracted from the same bytes during a saved Codex import.
- Trajectory has no accepted native usage contract and remains `source_usage_unsupported`.
- Claude Code usage cannot be attached to a `LocalInsight` until a saved Claude Code analysis adapter exists. The standalone CLI/FFI inspection remains useful but must not create an orphan usage record or attach it by path or model label.

## Persisted usage contract

Add contributor-owned schema-1 types in `insights/usage.rs`. They are cache evidence from native bytes, rather than a provider result, so they remain in the permissive contributor crate alongside `ModelObservations` and `RecordedTimeEvidence`.

```rust
pub struct PersistedUsageEvidence {
    pub schema_version: u32,
    pub source: UsageSource,
    pub scope: UsageScope,                 // exact_source_file
    pub source_format: SourceFormat,
    pub source_digest: String,
    pub coordinates: UsageRecordCoordinates,
    pub candidate_records: u64,
    pub complete_records: u64,
    pub duplicate_records: u64,
    pub counts: Option<NativeTokenCounts>,
    pub priced_interval: Option<UsageCounterInterval>,
    pub attribution: UsageAttribution,
    pub unavailable_reason: Option<UsageUnavailableReason>,
}

pub struct UsageCounterInterval {
    pub baseline: NativeUsageCounterSnapshot,
    pub final_snapshot: NativeUsageCounterSnapshot,
    pub observed_delta: NativeTokenCounts,
    pub excluded_prior_counts: NativeTokenCounts,
}

pub enum UsageAttribution {
    SingleDeclaredModel { model: String },
    PerModel { segments: Vec<ModelUsageSegment> },
    Unavailable { reason: AttributionUnavailableReason },
}

pub struct ModelUsageSegment {
    pub model: String,
    pub counts: NativeTokenCounts,
    pub source_record_refs: Vec<u64>,
    pub omitted_record_refs: u64,
}
```

Use stable JSONL physical line numbers, including blank lines, as coordinates. A counter snapshot includes its coordinate and accepted source-record timestamp. References identify accepted native usage records after adapter validation; they do not retain message IDs, session IDs, paths, prompts, or content. Bound references and segments using existing source and model limits. Omitted supporting references are recorded separately and do not make an otherwise complete numeric total unknown; omitted counters, identities, model declarations, or other semantic facts do. Counts remain `u64`; all validation sums and differences use checked arithmetic.

Validation requires schema 1, the exact source digest and format of the containing snapshot, a supported source/format pair, a complete count partition, canonical unique references, accounting variants that match the source, and mutually consistent counts/reason. It also rechecks source-specific invariants:

- Codex: `cached_input <= input`, `reasoning_output <= output`, and `input + output == total`. There is one cumulative total, never a sum of snapshots. An available aggregate requires one valid session ID and monotonic counters.
- Claude Code: each retained message contributes exactly once after stable-ID deduplication; cache-read and cache-creation remain independent of ordinary input. Per-model segment sums must equal the aggregate in every category.

Do not represent absent usage as zero. `counts: None` requires a typed reason. A present all-zero native record is a known zero only if every required counter and identity was present and valid.

### Attribution rule for the first priced release

The first priced release does not attribute the latest Codex cumulative total merely because the file contains one label. Counters may include work before the first retained event, timestamp, or model declaration.

Instead, retain the first and final complete monotonic cumulative snapshots with valid source timestamps and calculate only their checked category deltas. `excluded_prior_counts` is the first snapshot, and is never priced. The priced scope is explicitly `observed_counter_delta`, not full file or session cost. A nonzero excluded baseline makes coverage partial even when the later delta is priced. With only one snapshot, a known all-zero value can establish a zero observed delta; a nonzero snapshot has no baseline and is unavailable.

Accept attribution for that delta only when the latest `turn_context` at or before the baseline has a valid model, every later `turn_context` through the final snapshot is valid and names that same model, and every usage record belongs to the same validated Codex session. Earlier contexts can differ or be malformed because their counters are contained in the explicitly unpriced baseline. A malformed latest baseline context is not skipped in favor of an older valid context, and a missing or malformed later context is a semantic gap rather than permission to carry the earlier label forward. Store the qualified label as `SingleDeclaredModel`. Bounded omission of duplicate supporting record references is allowed when counters, ordering, session identity, timestamps, and model facts remain complete.

If the baseline context is absent or invalid, or later contexts change or omit the model, persist the aggregate counts for coverage but set attribution unavailable. Do not allocate cumulative deltas to a later `turn_context`, use the final label, or split evenly. A later schema may add per-turn attribution only after fixtures establish the ordering and reset semantics.

For Claude Code, evolve extraction to deduplicate each message first and then group its complete usage by that message's valid model label. Missing or invalid labels make the aggregate available but attribution incomplete; they do not become an `unknown` price bucket. `PerModel` is eligible only when all complete messages are attributed and no valid model or record reference was omitted. This work can land and be tested in the extractor, but persistence waits for the saved Claude adapter.

## Import, binding, and store schema 6

Add `#[serde(default)] pub usage_evidence: Option<PersistedUsageEvidence>` to `LocalInsight` and bump the index to version 6. Cost is derived from usage plus a price table; it is not a mutable field on the snapshot.

Retain the existing `estimated_cost_usd` and `cost_unavailable_reason` fields as deprecated compatibility fields during this schema. Continue serializing the float as `null`, never populate it, and preserve the existing fixed reason so older CLI and native decoders do not fail. New consumers use only the typed card result. Remove or version these fields only with an explicit service-response compatibility migration and updated all-shell decoders.

`analyze_file` already reads and validates the selected bytes once. From those same bytes it builds the report, model observations, timestamp evidence, and, for Codex, usage evidence. No second path read is allowed. The snapshot ID, report evidence, model observations, timestamp evidence, and usage evidence must all carry the same SHA-256 source digest and source format.

Trajectory analysis stores `usage_evidence: None`. Absence has the fixed meaning `source_usage_unsupported` when cards resolve that source. Do not run the Codex or Claude extractor over trajectory JSON and do not infer usage from normalized events.

Versions 1–5 deserialize the field as `None`. Opening a legacy store does not reread source files, consult aliases, or invent zero usage. The next successful mutation writes version 6 while preserving reports, annotations, model observations, timestamp evidence, outcome links, aliases, and episodes byte-for-semantic-byte. Explicit reimport of the original source is the only way to populate usage evidence.

Content-identical aliases continue sharing one report and one usage record. A changed digest creates a new snapshot and cannot inherit old usage. Deletion removes the usage with the report; an alias that still retains the report retains the same source-bound usage. Malformed schema-6 usage, a wrong digest/format, an impossible count relationship, an invalid attribution, or a future usage schema fails the whole cache read without panic.

Keep the existing ephemeral `usage` command for inspection. Add no attach-by-ID mutation in this sequence: accepting a separately supplied file would require proving it is the same bytes and source type, and for Claude there is no saved snapshot target. A future explicit reimport operation can use the normal snapshot lifecycle.

## Provider and rate-context qualification

Model attribution is not provider attribution. `has_attributed_interval()` only
establishes model-attributed observed counters; the current persisted evidence
does not retain provider identity. Codex can use custom endpoints, and its
configured `model_provider` label alone does not prove which service supplied
the usage. Do not infer OpenAI prices from the Codex adapter or a model slug.

Before automatic numeric cost projection, require source-bound qualification of
the provider and rate context, exact provider-plus-model matching, and applicable
interval/category coverage. Missing, conflicting, custom, or unqualified context
must produce typed unavailability. A future explicit user-selected rate table
could support a hypothetical estimate, but that is a distinct product operation
whose result must name the selected rate assumption; it must not silently fill
the automatic estimated-cost card. Neither operation establishes invoiced cost.

The [real pricing source audit](2026-09-11-insights-real-pricing-qualification.md) adds per-request cache-write, context-band, processing-mode, and regional qualification requirements before real rates can be admitted. The schema-1 calculator currently establishes synthetic fixed-rate arithmetic only.

## Immutable pricing evidence

Add a versioned, presentation-neutral pricing contract in `trace-commons-protocol`, because the deterministic card request and all shells consume its result. Keep catalog loading, validation, and local persistence in `trace-commons-contributor`.

```rust
pub struct PricingTable {
    pub schema_version: u32,
    pub table_id: String,
    pub version: u32,
    pub currency: Currency,                // initially USD only
    pub published_at: DateTime<Utc>,
    pub source: PricingTableProvenance,
    pub entries: Vec<ModelPriceEntry>,
}

pub struct PricingTableProvenance {
    pub publisher: String,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
    pub content_sha256: String,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
}

pub struct ModelPriceEntry {
    pub provider: String,
    pub model: String,
    pub effective_from: DateTime<Utc>,
    pub effective_until: Option<DateTime<Utc>>,
    pub rates: TokenRates,
}

pub struct TokenRate {
    pub category: BillableTokenCategory,
    pub usd_nanos_per_million_tokens: u64,
}
```

`table_id` is a domain-separated digest of canonical table content. Entries sort by provider, model, effective start, and category. Effective intervals for one provider/model/category cannot overlap. Strings and entry counts are bounded. URLs must be HTTPS without credentials. Provenance says which reviewed artifact supplied the rates; it does not prove provider billing or authorize runtime retrieval.

The initial production table must arrive as a separately reviewed data change with exact provider documentation evidence and effective dates. This plan asserts no current rate. Tests use clearly synthetic prices and domains. The binary contains an append-only set of accepted table versions; changing a rate creates a new table/version rather than editing an old one.

When a table is first used by a store, persist the complete canonical table under `pricing_tables[table_id]` in the local index. This keeps historical projection reproducible after application upgrades or catalog retirement. Reject a different body for an existing ID. A later mutation may add a new table but never rewrites or removes a referenced table. Store validation rehashes every table. Unused bundled tables need not be copied into a user's store.

Price applicability uses the timestamps on the baseline and final native usage counter records. V1 chooses one table only when a single price entry covers that complete observed-delta interval. It does not claim when the excluded prior counters accrued. If either endpoint is missing/invalid, attribution continuity is incomplete, or the interval crosses a rate boundary, return `price_window_unresolved`. A broader snapshot timestamp range cannot repair a missing usage-counter endpoint. Never substitute `analyzed_at`, table retrieval time, or the current date. Claude per-message pricing likewise needs each usage segment bound to a valid message timestamp before historical pricing is eligible.

This conservative rule means some complete usage remains unpriced. It is preferable to silently applying today's rate to historical or mixed-window tokens.

## Deterministic monetary arithmetic

Do not use `f32`/`f64`, decimal strings, or locale-formatted values in contracts or calculation. Store rates as integer USD nanos per one million tokens and calculate with checked `u128` intermediates:

1. Decompose Codex input into `uncached_input = input.checked_sub(cached_input)`. Bill cached input at its own rate. Bill output once; reasoning output is coverage information and is not an additional category in schema 1.
2. For Claude, bill ordinary input, cache-read input, cache-creation input, and output separately. Never subtract one category from another.
3. For each category retain the exact rational numerator `tokens * usd_nanos_per_million_tokens` over denominator `1_000_000`. The wire breakdown carries the integer tokens and rate from which shared Rust code reconstructs this exact fraction; it does not serialize `u128` or independently round categories.
4. Sum numerators with checked `u128`. Round once at the final card boundary to USD micros using a specified half-away-from-zero rule. Values are nonnegative, so ties round upward. Return unavailable on multiplication, addition, or narrowing overflow. Because only the total is rounded, displayed category detail must use shared text derived from the exact fractions rather than claim rounded category amounts sum to the total.

The protocol result should carry `estimated_cost_usd_micros: u64`, the pricing table ID/version, and a category breakdown containing tokens and rate for exact reconstruction and evidence display. The denominator is fixed by the schema, and intermediate `u128` values stay inside the calculator so native JSON decoders do not need a new integer representation. Shared presentation formats micros and category detail; shells do not multiply, sum, round, or convert currency. Complete zero usage under any applicable rates is a known zero. A nonzero exact estimate smaller than half a microdollar can also round to zero, so the presentation must distinguish “less than $0.000001” from a zero-token estimate rather than displaying both as an unexplained `$0.00`.

Call the result `deterministic estimate using table …`. Keep `actual_billed_cost` absent. Do not compare it with provider invoices without a separately designed reconciliation contract for service tiers, cache-duration modifiers, batch discounts, taxes, credits, and provider-specific rounding.

## Card request and projection changes

Extend `SnapshotCardInput` with optional validated usage facts and extend the request with the exact persisted pricing table selected by the host. The one-lock resolver reads snapshots, episodes, usage, timestamps, and stored pricing tables from one validated index image. It performs no file reread and no catalog/network refresh.

The card input digest must cover:

- usage schema/source/scope, exact source digest, counts, coverage, coordinates, attribution, references, and unavailable reason;
- the full canonical pricing table ID/version/content and provenance;
- the selected price-entry identities and effective intervals;
- the existing snapshot, model, time, episode, and assessment facts.

Update `expected_cost_card` to return:

- `observed` only when every selected snapshot with eligible native usage has complete usage, complete attribution, one applicable price entry per billable category, compatible USD currency, and checked arithmetic;
- `partial` when at least one selected snapshot is priced and another is explicitly unavailable;
- `unavailable` when none can be priced or the empty selection contains no evidence.

Coverage uses saved snapshots as the primary denominator and adds explicit usage records and attributed usage records where useful. Fixed missing reasons distinguish unsupported source, legacy/not persisted, invalid/incomplete usage, unattributed or mixed model, missing model price, unresolved effective window, currency unsupported, and arithmetic overflow. The card lists contributing snapshot evidence and the applied table ID. It never silently drops an unpriced selected snapshot.

Episodes add their deterministic resolved member union exactly as today. Duplicate snapshots shared by selected episodes are priced once because the card input contains a canonical snapshot union. Episode outcomes and estimated cost remain separate cards; the projector does not claim an episode, outcome, or model caused a cost.

## Service, CLI, FFI, and native shells

Keep `question_cards` as the read operation. The host selects the newest locally accepted pricing table only if its effective-window rule applies; callers cannot inject rates through the service request. Add an optional explicit `pricing_table_id` later only if users need historical what-if views, and validate it against the local immutable store.

The CLI prints the shared structured cost row, coverage, table identity, provenance summary, and missing reasons. Add a read-only `insights pricing-list` command only when more than one reviewed table ships; it must not download or edit tables.

The FFI needs no new pointer API because the bounded JSON operation already transports cards. Update response-size fixtures and header lifecycle tests for the larger cost card. Errors remain fixed labels and never include paths, model text, URLs, or source content.

macOS, Windows, and GTK decode the new integer value/breakdown types and shared copy. Shells format the supplied micros value for locale, show estimate and table provenance, and navigate only to returned evidence IDs. They do no token arithmetic, model allocation, table selection, or fallback pricing. Existing presentation-generation guards clear cards on snapshot, episode, usage, pricing, refresh, conflict reconciliation, cancellation, and disposal.

## Smallest coherent implementation sequence

Each step is a focused PR. Do not expose a numeric cost until step 4 is complete; that avoids shipping persisted data that appears price-ready but cannot produce a defensible answer.

1. **Source-bound usage evidence and schema 6.** Refactor the existing extractor to produce validated persisted evidence while preserving the ephemeral summary response. Extract Codex usage from the same bytes in `analyze_file`; store trajectory as unsupported/absent; add legacy 1–5 migration, alias, reimport, deletion, corruption, overflow, zero, reset, and mixed-model tests. This step includes the strict single-model Codex attribution gate.
2. **Immutable pricing contract and synthetic calculator.** Add protocol table/rate/result types, canonical digest validation, effective intervals, checked rational arithmetic, and synthetic fixtures. Add contributor catalog loading and store-on-first-use persistence, but ship no production rate and keep the cost card unavailable.
3. **Reviewed pricing data and applicability qualification.** Land one separately reviewed append-only pricing table with source artifact digest, effective dates, and explicit coverage of every billable category for the eligible Codex model(s). Qualify real, non-secret Codex fixtures for cumulative semantics, model completeness, and timestamp-window binding. If no table covers the full interval or the fixture exposes ambiguous attribution, stop here and keep cost unavailable.
4. **Cost projection and shared presentation.** Extend the card input/digest, one-lock resolver, complete deterministic projector, missingness dictionary, service/CLI/FFI fixtures, and shared copy. Enable a number only for cases that pass every gate above. Include exact request/result JSON fixtures and mutation/digest stability tests.
5. **All-shell presentation and release gates.** Add typed integer cost/breakdown decoding, applied-table provenance, partial/unavailable states, evidence navigation, and stale-result invalidation on macOS, Windows, and GTK. Run each native build and runtime suite against the same FFI artifact; platform-specific compilation remains a separate gate.
6. **Claude saved-source follow-on.** First add and qualify a saved Claude Code analysis adapter. Then persist same-byte per-message usage and timestamps, prove deduplication and model grouping, add reviewed Claude price categories, and extend the projector. Do not attach the current standalone Claude usage result to a Codex or trajectory snapshot.

Steps 1–4 are the minimum coherent backend delivery. Step 3 is deliberately a gate rather than documentation theater: without reviewed applicable rate evidence and a qualified attributable fixture, step 4 must continue returning unavailable.

## Required tests and reproducibility evidence

Protocol and contributor tests must cover:

- Codex cumulative snapshots, repeats, resets, multiple session IDs, cached-input and reasoning subset semantics, timestamped baseline/final deltas, nonzero excluded prior counters, checked subtraction, model switches before/within the priced interval, missing/invalid/omitted declarations, omitted supporting references, known zero, sub-micro nonzero estimates, and every overflow edge;
- Claude message deduplication, regressing/conflicting duplicate IDs, separate cache categories, per-model grouping, missing model/timestamp, and sum equality, while proving it is not persistable before a saved Claude adapter exists;
- exact same-byte alias/reimport stability; changed-byte digest replacement; mutation upgrade from store versions 1–5 preserving annotations, models, timestamps, links, outcomes, and episodes; deletion invalidation; malformed usage/table data failing closed without panic;
- pricing canonical order/digest, duplicate or overlapping intervals, boundary instants, gaps, a snapshot crossing a price boundary, missing category, unknown model, unsupported currency, table-body substitution, and old-table replay after the bundled catalog advances;
- integer golden vectors at zero, one-token fractions, half-micro ties, maximum accepted counts/rates, category aggregation, and overflow. Compare exact serialized integers and text; do not use approximate float assertions;
- card unknown versus zero, partial selection, mixed Codex models, unsupported trajectory, legacy usage unknown, duplicate episode-member union, input-digest change on usage/rate/effective-window changes, and digest stability on annotation or other irrelevant no-op changes;
- CLI JSON/text parity, bounded FFI responses and fixed errors, plus macOS, Windows, and GTK decoding/rendering from the same synthetic card fixture.

Reproducibility evidence for a numeric card consists of snapshot ID and source digest, usage schema and accounting source, attribution state, usage counts and references, pricing table ID/version/content digest and provenance, applied entry/effective interval, calculator version, input digest, and integer category breakdown. Source bodies, paths, message/session IDs, and billing credentials remain absent.

## Release refusal conditions

Keep estimated cost unavailable if any of these remain true: source usage is unsupported; the snapshot predates persisted usage; native counters are missing, invalid, reset, multi-session, or overflowed; model attribution is mixed/incomplete; a source record or model label was omitted; timestamps cannot prove one applicable price interval; a model/category price is missing; currencies differ; table provenance or digest is invalid; arithmetic overflows; or selected evidence changed during projection.

Passing synthetic tests does not qualify real provider prices or live source variants. Passing shared Rust tests does not establish macOS, Windows, or GTK readiness. A reviewed table establishes the deterministic estimate's inputs, not billed accuracy or permission to spend.
