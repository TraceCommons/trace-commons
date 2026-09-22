# Cross-Trace Dedup: Signal Escalation — Design

Date: 2026-09-22
Status: draft for review
Parent: [`2026-09-21-dedup-simhash-recluster-design.md`](2026-09-21-dedup-simhash-recluster-design.md)
(the v2 hash, the re-derivation pass, the two-phase rollout). This spec is
the next iteration that spec's "Candidate signals" section scheduled: the
ordered escalations, author-kind weighting first and MinHash second, now
that the v2 dry run has been read.
Scope: `trace-commons-server` (`dedup_simhash.rs`, `dedup_assign.rs`,
`trace_gate_service.rs`, the ingest binary's re-derivation pass), one
operator script, one runbook; one migration only if the second escalation
is reached. No production code in this PR.

## What the v2 dry run showed

Measured 2026-09-22 on the pilot under `events.v1+fnv1a-3shingle-set.v2`,
full corpus, 1,756 derived rows, through the pass's own dry-run report.
Nearest-representative Hamming distance at `tau_hamming = 8`:

| bucket | 0 | 1-2 | 3-4 | 5-6 | 7-8 | 9-10 | 11-12 | 13-14 | 15-16 | 17-20 | 21-24 | 25-32 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| rows | 109 | 18 | 59 | 72 | 73 | 55 | 49 | 97 | 181 | 801 | 233 | 8 |

Would-be largest cluster by candidate tau: 4 -> 36, 6 -> 47, 8 -> 48,
10 -> 77, 12 -> 89, 14 -> 58 (v1: 522). At tau 8: 95 single-tenant
multi-member clusters and 3 multi-tenant; on the pilot a tenant is one
contributor.

Read against the shape the parent spec expected ("a large mass at 20+ and a
small mass at 0-6 with little in between"):

- Unrelated traces sit at 15-24: 1,215 rows, 69%. That is the mode the
  calibration table predicts for J 0.2-0.5, not the 28-32 of J 0.1 or below,
  so unrelated sessions from one harness still share a fifth to a half of
  their distinct shingles. Set semantics removed the multiset defect; it did
  not remove the shared vocabulary.
- Near-copies sit at 0-2: 127 rows, 7%. The planted control is among them.
- **405 rows, 23%, sit at 3-14 with no valley.** Every bucket from 3-4 to
  13-14 holds 49-97 rows. The v2 threshold at 8 cuts through the middle of
  a continuum, and the largest cluster grows monotonically with tau up to 12,
  which is what a continuum does and a two-mode distribution does not.
- The parent spec's synthetic same-project pair was assumed at about 24 /
  at least 12 over seeds; the pass's own unit test measured it at mean 19.6,
  min 10, max 26 over 50 seeds (`v2_long_trace_same_project_different_session_is_not_within_tau`).
  The estimate was optimistic by a bucket, and the pilot is worse than the
  synthetic.

The parent spec and the runbook both say what to do with a populated 8-14
band: escalate the signal, do not move tau. `#980` ships the v2 flip at
tau 8 as the floor; this spec is the escalation.

## Goal

1. Find out what the 3-14 band is, with a read-only measurement that never
   exposes trace text, before changing the hash again.
2. Specify escalation 1 (author-kind weighting) as a third `DedupAlgorithm`
   variant that fits the existing stamp, pass, and two-phase rollout without
   a migration.
3. Specify escalation 2 (MinHash / Jaccard) far enough that its migration,
   grants, storage, and comparison rule are decided, and state the
   measurement that decides whether it is needed.
4. State the acceptance criterion as a number in the dry-run report, and
   frame it against the credit asymmetry that `dedup_cluster_size` will
   carry once the contributor cap leaves shadow.

## Non-goals

- No change to `CANONICAL_RENDER_VERSION`, `render_event_text`, or the
  chunker's spans. The spans exist (#967) and are consumed as they are.
- No change to the embedding arm, its threshold, or its wiring.
- No change to `correction_simhash`; it stays pinned to v1 (#538).
- No settlement, no credit event, no change to `contributor_cap` constants.
- No deletion of v1 or v2; every stored row keeps naming its algorithm.

## 1. What the 3-14 band probably is, and how to find out

### Four hypotheses

Any of these produces rows at 3-14 under a 64-bit set-simhash, and they
call for different responses:

| hypothesis | mechanism | what it predicts in the band | response |
|---|---|---|---|
| H1: order-statistic tail | the histogram is a MINIMUM over every same-stamped representative formed so far (about a thousand by the end of the sweep). The lower tail of a minimum over a thousand draws from a J 0.2-0.5 distribution (expected 17-22, std 3.5-3.8) reaches 9-12 by chance alone | pairs look like any other unrelated pair: different tenants at the base rate, no length affinity, J near the corpus median | resolution, not weighting: only a wider signal (MinHash) narrows a minimum's tail |
| H2: same project, different session | two sessions on one repository share file reads, test output, and build output; the parent spec put this at J 0.2-0.6 depending on how much of each is shared file content | same tenant far above the base rate; high tool-result Jaccard with low prose Jaccard; shared tool-call lines | author-kind weighting |
| H3: forks and edited copies | a session resubmitted after diverging in its last tenth or fifth (J 0.8-0.9, expected Hamming 7-10) or lightly reworded | same tenant; high Jaccard on every author kind; token-count ratio near 1 | already the intended positive; the threshold question is where they end and H2 begins |
| H4: short traces | set semantics stop occurrence COUNTS from dominating; they do not stop the SHARE of distinct shingles from dominating. A short session has a few hundred content shingles against a few hundred harness shingles (the `event_type (tool):` prefixes, the harness's banners, `cargo` output), so two short sessions from one harness sit at J 0.3-0.5 with nothing in common | band rows are short on `total_chunk_count`; not tenant-affine; tool-result share of distinct shingles high | author-kind weighting helps where the shared part is tool output; a length floor below which the arm abstains is the fallback |

The pilot number that points at H2 and H4 over H1: 95 single-tenant
multi-member clusters against 3 multi-tenant at tau 8, with 109 rows at
Hamming 0. A tail of chance collisions would be tenant-blind. But the count
is at tau 8; the band above it has not been characterised at all, and the
histogram says most of the band (about 270 of 405 rows) sits above 8.

### Layer one: the driver-role query, stored columns only

The `trace_gate_driver` role holds column SELECT on exactly `tenant_id,
submission_id, decision_id, decided_at, perplexity_micros,
peak_perplexity_micros, novelty_score_micros, perplexity_passed,
novelty_passed, credit_quality_micros, dedup_cluster_id, dedup_simhash,
dedup_cluster_size` (V45), `total_chunk_count, chunk_count, chunks_capped`
(V47), `correction_simhash, correction_cluster_id` (V48) and
`dedup_signal_version` (V57) on `trace_gate_decisions`. It cannot read
trace text, object keys, or the V73 per-author perplexity columns. That is
the whole surface, and it is enough to classify the band against the two
control bands without leaving SQL.

`scripts/operator/dedup-cluster-report.sh` gains a `--band-profile`
section. It reproduces the pass's nearest-representative statistic from the
written v2 result: representatives are each cluster's earliest-decided
member, and a row's nearest representative is the minimum Hamming distance
over representatives decided before it (first-member linkage is what the
sweep does, so this is the same number the dry run histogrammed, not an
approximation of it). One query, `psql` as the driver role:

```sql
WITH decided AS (
  SELECT tenant_id, decision_id, decided_at, dedup_simhash, dedup_cluster_id,
         total_chunk_count, chunks_capped, novelty_score_micros, perplexity_micros
  FROM trace_gate_decisions
  WHERE dedup_simhash IS NOT NULL
    AND COALESCE(NULLIF(dedup_signal_version, ''), 'events.v1+fnv1a-2shingle.v1') = :'stamp'
),
representative AS (
  SELECT DISTINCT ON (dedup_cluster_id)
         dedup_cluster_id, tenant_id, decision_id, decided_at, dedup_simhash, total_chunk_count
  FROM decided
  ORDER BY dedup_cluster_id, decided_at, decision_id
),
nearest AS (
  SELECT d.tenant_id, d.decided_at, d.total_chunk_count, d.chunks_capped,
         d.novelty_score_micros, d.perplexity_micros,
         r.tenant_id AS rep_tenant_id, r.decided_at AS rep_decided_at,
         r.total_chunk_count AS rep_total_chunk_count, r.hamming
  FROM decided d
  JOIN LATERAL (
    SELECT r.*, bit_count((d.dedup_simhash # r.dedup_simhash)::bit(64)) AS hamming
    FROM representative r
    WHERE (r.decided_at, r.decision_id) < (d.decided_at, d.decision_id)
    ORDER BY hamming, r.decided_at, r.decision_id
    LIMIT 1
  ) r ON true
)
SELECT CASE WHEN hamming <= 2 THEN 'near-copy 0-2'
            WHEN hamming <= 8 THEN 'band 3-8'
            WHEN hamming <= 14 THEN 'band 9-14'
            WHEN hamming <= 24 THEN 'unrelated 15-24'
            ELSE 'far 25+' END AS band,
       count(*) AS rows,
       round(avg((tenant_id = rep_tenant_id)::int), 3) AS same_tenant_share,
       round(avg((abs(extract(epoch FROM decided_at - rep_decided_at)) < 3600)::int), 3) AS within_1h_share,
       round(avg((abs(extract(epoch FROM decided_at - rep_decided_at)) < 86400)::int), 3) AS within_24h_share,
       percentile_cont(0.5) WITHIN GROUP (ORDER BY total_chunk_count) AS median_total_chunks,
       percentile_cont(0.5) WITHIN GROUP (ORDER BY
         least(total_chunk_count, rep_total_chunk_count)::float
         / greatest(total_chunk_count, rep_total_chunk_count, 1)) AS median_length_ratio,
       round(avg(chunks_capped::int), 3) AS capped_share,
       percentile_cont(0.5) WITHIN GROUP (ORDER BY novelty_score_micros) AS median_novelty_micros,
       percentile_cont(0.5) WITHIN GROUP (ORDER BY perplexity_micros) AS median_perplexity_micros
FROM nearest
GROUP BY band
ORDER BY min(hamming);
```

Counts and aggregates only; no id, no simhash value, no tenant appears in
the output. The script withholds any band row with fewer than 20 members
(`rescore_distribution::MIN_ROWS_FOR_PERCENTILES`, the threshold the dry
run already uses) so a small band cannot be read back to a pair. Requires
PostgreSQL 14 (`bit_count`), as the existing script already does.

How to read it, band by band against the two controls:

| column | near-copy 0-2 (control) | unrelated 15-24 (control) | band reads as H2/H3 if | band reads as H1 if | band reads as H4 if |
|---|---|---|---|---|---|
| `same_tenant_share` | near 1 | base rate: the sum over tenants of the squared share of rows; on the pilot well under 0.2 | near the near-copy control | near the unrelated control | near the unrelated control |
| `within_1h_share`, `within_24h_share` | high (a resubmission follows its original) | low | high with same tenant: a contributor's backlog upload of one project's sessions, decided minutes apart | low | any |
| `median_total_chunks` | as the corpus | as the corpus | as the corpus | as the corpus | markedly below the corpus median |
| `median_length_ratio` | near 1 | wide | near 1 for forks (H3), wide for same-project (H2) | wide | near 1 (two short traces) |
| `median_novelty_micros` (of the later row) | low: its neighbour is in the index | corpus median | between: the neighbour shares text but is not near-identical on bge-large | corpus median | corpus median or above |

`decided_at` is when the gate decided, not when the session ran; for a
backlog upload it is the upload batch, which is exactly the affinity being
looked for. `novelty_score_micros` is the trace-level `1 - cosine` to the
nearest vector-index neighbour; the index holds every earlier trace, so it
is an independent second signal about the same pair, on the embedding arm.

What layer one cannot say: whether the shared material is tool output. The
columns have no author breakdown (the V73 per-author columns are
perplexities, not overlap, and are not granted to the role anyway). Layer
two answers that.

### Layer two: pair features inside the enclave path

The re-derivation pass already decrypts and renders every row inside
`TraceGateService::derive_dedup_signal`, and returns a 64-bit value and a
stamp. A dry run with `features=true` (a third query parameter, refused
outside `dry_run=true` with a 400, `deny_unknown_fields` as today) asks the
gate service for a `DedupMeasurementFeatures` block per row instead, and
the report gains a pair-feature section. The plaintext still never leaves
the method; what crosses back is fixed-size and content-derived in the same
sense the simhash is. Nothing in the block is stored, and nothing in it is
logged per row.

```rust
pub struct DedupMeasurementFeatures {
    /// Every algorithm this build can derive, so one pass reports the v2
    /// and v3 histograms side by side over the same rows.
    pub simhash: [(DedupAlgorithm, u64); DedupAlgorithm::ALL.len()],
    /// 128 min-hashes of the distinct 3-shingles, over all shingles and
    /// over the shingles of each author kind (majority-of-chars kind, as
    /// escalation 1 defines it). In memory only, 64-bit.
    pub sketch_all: [u64; 128],
    pub sketch_by_kind: [[u64; 128]; AuthorKind::COUNT],
    /// 128 min-hashes of the distinct rendered `tool_call` lines: the
    /// closest thing the envelope has to a project label (see below).
    pub sketch_tool_calls: [u64; 128],
    /// Distinct 3-shingles per author kind, and tokens per author kind.
    pub distinct_shingles_by_kind: [u32; AuthorKind::COUNT],
    pub tokens_by_kind: [u64; AuthorKind::COUNT],
}
```

About 4 KB per row, under 8 MB for the pilot corpus, held for the duration
of the pass and dropped with it.

The MinHash sketch is the measurement instrument here before it is a
storage decision (section 3): from two sketches the Jaccard of the
underlying sets is estimated to about +/- 0.04 without holding either set,
so the pass can characterise every pair without keeping 1,756 shingle sets
(which would be up to a gigabyte on the longest traces) in memory.

**There is no project label.** The envelope carries no repository,
workspace, or project identifier; `TraceContributionEnvelope::conversation_id`
is the only session-scoped id, it is documented as attribution-only ("must
never reach a gate"), and it is neither stored on `trace_gate_decisions`
nor granted to the driver role. The stand-in is what two sessions on one
project actually share: the same `tool_call (Read): <path>` and
`tool_call (Grep): <pattern>` lines. `sketch_tool_calls` is the min-hash
of the set of distinct rendered `tool_call` lines, so `J_calls` between a
pair is "how much of their tool invocation set is common", which is high
for same-project sessions and near 1 for forks, and near 0 for unrelated
work. It needs no new envelope field and no new column.

`sweep_clusters` gains `nearest_representative_index: Option<usize>` on
`SweepAssignment` (the row index of the representative the existing
`nearest_representative_hamming` was measured against), so the report can
pair each row with the representative that put it in its bucket. A pure
change with a unit test; nothing else reads the field.

The report section, per band (the same five bands as layer one), counts
and quantiles only, withheld below 20 rows:

- rows; `same_tenant_share` (the pass has `tenant_id` per row already for
  the existing tenant-span counts);
- p25 / p50 / p75 of `J_all`, `J_prose`, `J_tool`, `J_other`, `J_calls`
  between each row and its nearest v2 representative;
- p50 of the row's tool-result share of distinct shingles
  (`distinct_shingles_by_kind[ToolResult] / sum`), and of its
  `distinct_shingles` total;
- the v3 Hamming distance between the same pair, histogrammed in the
  existing buckets: **this is the row-for-row answer to "does author-kind
  weighting move the band"**, on the real corpus, before any flip.

And, over all target rows, the full v3 nearest-representative histogram
and the v3 `by_candidate_tau` table, computed by sweeping the v3 values
with the v3 constants: the same shape as the v2 dry run, so the two are
read side by side.

Expected signatures:

| band is | `J_all` | `J_tool` | `J_prose` | `J_calls` | tool share |
|---|---|---|---|---|---|
| H1 chance tail | at the unrelated control's | at the unrelated control's | at the unrelated control's | near 0 | as the corpus |
| H2 same project | 0.2-0.6 | high (0.5+) | low (under 0.2) | high | as the corpus |
| H3 fork / edit | 0.8+ | 0.8+ | 0.8+ | near 1 | as the corpus |
| H4 short traces | 0.3-0.5 | moderate | low | low | high, with a low distinct-shingle total |

### The rederive route, dry run, after this spec

| parameter | change |
|---|---|
| `algorithm` | accepts `fnv1a-3shingle-set-authorweighted.v3` (escalation 1) |
| `dry_run` | unchanged |
| `features` | new, bool, default false; only with `dry_run=true` (else 400); adds the pair-feature section and the cross-algorithm histograms; costs the sketches per row |
| `limit` | unchanged |

Write mode is untouched by `features`; the block is never computed on a
write.

## 2. Escalation 1: author-kind weighting, `fnv1a-3shingle-set-authorweighted.v3`

### Inputs that already exist

`chunker::parse_envelope_events` returns each event as a `RenderedEvent {
text, spans }` where `spans: Vec<AuthorSpan { start, len, kind }>` tiles
the text in chars: the `event_type (tool): ` prefix and the trailing
newline are `Other`, the content is `AgentProse` for `assistant_message`,
`ToolResult` for `tool_result`, and `Other` for everything else
(`user_message`, `reasoning`, `tool_call`, `routing_decision`, `feedback`,
`http_exchange`). `dedup_canonical_text` today joins the `text`s with
`"\n"` and discards the spans. `reasoning` is `Other` on purpose: its
presence follows the contributor's `--no-reasoning` choice, and the chunker
docs say a score must not move with a consent flag; the same reasoning
applies to a hash, so v3 does not treat it specially either.

### The function

`dedup_canonical_events(plaintext) -> CanonicalText { text: String, spans:
Vec<AuthorSpan> }` replaces `dedup_canonical_text` as the shared
render-and-join: the same join, with each event's spans appended and the
joining `"\n"` pushed as a one-char `Other` span, through the chunker's
span-merge rule so adjacent same-kind runs coalesce. For an envelope the
renderer cannot parse, the lossy-UTF-8 fallback is one `Other` span over
the whole text. `dedup_canonical_text` becomes `dedup_canonical_events(..).text`
and its existing pin (v2 of a fixture envelope is unchanged by the
refactor) stays.

`trace_simhash_v3(canonical: &CanonicalText) -> u64`:

1. Tokenize as v1/v2 do (lowercase, split on non-alphanumeric, drop
   empties), but keep each token's char offset and char length
   (`tokens_with_offsets`). `tokens` becomes a projection of it, so the
   three algorithms cannot tokenize differently.
2. Form overlapping 3-shingles with the same width fallback as v2.
3. Each shingle's kind is the **majority of its tokens' chars by kind**,
   counted over the tokens only (not the separators between them); a tie
   goes to the non-`ToolResult` kind. This is the rule
   `author_attribution::majority_kind` already applies to BPE tokens, for
   the reason given there: a first-char rule hands the first word after
   every `": "` prefix to `Other`.
4. Deduplicate shingle hashes as v2 does. A shingle that occurs under two
   kinds (the same three tokens once in prose and once in tool output)
   takes the **higher** weight: the set is keyed by hash, and the weight
   stored for a hash is the max seen. Deterministic regardless of
   occurrence order.
5. Vote with integer weights: `AgentProse` 4, `Other` 4, `ToolResult` 1.
   Same 64 accumulators, same sign-to-bit, same `0` for empty text.

The weights are constants of the algorithm, named by it. Changing them is
v4, as changing the width was v2.

### Why it should widen the gap

The simhash estimates the cosine between two feature vectors, and in the
vote a feature weight `w` enters the cosine as `w^2`. With prose at 4 and
tool output at 1 the tool-result contribution to the cosine is one
sixteenth of what it was under v2, per shingle. Same-project sessions share
tool output and differ in prose; near-copies share both. So the cosine of a
same-project pair collapses and the cosine of a near-copy is unchanged.

Expected Hamming over 64 bits (`64 * arccos(c) / pi`), with `t` the share
of a trace's distinct shingles that are tool-result-authored, `s_t` and
`s_p` the shared fraction of tool-result and prose shingles between the
pair, equal-sized traces. On the pilot, tool output is 53% of TOKENS
(#967's measurement); its share of DISTINCT shingles is higher, since prose
repeats itself less, so `t` 0.75-0.9 is the working range:

| pair | `t` | `s_t` | `s_p` | v2 expected Hamming | v3 expected Hamming |
|---|---|---|---|---|---|
| same project, sharing half its tool output | 0.75 | 0.5 | 0.05 | 23.9 | 29.5 |
| same project, tool-heavy trace | 0.9 | 0.6 | 0.05 | 20.3 | 26.9 |
| same project, sharing most tool output | 0.75 | 0.8 | 0.05 | 18.6 | 28.6 |
| fork in the last tenth | 0.75 | 0.9 | 0.9 | 9.2 | 9.2 |
| resubmission, A6 shim | any | 1.0 | 0.99+ | 0-2 | 0-2 |
| prose reworded (a third of prose shingles gone), tool output identical | 0.75 | 1.0 | 0.7 | 7.9 | 14.8 |

The last row is the trade-off, stated plainly: v3 is more sensitive to
prose edits than v2, because prose is now the majority of the norm. A copy
with its assistant messages rewritten and its tool output untouched moves
from inside tau to outside it. That case is the parent spec's A7
("the same work reworded"), assigned to the embedding arm, and v3 does not
change that assignment; but it does mean the simhash arm under v3 reads
"same authored content", not "same session". Section 3 shows why the
MinHash escalation can express this case where a single weighted hash
cannot.

Why `Other` stays at 4. `Other` content is the user's messages, the
agent's tool CALLS (the commands and edits it issued), reasoning, and the
few dozen distinct prefix shingles. The first three are authored and
discriminative: two sessions on one project issue overlapping `Read` calls
but different `Edit`s and different prompts. Down-weighting `Other` would
throw away the prompt, which is the most session-specific text in a trace.
A three-tier scheme (prose 4, other 2, tool 1) was considered and rejected
for now: there is no measurement to set the middle weight from, and layer
two's `J_other` is the measurement that would.

### Failure modes

- **A trace that is almost all tool output.** With `t` 0.98 and 2% prose,
  the prose still carries a quarter of the norm (`0.02 / (0.02 + 0.98 /
  16)`), so weighting helps, but the hash is decided among tool-result
  shingles and two such traces from one project sit at v2's distance.
  Weighting cannot separate what has no authored content to separate on.
  Layer two's tool-share quantile per band says how many such rows there
  are; if they are the band, the response is a **length-and-share floor**
  below which the simhash arm abstains (`assign_cluster` gets no simhash
  candidate for a row whose non-tool distinct shingle count is under a
  constant, and the row is a singleton on that arm), not a different
  weight. That floor is not specified here; it is the fallback if the
  measurement calls for it.
- **A trace with no prose and no tool output** (the lossy-UTF-8 fallback,
  or an envelope of `user_message` and `tool_call` events only): every
  shingle is `Other` at weight 4, and v3 equals v2 numerically. Pinned as
  a test. Harmless: the stamps differ, so the values never meet.
- **Mixed-kind shingles at event boundaries** are decided by majority of
  chars, and a token like `tool_result` from the prefix is `Other`, so a
  boundary shingle leans `Other` (weight 4). There are at most two such
  shingles per event, against hundreds of content shingles; they are the
  prefix scaffolding the parent spec already dismissed as a few dozen
  distinct shingles.
- **Renderer drift.** v3 reads spans the chunker produces; a change to
  `render_event` that moves a prefix boundary moves which chars are
  `Other`. That is already a `CANONICAL_RENDER_VERSION` bump by the
  chunker's own rule, and the stamp carries it.

### Fit with the existing machinery

- `DedupAlgorithm::V3`, name `fnv1a-3shingle-set-authorweighted.v3`,
  `DedupAlgorithm::ALL = [V1, V2, V3]` (the `FromStr` array today,
  promoted to a constant so the measurement block iterates it).
  `simhash(&self, text)` keeps its signature for v1/v2; v3 needs spans, so
  the enum gains `simhash_canonical(&self, canonical: &CanonicalText) ->
  u64`, and `simhash(text)` for v3 is defined as v3 over a single `Other`
  span (which equals v2 on that text). `derive_dedup_signal` calls
  `simhash_canonical`; the correction path keeps calling
  `trace_simhash_v1` by name.
- `DEDUP_CONSTANTS_V3`: `tau_hamming` starts at **8**, `tau_e_micros`
  30_000 as v2, `version: 3`. The expected-Hamming table is the same
  function of cosine as before; what changes is where the pairs land on
  it. The v3 dry run's `by_candidate_tau` decides the value before the
  write, as the runbook step 6 says.
- Stamp `events.v1+fnv1a-3shingle-set-authorweighted.v3`, same `BIGINT`
  column, same `DedupAssignmentWrite`, same four-column writer, same
  refuse-across-stamps rule in `assign_cluster`. No migration, no grant.
- The pass targets v3 with `algorithm=fnv1a-3shingle-set-authorweighted.v3`;
  reuse, not-derivable, failed, sweep, and write behave as for v2. A v2 row
  and a v3 row never cluster together, so the runbook's step order (pass
  in write mode to completion, then flip) is unchanged.
- `ACTIVE_DEDUP_ALGORITHM` moves to `V3` in the flip PR, and
  `the_inline_simhash_is_the_active_algorithm` moves with it.

## 3. Escalation 2: MinHash / Jaccard

### The sketch

Per trace, over the distinct 3-shingle FNV-1a hashes `x`, `k = 128`
affine permutations `h_i(x) = (a_i * x + b_i) mod (2^61 - 1)` with
`(a_i, b_i)` fixed constants of the algorithm (seeded from a named
literal, listed in the source, so the sketch is reproducible across
builds without a dependency), keeping `min_x h_i(x)` per `i`. Stored as
the **low 16 bits** of each minimum (b-bit MinHash): `128 * 16 bits =
256 bytes`.

Three sketches are stored, not one: over all shingles, over the shingles
of kind `ToolResult`, and over the rest (`AgentProse` and `Other`
together), using the same majority-of-chars kind rule as v3. 768 bytes
per row, one `BYTEA` column with a fixed layout (`[all 256 B][tool 256 B]
[prose+other 256 B]`), length-checked on read. Why the kind split:
section 2's last table row. "Tool output identical, prose rewritten" is
`J_tool` near 1 with `J_rest` low, and "same project" is `J_tool` 0.5
with `J_rest` low. One combined Jaccard cannot tell them apart; two can,
and only the sketch form makes the split cheap to store. Why the union
sketch is stored rather than derived: at 64 bits the union's min-hash is
the position-wise minimum of the two kind sketches (the in-memory
measurement block in section 1 uses exactly that), but the 16-bit trim
discards the ordering, so from stored sketches the union has to be its
own 256 bytes.

Jaccard estimate between two sketches: `m` matching positions of `k`,
`J = (m / k - 2^-b) / (1 - 2^-b)`; at `b = 16` the correction is a
fifteen-thousandth and the estimate's standard error is
`sqrt(J (1 - J) / k)`.

### Resolution, compared with the simhash

Separation is the gap between a pair's expected value and the J 0.6
same-project pair's, divided by the two standard deviations combined in
quadrature:

| pair | J | 64-bit simhash: expected Hamming, std | MinHash-128: estimate std | separation from J 0.6 (same-project upper end) |
|---|---|---|---|---|
| resubmission | 1.0 | 0, 0 | 0 | exact |
| fork in the last twentieth | 0.9 | 6.7, 2.4 | 0.027 | simhash 1.9; MinHash 5.9 |
| fork in the last tenth | 0.82 | 9.2, 2.8 | 0.034 | simhash 1.3; MinHash 4.0 |
| light reword | 0.85-0.95 | 4.6-8, 2.1-2.7 | 0.02-0.03 | simhash 1.5-2.5; MinHash 4.9-7.3 |
| same project, upper end | 0.6 | 14.7, 3.4 | 0.043 | -- |
| same project, typical | 0.4 | 20.0, 3.6 | 0.043 | -- |

A 10%-divergent fork sits at expected Hamming 9 on 64 bits, hardly more
than one combined standard deviation from a J 0.6 same-project pair; the
parent spec called it "caught more often than not, not reliably". On 128
min-hashes the same fork is four combined standard deviations from that
pair. That is the whole case for the escalation: the simhash's resolution
is a property of 64 bits, and no weighting changes it. Weighting moves
the same-project pairs down the J axis (section 2); MinHash makes the
axis finer.

### Comparison rule

`ClusterCandidate` gains `minhash: Option<&[u8]>` beside `simhash`, and
`DedupConstants` gains `tau_jaccard_micros` (and `tau_jaccard_tool_micros`
for the two-feature rule). For an algorithm that stores the sketch,
`assign_cluster` joins when

- `J_all >= tau_jaccard`, estimated from the two rows' union sketches,
  **or**
- `J_tool >= tau_jaccard_tool` and `J_rest >= tau_jaccard_rest`, the
  "identical tool output, rewritten prose" rule, whose thresholds start at
  0.95 and 0.3 and are set from the layer-two scatter,

with the simhash arm retained as it is (OR semantics), so a v4 row still
joins on Hamming alone if it ever helps; the version gate is unchanged.
`tau_jaccard` starts at **0.75**, the J the parent spec's table puts
between the fork-in-the-last-tenth (0.82) and the same-project upper end
(0.6), each more than three standard errors away. Both new constants are
per-algorithm and ship in the v4 constants.

Candidate gathering stays a scan of every cluster representative through
`list_dedup_signals` (the gate-driver pool), now selecting the sketch
column too: 768 bytes per representative, about 1 MB per inline decision
at the pilot's 1,300-odd clusters. At 100,000 representatives that is
77 MB per decision and the scan needs replacing by LSH banding (16 bands
of 8 rows: a J 0.8 pair is a candidate with probability 0.95, a J 0.5 pair
with 0.06), which is a `(band index, band hash) -> cluster id` table and a
second migration. Not in this spec; the crossover is stated so the scan is
not mistaken for the design.

### Schema, grants, storage, cost

- Migration `V75__trace_gate_decision_dedup_minhash.sql` (V74 is the
  highest on `main`; the local test database has phantom versions 30-34
  applied, so any number above 34 is safe, and V75 is next): `ALTER TABLE
  trace_gate_decisions ADD COLUMN IF NOT EXISTS dedup_minhash BYTEA;` --
  nullable, no default, no backfill; `NULL` means "not derived under a
  sketching algorithm", and the stamp says which. `GRANT SELECT
  (dedup_minhash) ON trace_gate_decisions TO trace_gate_driver;` in the
  same file, in V57's shape, because `list_dedup_signals` and
  `list_dedup_rederive_rows` on the driver pool read it. The runtime
  role's existing UPDATE covers the new column if its grant is table-wide;
  if it is column-scoped, the migration adds the `UPDATE (dedup_minhash)`
  grant. The pg suite asserts `has_column_privilege` for the driver role
  on the new column, as it does for `dedup_signal_version`. Registered in
  the explicit migration list in `db/postgres.rs`, or it never runs.
- Applied **as the migrator, before the binary**, per
  `docs/operator/deployment.md` "Redeploying the binary": the pilot's
  ingest runtime role owns no table and a DDL at boot crash-loops the
  service (2026-09-21, V63-V73). An older binary ignores the column.
- `DedupAssignmentWrite` gains `dedup_minhash: Option<Vec<u8>>`;
  `update_trace_gate_decision_dedup` writes five columns; the
  touches-only-dedup-columns pin is extended to the fifth.
- Algorithm `minhash128b16-authorsplit.v4`, stamp
  `events.v1+minhash128b16-authorsplit.v4`. A v4 derivation fills
  `dedup_simhash` with the v3 value (kept: the report script's Hamming
  statistics, the cheap prefilter, and a rollback target that needs no
  re-derivation) and `dedup_minhash` with the three sketches. One stamp
  names both columns; a row is on v4 only if both are present, and the
  pass treats a v4-stamped row with a `NULL` sketch as not reusable.
- Storage: 768 bytes per row; about 1.3 MB for the pilot, 768 MB per million
  rows. Not indexed.
- Pass cost per row: the same load, unwrap, decrypt, and render as today,
  plus 128 multiply-mods per distinct shingle. A 100,000-shingle trace is
  about 13 million multiply-mods, tens of milliseconds; the decrypt and
  render still dominate. Inline cost per decision: the same, once.
- Invertibility. A 64-bit min-hash is the FNV hash of one specific shingle
  and a dictionary of common shingles would identify it; 16 bits match one
  in 65,536 of any dictionary, so a stored position names thousands of
  candidate shingles and none. The same reasoning is why the in-memory
  measurement sketch (section 1) is 64-bit and the stored one is not.

### When it is worth it

When the layer-two report, read after the v3 dry run, shows either:

- the v3 band (rows at 3-14 under v3, nearest v3 representative) is still
  above the acceptance share in section 4, **and** its `J_all` quantiles
  sit at 0.3-0.7 (H1's chance tail or H2 residue at 64-bit resolution,
  where more bits are the only lever); or
- the two-feature signature (`J_tool` near 1, `J_rest` low) appears in the
  band or in the near-copy control at more than a handful of rows: a
  population of prose-rewritten copies exists on the pilot, and the
  simhash arm under any weighting will let them through.

Not worth it if v3 alone opens the valley: 768 bytes a row, a migration, a
second write column, and a second comparison rule are a cost the pilot
should not carry for a resolution it does not need.

## 4. Decision rule

**Order: measure, then escalation 1, then escalation 2 only if measured
necessary.** The measurement comes first because it is read-only, needs no
migration, and reports the v3 histogram on the real corpus before any v3
row is written, so the flip decision is made on the pilot's numbers and
not on a synthetic generator that already proved optimistic once. v3 is
implemented in the measurement PR because the measurement has to compute
it; that costs nothing extra.

Concretely:

1. **PR A (measurement + v3, no flip, no migration):** `DedupAlgorithm::V3`,
   `trace_simhash_v3`, `dedup_canonical_events`, `DEDUP_CONSTANTS_V3`,
   `DedupMeasurementFeatures`, `features=true` on the dry run,
   `nearest_representative_index`, the `--band-profile` script section,
   the runbook section. `ACTIVE_DEDUP_ALGORITHM` stays where #980 put it.
2. **Operator:** the layer-one query as the driver role (before), then
   `POST /v1/admin/rederive-dedup?algorithm=fnv1a-3shingle-set-authorweighted.v3&dry_run=true&features=true`,
   `limit=5` smoke first, then the full corpus. File both outputs in
   `docs/superpowers/reports/` (aggregates only).
3. **Decide on the v3 histogram**, against the acceptance criterion below.
   Pass: PR B. Fail: section 3's "when it is worth it" says whether the
   failure is one MinHash addresses; if it is, PR C and PR D; if it is
   not (the band is H4 short traces), the length-and-share floor is
   specified in a follow-up and v3 still ships as the better hash.
4. **PR B (flip to v3):** the parent spec's PR 2, one variant further:
   `ACTIVE_DEDUP_ALGORITHM = V3`, the tests move, the runbook's step 9-10
   run again. Gated on the v3 write-mode pass having completed on the
   pilot, which the PR body states.
5. **PR C (MinHash pass) and PR D (MinHash flip)**, conditional, in the
   same two-phase shape with V75 applied by the migrator before PR C's
   binary.

**Acceptance criterion** for a hash to ship as the inline algorithm, all
three required, read off its own dry run at its own tau:

- **Valley:** rows whose nearest-representative Hamming is in `3..=14`
  are **at most 5% of target rows** (about 88 of 1,756 today; today's v2
  figure is 23%). The near-copy mode (0-2) and the unrelated mode (15+)
  are then separated by a band holding less than a quarter of what it
  holds now, and every candidate tau from 4 to 14 gives a largest-cluster
  share within a few rows of every other, which is what "the threshold
  goes in the valley" looks like in the `by_candidate_tau` table.
- **Joins are near-copies:** among rows that JOIN a cluster at the
  algorithm's tau, at least 90% joined at Hamming 0-2. (Add
  `joins_at_0_to_2` and `joins_at_3_to_tau` to the report: two counters
  over the same sweep, no ids.) A join at 5 under a valley-shaped
  histogram is a fork or an edit; a join at 5 under today's histogram is
  as likely a same-project pair. This counter is what distinguishes the
  two once the valley exists.
- **Synthetic:** the same-project synthetic pairs, regenerated so the
  shared block is `tool_result` content (section 6), sit at minimum
  Hamming **greater than `2 * tau_hamming`** over 50 seeds (greater than
  16 at tau 8), and the unrelated pairs at 20 or more; the resubmission,
  A6 shim, light-reword, and late-fork pins hold at their v2 bounds.

For MinHash the same three, in Jaccard: rows with nearest-representative
`J_all` in `(0.5, tau_jaccard)` at most 5% of target rows; at least 90%
of joins at `J_all >= 0.9`; synthetic same-project pairs at `J_all` at
most 0.5 over 50 seeds with the fork-in-the-last-tenth at 0.8 or above.

## 5. Consequences for credit

`dedup_cluster_size` is read by one function today:
`contributor_cap::increment_micros(q, size) = q / size`, the per-decision
raw increment `r = q * dup_pen` that `run_recompute_contributor_caps_pass`
folds into the concave per-epoch cap (`K = 25.0` per 7-day epoch). Both
columns it writes are shadow; nothing reads them back (parent spec, "Who
consumes `dedup_cluster_size`"). The day the cap goes live, the size is
money, and the two errors are not the same size:

| error | what happens to `r` | who bears it | bounded by | detectable afterwards? |
|---|---|---|---|---|
| **false split** (a resubmission or fork lands in its own cluster) | the contributor earns `q` twice instead of `q / 2` | the commons: one `q` over-paid per incident | the cap's saturation: `effective(R) = K (1 - e^{-R/K})`, so the tenth resubmission in an epoch earns a fraction of the first; and each resubmission is an upload the same tenant made, visible as a same-tenant cluster once the signal improves | yes: the pair is at Hamming 0-2 under the next algorithm and the re-derivation pass finds it |
| **false merge** (two pieces of honest work land in one cluster) | each earns `q / n` for a cluster of `n` | the honest contributor: `q (n-1) / n` lost, with no action they can take | nothing: a contributor who uploads twenty sessions from one project and has them merged loses 95% of all twenty | only by re-deriving under a better signal, and by then the epoch's cap has been computed |

A false split is bounded, self-limiting under the cap, exploitable only by
repeating an upload, and correctable. A false merge is unbounded in `n`,
falls on the contributors the commons most wants (the ones who work on one
project for weeks), and is the parent spec's 522-cluster in miniature. The
threshold must therefore err toward splitting: a same-project pair inside
tau is the worse error, and the acceptance criterion's "at least 90% of
joins at 0-2" is the operational form of that preference. It is also why
the 95 single-tenant multi-member clusters at tau 8 are the first thing
the layer-one query should explain: under the cap, each of them is either
a caught resubmission or a contributor's project sessions being merged,
and today nobody knows which.

Two consequences carried forward to the settlement sub-project rather
than acted on here:

- The `dedup_assign.rs` precondition stands: a reader that turns
  `dedup_cluster_size` into money refuses a row whose stamp is not the
  build's. Each escalation is one more stamp that rule protects against.
- Under v3 and under the two-feature MinHash rule, "same authored content
  with identical tool output" is a join. A contributor who reruns the same
  session twice (same prompt, same repository state, the agent producing
  near-identical prose) is a legitimate near-copy on every arm; that is
  the one honest case the signal will merge, and it is correct to.

## 6. Tests, rollout, open questions

### Tests

**`dedup_simhash.rs` (unit, no I/O).** The synthetic generator gains a
`tool_result` block: `same_project_pair` today draws its shared 40% block
from `content_line`, which emits `assistant_message` lines, so it models
shared PROSE and is exactly the pair v3 is not meant to move. A new
`same_project_pair_tool_shared(layout, shared, a, b)` renders the shared
block as `tool_result (Read): w.. w..` lines and the own blocks as
assistant prose; both generators stay, and the v2 pins on the old one are
untouched. The generator also emits the spans (`CanonicalText`) alongside
the text, by kind of the line's event type, so v3 is tested on the same
shape the chunker produces. Pins on v3, fixed seeds:

- identical and resubmitted: 0; A6 shim: at most 2; 2% content reword:
  at most tau; late fork: at most tau (the v2 pins, re-asserted on v3);
- unrelated content, same scaffolding: at least 20;
- same project, tool-shared block: at least `2 * tau + 1` for the pinned
  seed, and the minimum over 50 seeds greater than `2 * tau`; the mean
  reported in the assertion message so the report can quote it;
- **v3 moves what v2 does not:** on the tool-shared pair, `d3 > d2 + 4`
  for the pinned seed (the widening is the claim; the pin is what makes
  it a claim about the mutation rather than about the seed);
- a text with a single `Other` span: v3 equals v2 bit for bit; a text
  with a single `ToolResult` span: v3 equals v2 bit for bit (uniform
  weight, whatever it is, is no weight);
- a shingle occurring once as prose and once as tool output takes weight
  4, tested by a text where the sign of one accumulator flips between the
  two readings;
- `tokens_with_offsets` projected to strings equals `tokens` on every
  fixture the v1 oracle test uses, so v1 and v2 are unchanged by the
  tokenizer refactor;
- the tie rule: a shingle with equal chars under `ToolResult` and
  `AgentProse` weighs 4;
- `DedupAlgorithm::ALL` round-trips names, and `"fnv1a-3shingle-set.v3"`
  and `"minhash128b16-authorsplit.v4"` (before PR C) parse as errors.

**MinHash (unit, PR C).** The sketch of a set equals the sketch of the
same set in another order; `J` of a set with itself is 1; of disjoint sets
0; over 200 random pairs at planted J from 0.1 to 0.9 the estimate is
within 0.1 of the truth in every case and within 0.05 in 95% (a seeded
property, fixed seed); the 16-bit trim changes the estimate by less than
0.002 on those pairs; the affine constants are the listed literals; the
stored layout round-trips and a wrong length is refused.

**`dedup_assign.rs` (unit).** `nearest_representative_index` is the row
index of the representative at the reported distance, `None` where the
distance is `None`; the existing sweep pins unchanged. PR C: the
two-feature rule joins on (`J_tool` 0.96, `J_rest` 0.35) and not on
(0.96, 0.2) nor (0.9, 0.35); a candidate without a sketch is judged on
Hamming alone; the version gate refuses a v4 candidate for a v3 row.

**`trace_gate_service.rs` (unit).** `dedup_canonical_events(..).text` is
byte-identical to the old `dedup_canonical_text` on the fixture envelope
(the existing v2 pin), and its spans tile the text exactly with the
joining newlines as `Other`; the lossy fallback is one `Other` span;
`derive_dedup_signal` under v3 equals `trace_simhash_v3` over
`dedup_canonical_events` of the decrypted fixture.

**Ingest internal tests.** `features=true` with `dry_run=false` is a 400;
`features=true` with `dry_run=true` over the fixture set produces a report
whose pair-feature section is withheld under 20 rows and, on a 25-row
fixture set with three planted pairs (a resubmission, a tool-shared
same-project pair, a prose-reworded copy), reports `J_tool` and `J_rest`
quantiles in the expected order for each band, and counts
`joins_at_0_to_2` and `joins_at_3_to_tau` correctly; no per-row value
appears in the serialized report (assert the JSON contains no 64-bit
integer outside the count fields and no `tenant`); write mode never
constructs the features block (the derivation is called with the plain
signature).

**PostgreSQL (`tests/trace_corpus_pg_store.rs`), PR C.** V75 adds a
nullable `BYTEA`; the driver role has `has_column_privilege` on it; the
five-column write round-trips 768 bytes; the must-not-touch assertion
holds for the fifth column; `list_dedup_signals` returns the sketch across
two tenants on the driver pool.

**Operator script.** `--band-profile` runs the layer-one query; a fixture
run against a database seeded with the pg suite's rows prints five bands
and withholds a four-row band.

### Rollout

The `dedup_assign.rs` rule again: pass completes before the constant
flips, and the two never ship in one binary. Per escalation:

**Escalation 1** -- PR A carries no migration (`git diff --name-only
<running> <PR A> -- migrations` prints nothing), so it is a plain
build-and-install on the pilot under the runtime role. Operator steps are
the runbook's 1-12 with `algorithm=fnv1a-3shingle-set-authorweighted.v3`,
plus the layer-one query at step 1 and `features=true` on the step 6 dry
run. PR B is the flip; step 10 (write mode once more after the flip)
closes the window; step 11 recomputes the shadow cap columns.

**Escalation 2** -- V75 is applied by the migrator before PR C's binary is
installed, with the driver-role grant in the file and the runtime-role
grant verified by the pg suite's privilege assertions before the PR
merges. PR C's pass targets `minhash128b16-authorsplit.v4`; its dry run
reports the Jaccard histogram in the buckets `1.0, 0.95-0.99, 0.9-0.95,
0.8-0.9, 0.75-0.8, 0.7-0.75, 0.6-0.7, 0.5-0.6, 0.3-0.5, 0-0.3` and
`by_candidate_tau_jaccard` for `[0.6, 0.7, 0.75, 0.8, 0.9]`, the analogue
of `DRY_RUN_CANDIDATE_TAU_HAMMING`. PR D flips.

Rollback for each is the parent spec's: a flip build rolled back to its
pass build puts inline writes on the previous stamp, rows cluster within
their stamps, nothing fuses; re-deriving back is the same route with the
previous algorithm name. A rollback across V75 leaves the column in place
and unread.

Least privilege on the pilot: nothing here needs a new credential. The
route is admin-JWT as today; the script is the driver role; the migration
is the migrator; the writes are the tenant-scoped pool. The features
block adds no storage read.

### Open questions

1. Which hypothesis the band is. This spec does not assume one; layer one
   and two answer it, and the decision rule branches on the answer.
2. Whether `Other` should carry a middle weight. Decided by `J_other` in
   layer two: if same-project pairs share `Other` shingles at a rate
   between their tool and prose rates, a three-tier v4 is the cheaper
   escalation than MinHash and should be measured before PR C.
3. The length-and-share floor for tool-only traces (H4): what count of
   non-tool distinct shingles is "too few to compare", and whether the
   arm abstains or the row is flagged. Not specified until the tool-share
   quantiles say the population exists.
4. Whether the planted control from the parent rollout is still in the
   corpus for the v3 dry run; if it was retained, no new control is
   needed, and it is the one pair whose v3 distance is known to be 0.
5. The inline cost of `list_dedup_signals` carrying 768-byte sketches on
   the pilot's CPU-bound host, and the row count at which the
   representative scan is replaced by banding. Stated as a crossover
   above; measured when PR C's dry run runs.
6. Whether `reasoning` presence, which follows a consent flag, should
   move the hash at all. It does today under v1 and v2 and continues to
   under v3; a resubmission with `--no-reasoning` toggled is a different
   canonical text on every algorithm. Out of scope, noted because v3's
   `Other` weight makes it no worse and no better.

## Self-review

- No placeholders: every weight, threshold, name, column, bucket set,
  parameter, and PR is stated; the two thresholds that are starting
  values (`tau_jaccard_tool` 0.95, `tau_jaccard_rest` 0.3) say what sets
  them.
- Consistent with the parent: same stamp grammar, same four-column
  writer extended rather than replaced, same first-member linkage, same
  version gate, same dry-run shape and withholding threshold, same
  two-PR-per-escalation order, `tau_e_micros` untouched, correction path
  untouched. The parent's "first escalation" and "second escalation" rows
  in its candidate table are what sections 2 and 3 specify.
- Where the code contradicted the starting assumptions: there is no project label to
  hash (the tool-call sketch stands in); the existing same-project
  generator shares prose, not tool output, and is not the pair v3 moves;
  the sweep reports a nearest distance but not the representative, which
  the pair features need.
- Hash-only: the query prints counts and quantiles; the report prints
  counts and quantiles; no id, no simhash, no sketch, no tenant, no text
  is logged or printed by anything here. The stored sketch is 16-bit per
  position for the invertibility reason stated.
- Sized for one implementation plan: PR A and PR B are one plan; PR C
  and PR D are a second, conditional plan with their migration and
  constants already named.
