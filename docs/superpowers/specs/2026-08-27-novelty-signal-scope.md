# Scoping the novelty signal

The gate's novelty floor rejects essentially everything. This document is not
a design for fixing it. It is the investigation that has to happen before
anyone can say what "fixing it" means, because the purpose of the signal has
never been written down and neither of the two duplicate detectors in the tree
has ever been validated.

Written against pilot data measured 2026-08-27. Related: #446 (tracking),
#444, #204, #205, #211, #199.

## 1. What the gate does today

Two independent duplicate signals exist. One gates; the other does not.

**Embedding novelty — live, gating.** `EnclaveGateOrchestrator::evaluate`
(`gate-enclave/src/orchestrator.rs:157`):

1. Chunk the envelope's rendered events (target 2048 tokens, max 3072, cap 16).
2. For each chunk, embed with `BAAI/bge-large-en-v1.5` (1024-dim) via
   `embed_chunk_mean_pooled`.
3. Query top-k (pilot: 5) nearest neighbours in the tenant's usearch index.
4. Per-chunk novelty is `1 - max(cosine_similarity)`, clamped at 0; when no
   neighbour exists it is exactly `1.0`.
5. Aggregate per-chunk novelty to a representative and a peak.
6. `novelty_passed = novelty_score_micros >= novelty_floor_micros`, pilot
   floor `500_000` (cosine 0.50).
7. **Only if both floors pass** is the embedding inserted into the index.

**Simhash clustering — shadow, non-gating.** V40 (`dedup_simhash`,
`dedup_cluster_id`, `dedup_cluster_size`), a 64-bit token simhash over the
enclave canonical event text, with `dup_pen = 1 / cluster_size`. It records a
verdict and changes nothing.

## 2. What the pilot evidence supports

`tenant-zaki-pilot`, excluding `skip_duplicate` rows (which carry no real
score and serialise as passes — see #444).

**The floor sits far above the distribution.** Last 14 days, 226 decisions:

| floor | would pass | pct |
|---:|---:|---:|
| **500000 (current)** | 10 | **4.4%** |
| 300000 | 20 | 8.8% |
| 250000 | 46 | 20.4% |
| 200000 | 77 | 34.1% |
| 150000 | 197 | 87.2% |
| 100000 | 226 | 100.0% |

53 percentage points of mass sit between 0.15 and 0.20 novelty. Half the
corpus has a max cosine similarity of roughly 0.80-0.85 against something
already indexed, tightly bunched.

**It is a step, not a decay.** Weekly p90 was a perfect `1000000` every week
until 2026-08-24, then `214984`. A novelty of exactly 1.0 is the no-neighbour
case. Early traces arrived at a near-empty index and were passed for free; the
index is now populated, and that week's volume (151 decisions, more than the
prior three months combined) is what exposed it. Nothing regressed.

**The signal discriminates, weakly.** Refereed against simhash, 242 decisions:

| simhash verdict | decisions | p50 novelty | p90 novelty |
|---|---:|---:|---:|
| clustered | 210 | 184002 | 293656 |
| singleton | 32 | 230268 | 1000000 |

Singletons score higher at median and tail, so the signal measures something.
But **the singleton median (230268) sits below the clustered p90 (293656)**.
No threshold separates these cleanly.

**86.8% of pilot traces are simhash-clustered** (210 of 242). A gate accepting
4.4% against an honest target near 13% is wrong by a factor of three, not
twenty — and wrong in a way that also rejects most genuine singletons.

## 3. The problem underneath: there is no ground truth

Section 2 used one unvalidated signal to referee another. That is the best
available evidence and it is not evidence of correctness, only of agreement.
Both could be wrong in the same direction — and they share an input, so a
common-mode failure is not hypothetical.

- Embedding novelty was never calibrated. The 2026-05-14 recalibration report
  says so in its own "what this report does not claim" section: *"the
  novelty-floor path is unmeasured at pilot launch and should be validated
  against the first real traces."* It never was.
- Simhash clustering has never been validated either. It runs over the same
  canonical text as the embedder.
- #204 established that the A2.6 corpus cannot measure duplicate
  discrimination and that its paraphrase slice is not a usable control.

**No labelled set of "these two traces are the same work" exists anywhere in
this repository.** Without one, every floor is a preference, no rejection can
be defended to a contributor, and there is no way to tell a better signal from
a differently-wrong one.

This is the finding. Everything below is downstream of it.

## 4. Two mechanical hypotheses to test before redesigning anything

Both are cheap. If either dominates, "the embedder cannot tell coding traces
apart" is the wrong conclusion and a redesign would be solving the wrong
problem.

### H1: the input has collapsed (#211)

`render_event_text` (`gate-enclave/src/chunker.rs:66`) emits
`"{event_type} ({tool_name}): {content}"` and reads neither `tool_category`
nor `side_effect`. #211 documents that structurally different traces can
therefore receive byte-identical canonical text.

Beyond that specific bug: every trace's canonical text is dominated by
identical scaffolding — the same event-type prefixes, the same tool names, the
same shell-output shapes. The embedder is being asked to distinguish documents
that are, by construction, mostly the same tokens.

**Test.** `trace-commons-gate-calibrate canonical-text --input <envelopes.jsonl>`
implements the first half and is offline: no database, no embedder, no model
weights. It calls the production `render_event_text`, so it measures what the
gate actually embeds rather than a reimplementation of it. Reports the
scaffolding share of the canonical text (per-envelope percentiles and a
size-weighted corpus figure) and counts how many events render to
byte-identical strings — #211's claim, as a number.

The second half — pairwise cosine similarity with and without scaffolding —
needs the embedder and is deferred with H2, which needs it too.

### H2: mean-pooling compresses toward the centroid

`bge-large-en-v1.5` caps at 512 tokens. `embed_chunk_mean_pooled`
(`gate-enclave/src/embedder.rs:15`) splits a chunk into 512-token windows and
averages the resulting vectors. At the pilot's 2048-token chunk target that is
**4 windows averaged**, up to 6 at the 3072 maximum.

Averaging several windows of same-domain text pulls every vector toward the
corpus centroid and compresses the usable similarity range — which is exactly
the shape observed: everything bunched at 0.80-0.85 similarity. This would
happen whether or not the underlying traces are alike.

**Test.** For a sample, compare pairwise similarity of mean-pooled chunk
vectors against (a) single-window vectors from the first 512 tokens and
(b) max-similarity over windows instead of similarity of the mean. If the
range widens materially, the aggregation is the problem, not the embedder.

Note these are separable and both may hold. H2 is testable offline against the
existing index; H1 needs the canonical texts, which the enclave holds.

## 5. The smallest labelled set that would settle it

Labelling is the unblocking step and it does not need to be large.

**Unit.** A pair of traces, labelled: same work / related but distinct /
unrelated. Pairs, not individual traces — "novel" is not a property a trace
has alone, and the current gate's central confusion is treating it as one.

**Sampling.** Stratify by where the two signals disagree, because agreement
regions teach nothing:
- simhash-clustered and high embedding novelty
- simhash-singleton and low embedding novelty
- both agree duplicate, both agree novel (controls)

A few hundred pairs, weighted toward the disagreement cells, is enough to
estimate discrimination for both signals and to place a floor with a stated
error rate instead of a round number.

**Who labels, and what may be labelled.** A maintainer labels. The set
includes external-contributor traces as well as dogfooding ones: excluding
them would bias the labelled set toward exactly the population whose
behaviour is already best understood, and the singleton cell is small enough
(32 of 242) that dropping any of it costs real statistical power.

The basis is `ConsentScope::DebuggingEvaluation`, which is what calibrating a
gate against real traces is. Two constraints follow and are not optional:

- **Restrict the set to traces whose consent scopes include
  `DebuggingEvaluation`.** A trace carrying only `BenchmarkOnly`,
  `RankingTraining` or `ModelTraining` is out of scope for a human read and
  must be filtered out, not assumed in.
- **Exclude anything quarantined or awaiting privacy review.** Those are held
  precisely because their residual-risk classification is unresolved, and a
  calibration read is not the process for resolving it.

Recorded as an operator decision: the exclusion originally proposed here was
overruled by the repository owner, with the scope basis above.

**What it produces.** An AUC for each signal, a floor with a defensible error
rate, and — the part that matters most — the ability to say which of the two
signals is worth keeping.

## 6. The decision this scoping exists to inform

Once sections 4 and 5 are done, the purpose question can be answered on
evidence rather than intuition. The options and what each implies:

**Dedup — do not pay twice for the same work.** Then simhash is the better
candidate on current evidence: no embedder, no vector index, no per-chunk
inference, and it produced the more usable verdict in section 2. The work is
promoting it from shadow to gate and retiring or demoting embedding novelty.
Smallest scope. Note it inherits H1: simhash reads the same canonical text.

**Quality — is this trace worth collecting.** Then novelty is the wrong proxy
and always was: a trace can be unlike anything indexed and still be worthless.
This is the largest scope and it is adjacent to #199, which points out that
the utility claim on the landing page has never been testable. Defining
quality is a product question before it is an engineering one.

**Both, conflated.** The gate currently produces one number that is asked to
mean both, which would explain why it satisfies neither. Separating them means
distinct signals, distinct floors, and distinct contributor-facing
explanations — a rejection for "we already have this" and a rejection for
"this is not useful" are different messages and a contributor deserves to know
which one they got.

## 7. What this does not cover

- The floor value itself. Retuning 500000 to roughly 270000-280000 to match
  the 13.2% singleton rate is a cheap mitigation and strictly better than an
  uncalibrated round number, but it is a mitigation and should not be
  mistaken for the outcome of this work.
- Perplexity. It is healthy on current data (93.9-97.2% pass) and is not the
  binding constraint. #205 covers its own calibration problem.
- The chunk cap. #444's measured conclusion is that capping is not driving
  outcomes; capped traces currently pass marginally better than uncapped.

## 8. Sequencing

1. H1 and H2 tests. Cheap, offline, and either result changes what comes next.
2. Labelled pair set, stratified by signal disagreement.
3. Purpose decision, on the evidence from 1 and 2.
4. Implementation, scoped by that decision.

Only step 4 is engineering work of any size, and it should not start before
step 3.
