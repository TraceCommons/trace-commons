# Novelty floor calibration

Date: 2026-09-21
Status: **applied to the pilot 2026-09-21 04:58Z as a holding value.** The
running process reports `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=100000`;
the previous env file is kept beside it as a timestamped
`bak-novelty-floor` backup. Health and `/v1/source` returned 200 after the
restart and no errors were logged. The pilot received two submissions on
2026-09-20 and none on 2026-09-21, so **no trace had been scored under the
new floor when this was written**: its effect is calibrated here, not yet
observed. The same is true of the perplexity floor changed on 2026-09-19.

Floor: **100,000 micros (0.10).** Previous: 500,000 (0.5).

## What was wrong

**The 0.5 floor was never calibrated.** `docs/operator/calibration.md`
records it as "Unchanged from A2 deployment guidance ... primary active gate
at launch". The binary refuses to start unless at least one of the three
gate floors is positive, and the perplexity and tail-fraction floors shipped
at 0, so novelty carried that invariant. It was not derived from the
production embedder's similarity distribution.

**It is outside the embedder's range.** Novelty is
`1 - max cosine similarity` against the tenant's index, and the embedder is
`BAAI/bge-large-en-v1.5`. Its model card says: "the similarity distribution
of the current BGE model is about in the interval [0.6, 1]. So a similarity
score greater than 0.5 does not indicate that the two sentences are
similar", and that a filter threshold should be chosen from the similarity
distribution of one's own data, "such as 0.8, 0.85, or even 0.9". A novelty
floor of 0.5 asks for a nearest neighbour with similarity under 0.5, which
this model does not produce for two English texts. Two choices in the gate
push novelty lower still, though neither was measured here: chunk embeddings
are mean-pooled over sub-windows, and novelty takes the maximum similarity
over the top-k neighbours in the tenant's whole index.

**It froze the index.** A trace's chunks are inserted only
`if perplexity_passed && novelty_passed` (`orchestrator.rs`). With novelty
refusing nearly everything, almost nothing has entered the index since
mid-August, so the corpus that defines "duplicate" stopped growing and
novelty has since measured distance to a mid-August corpus.

**It held credit quality down.** `credit_quality` pins its novelty floor to
the gate's, so `g` has been the 0.30 graded multiplier for nearly every
trace. That is a larger effect on `q` than the perplexity term.

Weekly, scored rows only:

| week of | scored | novelty passed | novelty p50 | novelty p95 |
|---|---|---|---|---|
| 2026-07-06 | 348 | 82.5% | 0.050 | 0.214 |
| 2026-08-17 | 105 | 10.5% | 0.231 | 1.000 |
| 2026-08-24 | 246 | 1.2% | 0.188 | 0.312 |
| 2026-08-31 | 45 | 4.4% | 0.193 | 0.309 |
| 2026-09-07 | 72 | 2.8% | 0.245 | 0.320 |
| 2026-09-14 | 235 | 1.3% | 0.156 | 0.263 |

The week of 2026-07-06 passed 82.5% with a median novelty of 0.05, which is
only possible under a much lower floor, or none, at the time. What the floor
was then has not been established. Rows with novelty exactly 1.0, which an
empty index produces, were rare throughout (6 of 348 that week, 11 of 105 in
the week of 08-17), so the index being held only in process memory before
#406 (2026-08-24) explains little of the earlier pass rate.

## Calibration against an independent duplicate label

598 rows scored since 2026-08-24, labeled by the simhash dedup cluster,
which is computed by a different method from novelty. Read through the
column-scoped `trace_gate_driver` role, aggregates only.

| simhash label | n | p05 | p25 | p50 | p75 | p95 |
|---|---|---|---|---|---|---|
| near-duplicate (cluster > 1) | 515 | 0.131 | 0.152 | 0.173 | 0.211 | 0.305 |
| unique (cluster of 1) | 41 | 0.156 | 0.194 | 0.232 | 0.274 | 0.316 |
| unclustered | 42 | 0.154 | 0.183 | 0.193 | 0.205 | 0.297 |

Share each candidate floor would refuse:

| floor | 0.02 | 0.05 | 0.08 | 0.10 | 0.12 | 0.15 | 0.20 | 0.30 | 0.50 |
|---|---|---|---|---|---|---|---|---|---|
| unique refused | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 0.0% | 26.8% | 85.4% | 97.6% |
| near-duplicate refused | 0.0% | 0.0% | 0.0% | 0.0% | 1.9% | 23.7% | 70.1% | 93.8% | 98.4% |

At 0.5 the floor does not discriminate: it refuses 97.6% of unique traces
and 98.4% of near-duplicates. The best single cut in this data is 0.15,
which refuses no unique trace and 23.7% of near-duplicates, and agrees with
the model card's 0.85 similarity.

## Why 0.10 and not 0.15

The distribution above was measured against a frozen index. Near-duplicates
do not score near 0 because their earlier copies never entered the index;
once the floor stops freezing it, new traces will be compared with recent
ones from the same contributors and repositories, nearest-neighbour
similarity will rise, and novelty will fall. A floor fitted to today's
distribution could then refuse far more than intended. 0.10 refuses nothing
in the current data and corresponds to similarity of at least 0.90, the
strict end of the model card's range: below it a trace is close to
identical to something already indexed. It is a holding value until the
index has filled.

## Limits

- 41 unique traces is a small class.
- 86% of scored rows sit in a simhash cluster larger than 1. Either
  contributors are resubmitting heavily or the simhash is coarse, for
  example clustering on shared system prompts. Which one is not known, and
  the label is only as good as the answer.
- Novelty separates the two labels weakly even at its best cut.
- Every figure here describes distance to a mid-August corpus.

## Follow-ups

- **Recalibrate after one to two weeks of a filling index**, against the
  simhash label, once novelty can see recent duplicates. Examine the simhash
  label at the same time.
- **Credit-quality novelty constants** (floor and ceiling) should follow
  that recalibration as a new era in the calibration schedule, not be fitted
  now to a distribution that is about to move.
- **Watch the host.** Nearly every trace now enters the vector index, so
  index growth and embedding work resume on a host previously found to be
  CPU-bound by the embedder. At the time of the change: load average 0.02,
  index 304K on disk.
- **Traces kept out of the index since mid-August.** Whether to replay them
  into it, so novelty can see them, has not been decided.
