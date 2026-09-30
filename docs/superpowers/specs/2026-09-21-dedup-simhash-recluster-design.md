# Cross-Trace Dedup: Simhash Re-derivation and Recluster — Design

Date: 2026-09-21
Status: draft for review
Scope: `trace-commons-server` (`dedup_simhash.rs`, `dedup_assign.rs`,
`trace_gate_service.rs`, the ingest binary, one storage method), one
operator script, one operator runbook. No migration. Two PRs, ordered.

## Problem

Cross-trace dedup puts three quarters of the pilot corpus in one cluster.

Measured 2026-09-21 on the pilot, read-only, as the narrow
`trace_gate_driver` role:

| statistic | value |
|---|---|
| `trace_gate_decisions` rows with a `dedup_cluster_id` | 696 |
| clusters | 67 |
| singleton clusters | 48 |
| clusters of size 2-9 | 16 |
| clusters of size 10-99 | 2 |
| clusters of size 100+ | 1, of **522** rows (75% of clustered rows) |

Every session a contributor uploads joins the 522-cluster: the eleven
scored uploads on 2026-09-21 were written with `dedup_cluster_size` 514,
515, ..., 522 (two unrelated ones landed at 5 and 3). Inside the cluster:

- every member is within Hamming distance 10 of the cluster's first
  member (median 6);
- the median nearest-member Hamming distance is 0;
- there are only 246 distinct `dedup_simhash` values among 522 traces, and
  none of them is 0.

Unrelated sessions from different contributors collide on a 64-bit hash.
The cause is the simhash, not the embedding arm, and not the linkage rule.

### Root cause: multiset simhash on scaffolding-heavy text

`dedup_simhash::trace_simhash` lowercases, splits on non-alphanumerics,
forms overlapping 2-token shingles, FNV-1a hashes each shingle, and adds
+1/-1 into each of 64 accumulators **for every occurrence** of every
shingle. There is no set semantics and no damping. The sign of each
accumulator is a majority vote weighted by occurrence count.

The text it hashes is the canonical render
(`chunker::parse_envelope_rendered_events`, `events.v1`): one line per
event of the form `event_type (tool): content`, joined by newlines. In a
long agent session most of that text is tool output, and tool output
repeats a small vocabulary of shingles thousands of times: JSON keys,
paths, line-number prefixes, the harness's own headers, `cargo test`
output. A handful of shingles that occur 1,000+ times each outweigh tens
of thousands of shingles that occur once. The accumulator signs converge
to the harness's scaffolding fingerprint, and every long trace from the
same harness lands within a few bits of every other one.

`DEDUP_CONSTANTS_V1.tau_hamming = 10` was set from the unit tests in
`dedup_simhash.rs`: about 7 bits for a one-token reword of a 15-token
sentence, at least 18 for two unrelated 12-token sentences. Those tests
never exercise a text long enough for occurrence counts to matter, so the
calibration is true of the tests and false of the corpus.

### What the code does today (established from the call graph, not the docs)

Two things the task brief assumed turn out differently in the code, and
the design depends on both.

1. **Linkage is representative-based, not single-linkage.** Both the
   inline path (`evaluate_and_record_gate`, ingest binary near line
   51262) and the batch sweep (`run_recluster_dedup_pass`, near line
   52212) build one `ClusterCandidate` per cluster, keyed by the
   cluster's earliest-decided member's simhash, and call
   `assign_cluster` against that. A new row is compared with each
   cluster's first member only, never with every member. The
   measurement agrees: all 522 members are within 10 of the first
   member. Single linkage would have made this worse, but it is not what
   chained the corpus; the hash did.
2. **The embedding arm is not wired.** Every production call passes
   `embed_cosine_micros: None`. `AppState.dedup_vector_index` is
   constructed under `local-gpu-models` / `near-ai-scorer` and flushed at
   shutdown, and is `#[allow(dead_code)]` otherwise (ingest binary near
   line 1796); `dedup_index_query` / `dedup_index_insert` were staged for
   a "Task 6" that never landed. So `tau_e_micros = 150_000` has never
   joined anything. It is still wrong (see "Embedding arm" below) and is
   fixed in the same constants bump so that wiring the arm later cannot
   go live at 0.15.

Also established:

- **No re-derivation pass exists.** `POST /v1/admin/recluster-dedup`
  re-sweeps clusters from the **stored** `dedup_simhash` values and
  writes only `dedup_cluster_id` / `dedup_cluster_size`; it never
  decrypts, renders, or re-hashes. The module docs at the top of
  `dedup_assign.rs` require a pass that does, and require it to run
  before any simhash or renderer bump. This spec designs it.
- **The stamp machinery is in place.** `dedup_signal_version` (V57,
  TEXT, nullable, no default, no backfill) names the derivation as
  `<CANONICAL_RENDER_VERSION>+<DEDUP_SIMHASH_ALGORITHM>`; NULL reads as
  the frozen literal `LEGACY_DEDUP_SIGNAL_VERSION =
  "events.v1+fnv1a-2shingle.v1"` via `DedupSignalRow::
  effective_signal_version`. `assign_cluster` refuses any candidate
  whose stamp differs from the incoming row's. The gate-driver role
  already holds column SELECT on `dedup_signal_version` (V57) and on
  `tenant_id, submission_id, decision_id, decided_at, dedup_cluster_id,
  dedup_simhash, dedup_cluster_size` (V45).
- **The four dedup columns have one tenant-scoped writer.**
  `update_trace_gate_decision_dedup` takes a `DedupAssignmentWrite`
  (simhash, cluster id, cluster size, stamp) and touches nothing else
  (`pg_store_update_trace_gate_decision_dedup_touches_only_dedup_columns`
  pins it). `update_trace_gate_decision_dedup_cluster` writes the two
  cluster columns only.

### Who consumes `dedup_cluster_size` today

Traced every read in `crates/trace-commons-server/src`, excluding tests:

| reader | what it does with the size | anything downstream? |
|---|---|---|
| `list_dedup_signals` (gate-driver pool) | not selected; sizes are recounted in memory from `dedup_cluster_id` | no |
| `list_contributor_cap_signals` -> `run_recompute_contributor_caps_pass` (`/v1/admin/recompute-contributor-caps`) | `contributor_cap::increment_micros(q, size) = q / size`, then the concave factor; writes `contributor_factor_micros` and `contributor_cumulative_raw_micros` | **nothing reads those two columns.** Their only non-test mentions are the UPDATE that writes them and a must-not-touch assertion in another writer's test |
| `correction_value.rs` | uses `correction_cluster_size`, a separate column from a separate clustering of `outcome.human_correction`; the mention near line 331 is a test asserting the trace cluster size is NOT an input to the correction value | no |

The passing-gate credit event (`novelty_utility`, ingest binary near line
53227) is minted from a synthetic `GateDecision` with `dedup_simhash: 0`
and does not read the cluster size. The app's `credit_points_pending` is
`trace_commons_protocol::trace_contribution::estimate_initial_credit` at
submit time and does not read it either.

**Conclusion: no contributor-visible number and no ledger number depends
on `dedup_cluster_size` today.** The wrong sizes have corrupted two shadow
columns that nothing reads. That is why this can be fixed by a
re-derivation pass rather than a credit-correction event, and why the
withhold-flag fallback in the module docs is not needed now (see "Credit
in the meantime").

## Goal

1. Replace the simhash with a signal under which unrelated long traces
   from one harness are far apart and resubmissions, late forks, and
   lightly edited copies of one session are close.
2. Re-derive the stored signal for the whole corpus from the encrypted
   envelopes, re-stamp it, re-cluster, and write consistent sizes,
   through an admin route that follows the `rescore-perplexity` pattern.
3. Ship it in the order the `dedup_assign.rs` module docs require: pass
   first, constant flip second, never in one binary.
4. Fix the embedding-arm threshold in the same constants bump so the arm
   cannot be wired at a value that sits inside the unrelated-trace
   distance distribution.

## Non-goals

- No change to `CANONICAL_RENDER_VERSION` or to `render_event_text`.
  The renderer is not the problem, and leaving it alone keeps the
  perplexity, novelty, and vector-index text unchanged.
- No wiring of the embedding arm. Its threshold is corrected; the arm
  stays unwired until it has its own calibration (open question 3).
- No MinHash, no LSH banding, no new column. The 64-bit BIGINT column and
  Hamming distance are kept; MinHash is the documented escalation if the
  dry run shows no valley (see "Candidate signals").
- No `correction_signal_version` column (#538). The correction path is
  pinned to the v1 function so this change does not trigger that defect
  (see "Correction simhash stays on v1").
- No settlement, no credit event, no change to `credit_quality_micros`,
  the contributor-cap constants, perplexity, novelty, or gate status.
- No removal of the old `/v1/admin/recluster-dedup` route in these PRs.

## Design

### 1. The new signal: set-semantic simhash over 3-token shingles

`DEDUP_SIMHASH_ALGORITHM` moves from `fnv1a-2shingle.v1` to
`fnv1a-3shingle-set.v2`. The function keeps the tokenizer (lowercase,
split on non-alphanumeric, drop empties) and FNV-1a, and changes two
things:

- **Set semantics.** Shingle hashes are deduplicated before voting. Each
  distinct shingle contributes +1/-1 to each accumulator exactly once,
  however many times it occurs. A `HashSet<u64>` of shingle hashes; no new
  dependency.
- **Shingle width 3** (fall back to width 2 for a two-token text and to
  the unigram for a one-token text).

Everything else is unchanged: 64 accumulators, sign to bit, `u64` stored
as `BIGINT`, `hamming_distance` as the comparison, `0` for empty text.

`dedup_simhash.rs` keeps **both** functions, named by version:
`trace_simhash_v1` (the current body, renamed) and `trace_simhash_v2`.
A `DedupAlgorithm` enum with a closed set of two variants maps each name
to its function and its constants, and is the type the re-derivation
route parses its `algorithm` parameter into. The inline gate path calls
whichever function `DEDUP_SIMHASH_ALGORITHM` names; in PR 1 that is still
v1.

#### Candidate signals considered

| candidate | scaffolding immunity | resubmission / late fork | light reword | same project, different session | schema / grants | verdict |
|---|---|---|---|---|---|---|
| Multiset 2-shingle simhash (today) | none: counts dominate | close | close on short text; undefined on long | fused (the defect) | as-is | replaced |
| **Set 3-shingle simhash** | full: a shingle counts once | close (J near 1) | close only for a few percent of tokens | far unless most distinct shingles are shared (table below) | as-is | **recommended** |
| TF-damped weights, `log(1 + count)` | partial: a 1,000-occurrence shingle still weighs 7x a singleton | close | as set | closer than set: repeated shared tool output is up-weighted | as-is | rejected: keeps the failure mode, weaker |
| Wider shingles (4-5) on a multiset | none: counts still dominate | close | worse | fused | as-is | rejected |
| Drop rendered scaffolding before hashing | only the `event_type (tool):` prefix, which is a few dozen distinct shingles; the repetition is inside `content` | same | same | same | renderer or a second render path; a `CANONICAL_RENDER_VERSION` question | rejected: does not reach the cause |
| Corpus IDF weighting | full | close | good | good | the hash becomes a function of the corpus; not reproducible, not stampable, breaks the pure no-I/O contract | rejected |
| Author-kind weighting (tool-result shingles at weight `w < 1`, authored spans at 1) | full | close | authored rewording moves it more | much better: tool output is what same-project sessions share | as-is; spans already exist from `parse_envelope_events` | **first escalation**, decided by the dry run |
| MinHash (128 hashes, Jaccard) | full | resolution 1/sqrt(128) on J, about 0.09 | as set | separable from resubmission at J 0.6 vs 0.9 where 64-bit simhash is marginal | new BYTEA column, new grant, new comparison, stamp name | **second escalation**, if the dry run shows no valley |

#### Why width 3

Width does not change what two sessions on the same repository share
(an identical file read contributes identical shingles at any width);
it changes incidental overlap of common phrases and code idioms between
unrelated traces, and it changes how far a token-level edit moves the
hash. Width 2 has the most incidental overlap; width 4+ means one changed
token removes four shingles, so a 10%-token reword removes up to 40% of
shingles. Width 3 is the compromise: markedly less incidental overlap
than 2, and an edit still costs at most three shingles per token. Width
is part of the algorithm name; changing it is a bump.

#### What a near-duplicate means for credit

The simhash arm answers one question: is this the same session's
content, resubmitted, forked late, or lightly touched? It does not answer
"is this the same work reworded", which is the embedding arm's job (the
2026-07-12 design's A7 case) and stays so.

| case | expected Jaccard of distinct 3-shingles | should cluster? |
|---|---|---|
| resubmission of the same session (new submission id, new timestamps; the render omits both) | 1.0 | yes |
| resubmission with a few nonce tokens appended (A6 shim) | above 0.99 on a long trace | yes |
| fork of a session that diverges in its last twentieth | about 0.9 | yes |
| fork that diverges in its last tenth | about 0.82 | borderline: expected Hamming 9 on 64 bits, see the table below; caught more often than not, not reliably |
| fork that diverges at its first tenth | about 0.1 | no: two pieces of work |
| light reword: a few percent of tokens changed | 0.85 to 0.95 | yes |
| heavy reword: a third of tokens changed | below 0.4 | no on this arm; the embedding arm's case |
| same project, different session, overlapping file reads | 0.2 to 0.6 depending on how much of each session is shared file content | **no** |
| unrelated sessions, same harness | below 0.2 | no |

The credit unit is the session's content. Two sessions that share most
of their content are credited once between them; two sessions that share
the same repository but do different things are two units, even if every
`Read` of `src/main.rs` in both is byte-identical.

#### Calibrating `tau_hamming`

For a unit-weight simhash, the probability that one bit differs between
two feature sets with cosine similarity `c` is `arccos(c) / pi`, and for
two sets of equal size with Jaccard `J`, `c = 2J / (1 + J)`. Expected
Hamming distance over 64 bits, with its standard deviation:

| J | c | expected Hamming | std |
|---|---|---|---|
| 1.0 | 1.00 | 0 | 0 |
| 0.95 | 0.974 | 4.6 | 2.1 |
| 0.9 | 0.947 | 6.7 | 2.4 |
| 0.8 | 0.889 | 9.7 | 2.9 |
| 0.7 | 0.824 | 12.3 | 3.2 |
| 0.6 | 0.750 | 14.7 | 3.4 |
| 0.5 | 0.667 | 17.1 | 3.5 |
| 0.3 | 0.462 | 22.2 | 3.8 |
| 0.1 | 0.182 | 28.3 | 4.0 |
| 0.0 | 0.00 | 32 | 4.0 |

Two consequences. First, the current `tau_hamming = 10` corresponds to
roughly J 0.8 under set semantics, which is the right neighbourhood for
"mostly the same session". Second, 64 bits are coarse: a J 0.6 pair
(same-project sessions sharing a large file read) sits at 14.7 +/- 3.4
and falls at or below 10 about one time in nine. At `tau_hamming = 8`
that falls to about one in twenty-five, while a J 0.9 pair (late fork,
light edit) still lands at or below 8 about three times in four.

**`DEDUP_CONSTANTS_V2.tau_hamming` starts at 8**, and the dry run
decides. The dry run reports, for the re-derived corpus, the would-be
cluster-size distribution at each candidate threshold in
`DRY_RUN_CANDIDATE_TAU_HAMMING = [4, 6, 8, 10, 12, 14]` (the analogue of
`DRY_RUN_CANDIDATE_FLOORS_MICROS` in the perplexity dry run), plus the
histogram of each row's Hamming distance to its nearest cluster
representative. The expected shape is a large mass at 20+ and a small
mass at 0-6 with little in between; the threshold goes in the valley.
If the 8-14 band is populated, the escalations apply in order:
author-kind weighting, then MinHash.

One known-positive control is planted before the dry run: the operator
submits one already-scored session a second time from a test tenant (the
`scripts/operator/smoke-gate.sh` path already submits synthetic
envelopes). The dry run's aggregate report cannot name it, but the
operator can confirm afterwards with the measurement script that the
control's decision row and its original share a `dedup_cluster_id`
under the written result. Under v1 the control lands in the 522-cluster,
so it proves nothing there; under v2 it is the one pair whose distance
is known to be 0.

### 2. Embedding arm: correct the threshold, keep it unwired

The embedder is BAAI/bge-large-en-v1.5. Cosine similarity between
unrelated traces on it sits around 0.84-0.88; measured novelty
(`1 - similarity`) on the pilot is p05 0.123, p50 0.164, p95 0.291. A
join threshold of cosine distance 0.15 is therefore the median distance
between two unrelated traces. It has never joined anything because the
arm is unwired, and the first day it is wired at 0.15 it will chain the
corpus on its own.

`DEDUP_CONSTANTS_V2.tau_e_micros = 30_000` (cosine distance 0.03): four
times below the fifth percentile of unrelated pairs, and 0 for identical
text. OR semantics are retained: the two arms cover each other's blind
spots, and that is the anti-gaming stance the 2026-07-12 design chose.
What changes is that an embedding-only join must mean "near-identical
on this embedder", not "in the same neighbourhood".

Calibration plan for the value: it is not measured by the re-derivation
pass (the pass never embeds; the pilot host is CPU-bound on bge-large
already). When the arm is wired, its own dry run reports the trace-level
cosine distance to the nearest dedup-index neighbour under the same
candidate-threshold shape as this pass, with the same planted control,
and the value is confirmed or moved then. Until that PR, 0.03 is a
ceiling nobody can accidentally exceed, and the `assign_cluster` test
`embedding_over_threshold_does_not_join_on_embedding_alone` keeps pinning
that a distance above it does not join.

### 3. Linkage: keep representative-based, first-member representative

Options:

| linkage | order dependence | chaining | cost | drift |
|---|---|---|---|---|
| single linkage (any member within tau) | low | high: a chain of J 0.8 pairs fuses J 0.3 endpoints | O(members) | grows with every join |
| **first-member representative** (today) | assignment depends on `decided_at` order, which is fixed | none: every member is within tau of one fixed point | O(clusters) | none: the representative never changes |
| bitwise-majority centroid | high: the centroid moves as members join, so the same row assigned earlier or later can land differently | low | O(clusters) plus a recompute per join | present by construction |

Keep first-member representative. It is what the code does, it is what
the measurement was taken against, it cannot chain, and its
determinism is what makes the pass idempotent (a re-sweep of a written
corpus in `decided_at` order reproduces itself). The tie-break "larger
cluster wins, then lower UUID" stays; with a sound signal it is
harmless, and with a collision-prone one no tie-break helps.

The sweep is factored out of the ingest binary into
`dedup_assign::sweep_clusters(rows, constants) -> SweepResult` (pure,
in the library crate): input is `(row key, simhash, stamp)` in
`decided_at` order, output is each row's cluster id and each cluster's
final size. Both the old recluster route and the new pass call it, so
the two cannot disagree on membership, and it is unit-testable without
PostgreSQL.

### 4. The re-derivation pass

**Route:** `POST /v1/admin/rederive-dedup`, in the ingest binary beside
`recluster_dedup_handler`, copying the shape of
`rescore_perplexity_handler`:

- auth: `authenticate_with_tenant_access_grant` then `require_admin`;
  no new credential;
- fail-closed preconditions: 503 without a DB mirror, 503 without an
  artifact store (the pass loads ciphertext);
- spawns a background task, returns a hash-only ack immediately;
- query parameters, `#[serde(deny_unknown_fields)]` as the perplexity
  route does (a mistyped `dry_run` must be refused, not read as a write):

| parameter | type | meaning |
|---|---|---|
| `algorithm` | required, one of `fnv1a-2shingle.v1`, `fnv1a-3shingle-set.v2` | the target derivation; unknown names are a 400 |
| `dry_run` | bool, default false | derive and sweep, write nothing, log aggregates |
| `limit` | optional | bound the enumeration, for a `?limit=5` smoke |

The ack is `{accepted, limit, mode, algorithm}` where `mode` is
`dry_run` or `write`, so the operator's one confirmation says which.

**Target stamp** is `format!("{CANONICAL_RENDER_VERSION}+{algorithm}")`
where the renderer half is the build's constant, not a parameter: the
pass renders with the code it has, so it can only produce the render
version it has.

**Enumeration:** a new storage method on the gate-driver pool,
`list_dedup_rederive_rows(limit) -> Vec<DedupRederiveRow>` with
`tenant_id, submission_id, decision_id, decided_at, dedup_simhash,
dedup_signal_version`, ordered `decided_at ASC, decision_id ASC`, all
rows with a decision (not filtered by stamp; see "resumable"). Every
column is already granted to `trace_gate_driver` by V45 and V57, so no
migration and no grant.

**Per row:**

1. If `effective_signal_version() == target` and `dedup_simhash` is
   present: reuse the stored simhash. No load, no decrypt. This is what
   makes a rerun cheap and a crash resumable.
2. If the stored stamp is `DETERMINISTIC_DEDUP_SIGNAL_VERSION`
   (`digest-prefix.v1`) or `PLACEHOLDER_DEDUP_SIGNAL_VERSION`: the value
   is not a simhash of any text and cannot be re-derived from one. Skip;
   count as `not_derivable`. Such rows come only from development and
   test services.
3. Otherwise `load_trace_ciphertext_and_wrapped_dek(state, tenant_id,
   submission_id)` (the exact loader the perplexity re-score uses), then
   a new `TraceGateService` method:

   ```rust
   fn derive_dedup_signal(
       &self,
       tenant_ctx: &TenantCtx,
       envelope_ciphertext: &[u8],
       wrapped_dek: &WrappedDek,
       object_kind: TraceArtifactKind,
       algorithm: DedupAlgorithm,
   ) -> anyhow::Result<DedupSignalDerivation>; // { simhash: i64, signal_version: String }
   ```

   The default impl bails `DedupRederiveUnsupported`, as
   `evaluate_trace_perplexity_only` bails
   `PerplexityOnlyRescoreUnsupported`. The enclave impl does exactly
   what `evaluate_trace` does up to the hash: same `KekContext` under
   the canonical `tenant_storage_ref`, `unwrap_dek`,
   `aead_decrypt_with_dek`, then
   `parse_envelope_rendered_events(&plaintext).map(join "\n")` with the
   lossy-UTF-8 fallback, then the selected hash. That render-and-join is
   moved into one function `dedup_canonical_text(plaintext) -> String`
   shared by `evaluate_trace` and `derive_dedup_signal`, so the inline
   path and the pass cannot render differently. No scoring, no
   embedding, no vector-index read or write, no per-chunk work. The
   plaintext never leaves the method; only the 64-bit value and the
   stamp cross back.

   The deterministic gate service does not implement it, because it
   never sees plaintext.

4. A load or derive failure is counted `failed`, logged hash-only
   (tenant hash, submission hash, error hash — the perplexity re-score's
   three fields), and the row keeps its old stamp and old value. It is
   therefore still refused against every re-derived row until a rerun
   succeeds on it, which is the safe direction.

**Sweep:** once every row has a `(simhash, stamp)`, `sweep_clusters`
runs over all of them in `decided_at` order with the constants for the
target algorithm. Rows carrying a non-target stamp (skipped or failed)
are in the sweep, so they keep clustering among themselves as today, and
the version gate inside `assign_cluster` keeps them out of the target
clusters.

**Dry run** writes nothing, structurally: the mode check sits before any
storage call, as in `rescore_perplexity_one`. It logs one
`RederiveDedupDryRunReport` on completion, aggregates only, every
percentile withheld below `rescore_distribution::MIN_ROWS_FOR_PERCENTILES`:

- rows enumerated, derived, reused, `not_derivable`, failed;
- distinct simhash values among derived rows;
- histogram of Hamming distance to the nearest cluster representative,
  in the buckets `0, 1-2, 3-4, 5-6, 7-8, 9-10, 11-12, 13-14, 15-16,
  17-20, 21-24, 25-32`;
- for each candidate tau in `DRY_RUN_CANDIDATE_TAU_HAMMING`: cluster
  count, singleton count, counts of clusters sized 2-9, 10-99, 100+, the
  largest cluster's size, and the share of rows in it (the shape of the
  table at the top of this document, so before and after are read the
  same way);
- at the constants' own tau: the number of multi-member clusters whose
  members all share one `tenant_id` and the number that span two or
  more. On the pilot a tenant is one contributor, so the first is
  resubmission or forking by one person and the second is the sybil case
  or a shared session. Counts only; no ids.

**Write mode** writes every row whose derived `(simhash, stamp,
cluster_id, cluster_size)` differs from what is stored, all four through
`update_trace_gate_decision_dedup` with a `DedupAssignmentWrite`, on the
tenant-scoped pool as the inline path does. Sizes are the sweep's final
per-cluster totals, computed before the first write (the existing
recluster pass's two-phase shape), so every member of a cluster is
written with the same total. A row that moves cluster takes the size of
the cluster it moves to, and the cluster it left is written with its
new smaller total in the same pass, because the sweep assigned every row
from scratch. Rows whose four values are unchanged are not written, so a
rerun over a converged corpus performs zero updates and its summary
reads `unchanged = rows`.

Cluster ids are minted fresh by the sweep (`Uuid::new_v4` per new
cluster, as today). The pass therefore rewrites every `dedup_cluster_id`
on its first write-mode run. Nothing keys on a cluster id across passes
(established above), and the old recluster route already re-mints them
on every run.

**Resumable and idempotent:** a crash mid-write leaves some rows on the
target stamp with the new value and some on the old; the next run reuses
the former without decrypting, derives the latter, re-sweeps everything,
and writes what differs. Two consecutive runs on a quiet corpus produce
identical `dedup_cluster_size` sets and the second writes nothing.

**Concurrency with inline writes:** a submission scored while the pass
runs is assigned inline against the pre-pass state and is not in the
pass's snapshot. The runbook step is to run the pass in a quiet window
and run it once more afterwards; the second run is a reuse-only sweep.

**Cost on the pilot:** about 1,800 decision rows. Per row the pass does
what the perplexity re-score does up to the scorer call — artifact
fetch, DEK unwrap, AEAD decrypt, envelope parse and render — and then a
hash instead of a model call. It is strictly cheaper per row than a
re-score; the re-score runbook (`docs/operator/perplexity-scoring-driver.md`)
is the throughput reference and the pass logs a completion line the same
way. Duration is not estimated here; the operator watches the log.

**Logging:** hash-only throughout. Completion line carries the counts
and, in dry-run mode, the serialized report. No simhash values are
logged row by row: a simhash is content-derived, and the report's
distinct-value count is the only place values are aggregated.

### 5. Two-phase rollout on the pilot

The `dedup_assign.rs` rule: the pass completes before the constant
flips, and the two never ship in one binary. Concretely:

**PR 1 (pass).** `trace_simhash_v1` / `trace_simhash_v2`,
`DedupAlgorithm`, `DEDUP_CONSTANTS_V2` (not referenced by the inline
path), `sweep_clusters`, `derive_dedup_signal`, `dedup_canonical_text`,
`list_dedup_rederive_rows`, the route, the tests, the measurement
script, the runbook. `DEDUP_SIMHASH_ALGORITHM` stays `fnv1a-2shingle.v1`;
the inline path still stamps and clusters as today. No migration.

**PR 2 (flip).** `DEDUP_SIMHASH_ALGORITHM = "fnv1a-3shingle-set.v2"`, the
inline path calls `trace_simhash_v2` and `DEDUP_CONSTANTS_V2`, the
`dedup_simhash.rs` unit tests assert against v2, and `DEDUP_CONSTANTS_V1`
is kept as the named constants of the v1 algorithm (it is what
`DedupAlgorithm::V1` maps to, and the pass can still target v1 for a
rollback). `LEGACY_DEDUP_SIGNAL_VERSION` is not touched: it is frozen,
and PR 2 is exactly the bump it was frozen against.

**Operator steps** (host: ingest runs as `tc_ingest_runtime_login`,
which owns no table; migrations are applied as the migrator role by the
operator, per `docs/operator/deployment.md` "Redeploying the binary"):

1. Before anything: run `scripts/operator/dedup-cluster-report.sh` as
   `trace_gate_driver` and keep the output. This is the "before".
2. Plant the control: submit one already-scored session again from the
   test tenant. Note its submission id locally.
3. `git diff --name-only <running build_commit> <PR 1 commit> -- migrations`
   prints nothing: PR 1 carries no migration, so this is a plain
   build-and-install. No migrator step, no grant step.
4. Install the PR 1 build. Confirm `/health` reports its commit.
5. Smoke: `POST /v1/admin/rederive-dedup?algorithm=fnv1a-3shingle-set.v2&dry_run=true&limit=5`
   with the admin JWT the perplexity runbook uses. Check the ack says
   `"mode": "dry_run"` and the completion line reports five rows.
6. Full dry run without `limit`. Read the report against the expected
   shape. Choose the threshold. If the chosen tau is not 8, that is a
   one-line change to `DEDUP_CONSTANTS_V2` and a rebuild of PR 1 before
   step 7; the dry run is re-run on the rebuilt binary so the written
   result is the one that was inspected.
7. Write mode, no limit. Wait for the completion line.
8. Run the report script again ("after, v2 rows"). Confirm: the
   largest cluster's share is no longer 75%; the control and its
   original share a cluster id; the stamp distribution shows the corpus
   on `events.v1+fnv1a-3shingle-set.v2` except `not_derivable` rows.
9. Install the PR 2 build (again no migration). From this moment inline
   writes stamp v2.
10. Write mode once more. This run reuses every stored v2 value and
    derives only the rows scored between steps 7 and 9. Confirm the
    summary's `derived` count is that window's row count.
11. `POST /v1/admin/recompute-contributor-caps` so the shadow cap
    columns are recomputed from the corrected sizes.
12. Report script a final time and file the before/after in
    `docs/superpowers/reports/`.

Between steps 7 and 9 the corpus is mixed again: new inline rows carry
v1 stamps and cluster only among themselves, so a resubmission of an
older session in that window gets `dup_pen = 1`. That is the module
docs' credit-event warning, and it is why nothing consumes the size
today matters: the window is harmless to any number anyone sees, and
step 10 closes it. Keep it short anyway.

Rollback: PR 2 rolled back to PR 1 puts inline writes back on v1 while
the corpus is on v2; those rows cluster among themselves until PR 2 is
reinstalled and step 10 repeated. A full rollback to a pre-PR-1 build
ignores the stamp it does not know, reads every v2 row as v2 via the
stored stamp, and refuses them against its own v1 rows; nothing fuses.
Re-deriving back to v1 is the same route with `algorithm=fnv1a-2shingle.v1`.

### 6. Credit in the meantime

Established above: `dedup_cluster_size` feeds two shadow columns that
nothing reads. Therefore:

- No withhold flag, no credit-quality era, no correction event. The
  `credit_withheld_reason` column (V25) is a label for ABAC withholding
  on a passing gate and is not touched.
- After step 10, `recompute-contributor-caps` (step 11) rewrites the two
  shadow columns from the corrected sizes, which is the only correction
  the wrong sizes need.
- The module docs' fallback ("withhold or flag credit for any decision
  whose `effective_signal_version` is not the build's stamp") becomes a
  stated precondition on the settlement sub-project rather than a column
  now: a reader that turns `dedup_cluster_size` into money must refuse a
  row whose stamp is not the build's `DEDUP_SIGNAL_VERSION`, in the same
  way `assign_cluster` refuses a candidate. That sentence goes into the
  `dedup_assign.rs` module docs in PR 1 so it is where the next reader
  looks.

### 7. Correction simhash stays on v1

`correction_simhash_from_plaintext` and the correction path in
`evaluate_and_record_gate` also call `trace_simhash`. Correction
clustering stores no stamp (#538), so flipping its hash would fuse old
and new values silently. In PR 1 both call sites are pointed at
`trace_simhash_v1` by name; PR 2 does not change them. Corrections are
short prose, where multiset and set semantics agree, so nothing is lost.

## Testing

**`dedup_simhash.rs` (unit, no I/O).** A deterministic synthetic-trace
generator with a small seeded LCG (no new dependency): `N` rendered
events in the `events.v1` shape. Scaffolding is modelled the way it
occurs in a real trace, as a **fixed set of about fifty literal lines**
(event headers, JSON key runs, path prefixes, a test-runner banner)
repeated until they make up about 70% of tokens, so that they contribute
a few hundred distinct shingles however often they recur; content is
drawn from a seeded vocabulary large enough that a trace has thousands
of distinct content shingles. That shape is what separates multiset
from set semantics: under v1 the repeated lines own the vote, under v2
they are a few hundred votes among thousands. Every assertion uses a
fixed seed, so each is a regression pin, not a statistical claim; the
expected-Hamming table above is what the pins were chosen against.
Assertions on v2:

- identical text: 0;
- resubmission (same events, different ids and timestamps — the render
  omits both): 0;
- A6 shim (a handful of nonce tokens appended): at most 2;
- light reword (2% of content tokens replaced): at most `tau_hamming`;
- late fork (last twentieth replaced): at most `tau_hamming`;
- unrelated content, same scaffolding: at least 20;
- same project: same scaffolding, the same 40% block of "file read"
  content in both, different conversation content (expected Hamming
  about 24): at least 14 for the pinned seed, and a property over 50
  seeds that the minimum across seeds stays at least 12;
- a regression pin that v1 on the unrelated pair from the same generator
  is at most 10, with the pilot numbers in the comment. It documents the
  defect; delete it when v1 is deleted.

**`dedup_assign.rs` (unit).** `sweep_clusters`: every member of a cluster
reports the same size; sizes sum to the row count; sweeping its own
output reproduces it; a mixed-stamp corpus never places two stamps in
one cluster; the existing `assign_cluster` tests unchanged.

**`trace_gate_service.rs` (unit).** `dedup_canonical_text` equals the
text `evaluate_trace` hashed before the refactor (a fixture envelope with
a pinned expected v1 value, so the refactor is proven to be a move).

**Ingest internal tests (`trace_commons_ingest_internal/tests.rs`).**
The pass's orchestration against the test gate service the perplexity
re-score tests use, extended with `derive_dedup_signal` over fixture
envelopes: dry run writes nothing and reports the fixture's known
distribution; write mode writes all four columns; a second write-mode
run writes nothing; a failing row keeps its old stamp; a
`digest-prefix.v1` row is counted `not_derivable` and untouched; an
unknown `algorithm` is a 400; a mistyped parameter is a 400; no DB
mirror and no artifact store are each a 503.

**PostgreSQL (`crates/trace-commons-server/tests/trace_corpus_pg_store.rs`,
run in CI against PostgreSQL 16).** `list_dedup_rederive_rows` on the
gate-driver pool returns rows across two tenants ordered by
`decided_at`, with the stamp column; the role's column grants cover the
query (the suite already asserts `has_column_privilege` for
`dedup_signal_version`, so the new query needs no new grant and the test
proves it by running as the role); the four-column write through
`update_trace_gate_decision_dedup` followed by a re-read shows the
stamp, value, and sizes, and the must-not-touch assertion still holds.

**Operator measurement script** `scripts/operator/dedup-cluster-report.sh`:
read-only `psql` as `trace_gate_driver`, touching only the columns that
role can read, printing counts only:

- rows with a cluster, cluster count, singletons, clusters of 2-9,
  10-99, 100+, largest cluster size and share;
- distinct `dedup_simhash` count and zero-value count;
- rows per `dedup_signal_version` (NULL shown as the legacy label);
- per-cluster Hamming distance from each member to the cluster's
  earliest-decided member, via `bit_count((dedup_simhash # rep)::bit(64))`
  (PostgreSQL 14+), summarized as min, median, p95, max over all
  members, and the median nearest-member distance inside the largest
  cluster.

Run before step 3, after step 7, and after step 10. Its output is the
table at the top of this document, so the before and after are directly
comparable.

## PR split

1. Pass, algorithms, constants, sweep refactor, tests, script, runbook.
   Inline behaviour unchanged. Deployable without a migration.
2. Constant flip and inline switch to v2. Deployable only after PR 1's
   pass has been run in write mode on the target deployment, which the
   PR body must state and the runbook must have recorded.

## Out of scope

- Wiring the embedding arm (`dedup_index_query` / `dedup_index_insert`),
  its persistence of the usearch key map across restarts, and its own
  calibration dry run.
- MinHash / Jaccard signatures and LSH banding.
- `correction_signal_version` (#538).
- Deleting `trace_simhash_v1` and the old `/v1/admin/recluster-dedup`
  route; both go when no row carries a v1 stamp, which the report script
  shows.
- Any settlement or credit-event change; the contributor-cap constants.
- Author-kind weighting and its `w`; specified only as the first
  escalation.

## Open questions

1. How much same-project overlap the pilot corpus actually has. The
   dry run's same-tenant multi-member cluster count answers it; this
   spec does not assume an answer.
2. Whether the planted control can be submitted through the normal
   contributor path on the pilot without tripping the per-contribution
   evidence rule for uninvited uploads (#706). The smoke-gate script's
   path is the fallback.
3. The embedding arm's threshold value. 0.03 is set from the
   unrelated-pair distribution; the near-identical-pair distribution on
   bge-large has not been measured on this corpus and is measured when
   the arm is wired.
4. Pass throughput on the pilot's decryptor. Not estimated; observed on
   the `limit=5` smoke and the full dry run.

## Self-review

- No placeholders: every threshold, name, route, parameter, and step is
  stated.
- Consistent: `tau_hamming = 8` and `tau_e_micros = 30_000` are the same
  in the signal section, the constants, and the rollout; the stamp
  literal `events.v1+fnv1a-3shingle-set.v2` is the same everywhere; the
  sweep is one function called from both routes.
- Sized for one implementation plan with two PRs, the second a
  constant flip gated on an operator step.
- Hash-only: the pass, its logs, its report, and the script carry no
  trace content, no contributor identity, no credentials, and no
  per-row simhash values.
