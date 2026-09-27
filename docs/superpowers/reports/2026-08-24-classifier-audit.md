# Version the classifiers

Date: 2026-08-24
Scope: the Novelty and Substance gates, the residual-risk classifier, and the
scorecard metrics.
Sources: issues #128, #192, #199, #204, #205, #206, #210, #211, #219, #298,
#325, #326, #331, #373, plus a full read of the gate code paths.

## Summary

The Novelty and Substance gates do not fail because of one bug. Fourteen
issues describe the same defect from different angles. The system ships
classifications, but no unit in the system holds a classifier as a whole. The
canonical text format, the model weights, the thresholds, the decision
function, the calibration corpus, and the evaluation evidence live in six
places. No single version binds them. No repeatable test defends them.

The consequences, each confirmed by an issue against the tree:

- The model bake-off measured source format, not duplicate discrimination.
  Paragraph count alone scores AUC 1.000 on the A2.6 corpus; the selected
  model scores 0.936 (#204).
- The shipped perplexity floor gives back most of the separation the bake-off
  optimized. It admits about two thirds of near-duplicates (#205). The pilot
  then ships that floor at 0, so the substance gate is off entirely.
- A missing measurement reads as favorable evidence. Failed determinism
  replays become stddev 0.0 and pass the gate (#206). Absent replay fields
  scored 1.0 on 330 traces of which 0 replay (#298).
- A text-format change is a silent corpus migration. The canonical renderer
  has no version, `dedup_simhash` has no version column, and
  `gate_version_hash` does not cover the renderer (#211). The tree already
  records one silent break of this kind (`docs/trace-commons.md:141-147`).
- A policy change is invisible in the data. `ResidualPiiRisk` boundaries
  changed five times with identical wire values and no rubric version (#325).
- A merge reverted three landed fixes and stripped `#[test]` from the
  functions that guard them. The suite stayed green (#326).
- The composite score that credit keys on is never persisted, the vector
  index state at scoring time is never snapshotted, and the utility claim has
  never been checked against the outcome data already in the database (#199).

One more fact prices out every fix: a real evaluation needs a 27B model on
GPU hardware, and the calibration harness downloads the full corpus for a
`--count 3` run, cannot resume, and ignores its own `--seed` (#331). When one
measurement costs a GPU day plus a fragile operator run, nobody re-measures,
and every defect above stays in place.

This report proposes a direction change: stop treating these as separate
bugs, and give the system a first-class, versioned classifier unit with a
cheap, layered evaluation harness. The sections below give the evidence, the
synthesis, the target architecture, and an incremental path.

## 1. What the issues say together

### 1a. The evaluations validate the evaluator, not the classifier

#204 shows the A2.6 corpus entangles class with source format completely.
Every duplicate file has exactly one paragraph; novel files have 7 to 163.
Six no-model measures beat the winning model. There is no region of common
support, so no later analysis can recover the intended question from this
corpus. The paraphrase slice replaces the format confound with a length
confound of the same size.

#205 shows the deployed operating point discards the measured separation.
At the shipped floor, duplicate rejection falls from 90.7% to 35.7%, and 62%
to 70% of back-translated near-duplicates pass at every candidate threshold.
Model selection optimizes an AUC that the deployed threshold does not act on.

#199 shows the top of the chain is unfalsifiable. Credit keys on a composite
score that is not persisted. Novelty is relative to index state that is not
snapshotted. `task_success` is collected, stored, and read by nothing in the
scoring path. The claim "credits attest to utility" has never been tested.

#298 shows a scorecard metric that restated its input. `replayability` was
`replay.replayable` as a float, weighted 0.20, and 330 non-replayable traces
scored 1.0 and accrued 554.8 pending credit points. (#300 fixed this specific
metric; the pattern that produced it remains.)

### 1b. Missing data becomes favorable evidence

#206: a failed replay sample becomes `0.0`; the stddev of all-zero rows is
exactly `0.0`; the determinism gate passes with zero successful samples, and
`pick_winner` conditions everything downstream on that flag.

#128: migrations V23 to V25 existed in-tree but were not registered in the
hand-rolled runner. The live pilot ran without the gate-decision schema, and
the failure surfaces at request time, not at startup.

The shape is the same in both: absence has no representation, so absence is
coerced to whatever value the happy path expects.

### 1c. Semantics move; identifiers do not

#211: two independent canonicalizers feed the summary hash, the neighbor
search, the gate perplexity, the gate novelty, and the plaintext simhash.
Neither reads `tool_category`. Neither carries a version. Fixing the
collapsed rendering changes summary hashes, splits dedup clusters, and misses
the decision cache — unrepairably, because stored simhashes have no version
column and recluster reuses stored values rather than regenerating them.

#325: the residual-risk rubric changed five times (Apr 30, #179, #185, #223,
#267) behind unchanged wire values, and the label conflates four concepts:
content profile, scrub outcome, assessment confidence, and policy
disposition. #219/#373 show the cost of the conflation: "High" means the
redactor succeeded, and 0 of 99 real sessions reach acceptance.

The versioning that does exist is partial and unanchored. `gate_version_hash`
covers floors and model ids but not the renderer that produces the text they
score. `credit_quality` constants carry a version integer; the dedup
constants carry one too, but it is persisted nowhere. No decision row links
to the calibration run, the corpus digest, or the bake-off report that
justified its thresholds.

### 1d. Nothing defends decided behavior

#326: the #267 squash merge reverted the #223 polarity fix, the #225
cued-secret boundary, and the #236 scoring fix, and de-registered the tests
for all three. `cargo test` reported 70 passed, 0 failed. Classifier behavior
is encoded only as scattered code plus env vars, so there is no executable
statement of "what this classifier version must do" for CI to hold.

## 2. Synthesis

The deep problem, in one sentence: the system ships classifications whose
defining inputs are not versioned together, whose evaluations do not run
repeatedly, and whose scores can be neither reproduced nor falsified after
the fact.

Trace what "the substance classifier" actually is today:

    render_event_text            (unversioned)
      -> chunker                 (env knobs)
      -> Qwen 3.6 27B weights    (id string inside a hash)
      -> aggregate_chunked_perplexity
      -> floor from env var      (0 on the pilot: the gate is off)
      -> credit_quality V2       (constants versioned in code;
                                  evidence frozen in a docs/ JSON file)

The evaluation that justified this chain ran once, on a corpus that a
blank-line test classifies perfectly, under a decision rule that has already
needed three versions. The per-trace scores that made the #204 and #205
audits possible exist only because a report file happened to embed them.

Under measurement this weak, model work cannot converge. A better scorer is
indistinguishable from a worse one. A regression is indistinguishable from an
improvement. Each landed fix — the v3 baseline-dominance floor, the
replay-sufficiency score — patches one instance while the generating class
stays open, which is why the issue list keeps growing the same way.

The honest positive: the raw material is good. The trait seams exist
(`PerplexityScorer`, `Embedder`, `VectorIndex`). Fail-closed is already the
convention. `gate_version_hash`, `credit_quality_calibration_version`, and
`DECISION_RULE_VERSION` show the instinct is present. The per-trace score
arrays in the archived reports are exactly the right artifact in the wrong
place. What follows completes these instincts; it is not a rewrite.

## 3. The proposed architecture

### Name the unit: the classifier bundle

A bundle is one versioned artifact that closes over everything that
determines a score:

| component | today | in the bundle |
|---|---|---|
| canonicalizer | unversioned functions in two crates | id + version, part of the digest |
| model | id string folded into `gate_version_hash` | weights digest or pinned API model id |
| config | env vars at startup | floors, chunking, top_k, recorded |
| decision function | inline code | pure function, versioned |
| calibration corpus | tarball + sha in a doc | digest + admissibility report |
| evaluation evidence | frozen JSON in docs/ | report digest, per-trace scores retained |

The bundle id is a digest over all six. It extends `gate_version_hash` to
actually close over the renderer and the evidence. Bundles live in a registry
table. Every decision row carries its bundle id.

### Principle 1: persist facts, derive labels

Decision rows store measurements: per-chunk scores, neighbor ids and
similarities, index snapshot id and cardinality (#199's exact columns), scrub
outcome, and assessment confidence (#325's exact split). Labels — pass/fail,
risk tier, credit quality — come from pure functions of stored facts plus a
versioned policy.

This one split changes the economics. A policy change becomes: run the new
function over stored facts, in SQL, with no GPU. Reclassification becomes
reversible and auditable. Two identical payloads can no longer carry
different labels for reasons the row does not record.

### Principle 2: three evaluation tiers; the cheapest runs most often

**Tier 0 — golden traces. CI, milliseconds.** Fixed envelopes with pinned
canonical text, summary hash, simhash, rendered chunks, and reference-scorer
outputs. Any semantic drift breaks a pinned hash and forces an explicit
version bump. This tier would have caught the #211 collapse, the 2026-07-25
silent break, and every #326 reversion. Add a test-registration guard (assert
the count of registered semantic tests) so de-registration is visible.

**Tier 1 — corpus and policy contracts. CI, seconds.** A calibration corpus
is itself a versioned artifact and is admissible only if the preregistered
no-model baselines (byte count, word count, line count, paragraph count) all
score under a stated ceiling on it — #204's acceptance criterion, promoted
from a decision-rule patch to a corpus contract. Every calibration emits
operating-point tables (per-slice pass rates at the proposed threshold), not
only AUC — #205's ask. Missing measurements are typed as missing and fail
closed; every gate carries "successful samples >= required samples" — the
#206 fix as a rule, not a patch.

**Tier 2 — model evaluations. On demand, GPU.** Per-trace scores are the
primary artifact and land in the registry, not inside doc JSON. The
calibration harness is seeded, checkpointed, resumable, and bounded — #331's
acceptance list. Because facts persist, most questions of the #204/#205 kind
become SQL over the registry rather than new GPU runs.

**Standing ground-truth join.** The #199 preregistered analysis — composite
score against `task_success`, chronological, within tenant, bootstrap
uncertainty, no analysis before ~125 per class — runs as a scheduled report.
Two guards from #199's dry run are part of the contract: compute the gate
signal over content that excludes outcome-bearing fields, and report a length
covariate beside every headline AUC. If the covariate matches the gate score
to two decimals, the gate score measures the covariate.

### Principle 3: promote by shadow; migrate by dual-read

One active bundle, plus optional shadow bundles. A shadow scores the same
traffic; both rows persist with their bundle ids. Promotion requires Tier 0
and Tier 1 green, Tier 2 evidence, and a reviewed divergence report between
shadow and active. A bundle that changes the canonical text (#211) runs
dual-read on hashes and simhashes until backfill completes, with #211's own
regression as the exit criterion: an identical pre-deployment trace remains
an exact duplicate and joins the same cluster.

## 4. Incremental path

Each step lands alone and pays for itself.

1. **Stamp what exists.** Fold the renderer version into `gate_version_hash`.
   Add a version column to `dedup_simhash`. Add #199's three columns
   (`composite_score`, `vector_index_snapshot_id`,
   `index_cardinality_at_scoring`), prospective-only. Small migrations.
2. **Golden traces plus the registration guard in CI.** No GPU. Directly
   closes the #326 acceptance items and the #211 detection gap.
3. **Corpus contract and operating-point reporting in `gate-calibrate`.**
   Closes #204's acceptance criterion and #205's ask; fixes #206 as a rule
   (missing is not zero).
4. **Registry table; bundle id on every decision row.** Move calibration and
   evaluation artifacts out of `docs/` into the registry; docs keep the human
   summaries.
5. **Fact/policy split** for residual risk (#325's proposed shape) and for
   the gate labels; then shadow scoring.
6. **Pilot-bootstrap resumability (#331).** Worth doing on its own; it then
   becomes Tier 2's harness.

## What this buys, per issue

| issue | mechanism that answers it |
|---|---|
| #199 | facts persisted; standing preregistered join |
| #204 | corpus admissibility contract in Tier 1 |
| #205 | operating-point tables required by Tier 1 |
| #206 | missing-is-missing rule; sample-count gates |
| #210, #219, #373 | fact/policy split; policy re-run over stored facts |
| #211 | versioned canonicalizer in the bundle; dual-read migration |
| #298 | golden traces pin scorecard semantics |
| #325 | rubric version = policy version in the bundle |
| #326 | Tier 0 pins behavior; registration guard |
| #331 | Tier 2 harness requirements |
| #128, #192 | registry replaces hand-wired, doc-frozen state |

The alternative is the current course: fix each issue where it stands, and
wait for the next instance of the class. The class has produced at least
fourteen instances so far. It will produce more, because nothing in the
architecture stops it.
