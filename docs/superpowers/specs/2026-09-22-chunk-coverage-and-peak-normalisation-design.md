# Chunk Coverage in the Contributor Surfaces, and Peak Normalisation — Design

Date: 2026-09-22
Status: draft for review
Scope: `trace-commons-server` (`credit_quality.rs`, `trace_score_attestation.rs`,
`trace_corpus_storage.rs`, one storage query, the ingest binary), one
migration, one operator script, one collector-doc revision, one
contributor-CLI copy check. Three PRs, ordered. No change to the gate's
pass/fail, to the chunker, or to the cap.

## Problem

The gate scores a sample of a long trace and then applies a credit term
that grows with the size of the sample.

Measured 2026-09-21 on the pilot, read-only, as the narrow
`trace_gate_driver` role:

| statistic | value |
|---|---|
| scored traces at the 16-chunk cap, per week since mid-August | 63% to 82% |
| longest trace | 2,362 chunks, scored on 16 |
| capped traces since the strided selection landed (2026-08-23) | 467 |
| chunks those traces held / chunks scored | 74,260 / 7,472 (10.1%) |
| share of all submitted text that was scored | 11.4% |
| credit quality `q` under V2, uncapped traces | 0.097 |
| `q` under V2, capped, every coverage band from 50-99% down to under 10% | about 0.03, flat |
| median `peak / representative` perplexity, Qwen3.8 era | 8.18 capped, 2.93 uncapped |
| capped traces zeroed by V2's anomaly ramp (3..10), Qwen3.8 era | 99 of 244 |
| `q = 0` in the Qwen3.8 era, V2 then V3 | 115 then 5 |

Two things follow from the flat row. First, the cap is not what depresses
`q`: a trace sampled at 8% scores the same as one sampled at 90%, so the
strided sample estimates the token-weighted mean stably, as the
representative is defined to be
(`chunk_aggregate.rs::aggregate_chunked_perplexity`: `exp` of the
token-weighted mean negative log-likelihood over the scored chunks). The
step is between "uncapped" and "capped" — between short and long — not
between well-sampled and thinly-sampled.

Second, the step is the peak term. `credit_quality` computes
`q = f(ppl_rep) * g(nov_rep) * a(ppl_peak / ppl_rep)` where the peak is a
maximum over the scored chunks (`chunk_aggregate.rs`, line 83: the largest
per-chunk perplexity among chunks holding at least `min_chunk_tokens`). A
maximum over 16 samples is larger than a maximum over 3 for no reason to
do with the trace. V3 (PR #969, live on the pilot since
2026-09-21 21:26Z) moved the ramp from 3..10 to 40..310, which is why the
zeroed count fell from 115 to 5; it did not make a 16-sample maximum
comparable to a 2-sample one, and the V3 constant's own doc comment says
so: "normalising the ratio by chunk count is the real fix and is a change
to the function, not to a constant."

### What the code does today (established from the call graph, not the brief)

Four things the task brief assumed turn out differently in the code, and
the design depends on each.

1. **The score attestation already carries coverage, signed.**
   `trace_score_attestation.rs` stamps
   `SCORE_ATTESTATION_SCHEMA_VERSION = "trace_commons.score_attestation.v2"`,
   and every `ScoreAttestationSubmissionEntry` has a `coverage` field of
   type `ScoreAttestationCoverage`, on the wire as
   `{ coverage_state, chunks_scored, chunks_total? }` with the three
   states `complete`, `partial`, `partial_unknown_total`. It landed in
   PR #404 (2026-08-23, the same day as the strided selection, #403).
   `docs/collector-integration.md` step 5 documents it, and the
   contributor CLI (`commands.rs::partial_coverage_lines`) prints it. The
   brief's "NO coverage field" is not true of `main`. What is missing is
   narrower and is stated under "Coverage" below.
2. **Only peak perplexity enters `q`.** `peak_novelty_micros` is
   computed (`aggregate_chunked_novelty`) and stored (V37), but
   `credit_quality(ppl_rep, ppl_peak, nov_rep, k)` never reads it. The
   novelty term is the representative alone. The peak distortion is one
   term, not two.
3. **Per-chunk values are not stored.** A decision row holds the
   representative, the peak, `chunk_count`, `total_chunk_count`, and
   `chunks_capped`; the per-chunk perplexities that produced the peak are
   discarded after aggregation. Any correction that needs more than
   `(rep, peak, n)` cannot be calibrated on the stored corpus and cannot
   be applied by the batch pass. That rules out a quantile-of-chunks
   statistic for the correction shipped here (it stays as an escalation;
   see "Candidates").
4. **The narrow role reads less than the brief lists.** `trace_gate_driver`
   holds column grants (V45, V47, V48, V57) on `tenant_id`,
   `submission_id`, `decision_id`, `decided_at`, `perplexity_micros`,
   `peak_perplexity_micros`, `novelty_score_micros`, `perplexity_passed`,
   `novelty_passed`, `credit_quality_micros`, `chunk_count`,
   `total_chunk_count`, `chunks_capped`, and the dedup and correction
   columns. It cannot read `credit_quality_calibration_version`,
   `credit_quality_anomaly_ratio_micros`, or `peak_novelty_micros`. The
   measurement script therefore buckets eras by `decided_at` against the
   schedule boundary, exactly as the V3 report did, and recomputes the
   anomaly statistic from the three columns it can read rather than
   reading the stored one.

Also established:

- **The chunk-selection algorithm is not readable from a row.**
  `CHUNK_SELECTION_ALGORITHM = "stride_endpoint_inclusive.v1"` is folded
  into `gate_version_hash` (ingest binary, `gate_version_hash` builder
  near line 6204) and nowhere else. A row decided before #403 on a given
  deployment was scored on a prefix, a row after it on a spread sample,
  and nothing stored on the row distinguishes them; the deployment date
  is a per-host fact, not a repository one.
- **Two contributor-facing strings still say "prefix".** The status basis
  line (`gate_credit_basis_line`, ingest binary near line 58208) renders a
  capped decision with an unknown total as "over the first N chunks", and
  `docs/collector-integration.md` step 5 says a capped `gate_passed` "is
  then a judgment on a prefix". Both were true before #403 and are false
  for every decision since.
- **The calibration schedule is in place.** `CREDIT_QUALITY_SCHEDULE` maps
  `decided_at` to a constant set (`constants_at`); both the inline score
  (ingest binary near line 51220) and the batch pass
  (`score_credit_quality_one`, near line 52136) select by it. V3 shipped
  as an era starting at the scorer-model switch
  (`QWEN3_8_EFFECTIVE_FROM_UNIX`, 2026-09-09T05:49:19Z), ten days before
  the code landed, and the rows in between were moved from V2 to V3 by
  `POST /v1/admin/score-credit-quality`. That is the mechanism this spec
  reuses.
- **The batch pass already has what V4 needs, except one column in one
  query.** `list_gate_decisions_for_credit_scoring` (gate-driver pool)
  selects `perplexity_micros`, `peak_perplexity_micros`,
  `novelty_score_micros`, `decided_at`; the role's grant on
  `chunk_count` exists since V47. The write side
  (`update_trace_gate_decision_credit_quality`, tenant-scoped through the
  runtime pool) touches only the three `credit_quality_*` columns.
- `migrations/V74__public_run_function_acl.sql` exists on `main`; the
  migration in this spec is V75, and the number is re-checked when the
  PR is opened.

## Goal

1. Make the chunk-selection algorithm part of the signed coverage
   statement, so a collector or contributor can tell a sample spanning
   the trace from a prefix, and fix the two strings that say "prefix".
2. Replace the raw `peak / representative` ratio with a statistic whose
   distribution does not move with the number of scored chunks, so that
   the anomaly term penalises spiky traces and not long ones; calibrate
   it from the stored corpus without re-scoring; ship it as calibration
   era V4 through the schedule and the existing batch pass.
3. Give the operator one read-only script that prints the coverage and
   anomaly tables in this document, so a before and an after are
   directly comparable.
4. Decide the cap: leave it at 16, and say what would change that.

## Non-goals

- No change to `q`'s form beyond the anomaly statistic: `f`, `g`, the
  graded floors, the perplexity and novelty floors and ceilings, and
  `CREDIT_QUALITY_CONSTANTS_V3`'s values all stay. V3 remains defined.
- No credit scaled by coverage. Coverage is a statement about the
  evidence behind `q`, not a multiplier on it (see "What coverage does
  not do").
- No change to `chunk_cap`, `strided_selection_indices`,
  `CHUNK_SELECTION_ALGORITHM`, or `min_chunk_tokens`.
- No per-chunk persistence. It is named as the escalation and costed in
  the open questions; it is not in these PRs.
- No change to the dedup path. The simhash reads the full canonical text
  while the gate reads a sample; that asymmetry is real, is noted, and is
  the dedup spec's concern (`2026-09-21-dedup-simhash-recluster-design.md`).
- No embedding of unscored chunks. Section "The unscored 89%" states the
  experiment and stops.
- No settlement, ledger, or contributor-cap change.

## Design

### 1. Coverage

#### Definition

Coverage of a gate decision is `chunks_scored / chunks_total`, where:

- `chunks_scored` is `trace_gate_decisions.chunk_count`: the number of
  chunks the gate sent to the scorer, which is `min(total, chunk_cap)`
  by construction (`chunker.rs::finalize_plan`). NULL reads as 1 (V37).
- `chunks_total` is `total_chunk_count`: the number of chunks the
  chunker packed before the cap. NULL means the decision predates V47
  and the denominator was never recorded (V47's comment: readers MUST
  report that as unknown, never estimate it).
- When the envelope carried no parseable `events` array the chunker
  falls back to fixed character windows over the raw text
  (`chunk_envelope_plaintext`). "Total" is then the number of fixed
  windows. It is a count of the same kind — scoring windows of about
  2048 tokens — and coverage is still `chunk_count / total_chunk_count`;
  nothing on the row records which packing was used, and this spec does
  not add that, because the two packings have the same target size and
  the ratio means the same thing under both.
- `chunks_capped` NULL reads as false (V37). A row with `chunks_capped`
  true and `total_chunk_count` NULL is coverage-unknown, not coverage-1.

The wire and presentation rules for these three cases are already in
`ScoreAttestationCoverage::from_decision_columns` and are unchanged:
`complete` (all chunks scored, `chunks_total = chunks_scored`), `partial`
(capped, total known and strictly greater than scored),
`partial_unknown_total` (capped, total NULL or inconsistent). Coverage is
reported as the two counts, not as a fraction: a fraction is derivable,
and a derivable field inside a signed document is a place for two
statements to disagree.

#### What is added: `selection`

The missing fact is which chunks. A collector discounting a `partial`
entry "by its ratio", as `collector-integration.md` suggests, is applying
a prefix-era policy to a spread sample; a contributor reading "over the
first 16 chunks" is being told something false. So:

**Schema v3.** `SCORE_ATTESTATION_SCHEMA_VERSION` becomes
`trace_commons.score_attestation.v3`. `CoverageWire` gains a mandatory
`selection` string with exactly three values:

| `selection` | when | meaning |
|---|---|---|
| `all` | `coverage_state = complete` | every chunk was scored; selection is moot |
| `stride_endpoint_inclusive.v1` | capped, and the row carries that stamp | the scored chunks are at positions `round(j * (total - 1) / (scored - 1))`, so the first and last chunks are always among them and the rest are evenly spread (`chunker.rs::strided_selection_indices`) |
| `unknown` | capped, and the row carries no stamp | recorded before the stamp existed; on the pilot that is every capped decision before this migration, and whether the 16 were a prefix or a spread depends on the deployment date, which the document does not know |

`selection` is present on every entry in a v3 document. Verifiers treat
it as they treat `coverage_state`: a v3 verifier rejects an entry whose
`selection` it does not recognise rather than defaulting it, and a new
selection algorithm (a bump of `CHUNK_SELECTION_ALGORITHM`) is a new
schema version, not a new value slipped into v3. This follows the rule
already written in `collector-integration.md` step 4: additive top-level
fields need no bump, but "it does not extend to new fields inside
`submissions` entries or inside `coverage`, where a silently ignored
field could change what an existing field means". `selection` is exactly
such a field — it changes what `chunks_scored` of `chunks_total` means —
so it bumps. A verifier pinned to v2 rejects v3 until updated; that is
intended, as it was for v1 to v2, and `collector-integration.md` gets a
"Migrating from v2" paragraph beside the existing "Migrating from v1".

The contributor CLI's `partial_coverage_lines` reads `coverage` leniently
by key and does not pin the version string; it keeps working unchanged,
and its copy ("the rest was not read") is true under either selection.
One test is added to pin that a v3 entry with `selection` still renders.

**Where the stamp comes from: migration V75.** A nullable TEXT column
`trace_gate_decisions.chunk_selection`, following V57's pattern exactly:
no default, no backfill, NULL means "recorded before the stamp". The
inline path writes `CHUNK_SELECTION_ALGORITHM` into it on every decision
row it inserts (capped or not; the invariant "every row from a binary at
or after V75 carries the stamp" is simpler to test than a conditional
one). The same migration grants `SELECT (chunk_selection)` to
`trace_gate_driver`, as V47 did for the chunk columns, so the
measurement script and any later batch pass can read it. RLS is
untouched.

The alternative — deriving `selection` from `decided_at` against a
deployment-date constant — is rejected: the deployment date is not a
repository fact (`main`'s history does not say when the pilot took
#403), and a signed statement must not depend on one.

`ScoreAttestationCoverage` gains the stamp as data:
`Partial { chunks_scored, chunks_total, selection: ChunkSelection }` and
`PartialUnknownTotal { chunks_scored, selection }`, with
`ChunkSelection::{StrideEndpointInclusiveV1, Unknown}`; `Complete`
carries none and serialises `selection: "all"`. `from_decision_columns`
takes the stamp as a fourth `Option<&str>` argument and maps
`Some("stride_endpoint_inclusive.v1")` to the variant, anything else
(NULL or an unrecognised literal) to `Unknown`. The two attestation
queries (`list_own_gate_decision_scores` and the scoped variant, both
in `db/postgres.rs` near lines 5134 and 5209) select the new column.

#### Where coverage appears

| surface | today | after this spec |
|---|---|---|
| `GET`/`POST /v1/contributors/me/score-attestation` (signed) | v2: `coverage_state`, `chunks_scored`, `chunks_total?` | v3: the same plus `selection` |
| status / receipt basis line (`gate_credit_basis_line`) | "over N of M chunks" / "over the first N chunks" / "over all N chunks" / "over the whole trace" | "over N of M chunks spread across the trace" when the stamp is `stride_endpoint_inclusive.v1`; "over N of M chunks" when the stamp is absent; "over N chunks of an unrecorded total" replaces "over the first N chunks"; the uncapped strings unchanged |
| contributor CLI `status` | prints the v2 coverage sentence | unchanged |
| `docs/collector-integration.md` step 5 | "a judgment on a prefix" | describes the strided sample, the `selection` field, and the v2 to v3 migration |
| operator calibration outputs | the V3 report's tables were built ad hoc in `psql` | `scripts/operator/chunk-coverage-report.sh` prints the coverage-band and chunk-count-band tables (section 5); future calibration reports cite its output |

#### What coverage does not do

Coverage does not change `q`, and no consumer in this spec multiplies by
it. The flat 0.03 row is the reason: the representative is a
token-weighted mean, a spread sample of 16 chunks estimates it as well
at 8% coverage as at 90%, and a discount by coverage would penalise the
one thing the data says is estimated correctly. What coverage changes is
the confidence a reader attaches to `q` and to `gate_passed`, and that is
the reader's policy (the collector doc already says so). Whether a trace
of 2,362 chunks should earn more than a trace of 3 for the same `q` is a
quantity question, not a coverage one; it is an open question below and
is not touched here.

### 2. Peak normalisation

#### The statistical problem

Let the scored chunks have per-chunk log-perplexities `x_1..x_n`. The
representative is `exp(mean of x)` (token-weighted; for chunks of the
packing target size the weights are nearly equal). The peak is
`exp(max of x)`. So the stored ratio is

    r = peak / rep,   ln r = max(x) - mean(x).

If the `x_i` are draws from one distribution with spread `sigma`, then
`E[max(x) - mean(x)]` is about `sigma * m(n)`, where `m(n)` is the
expected maximum of `n` standard draws — for a normal, `m(2) = 0.56`,
`m(4) = 1.03`, `m(8) = 1.42`, `m(16) = 1.77`. The expected log-ratio of an
ordinary 16-chunk trace is three times that of an ordinary 2-chunk trace
with the same spread, and `n` is fixed by the trace's length, not by
anything a contributor did.

The V3 report's own table shows this. Under Qwen3.8, by chunks scored:

| chunks | n | r p50 | r p90 |
|---|---|---|---|
| 1 | 19 | 1.00 | 1.00 |
| 2-8 | 26 | 3.85 | 13.36 |
| 9-15 | 24 | 7.80 | 41.06 |
| 16 | 238 | 8.30 | 43.94 |

Divide the log of each by `m(n)` at a band's typical `n` (5, 12, 16):

| chunks | ln(r p50) / m(n) | ln(r p90) / m(n) |
|---|---|---|
| 2-8 | 1.16 | 2.23 |
| 9-15 | 1.26 | 2.28 |
| 16 | 1.20 | 2.14 |

The raw median moves by a factor of 8 across bands; the normalised
median moves by 8%. The normalised p90 is flat too. That is the whole
case: the per-chunk spread of an agent trace under Qwen3.8 is about 1.2
in log-perplexity whatever its length, and the peak term should be a
statement about that spread.

#### Candidates

| correction | what it needs | trade-off | verdict |
|---|---|---|---|
| **A. Normalised log-excess** `s = max(0, ln(peak / rep)) / m(n)` | `rep`, `peak`, `chunk_count` — all stored | one table of 16 numbers; `s` estimates the trace's own per-chunk spread and is comparable across `n`; calibrated on stored rows; batch-recomputable | **recommended** |
| B. Ramp thresholds scaled by `n`: `soft(n) = exp(soft_s * m(n))`, likewise hard | the same | mathematically identical to A with the thresholds moved into ratio space; the stored diagnostic (`anomaly_ratio`) stays incomparable across `n`, and every future report has to undo the scaling | subsumed by A |
| C. A high quantile of the scored chunks instead of the max | per-chunk values | not stored; cannot be calibrated on the corpus or applied by the batch pass; and a p90 of 16 chunks is the second-largest, which still grows with `n` (more slowly); it separates "one outlier" from "wide spread", which A cannot | escalation, needs per-chunk persistence (open question 2) |
| D. Drop the anomaly term for capped traces | nothing | turns the cap into an evasion: pad a trace past 16 chunks and the fraud flag switches off | rejected |
| E. Raise the cap | scorer time | does not touch the growth; makes it worse (`m(32) = 2.07`) | rejected; see section 3 |

#### Recommendation: `s`, and calibration era V4

**The statistic.**

    s = max(0, ln(peak_micros / rep_micros)) / m(n),  n = chunk_count (NULL reads as 1)
    s = 0 when n <= 1, when rep_micros <= 0, or when peak_micros <= rep_micros

`m(n)` for `n = 1..16` is a pinned table of the expected maximum of `n`
independent standard normal draws (`m(1) = 0`, then 0.5642, 0.8463,
1.0294, 1.1630, 1.2672, 1.3522, 1.4236, 1.4850, 1.5388, 1.5865, 1.6292,
1.6680, 1.7034, 1.7359, 1.7660). For `n > 16` — only reachable if
`TRACE_COMMONS_GATE_CHUNK_CAP` is raised — `m(n)` is Blom's
approximation `Phi^-1((n - 0.375) / (n + 0.25))` with an inline rational
approximation of `Phi^-1` (Acklam's, about 25 lines, no dependency); at
the join it gives 1.769 against the table's 1.766. A test pins both.

The normal model is a scaffold, not a claim: per-chunk log-perplexity has
a heavier right tail than a normal (tool output). The acceptance
criterion is empirical and is what the script measures — the `s`
quantiles per chunk-count band agree to within 15% of one another, as
the band table above already suggests.

Zero at `n = 1` matches today's behaviour (peak equals representative,
ratio exactly 1, no penalty). Zero when the peak is at or below the
representative covers the row shapes where `peak_perplexity_micros` is
NULL and read as 0 (`COALESCE` in the credit-scoring query), or where no
chunk cleared `min_chunk_tokens` and the aggregate fell back to the
representative; neither carries an anomaly signal, and `ln` of a ratio
below 1 must never reach the ramp.

**The function.** `CreditQualityConstants` gains
`anomaly_statistic: AnomalyStatistic`, an enum `{ RawRatio,
NormalisedLogExcess }`. `credit_quality` gains a `chunk_count: u32`
argument. Under `RawRatio` the argument is ignored and the ramp reads
`r` exactly as today, so V1, V2 and V3 outputs are byte-identical to
`main` for every input (the existing pinned values 19_017, 198_357,
206_043, 103_022 stay in their tests untouched). Under
`NormalisedLogExcess` the ramp reads `s`, and `anomaly_soft_ratio_micros`
/ `anomaly_hard_ratio_micros` are in units of `s`, renamed
`anomaly_soft_micros` / `anomaly_hard_micros` so a reader cannot mistake
3_300_000 for a ratio of 3.3. `CreditQualityScore.anomaly_ratio_micros`
keeps carrying `r * 1e6` under both statistics — the column
`credit_quality_anomaly_ratio_micros` keeps one meaning — and gains
`anomaly_statistic_micros` (`s * 1e6` under V4, `r * 1e6` under
`RawRatio`) for callers and tests; `s` is not persisted, because it is a
pure function of three stored columns and the calibration version.

**The constants.**

| constant | V3 | V4 (provisional) | source |
|---|---|---|---|
| `anomaly_statistic` | `RawRatio` | `NormalisedLogExcess` | this spec |
| `anomaly_soft_micros` | 40,000,000 (r = 40) | 2,200,000 (s = 2.2) | p90 of `s`, Qwen3.8 era; provisional from the band table above |
| `anomaly_hard_micros` | 310,000,000 (r = 310) | 3,300,000 (s = 3.3) | p99 of `s`, Qwen3.8 era; provisional from `ln 310 / m(16) = 3.25`, rounded up to a tenth |
| everything else | | unchanged from V3 | |

"Provisional" is a defined procedure, not a placeholder: the values
above are what the published band medians give; the values pinned in
`CREDIT_QUALITY_CONSTANTS_V4` are the p90 and p99 of `s` over every
Qwen3.8-era row with `perplexity_micros > 0`, as printed by the script's
"anomaly statistic by era" table on the day the PR is opened, each
rounded to a tenth (p99 rounded up, so the hard threshold never sits
below an observed p99). The PR body states the run date, the row count,
and the two percentiles. If they land within 0.1 of 2.2 and 3.3, the
values above stand; the point of the rule is that the tie to the
measurement is written down either way.

**What V4 does to the examples the V3 tests pin.**

| trace | n | V3 (`r`, `a`) | V4 (`s`, `a`) |
|---|---|---|---|
| the median capped Qwen3.8 trace: rep 4.62, peak 38 | 16 | r 8.23, a 1, q 198_357 | s 1.19, a 1, q 198_357 |
| one garbage chunk at 2000 in an ordinary trace: rep 5, peak 2000 | 16 | r 400, withheld | s 3.39, withheld |
| the same inserted into a two-chunk trace: rep 5, peak 100 | 2 | r 20, a 1, q 206_043 | s 5.31, withheld |
| halfway up the ramp, s = 2.75: rep 5, peak 642.9 | 16 | r 128.6, a 0.67 | a 0.5, q 103_022 against clean 206_043 |

The third row is the change in kind: a 20x peak on a 2-chunk trace is
implausible as ordinary spread and V4 says so, where V3's ramp, set from
16-chunk traces, does not penalise it at all (20 is under the soft
threshold of 40). At the cap V4 and V3
agree almost exactly (hard 3.3 against V3's `ln 310 / 1.766 = 3.25`), so
the 3.8-era zeroed count is expected to stay near 5; what moves is the
penalised set, from "long traces" to "spiky traces of any length", and
the script's simulated column reports the count before the constant is
pinned.

**Calibration without re-scoring.** Every input to `s` is a stored
column readable by the narrow role. The script computes `s` in SQL from
`peak_perplexity_micros`, `perplexity_micros`, `chunk_count` and a
`VALUES` table of `m(n)`, and prints its percentiles by era and by
chunk-count band, and the counts a V4 ramp at the chosen thresholds
would penalise and zero. No scorer, no decrypt, no GPU. This is the
V3 method (its SQL re-implementation reproduced the stored V2 mean to
three places) with `n` as one more column.

**Shipping as era V4.** `CREDIT_QUALITY_CONSTANTS_V4 = { anomaly_statistic:
NormalisedLogExcess, anomaly_soft_micros, anomaly_hard_micros, version: 4,
..CREDIT_QUALITY_CONSTANTS_V3 }`. In `CREDIT_QUALITY_SCHEDULE`, V4
replaces V3 as the constants of the era starting at
`QWEN3_8_EFFECTIVE_FROM_UNIX`; the schedule stays two entries, `[V2 from
i64::MIN, V4 from the switch]`, so `the_schedule_is_ordered_and_active_is_its_last_entry`
holds and `CREDIT_QUALITY_ACTIVE` becomes V4. `CREDIT_QUALITY_CONSTANTS_V3`
stays defined and tested as the named constants of the raw-ratio
calibration; it is what a row stamped 3 was scored with, and what the
V3 report describes.

This mirrors #969 exactly. A calibration belongs to a scorer model; V4 is
the corrected calibration for the same model, so it owns the same era.
The alternative — a third era starting at V4's deploy time — would leave
every Qwen3.8 row before that instant on V3 forever and split one model
across two calibrations, which is the confusion the schedule was built
to prevent.

Consequences, in order:

- On deploy, the inline path scores new decisions with V4 and stamps
  `credit_quality_calibration_version = 4`. Rows stamped 3 keep their
  stored `q` and stamp: `constants_at` is consulted only when a row is
  scored, and nothing re-scores a row by itself.
- `POST /v1/admin/score-credit-quality` (admin JWT, optional `?limit=N`)
  recomputes every row with `constants_at(decided_at)`: pre-switch rows
  come out V2 again, byte-identical (the pass is idempotent for them);
  Qwen3.8-era rows move from 3 to 4. This is the "deliberate batch pass".
- `composite_score_micros` (V53) is the inline value production used and
  is never rewritten; a row decided under V3 keeps its V3 composite. That
  is by design (#199) and is unchanged here.
- Between deploy and the pass, the corpus is mixed 3/4 exactly as it was
  mixed 2/3 between 2026-09-19 and the V3 pass. The status line names
  the calibration version per row, so a contributor sees which one they
  are looking at.

`GateCreditInput` gains `chunk_count: Option<i32>`;
`list_gate_decisions_for_credit_scoring` selects it (the V47 grant covers
it; no migration); `score_credit_quality_one` and the inline path pass
it through, NULL as 1.

### 3. The cap stays at 16

The numbers argue against a raise on both sides of the ratio.

- What the cap costs: on the pilot's CPU with the NEAR AI scorer a capped
  trace takes about 78 s to score — roughly 5 s per chunk. Doubling the
  cap doubles that for 63-82% of traces, on a host that memory says is
  already CPU-starved by the local embedder, and doubles NEAR AI billed
  inference (the runbook's rule: a cap raise multiplies billed
  inference, and a timeout is never retried precisely so it cannot
  multiply it again).
- What a raise buys for the mean: nothing measurable. `q` is flat from
  50-99% coverage down to under 10%; the representative is a
  token-weighted mean and 16 spread chunks estimate it as well as 32
  would.
- What a raise buys for the peak: harm, without normalisation
  (`m(32) = 2.07` against `m(16) = 1.77`, so raw `r` grows again) and
  nothing with it (`s` is comparable across `n` by construction).
- What a raise buys for the fraud flag on the longest traces: nothing
  worth having. One inserted chunk in a 2,362-chunk trace is sampled with
  probability 16 / 2362 = 0.7%; at a cap of 32 it is 1.4%. The anomaly
  term is blind on the longest traces at any cap a CPU can afford, and
  that is a property of sampling, not of 16.

So the cap stays at 16 unless one of two stated conditions holds:

1. **Per-chunk cost falls below about 1 s.** A GPU scorer (the spot-L4
   drain measured about 750x the pilot CPU) makes the cap a billing
   knob rather than a latency one, and the decision moves to the cost
   ledger.
2. **The index-coverage experiment (section 4) shows the unsampled chunks
   carry materially more novelty than the sampled ones** — the sample is
   then biased for the signal that decides insertion, and the fix is
   more chunks for the embedder (which is the cheap arm), not
   necessarily for the perplexity scorer.

A cap change re-stamps `gate_version_hash` (the cap is in its
`chunking=` line) and needs no selection-algorithm bump.

### 4. The unscored 89% as an index-coverage question

Text that is not scored is not embedded, and text that is not embedded
never enters the novelty index. Of the chunks capped traces held since
2026-08-23, 89.9% were never seen by the embedder, so the index is built
from about 11% of the corpus text, and the novelty of every later trace
is measured against that 11%. A later trace that repeats an unsampled
region of an earlier one reads as novel. This is the same asymmetry the
dedup simhash does not have (it reads the full canonical text); the two
signals answer "have we seen this" over different corpora.

Proposed experiment, out of scope for this spec: an embedding-only pass
over the unsampled chunks of a sample of capped traces — decrypt, chunk,
skip the selected indices, embed the rest, query the index for each,
and report the distribution of nearest-neighbour cosine for unsampled
chunks against the distribution for sampled ones. No perplexity call,
no insertion, no write. If the unsampled chunks are not more novel than
the sampled ones, the index is missing volume but not content, and the
cap stands on cost grounds. If they are, condition 2 above is met. The
pass is the same shape as `rederive-dedup`'s dry run and belongs beside
it; the local embedder's CPU cost on the pilot is the constraint, and
the GPU drain is the way round it.

### 5. Operator measurement: `scripts/operator/chunk-coverage-report.sh`

Read-only `psql` as `trace_gate_driver`, `PGOPTIONS` read-only, touching
only granted columns, printing counts, percentiles and means only — no
ids, no tenant references, no per-row values. Same flag and failure
conventions as `dedup-cluster-report.sh` (`--db-url=`, else
`$TRACE_COMMONS_GATE_DRIVER_DATABASE_URL`; `ChunkCoverageReportFailure:`
labels). Sections:

1. **Role and time.**
2. **Capping by week** (`date_trunc('week', decided_at)`): scored rows,
   capped rows, capped share; rows with `total_chunk_count`, sum of
   `total_chunk_count` and of `chunk_count` over capped rows, the scored
   share. The first two tables in this document.
3. **Coverage bands** over capped rows with a known total: bands
   `>= 50%`, `25-50%`, `10-25%`, `< 10%`, plus "unknown total"; per band
   the row count and mean `credit_quality_micros`; and the uncapped mean
   beside them. The flat row.
4. **Anomaly statistic by chunk-count band** (`1`, `2-8`, `9-15`, `16`,
   `> 16`), per era (`decided_at` against the schedule boundary, since
   the role cannot read the version column): row count, `r` p50/p90/p99,
   `s` p50/p90/p99. The band table above, with `s` computed per row from
   the row's own `n`.
5. **Anomaly statistic by era**: `s` p90 and p99 over all rows in the
   era. These are the V4 constants.
6. **Simulated V4** at thresholds given by `--soft=<s>` and `--hard=<s>`
   (defaulting to 2.2 and 3.3): per chunk-count band, rows a V4 ramp
   would penalise and zero, beside the count of rows whose stored `q`
   is 0. Before the batch pass this is the prediction; after it, the
   comparison.
7. **Selection stamp** (after V75): rows per `chunk_selection` value,
   NULL shown as `unstamped`.

Run before PR 2 is deployed, after the batch pass, and after PR 3's
migration; file the three outputs in
`docs/superpowers/reports/2026-XX-XX-credit-quality-v4-calibration.md`
in the V3 report's shape.

## Testing

**`credit_quality.rs` (unit, no I/O).**

- `m(n)` table: the sixteen pinned values; Blom's formula evaluated at
  `n = 16` within 0.5% of the table's 1.7660 (the join), and the
  extension strictly increasing from `n = 17` to `n = 64`.
- `s`: 0 at `n = 1`; 0 when `peak <= rep`; 0 when `rep <= 0`; unchanged
  when every chunk perplexity is multiplied by a constant (both `rep`
  and `peak` scale, so the ratio does not).
- Raw-ratio path is byte-identical: every existing V1/V2/V3 test passes
  with a `chunk_count` argument added and no expected value changed,
  including a row with `chunk_count = 16` and one with `1` giving the
  same V3 result.
- V4 pins: the four rows of the table in section 2 —
  `(4.62, 38, 0.16, n = 16)` gives `q = 198_357`, `a = 1`;
  `(5, 2000, 0.2, n = 16)` is withheld; `(5, 100, 0.2, n = 2)` is
  withheld under V4 and not under V3; `(5, 642.9, 0.2, n = 16)` against
  `(5, 5, 0.2, n = 16)` gives `(103_022, 206_043)` within one micro.
- **The i.i.d. property, the test the brief asks for.** A seeded LCG
  (no dependency) draws per-chunk log-perplexities from a normal with
  spread 1.2 (the pilot's), builds `rep = exp(mean)` and `peak =
  exp(max)`, and scores under V4 for `n` in `{2, 4, 8, 16}` over 400
  seeds each. Assertions, all against pinned seeds so each is a
  regression pin: the mean of `s` at `n = 16` is within 10% of the mean
  at `n = 2`; the penalised fraction at `n = 16` is at most the
  penalised fraction at `n = 2` plus 0.02; the withheld fraction at
  every `n` is at most 0.02. The same generator scored under V3
  (`RawRatio`) has a penalised fraction at `n = 16` at least three times
  that at `n = 2` — a pin that documents the defect, to be deleted with
  V3.
- Schedule: `constants_at(switch)` is version 4; `constants_at(switch - 1)`
  is 2; a Qwen3.6-era row is never scored with V4; V3 is not in the
  schedule and is still defined; `CREDIT_QUALITY_ACTIVE` is V4.

**Ingest internal tests (`trace_commons_ingest_internal/tests.rs`).**
The batch pass passes `chunk_count` through and a NULL scores as
`n = 1`; the inline path passes `decision.chunk_count`; a capped
decision inserted by the inline path carries
`chunk_selection = "stride_endpoint_inclusive.v1"` and an uncapped one
carries it too; `gate_credit_basis_line` renders the five strings in the
table in section 1 from five decision rows.

**`trace_score_attestation.rs` (unit).**

- `schema_version_is_v3`.
- `from_decision_columns` over the stamp: `(16, 61, true, Some(stride))`
  is `Partial` with `StrideEndpointInclusiveV1`; `(16, 61, true, None)`
  is `Partial` with `Unknown`; `(16, None, true, Some(stride))` is
  `PartialUnknownTotal` with the stamp; `(4, 4, false, anything)` is
  `Complete`; an unrecognised stamp literal is `Unknown`.
- Wire: `Complete` serialises `selection: "all"`; `TryFrom` rejects a
  `partial` entry without `selection`, rejects an unknown `selection`
  value, and rejects `partial` without `chunks_total` as today.
- **Pinned golden.** One test signs a three-entry document (one per
  coverage state, the partial one stamped) with a fixed test key, fixed
  `issued_at`, `expires_at` and `nonce`, decodes the JWS payload, and
  asserts the claims JSON byte-for-byte against a literal in the test.
  Field order, the `selection` literals, the absent `chunks_total` on
  the unknown-total entry, and the version string are all in the
  literal; any change to the wire shape fails it by name. The existing
  round-trip tests are updated from v2 to v3.

**Contributor CLI (`commands.rs::coverage_tests`).** A v3 entry with
`selection` renders the same sentence as a v2 entry; the CLI does not
pin the version string, and a test says so.

**PostgreSQL (`tests/trace_corpus_pg_store.rs`, CI against PostgreSQL
16).** `list_gate_decisions_for_credit_scoring` as the gate-driver role
returns `chunk_count`, and `has_column_privilege` holds for it (V47) and
for `chunk_selection` (V75); the inline insert writes the stamp and the
two attestation queries read it back; `update_trace_gate_decision_credit_quality`'s
must-not-touch assertion still holds after a V4 write.

**Operator script.** Exercised against the CI PostgreSQL in a test that
loads a fixture of twelve decision rows across two tenants and the two
eras, runs the script as the role, and asserts the band counts and one
`s` percentile; the SQL's `m(n)` `VALUES` table is asserted equal to the
Rust table by a test that greps the script (the two are the same sixteen
numbers and must stay so).

## PR split

1. **Measurement.** `chunk-coverage-report.sh`, its CI test, the
   V4-calibration report skeleton. No code change. Its "before" output
   is what PR 2's constants are pinned from.
2. **V4.** `AnomalyStatistic`, `m(n)`, the `chunk_count` argument,
   `CREDIT_QUALITY_CONSTANTS_V4` with the measured constants, the
   schedule change, `GateCreditInput.chunk_count`, the query column, the
   two call sites, the tests. No migration. Deployable alone; takes
   effect for new decisions on deploy and for the era on the batch pass.
3. **Selection in the attestation.** V75, the inline stamp, schema v3,
   `ChunkSelection`, the two query columns, the basis-line strings, the
   collector doc, the CLI test, the golden. Independent of PR 2 and
   deployable before or after it; needs the migrator step.

PR 3 is the one a reviewer may reasonably defer (open question 1); PRs
1 and 2 do not depend on it.

## Rollout on the pilot

Roles: ingest runs as `tc_ingest_runtime_login`, which owns no table;
migrations are applied as the migrator role, `app`, by the operator
(`docs/operator/deployment.md`, "Redeploying the binary"); the batch pass
is the admin route with the admin JWT the perplexity runbook uses (the
pilot refuses static admin tokens). The report script runs as
`trace_gate_driver` — the narrow role, not a superuser URL, which would
pass vacuously.

1. **Before.** Run the report script; keep the output. Read section 5's
   era percentiles and pin `CREDIT_QUALITY_CONSTANTS_V4` from them in
   PR 2 per the rounding rule. Confirm section 4's `s` quantiles are
   flat across bands to within 15%; if they are not, stop, and the
   report says why (open question 3).
2. `git diff --name-only <running build_commit> <PR 2 commit> -- migrations`
   prints nothing. Install the PR 2 build; confirm `/health` reports its
   commit. From this instant new decisions are stamped 4.
3. Smoke: `POST /v1/admin/score-credit-quality?limit=5`. Read the
   completion line (`scored`, `failed`), hash-only.
4. Full pass, no `limit`. Wait for the completion line; `failed` is 0.
5. Report script again ("after V4"). Confirm: the zeroed count in the
   Qwen3.8 era is near 5; the penalised set is spread across chunk-count
   bands rather than concentrated at 16; the pre-switch era's mean `q`
   is unchanged to three places (the V2 rows were recomputed to the same
   values).
6. `POST /v1/admin/recompute-contributor-caps`, so the shadow cap columns
   are recomputed from V4's `q` — the same step the dedup rollout ends
   with, for the same reason.
7. For PR 3: `git diff --name-only ... -- migrations` prints
   `V75__trace_gate_decision_chunk_selection.sql`. Apply it as `app`;
   confirm `has_column_privilege('trace_gate_driver', 'trace_gate_decisions', 'chunk_selection', 'SELECT')`.
   Install the PR 3 build. From this instant new rows carry the stamp and
   the attestation is v3.
8. Report script a final time (section 7 shows the stamp split). Submit
   one capped session from the test tenant and fetch its attestation;
   confirm `selection: "stride_endpoint_inclusive.v1"` and the status
   line's "spread across the trace".
9. File the three outputs and the pinned constants in the V4 report.

Rollback: PR 2 rolled back to the previous build puts new decisions on
V3 while the corpus is on 4; the old binary's `constants_at` returns V3
for the era and its batch pass would move every row back, so do not run
it unless that is the intent. PR 3 rolled back leaves the column in
place and unstamped for rows written by the old binary (NULL, reads as
`unknown`), and the attestation back on v2; a collector updated to v3
rejects v2 until the build is restored, which is the same posture as any
version bump's rollback.

## Out of scope

- Per-chunk persistence and the quantile statistic (candidate C).
- The embedding-only index-coverage pass (section 4).
- Any change to the cap, the selection algorithm, or the renderer.
- The dedup path's full-text/sample asymmetry.
- Credit scaled by length or by coverage.
- The novelty floor (`2026-09-21-novelty-floor-calibration.md`) and the
  `g` term; V4 inherits V3's novelty constants unchanged.
- Backfilling `chunk_selection` for rows before V75 from a deployment
  date; they stay `unknown`.

## Open questions

1. **Whether the v3 bump is wanted now.** The signed document today
   cannot distinguish a prefix from a spread sample and does not carry
   `decided_at`, so a collector cannot reconstruct it either. The cost is
   a coordinated verifier update and a migration; the benefit accrues
   only to a collector that discounts by coverage. If no such collector
   exists yet, PR 3 can wait, and PRs 1 and 2 ship without it.
2. **Per-chunk persistence.** Storing the sixteen per-chunk perplexities
   (a `BIGINT[]` column, no content) would let the anomaly term
   distinguish one outlier from a wide spread, let a quantile statistic
   be calibrated, and let the script measure the log-normal assumption
   directly instead of through the band table. About 128 bytes per
   row. It is the natural V5 and the reason `AnomalyStatistic` is an enum
   rather than a boolean.
3. **What if `s` is not flat.** The band table is three points from
   published medians at approximate `n`. If the per-row computation in
   step 1 shows the `s` quantiles drifting with `n` by more than 15%, the
   spread is not the only thing growing with length — for instance a
   trace's later chunks may be systematically more surprising than its
   earlier ones, which endpoint-inclusive selection would then sample
   differently at different `n`. The fix would be an empirical `m(n)`
   fitted from the corpus rather than the normal table; the spec's
   structure holds and the table becomes a calibration output.
4. **`n` for the peak is not exactly `chunk_count`.** The peak is a max
   over chunks holding at least `min_chunk_tokens` (64); `chunk_count`
   counts all scored chunks. A trace with several tiny chunks has a
   smaller effective `n` than the table assumes, and its `s` is slightly
   understated. The eligible count is not stored. Expected to be rare at
   a 2048-token packing target; the script cannot measure it, and
   per-chunk persistence would.
5. **Quantity.** A 2,362-chunk trace and a 3-chunk trace with the same
   `q` earn the same `credit_points_pending = 10 * q`. That is the
   current policy and this spec does not change it; whether it should
   be is a settlement question, and the coverage field is the evidence
   any answer would need.

## Self-review

- No placeholders: the statistic, the table, the three `selection`
  literals, the column, the migration number, the routes, the roles, the
  script sections and the rollout steps are stated; the two V4
  thresholds are provisional under a written rule that names the query,
  the era, the percentiles and the rounding, and the PR body records the
  measured values.
- Consistent: `s` is defined once and used the same way in the
  function, the script, the tests and the examples; the V3 pinned values
  are unchanged everywhere they appear; the schedule is `[V2, V4]` in the
  design, the tests and the rollout; `stride_endpoint_inclusive.v1` is
  the same literal in the chunker, the column, the wire and the status
  line.
- Contradictions with the brief are stated where they matter rather than
  papered over: the attestation already carries coverage; only peak
  perplexity enters `q`; per-chunk values are not stored; the narrow role
  cannot read the version column; two strings still say "prefix".
- Sized for one implementation plan with three PRs, the third
  independent of the first two.
- Hash-only: the script, the pass's logs, the report and the attestation
  carry counts, percentiles, version literals and a stamp; no trace
  content, no contributor identity, no credentials, no per-row values.
