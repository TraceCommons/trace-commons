# Partial Scoring Visibility — Design

Status: design only, no implementation. Written 2026-08-21 against `main`
(902a3011). A release is pending; the release verdict is at the end.

## The mechanism, in one paragraph

`finalize_plan` truncates a trace's chunk list to `chunk_cap` and records the
fact
(`crates/trace-commons-gate-enclave/src/chunker.rs:159-181`). The truncation
keeps chunks `0..cap` — the **beginning** of the trace — and discards the rest.
The envelope is still accepted, stored, and exported in full; only the gate's
view is a prefix. `chunks_capped` and `dropped_chunk_count` are set, the
orchestrator logs a hash-only `TraceChunkCapExceeded` warning
(`crates/trace-commons-gate-enclave/src/orchestrator.rs:79-87`), `chunks_capped`
flows to `GateDecision` (`crates/trace-commons-server/src/trace_gate_service.rs:634`)
and is persisted (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:48798`,
column added in `migrations/V37__large_trace_chunked_scoring.sql:11`).
`dropped_chunk_count` is **not** persisted anywhere — it exists only in a log line.

---

## 1. The real numbers

### The cap value and where it comes from

| Knob | Env var | Default | Source |
|---|---|---|---|
| chunks per trace | `TRACE_COMMONS_GATE_CHUNK_CAP` | `16` | `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:275` |
| packing target | `TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS` | `2048` | ingest.rs:271 |
| hard per-chunk max | `TRACE_COMMONS_GATE_CHUNK_MAX_TOKENS` | `3072` | ingest.rs:273 |

Token budgets are a char proxy at 4 chars/token
(`chunker.rs:15`, `APPROX_CHARS_PER_TOKEN = 4`).

**How much text 16 chunks is.** Packing is greedy over rendered events: a chunk
is flushed when the *next* event would push it past `target_chars` (8192), and
the oversized-event path splits at `target_chars`, so a chunk lands between
~8 KB and `max_chars` = 12,288 chars (`chunker.rs:120-155`). The cap therefore
admits roughly **128 KB–192 KB of rendered event text**, i.e. **~32k–49k tokens**.
The observed pilot number is consistent: a 169 KB trace produced 15 chunks
(~11.5 KB/chunk), one chunk under the cap.

**What that is a fraction of.** `MAX_TRACE_ENVELOPE_BYTES` is **16,000,000**
(`crates/trace-commons-protocol/src/trace_contribution.rs:77`), and its own
doc comment states the observed worst case: "a 42 MB raw session redacts to
roughly 2.8 MB of envelope" (lines 73-76). A 2.8 MB envelope is ~250 chunks
against a cap of 16 — **about 6% of the trace is scored, 94% is not.** The same
comment names the design intent plainly: "Scoring cost does not scale with it —
the gate chunk cap bounds how much of a trace is ever scored." The cap is a
deliberate cost bound, not a safety bound.

That distinction matters and is easy to get wrong. The TEE-OOM root cause the
chunker exists to fix is bounded by `target_tokens`/`max_tokens` — the
*per-request* budget. `chunk_cap` bounds the *per-trace* number of requests. No
backend-safety property depends on `chunk_cap`; removing it cannot reintroduce
the OOM.

### Pilot configuration

None of the chunk knobs appear in `deploy/pilot-gcp/ingest.env.template` (the
only gate knobs set there are `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0`
at line 64 and `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000` at line 66).
Absent an unrecorded systemd drop-in, the pilot runs **cap = 16**. Verify on the
host before acting on this: the deployed binary and the host checkout have
diverged before.

```
sudo systemctl show tracecommons-ingest -p Environment
sudo grep -r GATE_CHUNK /etc/tracecommons/
```

### Queries that answer "how many traces are affected"

Run as `trace_gate_driver` (cross-tenant SELECT is authorized by the permissive
policies in `migrations/V36__trace_gate_driver.sql:37-39`) or as a superuser.
Do not run these through the tenant runtime pool.

Affected fraction:

```sql
SELECT count(*) AS decisions,
       count(*) FILTER (WHERE chunks_capped) AS capped,
       round(100.0 * count(*) FILTER (WHERE chunks_capped) / nullif(count(*),0), 2) AS pct
FROM trace_gate_decisions;
```

Chunk-count distribution (a pile-up at exactly 16 is the cap; NULL is a
pre-V37 row and reads as 1):

```sql
SELECT coalesce(chunk_count, 1) AS chunks, count(*)
FROM trace_gate_decisions GROUP BY 1 ORDER BY 1;
```

Does capping correlate with failing the gate — the fairness question:

```sql
SELECT coalesce(chunks_capped,false) AS capped,
       count(*),
       count(*) FILTER (WHERE perplexity_passed AND novelty_passed) AS passed,
       round(avg(perplexity_micros)) AS avg_ppl,
       round(avg(novelty_score_micros)) AS avg_nov,
       round(avg(credit_quality_micros)) AS avg_q
FROM trace_gate_decisions GROUP BY 1;
```

How much was dropped — only estimable, because `dropped_chunk_count` is never
stored. `trace_object_refs.size_bytes` (V1 schema line 124) is the proxy:

```sql
SELECT d.chunks_capped,
       count(*),
       round(avg(o.size_bytes)) AS avg_envelope_bytes,
       max(o.size_bytes) AS max_envelope_bytes,
       -- scored ceiling is cap*max_chars = 16*12288 = 196608 chars
       round(sum(greatest(o.size_bytes - 196608, 0)) / 1048576.0, 1) AS unscored_mb_est
FROM trace_gate_decisions d
JOIN trace_object_refs o
  ON o.tenant_id = d.tenant_id AND o.submission_id = d.submission_id
 AND o.artifact_kind = 'contribution_envelope'
GROUP BY 1;
```

`unscored_mb_est` is the number that decides the cost question in section 5:
it is, to within the redaction/ciphertext framing error, the extra text a
higher cap would have to score.

---

## 2. Chunk selection is biased, and biased in the worst direction

`texts.truncate(cap)` keeps the first `cap` chunks (`chunker.rs:163`). Chunks
are produced by walking the envelope's `events` array in order
(`chunker.rs:76-101`, `chunker.rs:120-155`), and that array is chronological.
So the gate always sees **the opening of the session** and never the end.

This is not a neutral sample, for both signals:

- **Perplexity.** The representative value is the token-weighted mean NLL over
  scored chunks (`chunk_aggregate.rs:41-75`). The opening of an agent session is
  the most predictable part of it: system prompt, environment banner, directory
  listings, the first file reads. The hard-won part — the debugging, the failed
  attempt and the recovery — is at the end. Truncation to the prefix
  systematically *depresses* representative perplexity. On the pilot,
  `PERPLEXITY_FLOOR_MICROS` is enabled at 6,000,000, so this is not a cosmetic
  shift: it can flip `perplexity_passed`.
- **Peak perplexity.** Peak is `max` over scored chunks
  (`chunk_aggregate.rs:59-67`). A max over a prefix is a lower bound on the max
  over the trace. Long traces are structurally handicapped on the exact
  statistic that is supposed to find their best region.
- **Novelty.** Per-chunk novelty is `1 - max cosine similarity` against the
  tenant's existing chunk entries (`orchestrator.rs:155-172`). Session openings
  are the *most* repetitive content a contributor produces — same repo, same
  system prompt, same environment probe, every session. So prefix-only novelty
  is also biased down, and toward "duplicate".
- **Credit quality.** `q` is multiplicative in a perplexity term and a novelty
  term, with an anomaly term from `peak/representative`
  (`crates/trace-commons-server/src/credit_quality.rs:60`, active constants V2).
  Both inputs are biased down by truncation and the anomaly ratio is computed
  from two prefix-derived numbers, so the fraud flag is also evaluated on the
  wrong object.
- **Corpus/index integrity.** Only scored chunks are embedded and inserted into
  the vector index (`orchestrator.rs:174-190`). The dropped tail never enters
  the dedup index at all, so a later trace that genuinely duplicates that tail
  measures as novel. The cap does not just mis-score one trace; it leaves holes
  in the tenant's duplicate-detection substrate.

One thing the cap does **not** distort: cross-trace dedup simhash is computed
over the full rendered event text, uncapped
(`crates/trace-commons-server/src/trace_gate_service.rs:657-663`). So the dedup
clustering slice already sees the whole trace while the gate does not — the two
signals disagree about what the trace is.

The design spec's own stated goal is contradicted: "Score the **whole** trace —
no buried-signal loss for either the perplexity or the embedding-novelty signal"
(`docs/superpowers/specs/2026-07-09-large-trace-chunked-scoring-design.md:30-31`),
with the cap listed one bullet later as the cost control (line 45). The operator
runbook is worse — it states as fact that "large traces contribute their full
content to both signals (no truncation)"
(`docs/operator/large-trace-chunked-scoring.md:3-5`), which is false for any
capped trace. That sentence should be corrected regardless of which option
below is chosen.

Finally: `chunks_capped` is **written and never read**. No handler, no gate
logic, no review surface, no export filter consumes it. The only reads are
row-mapping (`db/trace_corpus_pg.rs:5788`, `:6227`) and tests. It is evidence
nobody has ever looked at.

---

## 3. What is already visible to a contributor

Nothing about chunking, and — with one exception — nothing about the gate.

| Surface | Route | Carries gate scores? | Carries coverage? |
|---|---|---|---|
| Submission status (CLI `status`) | `POST /v1/contributors/me/submission-status` | No | No |
| Account trace list | `GET /v1/account/traces` (ingest.rs:6891) | No (see below) | No |
| Account trace detail | `GET /v1/account/traces/{id}` | No | No |
| **Signed score attestation** | `GET /v1/contributors/me/score-attestation` (route ingest.rs:7024, handler `:13825`) | **Yes** | **No** |
| Competition read-back (Devfolio) | `POST /v1/admin/scores-by-submission` (ingest.rs:7313) | Yes, operator-scoped | No |

Details that matter:

- The status explanation the CLI prints is derived **only from
  `record.status`** (`ingest.rs:53952-53970`) — accepted / quarantined /
  awaiting-backstop / revoked. No gate number ever reaches it. There is,
  however, an existing free-form `explanation: Vec<String>` field on the wire
  type (`crates/trace-commons-protocol/src/trace_contribution.rs:5656`), which
  is the natural insertion point for a disclosure line.
- `submission_score` in the account read-back is **not** the gate score. It is
  the client-side value scorecard's `online_score`
  (`trace_contribution.rs:1026-1037`), computed over the whole envelope and
  therefore unaffected by the cap. Same for `credit_points_pending` /
  `credit_points_final`.
- `novelty_score` / `duplicate_score` on the list item come from
  `trace_derived_records`, and the account handlers pass an **empty** derived
  map (`ingest.rs:14476`, `:14519`), so those fields are always omitted today.
- The exception is real and is the sharpest edge in this whole report:
  `/v1/contributors/me/score-attestation` returns an **Ed25519-signed** JWS
  asserting the caller's `perplexity_micros`, `novelty_score_micros`,
  `credit_quality_micros`, and `gate_passed` per submission
  (`ingest.rs:13860-13868`; row shape at
  `crates/trace-commons-server/src/trace_corpus_storage.rs:1891-1897`; query at
  `crates/trace-commons-server/src/db/postgres.rs:4810-4828`). The query does not
  select `chunks_capped` or `chunk_count`, and the claim schema is
  `trace_commons.score_attestation.v1`
  (`crates/trace-commons-server/src/trace_score_attestation.rs:29`). Attestation
  signing is configured on the pilot. So the system will today hand a
  contributor a *signed* statement of a score computed on 6% of their trace,
  with nothing in the document indicating that.

---

## 4. Credit consequences and the recompute path

**Today, no settled credit depends on a gate score.** The chain:

- Gate pass triggers a `novelty_utility` credit-ledger emission
  (`ingest.rs:49973-50000`), whose per-pass delta is
  `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`, **`0`** in the pilot env
  template (`deploy/pilot-gcp/ingest.env.template:71`; documented as "Set to `0`
  during calibration; flip to configured value at cutover" —
  `docs/operator/env-reference.md:203`).
- `credit_quality` is shadow-mode by module docstring
  (`credit_quality.rs:1-4`, "Shadow-only: nothing here settles or pays").
- Cross-trace dedup penalties and per-contributor caps are likewise shadow-mode.
- The credit numbers a contributor actually sees come from the client-side
  scorecard, not the gate (section 3).

So the honest current answer to "is my credit wrong because of this" is: your
credit does not yet come from that score. That is a temporary property, and it
is the whole reason this is fixable cheaply *now*.

**Is there a recompute path?** Partially, and with two sharp edges.

`POST /v1/admin/rescore-perplexity` (`ingest.rs:7300`, handler at `:49021`)
re-loads the same ciphertext and re-runs `evaluate_perplexity_only`
(`orchestrator.rs:117-140`), which calls the *same* `chunk_and_score_perplexity`
with the orchestrator's **current** config. So **raising the cap and then running
this route does re-score historical traces at the new coverage** — the mechanism
already exists. But:

1. It updates only three columns — `perplexity_micros`,
   `peak_perplexity_micros`, `perplexity_passed` — on the latest decision row
   (`crates/trace-commons-server/src/db/trace_corpus_pg.rs:6002-6009`).
   **Novelty is deliberately untouched**, by design, because re-embedding would
   mutate the vector index. Half the bias from section 2 is therefore *not*
   repairable by this route.
2. It does not update `gate_policy_version` / `gate_version_hash` /
   `chunk_count` / `chunks_capped`. A re-scored row keeps a version stamp that
   no longer describes how its numbers were produced. That is a correctness
   problem for any later audit, and it gets worse if the cap changes, because
   the chunk knobs are inside the gate version hash's canonical string
   (`ingest.rs:5704`) — changing the cap legitimately changes
   `gate_version_hash`, and a re-scored row would then be stamped with a hash
   it was not scored under.

There is no equivalent novelty re-score route, and building one means deciding
what to do with existing index entries.

---

## 5. Options

### Option A — Raise the cap; disclose nothing

Set `TRACE_COMMONS_GATE_CHUNK_CAP` higher (64 or 128). Env-only; no code
change; hot for a restart.

- **Cost.** Scoring cost and latency are linear in scored chunks. Each chunk is
  one NEAR AI completion with `max_tokens: 1` and `echo + prompt_logprobs`
  (`crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs:180-185`), prompt
  ≤3072 tokens, 30 s HTTP timeout (`:381`), scored **sequentially**
  (`orchestrator.rs:95-104`). Per-token pricing is not recorded in this repo, so
  the honest quantification is a multiplier, and the `unscored_mb_est` query in
  section 1 gives it exactly: raising the cap multiplies total gate spend by
  `(scored + recovered) / scored`. Cap only binds on traces above ~128 KB; every
  smaller trace costs the same as today. If capped traces are the minority the
  prior investigation suggests, the corpus-wide multiplier is small even though
  the per-trace multiplier for the affected traces is up to 8x.
- **The real cost is not dollars, it is latency and failure probability.**
  Sequential scoring at cap 128 is 128 round trips; and evaluation is
  **fail-closed per chunk** — one scorer error fails the whole trace
  (`orchestrator.rs:99-102`). With NEAR AI's known intermittent 502s, whole-trace
  failure probability is `1 - (1-p)^n` in the chunk count. Raising the cap 8x
  makes long traces roughly 8x more likely to fail evaluation outright. That is
  a genuine argument for a *finite, moderate* cap and against removing it.
- **Side effect.** Changing the cap changes `gate_version_hash` (ingest.rs:5704).
  That is correct — it is a policy change — but it means pre- and post-change
  scores are not directly comparable, and it should be recorded like the
  27B model swap was.
- Does nothing about the residual bias: whatever the cap is, the traces above
  it are still scored on their openings.

### Option B — Make the selection cover the trace instead of its opening

Keep a finite chunk budget, but when `total > cap`, select `cap` chunks spread
across the trace (deterministic stride, or first-k plus last-k plus stride)
instead of `texts.truncate(cap)`. Roughly a dozen lines in `finalize_plan`.

- **Cost: zero.** Identical number of backend calls.
- Removes the systematic opening bias from both signals and makes peak a
  sample of the whole trace rather than of its preamble.
- Requires care in three places: `chunk_index` must keep meaning the position in
  the *original* plan (the vector-entry table is keyed on it —
  `docs/operator/large-trace-chunked-scoring.md:29-31`); the selection strategy
  must be added to the gate version hash's canonical string, since two different
  strategies at the same knob values would otherwise share a hash; and the
  representative aggregate becomes an explicit *estimate* of the whole-trace
  value rather than an exact prefix computation — which is more honest, not
  less, but should be said in the runbook.
- Does not, by itself, make a 6%-coverage score trustworthy. Coverage still
  matters; sampling only stops it from being adversarially chosen.

### Option C — Disclose partial scoring to the contributor

Carry `chunks_capped` / a coverage fraction into the contributor surfaces: an
`explanation` line on submission status
(`trace_contribution.rs:5656`, populated at `ingest.rs:53952`) and a coverage
field in the score attestation (schema bump to
`trace_commons.score_attestation.v2`, plus adding the columns to the query at
`db/postgres.rs:4810`).

- Honest, and cheap in code.
- **Disclosure alone is the worst option to ship first.** It converts a silent
  defect into an explicit, signed statement — "this score judged 6% of your
  trace" — with no answer to the question it immediately raises. And the
  recompute answer available today is partial at best (section 4): perplexity
  can be recomputed, novelty cannot.
- Also note that persisting a coverage number properly requires storing
  `dropped_chunk_count` (or the pre-cap total), which is currently discarded —
  a new nullable column.

### Recommendation

**B, then A, then C — in that order, and treat them as one slice.**

1. **B (selection)** first, because it is the only change that costs nothing and
   strictly improves the score's meaning. A prefix is an indefensible sample; a
   spread sample is a defensible one.
2. **A (raise the cap)** second, to 64 rather than "remove". 64 chunks is
   ~512 KB–768 KB of rendered text, which covers essentially every real trace
   short of the 2.8 MB worst case, while keeping the fail-closed blast radius
   and the per-trace latency bounded. Removing the cap entirely is the wrong
   call *because* evaluation is fail-closed per chunk — an uncapped 250-chunk
   trace would mostly fail, which is a worse outcome for the contributor than a
   coverage-limited score. Do this as an env change on the pilot first and
   watch the gate failure rate before changing the compiled default.
3. **C (disclose)** last, once the number being disclosed is one we are willing
   to defend, and disclose *coverage* ("scored N of M segments") rather than a
   bare boolean.

Alongside: persist `dropped_chunk_count` (or the pre-cap chunk total) so the
"how much did we drop" question stops being unanswerable from the database, and
fix the false sentence at `docs/operator/large-trace-chunked-scoring.md:3-5`.

---

## What should and should not block the release

**This should not block the release.** Ship, then follow up.

Reasoning, stated plainly:

- No settled credit is derived from a gate score today. The emission delta is
  `0` on the pilot, `credit_quality`, dedup penalties, and contributor caps are
  all shadow-mode, and the credit numbers contributors actually see come from
  the client-side scorecard. Nobody is being paid wrong right now.
- The truncation is server-side and env-tunable. It is not coupled to the
  artifact being released, and nothing in the release makes it better or worse.
  Holding the release does not reduce anyone's exposure by a day.
- The fix worth making (Option B) is a behavioral change to gate scoring that
  moves `gate_version_hash`. Rushing that into a pending release is how you get
  a scoring change nobody calibrated shipping next to an unrelated client
  change. It deserves its own slice with its own before/after distribution.

**What must block something else — the credit cutover, not this release.** Do
not flip `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` off zero, and do
not promote `credit_quality` or the contributor caps out of shadow mode, until
B and A have landed and capped traces have been re-scored. The moment gate
scores settle to credit, every capped trace becomes a contributor with a
legitimate grievance and a partially-unavailable recompute path.

**One thing worth doing this week, independent of the release.** Run the
section-1 queries. The whole severity assessment currently rests on an
unquantified "affected by frequency" claim. If capped traces are 2% of the
corpus, this is a normal follow-up slice; if they are 30%, the credit cutover
gate above is the most important sentence in this document. Nothing about
running a read-only query needs to wait for a release.

**Consider, but do not require, before shipping:** a one-line disclosure in the
score attestation is *not* worth rushing (Option C's ordering argument), but
being aware that the endpoint signs prefix-derived scores should inform how the
attestation is marketed until the fix lands. If anything external is about to
describe attestations as "a signed statement of your trace's quality", that
wording is currently wrong for large traces.

---

## Open questions for a human

1. **Cap value.** 64, or something else? The tradeoff is coverage against
   fail-closed whole-trace failure probability under NEAR AI's flaky 502s. This
   needs the observed per-chunk error rate, which is in the pilot logs at
   `/var/log/tracecommons/ingest.log`, not the journal.
2. **Should fail-closed-per-chunk survive a higher cap?** At 64+ chunks, a v1
   "any chunk error fails the trace" rule may be the binding constraint. The
   alternative — score the chunks that succeeded and record the shortfall — is a
   policy change with its own disclosure question, and it is out of scope here.
3. **Re-score historical capped traces?** Perplexity can be recomputed via the
   existing admin route; novelty cannot without a new route and a decision about
   existing vector entries. Do we recompute perplexity only (leaving a row with
   mismatched-vintage signals), do both, or neither?
4. **Version-stamp drift.** The re-score route leaves `gate_policy_version` /
   `gate_version_hash` untouched while changing the numbers
   (`db/trace_corpus_pg.rs:6002-6009`). If the cap changes, this becomes a
   provenance bug. Fix the route to stamp the current version, or accept the
   drift and document it?
5. **Attestation schema.** Does adding coverage require
   `trace_commons.score_attestation.v2`, and does anyone verify v1 signatures
   today in a way a schema bump would break?
6. **Do capped contributors get anything retroactively** when credit goes live —
   a re-score, or an acknowledgement? This is a policy call, not an engineering
   one.
