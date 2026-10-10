# Compatibility mapping

This document records how current gate results map to the versioned
pipeline. It is the compatibility comparison input. Review it before you change
the Score adapter.

## Identities

| Input | Identity used by the local compatibility bundle |
|---|---|
| Admission and Review policies | `trace_commons.admission.authority_privacy.v1` and `trace_commons.review.authority_privacy.v1` |
| Scorer | `reference_perplexity.v1` (`ReferencePerplexityScorer`) |
| Embedder | `reference_embedder.v1` (`ReferenceEmbedder`) |
| Gate-path credit | `NoveltyUtility` flat delta from `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` (default 0) |
| Credit quality | Shadow only. Constants of the decision's era (`credit_quality::constants_at`) |
| Index | isolated `pipeline-test-index-v1` |
| Projection | `pipeline-test-projection-v1` |
| Corpus | `docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json` |
| Order | fixture order in that file |
| Initial index | empty |
| External payout | disabled |

The local bundle sets gate floors to zero. The reference scorer is not
calibrated against production models. A staging bundle must put the
deployed floors into the bundle configuration. Those values change the
bundle identifier.

## Legacy result to new decision

The current orchestrator returns one `OrchestrationDecision`. It also
inserts vectors while it scores. The new path splits that work.

| Legacy field or action | New record |
|---|---|
| `perplexity_passed` | Score evidence `quality_passed` |
| `novelty_passed` | Score evidence `novelty_passed` |
| `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros` | Score evidence measured values |
| `novelty_score_micros`, `peak_novelty_micros` | Score evidence measured values |
| `nearest_neighbor_hash` | Score evidence neighbor hash. Neighbor lists stay in an encrypted artifact. |
| `chunk_count`, `total_chunk_count`, `chunks_capped` | Score evidence coverage |
| `inserted_chunk_entries` not empty | Settle `Include` after Score commits |
| `inserted_chunk_entries` empty | Settle `Exclude` |
| insert during `evaluate` | Forbidden in Score. Settle writes a sealed command. |
| random `entry_id` | Deterministic index key: tenant, index, revision, projection, model, chunk |
| `NoveltyUtility` credit event when both floors pass | Score award of the configured delta for `trace_credit`. Settle records it as a `NoveltyUtility` ledger event, which does not settle. |
| credit quality `q_micros` | Score evidence `credit_quality_micros` and `credit_quality_version`. No award. |
| driver `skipped_duplicate` (duplicate score at or above `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS` while `..._SKIP_DUPLICATES`) | Not a shadow value: it decides what the contributor sees. The gate decision row records `credit_withheld_reason = skipped_duplicate` and no credit quality, and the `NoveltyUtility` leg is withheld under the same label. Score evidence is unchanged. |
| driver `cached` (an earlier submission with the same canonical summary hash has a decision) | As `skipped_duplicate`, under `cached`. |
| dedup penalty, contributor cap, `anomaly_withheld` | Shadow values on `main`. This contract does not store them. No award, and no effect on index membership. |
| Review before Score | Unchanged. A Score failure does not change a Review outcome. |

## Membership and credit rules

Score may query a read-only index. Score must not write.

Settle reads only the committed Score evidence. Settle must not query the
live index to decide membership. Settle must not repeat valuation.

Include the revision when all of these are true:

- `quality_passed`
- `novelty_passed`
- at least one chunk has novelty at or above `embed_insert_novelty_micros`

The compatibility Score award equals the gate-path credit on `main`. When
both gate floors pass, Score awards the configured `NoveltyUtility` delta to
the `trace_credit` instrument. The default delta is 0, so by default the award
set is empty. Settle records a positive award as a `NoveltyUtility` ledger
event. `main` does not settle that event type, so the pipeline must not settle
it either.

`q_micros` is a shadow value on `main`. Score evidence stores it as
`credit_quality_micros`, with `credit_quality_version` for the constants of
the decision's era, not `CREDIT_QUALITY_ACTIVE`. It makes no award.

The dedup penalty, the contributor cap, and `anomaly_withheld` are shadow
values on `main`. They make no award and do not change index membership.
This contract does not store them. The compatibility adapter adds fields
for them when it records them.

`main`'s perplexity-scoring driver has two duplicate short-circuits that are
not shadow values, because the contributor status reads them: a
`skipped_duplicate` or `cached` decision row shows 0.0 pending and "This trace
duplicates an earlier submission under your account and earns no separate
credit." A compatibility run applies both, in `main`'s order, when it writes
its gate decision row at Settle, with `main`'s knobs
(`TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES`, default true, and
`TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS`, default
900000), which ingest passes to the runtime as part of
`PipelineNoveltyUtilityChecks::duplicate_controls`, read whether or not the
driver itself is enabled, and refuses an assembly that does not hold:

- the Review commit records `canonical_summary_for_embedding` of the approved
  envelope and its hash on its current derived record (not on the submission
  row, whose `canonical_summary_hash` stays NULL);
- `skipped_duplicate` when `main`'s precheck duplicate score of that summary,
  against the derived records of the tenant's submissions received earlier,
  is at or above the threshold (an equal hash scores 1.0 and is checked
  first, without reading any summary);
- otherwise `cached` when a submission received earlier with the same
  canonical summary hash (on its submission row or a derived record) has a
  gate decision.

The verdict is decided once per run. With a Trace Credit leg (a positive
delta) it is decided by the leg's pre-dispatch check, before the leg can pay:
a duplicate is withheld under its label, and a leg that check let through is
never re-decided, so a neighbour's later progress cannot withhold a dispatched
leg or label a paid one. The gate decision row reads the leg. With no leg (a
zero delta) Settle decides it just before its commit and the commit records
it.

A duplicate row keeps the Score values and the outcome hash, records the label
as `credit_withheld_reason`, and holds no credit quality; the Trace Credit leg
is withheld under the same label, since `main` emits no `NoveltyUtility` event
for a submission it did not score. The Score evidence is unchanged.

Five differences from `main` follow:

1. Candidates are limited to submissions received earlier, so two runs
   settling in either order never withhold each other. `main`'s `cached` has
   no time bound.
2. A candidate counts only once its Review has committed, where `main` writes
   its derived record at submit, so an earlier duplicate still in quarantine
   is not seen.
3. Index membership is unchanged: `main` never puts a skipped duplicate into
   the vector index, and the pipeline still includes it.
4. The direction is one-way. A later legacy submission is never `cached`
   against an earlier pipeline one: `main`'s
   `find_gate_decision_by_canonical_hash` reads only
   `trace_submissions.canonical_summary_hash`, which stays NULL for a
   pipeline submission. Withdrawal tombstones, which read that column too,
   are unchanged.
5. Legacy readers of derived records (`list_trace_derived_records`, the
   reviewer metadata views, the ranker export's summary-hash dedupe) now see
   a pipeline submission's canonical summary and hash. The ranker exports,
   the benchmark export and process evaluation leave pipeline submissions
   out (#1185, L1-2), so their dedupe never sees one. The vector index worker reads only `duplicate_precheck` records,
   and the pipeline's are `summary` records, so it is unaffected.

An empty Score award set is a completed decision. It is not an incomplete
Score phase.

Inclusion does not require a positive Score. A positive Score does not
require inclusion.

## Required behavior changes

These differences are required by the proposal. They are not defects.

1. Score does not insert into an index.
2. Index keys are deterministic. They are not random UUIDs.
3. A later Score sees earlier traces only after Settle writes them.
4. Membership is fixed when Score commits. A later change to the live
   index does not change that decision.
5. Classifier-specific storage is not restored on this path.

## Shadow comparison

A shadow run uses a separate index namespace. It does not write to the
active index. It does not create a credit event.

## Privacy, rejection, and zero credit

Admission and Review stay on the authority and privacy policies. A terminal Admission
rejection has one outcome. A Review rejection has two. Score does not
run after those rejections. A clean fixture can receive a completed zero
or positive Score. The report must keep those states distinct.
