# Compatibility mapping

This document records how current gate results map to the versioned
pipeline. It is the Phase 5 comparison input. Review it before you change
the Score adapter.

## Identities

| Input | Identity used by the local compatibility bundle |
|---|---|
| Admission and Review policies | `trace_commons.admission.authority_privacy.v1` and `trace_commons.review.authority_privacy.v1` |
| Scorer | `reference_perplexity.v1` (`ReferencePerplexityScorer`) |
| Embedder | `reference_embedder.v1` (`ReferenceEmbedder`) |
| Credit quality | `CREDIT_QUALITY_ACTIVE` version 2 |
| Index | isolated `pipeline-test-index-v1` |
| Projection | `pipeline-test-projection-v1` |
| Corpus | `docs/redesign/fixtures/minimal-corpus-v1.json` |
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
| `chunk_count`, `chunks_capped` | Score evidence coverage |
| `inserted_chunk_entries` not empty | Settle `Include` after Score commits |
| `inserted_chunk_entries` empty | Settle `Exclude` |
| insert during `evaluate` | Forbidden in Score. Settle writes a sealed command. |
| random `entry_id` | Deterministic index key: tenant, index, revision, projection, model, chunk |
| credit quality `q_micros` | Score `credit_microcredits`, unless anomaly withhold sets zero |
| anomaly withhold | Zero credit and index exclusion |
| Review before Score | Unchanged. A Score failure does not change a Review outcome. |

## Membership and credit rules

Score may query a read-only index. Score must not write.

Settle reads only the committed Score evidence. Settle must not query the
live index to decide membership. Settle must not repeat valuation.

Include the revision when all of these are true:

- `quality_passed`
- `novelty_passed`
- anomaly withhold is false
- at least one chunk has novelty at or above `embed_insert_novelty_micros`

Award credit from `CREDIT_QUALITY_ACTIVE`. A zero Score outcome is a
completed decision. It is not an incomplete Score phase.

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

Admission and Review stay on the Phase 4 policies. A terminal Admission
rejection has one outcome. A Review rejection has two. Score does not
run after those rejections. A clean fixture can receive a completed zero
or positive Score. The report must keep those states distinct.
