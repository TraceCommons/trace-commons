# Qualifying real prices for saved Insights

Evidence reviewed: 2026-09-12 06:30 UTC.
Status: Source audit and implementation requirements. No production rates are accepted by this document.

## Findings from official sources

The current [OpenAI pricing page](https://developers.openai.com/api/docs/pricing) distinguishes ordinary input, cached input, cache writes, and output. It also separates context bands and processing modes, and describes regional processing adjustments. The [Astra model page](https://developers.openai.com/api/docs/models/gpt-6-astra) describes a per-request long-context threshold and Batch, Flex, and Fast adjustments. The [Sol model page](https://developers.openai.com/api/docs/models/gpt-5.6-sol) describes its own context adjustment, cache-write charge, and a promotional rate period.

These are observations of documentation available at the review time, not proof of historical applicability or what a particular account paid. The public tables do not establish the endpoint, processing mode, cache-write quantities, request sizes, region, or contractual terms of an imported rollout.

## Consequence for the current implementation

`NativeTokenCounts::Codex` records cumulative ordinary input, cached input, output, reasoning output, and total tokens. It does not separately retain cache-write quantities, individual request input lengths, effective processing tier, or region. The persisted baseline-to-final delta also does not retain those missing dimensions. In particular, a cumulative input count above a context threshold does not prove that any individual request crossed it.

The schema-1 pricing contract and catalog are correct for their explicit synthetic inputs: one qualified provider/model, one complete category set, one applicable interval, and one fixed rate per category. They do not encode every condition of the currently documented real prices. Adding those base rates as production schema-1 entries would silently assume missing facts. Keep the production catalog empty until a specific real model and source fixture satisfy all applicable conditions.

Provider identity is a separate requirement. Codex's configured provider label can remain unchanged with a custom endpoint. An adapter name, model slug, or unverified configuration label must not select a provider's rate table automatically. See the provider and rate-context section in the [persisted usage plan](2026-09-11-insights-persisted-usage-pricing.md).

## Required implementation sequence

1. Qualify a bounded per-request usage source. Identify stable request/counter relationships, complete disjoint cache categories, actual requested/effective processing modes, endpoint/provider evidence, request context size, and any applicable regional context. Preserve source coordinates and digest binding while omitting request IDs and transcript bodies from stored evidence. Missing fields remain typed unknowns, never assumed standard mode or zero cache writes.
2. Version the usage and pricing contracts where their semantics change. Represent applicable context bands and processing modes explicitly, with non-overlapping selection rules and exact provider/model identity. Do not overload timestamps or model labels with rate conditions. Preserve schema-1 cache readability without inventing newly required evidence.
3. Use a separate immutable catalog release with a dated source record, applicable interval, and review identity. Record unresolved or conflicting source conditions; do not choose the cheapest interpretation. Retrieval time alone does not establish a historical effective start or indefinite future validity.
4. Calculate each qualified request under its applicable conditions using checked integer arithmetic. Preserve one-rounding semantics for the reported aggregate and retain exact reconstruction inputs. Count incomplete or unpriced selected requests in coverage, and keep unknown quantities distinct from a known zero.
5. Extend the shared cost card and all-shell fixtures only after a complete source fixture and reviewed table qualify together. Show the estimate's scope, applied table, missingness, and rate assumptions. Keep actual billed cost absent unless a separate reconciliation contract exists.

A separate, explicitly selected hypothetical rate calculation could still be useful without observed provider billing context. It must identify the user's rate assumption and must not silently populate the automatic estimated-cost card. This document does not change the current service to accept arbitrary rates.

## Acceptance cases before real rates ship

Cover cache-write quantities that differ from uncached reads, multiple small requests whose cumulative total exceeds a context threshold, a single request crossing that threshold, mixed processing modes, regional ambiguity, provider-label/endpoint mismatch, missing request identity, replayed counters, a rate change during the selected interval, historical rates unavailable at retrieval, and complete known-zero usage. Include unsupported and partial selections and prove that neither case becomes a cheaper model claim.

The mission inbox and descriptive Insights work continue independently. This audit does not reduce the program's remaining goals: numeric estimates, defensible comparisons, prompting feedback, multiple providers, and mission participation and results remain required work.
