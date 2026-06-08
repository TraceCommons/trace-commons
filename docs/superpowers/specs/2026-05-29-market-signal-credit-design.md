# Market-Signal-Driven Credit Distribution — Design

**How buyer demand for specific tools and data patterns flows into the
contributor credit ledger.**

Status: design / hypothesis. Nothing here is built. This document combines
(a) a market-pricing hypothesis grounded in 2026 comparables research and
(b) a concrete mechanism for routing that demand into the existing two-stage
credit pipeline.

## Problem

Today every reward weight in the credit system is a **fixed constant or a
single deployment-wide env var**:

- Online (`compute_value_scorecard`, `trace_contribution.rs`): `coverage_bonus
  = (required_tools.len() / 5.0).clamp(0,1)`, weighted `0.15`. Rewards tool
  *breadth*, not specific tools.
- Delayed (`trace-commons-ingest.rs`): `BENCHMARK_CONVERSION_CREDIT_POINTS_DELTA
  = 2.0`, `RANKER_TRAINING_CANDIDATE_CREDIT_POINTS_DELTA = 0.5`,
  `RANKER_TRAINING_PAIR_CREDIT_POINTS_DELTA = 0.75`,
  `DEFAULT_NOVELTY_UTILITY_CREDIT_POINTS_DELTA = 1.0` (env-tunable).

There is **no per-tool, per-category, or demand-driven weight table.** When a
buyer (e.g. Illia's "small neo lab") says "we pay a premium for browser-tool
traces with verification failures," the system has no way to route that premium
to the contributors whose traces carried that signal.

This is the gap the design fills. It is the bridge between the external pricing
question ("how should we price?") and the internal distribution question ("how
does a buyer's premium reach a contributor?").

## What the market says (pricing hypothesis)

Source: deep-research synthesis, 2026-05-29 (22 confirmed claims, 3 refuted;
full report archived in the research run). **Every dollar figure below is
inferred from adjacent markets — no verified source prices finished,
privacy-scrubbed agentic execution traces as a standalone SKU.** Treat as a
starting hypothesis to validate against real buyer conversations.

### Findings that shape the mechanism

1. **No settled per-trace price exists.** The closest comparables are
   expert-labor marketplaces (Mercor, Surge, Scale) selling *human time*, and
   content-licensing deals (Reddit/News Corp) pricing *bulk corpus access*.
   Neither is our exact SKU. (confidence: high)
2. **Labor cost sets a supply-side floor.** Blended expert rates ~$85/hr
   (Mercor, vendor-self-reported), domain specialists $200–300/hr, expert RLHF
   examples ~$50–100 each, commodity image labels $0.02–0.10. A single
   multi-step, expert-quality agentic trace embodies *tens of dollars* of
   labor-equivalent cost. (confidence: high)
3. **Rarity/specialization is a steep multiplicative premium.** CPAs at $300/hr
   vs $85/hr generalist (~3.5×); task complexity is the primary cost multiplier
   across all vendors. Direct analog for rare-tool / failure / domain-specialist
   traces. (confidence: high)
4. **Agentic/tool-use trace data is a real, rising category** that OpenAI,
   Anthropic, and Meta are actively buying — framed as "the next battleground."
   (confidence: high)
5. **Winning vendors use a hybrid model:** per-unit/per-task base **plus**
   managed-bundle contracts, adjusted for complexity/domain/speed. Enterprise
   contracts cluster $93K–$400K+/yr; flat-rate corpus licensing reached
   $5M–60M/yr for large holders. (confidence: high / medium)
6. **The correct ceiling is value-based** — data is worth the marginal change in
   the buyer's model performance, not its production cost. But value-based
   pricing needs benchmark-lift attribution that neither we nor buyers can yet
   measure cleanly, so quality-weighting must initially ride on **proxy
   signals** (tool rarity, failure, domain, perplexity-novelty). (confidence:
   high)

### Recommended pricing model (the buyer-facing layer)

A **four-tier hybrid**, mirroring Surge's "per-task + managed contract" and
Mercor's domain-segmented agentic datasets:

| Tier | Unit | Hypothesis range | Who |
|---|---|---|---|
| Commodity | per accepted trace | ~$1–10 | self-serve / neo labs |
| Premium (quality-weighted) | per trace × multiplier | ~$25–150+ | rare-tool / failure / domain traces |
| Curated bucket | managed contract | five-to-six figures | frontier labs wanting a domain set (cf. Mercor ~2,000-case sets) |
| Exclusivity / usage-rights | flat-rate license | benchmarked vs $5M–60M/yr precedent | large frontier-lab programs |

The **premium tier is the one that drives this design**: it requires a
quality/demand multiplier that the credit ledger must be able to express and
attribute.

## The substrate: how credit works today

Credit accrues in two stages — and that shape is exactly what makes market
signals injectable.

### Stage 1 — online credit (at submission)

`compute_value_scorecard` → `credit_points_pending`. Local heuristics only:
quality, replayability, novelty, `coverage_bonus`, difficulty, correction
value; privacy-gated. The contributor's instant estimate. Explainable and
stable by contract ("initial score uses local heuristics only").

### Stage 2 — delayed/utility credit (after the fact)

Idempotent, external-ref'd, evidence-hashed `points_delta` events appended as
downstream jobs learn a trace's real worth. Existing event types
(`TraceCreditLedgerEventType`):

- `NoveltyUtility` — gate-minted (no operator API), env-tunable delta.
- `BenchmarkConversion` — trace became a benchmark case.
- `RankingUtility` — candidate / pair.
- `TrainingUtility` — set by evaluator jobs.
- `RegressionCatch`, `ReviewerBonus`, `AbusePenalty`.

Each delayed event carries a `utility_category`
(`ModelTraining`/`RankingTraining`/`Evaluation`/`Regression`), a `use_category`,
an `evidence_hash`, and source submission IDs. Bounded by
`MAX_DELAYED_CREDIT_POINTS_DELTA = 100`. Settlement then batches points into
non-transferable on-chain NEAR credit, under per-account caps.

The signals a buyer cares about are **already on the trace**: `tool_category` /
`tool_categories`, `coverage_tags`, `failure_modes`, `cartography_bucket`,
perplexity-novelty. What's missing is the *table that prices them*.

## Design: the Demand Schedule

Introduce a first-class, versioned, tenant-scoped **demand schedule** — a weight
table that maps trace signals to credit multipliers, populated from buyer
contracts. It is data, not constants.

### Shape

```
DemandSchedule {
  schedule_version: String,        // e.g. "demand-2026-06-01"
  policy_version: String,          // ties to consent/allowed-use policy
  entries: [DemandEntry],
  created_at, ...
}

DemandEntry {
  // match predicate — all present fields must match (AND)
  tool_category:      Option<String>,
  failure_mode:       Option<TraceFailureMode>,
  cartography_bucket: Option<CartographyBucket>,
  coverage_tag:       Option<String>,
  allowed_use:        Option<TraceAllowedUse>,   // which buyer use this priced
  // the price
  multiplier:         f32,          // bounded, e.g. 0.5 ..= 5.0
  // provenance — hash-only, never the buyer's identity or contract terms
  demand_ref_hash:    String,       // sha256 of the buyer contract ref
  reason_label:       String,       // safe label, e.g. "browser+verification-failure premium"
}
```

### Three injection points (ranked by fit)

**1. Demand-weighted delayed credit (primary — recommended first build).**
Replace the flat `points_delta` constants in the
`TrainingUtility` / `RankingUtility` / `BenchmarkConversion` paths with
`base_delta × demand_multiplier(trace_signals, schedule)`. When a buyer
contracts for "browser-tool traces with verification failures," that becomes a
`DemandEntry`, and the existing delayed-credit jobs multiply by it when they
fire. This is the cleanest path: the delayed lane is *already* "pay after we
know the use," and the use is now "a buyer paid for this shape." Buyer-driven
premiums appear as transparent ledger events with their own
`reason`/`evidence_hash`.

**2. Demand-weighted online coverage/novelty (secondary — supply steering).**
The `UnderrepresentedCoverage` and `NovelCluster` credit-event kinds already
exist as scaffolding. Feed them from a live scarcity signal: if buyers are
paying for a tool category that's rare in the corpus, raise its effective
`coverage_bonus` weight so contributors are steered toward producing it in
near-real-time. This is the "supply responds to demand" loop. **Caveat:** this
touches the online estimate, which is contractually "local heuristics only" —
so any buyer-driven online weighting must be (a) clearly disclosed and (b)
ideally limited to corpus-scarcity signals rather than per-buyer terms, to keep
the instant estimate explainable.

**3. Settlement-time clearing price (the economic close).** Keep online +
delayed credit as *relative* quality/effort points; let actual buyer revenue
set the conversion rate at settlement. A trace's final NEAR credit = its
accumulated points × (revenue realized for the bucket it sold into ÷ total
points in that bucket). The market signal enters as the literal clearing price
per allowed-use bucket. This is where Illia's "per-unit pricing" and the credit
system reconcile, and it is the most faithful implementation of the
value-based-pricing ceiling — the buyer's payment, not our cost estimate, sets
what a point is worth.

### Design invariants (match existing codebase conventions)

- **Tenant-scoped, versioned, hash-audited** — exactly like
  `TRACE_COMMONS_TENANT_POLICIES` and the ranking calibration registry. A
  demand schedule is a first-class, auditable config object with a
  `schedule_version`; credit is always reconstructable.
- **Hash-only provenance** — store buyer demand as `demand_ref_hash` +
  `reason_label`, never the buyer's identity, contract value, or terms. Honors
  the repo's hash-only audit rule.
- **Buyer-specific premiums ride the delayed lane, not the online lane** — so
  the contributor's instant estimate stays stable and explainable, and premiums
  show up as transparent `points_delta` events. Preserves the "credit can change
  after downstream scoring" contract contributors already see.
- **Bounded** — demand multipliers clamp (e.g. `0.5..=5.0`), and the resulting
  delta still clamps under `MAX_DELAYED_CREDIT_POINTS_DELTA` and per-account
  settlement caps. A single hot tool cannot distort the ledger.
- **Fail-closed / attribution-not-authority** — a missing or unmatched schedule
  yields multiplier `1.0` (no change), never an error or a silent zero. Demand
  entries price credit; they never grant or gate access (consent-scope and
  allowed-use still do that).

## One-sentence summary (for Illia)

Tool/pattern demand becomes a versioned, tenant-scoped weight table that
multiplies the existing delayed-credit events — so when a buyer pays a premium
for a specific tool or failure mode, that premium flows to the contributors
whose traces carried that signal, transparently and on-chain, without changing
the instant-estimate contract.

## Open questions

These came directly out of the research and need real buyer input before build:

1. **Is anyone selling this exact SKU today, and at what rate?** All comparables
   are labor or corpus-licensing, not recorded traces. The per-trace ranges are
   inferred.
2. **What measured benchmark-lift multiplier do labs assign to failure/rare-tool
   traces vs common successful ones?** Without this, quality-weighting rides on
   proxies, not proven impact.
3. **How should consent-scope / allowed-use restrictions be priced as
   usage-rights tiers?** What premium does exclusivity or tool-specific
   allow-listing command?
4. **Does our perplexity-based novelty signal correlate with the rarity premiums
   labs will pay?** If so, it can serve as the quality-weighting mechanism in
   place of unavailable benchmark-lift attribution. (See project memory:
   perplexity AUC scales with model size — the 27B signal may be the usable
   one.)

## Build order (if approved)

1. `DemandSchedule` type + tenant-scoped storage + versioned/hash-audited admin
   route (mirror `tenant-policy` admin surface).
2. Injection point 1 — demand multiplier on the delayed-credit job paths
   (`TrainingUtility`/`RankingUtility`/`BenchmarkConversion`). Smallest, highest
   fit, fully behind the existing delayed lane.
3. Injection point 3 — settlement-time clearing price per allowed-use bucket.
   Larger; reconciles revenue to points.
4. Injection point 2 — online coverage/novelty steering. Last, because it
   touches the contractually-stable online estimate and needs disclosure care.

Each step is independently shippable and independently reversible.

## Relationship to other documents

- `docs/trace-spec.md` — defines the signals (`tool_category`, `failure_modes`,
  `coverage_tags`, `cartography_bucket`, allowed-uses) this design prices.
- `docs/trace-commons.md` — the delayed-credit / utility-attestation pipeline
  and settlement surface this design extends.
- `docs/superpowers/specs/2026-05-13-novelty-utility-credit-emission-design.md`
  — the existing gate-minted novelty-credit path that injection point 1
  generalizes.

---

## Verification Summary

All codebase claims in this document were checked against
`crates/trace-commons-server/` and `crates/trace-commons-protocol/` on
2026-05-29.

- **Claims checked:** ~20 codebase facts + 9 market findings.
- **Codebase claims confirmed (all):**
  - `coverage_bonus = (required_tools.len() / 5.0).clamp(0,1)`, weighted `0.15`
    — `trace_contribution.rs:872,903`.
  - Delayed-credit constants: `BENCHMARK_CONVERSION_CREDIT_POINTS_DELTA = 2.0`
    (`:12661`), `RANKER_TRAINING_CANDIDATE = 0.5` (`:12663`),
    `RANKER_TRAINING_PAIR = 0.75` (`:12664`),
    `DEFAULT_NOVELTY_UTILITY_CREDIT_POINTS_DELTA = 1.0` (`:215`, env-tunable),
    `MAX_DELAYED_CREDIT_POINTS_DELTA = 100.0` (`:12660`) — all in
    `bin/trace-commons-ingest.rs`.
  - `TraceCreditLedgerEventType` variants (BenchmarkConversion, RegressionCatch,
    TrainingUtility, RankingUtility, ReviewerBonus, NoveltyUtility,
    AbusePenalty) — `:12650`.
  - Novelty credit is gate-minted with no operator API surface — confirmed by
    the `is_utility_job_type` whitelist comment (`:12686-12695`).
  - `TraceRankingUtilityCategory` = ModelTraining/RankingTraining/Evaluation/
    Regression — `trace_corpus_storage.rs:211`.
  - Delayed events carry `utility_category` / `use_category` / `evidence_hash` /
    `source_submission_ids` — `TraceUtilityAttestationRequest`, `:12731`.
  - Trace signal fields exist: `tool_category` / `tool_categories`,
    `coverage_tags` (`trace_contribution.rs:329`), `failure_modes`,
    `cartography_bucket` (`:506`); `UnderrepresentedCoverage` / `NovelCluster`
    credit-event kinds (`:610-611`).
  - Referenced spec
    `2026-05-13-novelty-utility-credit-emission-design.md` exists.
- **Corrections made:** none — all code references matched source exactly.
- **Not codebase-verifiable (by design):** every dollar figure and the
  pricing-model recommendations originate from the 2026-05-29 deep-research
  synthesis (22 confirmed / 3 refuted claims from adjacent labor-marketplace and
  content-licensing markets), not from this repository. They are explicitly
  caveated in the doc as inferred, not observed, and the four open questions
  flag exactly what real buyer conversations must still validate. The
  `DemandSchedule` type and the three injection points are proposed design, not
  implemented code.
