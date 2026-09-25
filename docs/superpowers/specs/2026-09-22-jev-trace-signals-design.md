# Semantic Trace Signals from a System One Model (TypeSafe Jev) — Design

Date: 2026-09-22
Status: **tabled 2026-09-22.** Hosted Jev is incompatible with Trace Commons'
privacy goals. This work resumes only once an open-source System One class
model runs on NEAR infrastructure. The experiments and combinations below are kept as the design to
pick up then. Nothing here is wired into ingest, the gate, or credit.
Scope: design only. No code, no migration. Records two offline experiments and
proposes ways to combine typed semantic judgments with the existing scoring
signals (perplexity, novelty, dedup cluster, replay sufficiency, credit
quality).

## Why

Every signal the gate computes today is statistical: token perplexity,
embedding distance, simhash distance, event and content counts. None of them
knows what a session *is*. The calibration reports show where that hurts:

- Whole-trace perplexity tracks length more than difficulty. On 40 public
  Swival sessions, Spearman rho vs length was -0.54 and vs rated difficulty
  0.05 (`2026-09-18-per-author-perplexity-shadow-design.md`).
- Perplexity cannot separate conversational junk from work. Under Qwen3.8-27B
  a greeting-only envelope scores 3.57, inside the range of real traces; the
  floor was set to refuse only structurally empty envelopes
  (`reports/2026-09-19-perplexity-floor-calibration-qwen3-8.md`).
- Novelty (1 - max cosine on bge-large) barely separates near-copies from
  singletons (`reports/2026-09-21-novelty-floor-calibration.md`).
- `EpisodeCategory` (`crates/trace-commons-protocol/src/insights_cards.rs`)
  has exactly one provenance, `UserReported`. Nothing infers it.

TypeSafe's Jev is a "System One" model: it takes a text or JSON `state` plus a
map of typed questions and returns calibrated answers instead of generated
text. Three primitives: **Choice** (one of N options, with a probability per
option and a confidence), **Noul** (probability a yes/no condition holds), and
**Score** (probability-weighted position on up to 10 described levels). All
questions in a request are evaluated in parallel against one state. Pinned
model `jev-1.13.0`; 32k tokens for state plus the longest question; USD 0.042
per million input tokens, output free.

The question this doc asks: can typed semantic judgments cover what the
statistical signals miss, and how should the two combine?

## Constraint first: data handling

A hosted Jev call sends the trace to a third party. On a standard account
there is no zero-data-retention (ZDR); TypeSafe offers ZDR to enterprise
customers. There is no attestation of the serving environment.

That rules out calling hosted Jev from ingest, the gate, or the contributor
apps on contributor traces. Every idea below therefore has two possible
deployments:

1. **Offline calibration.** Run Jev on public corpora and on sessions whose
   owner consents to the send. Use the results to derive thresholds, rules, or
   cheap local features that production computes without Jev.
2. **Production signal** only on an open-source System One class model
   served on NEAR infrastructure. It would sit behind a new trait in `trace-commons-gate-api`, beside
   `PerplexityScorer` and `Embedder`, held as a trait object per the
   gate-contract convention, with a `Reference*` implementation that is
   simple and uncalibrated.

Note the privacy paradox for idea 6 below: detecting that a session is
personal by sending it to a third party defeats the purpose. That idea only
works with a model served under our own data-handling guarantees.

## Experiments run

Both are out-of-tree sidecars (stdlib Python, dry-run default, redaction
before send). Neither touched pilot or contributor data.

### E1 — Jev dimensions vs perplexity (2026-09-18)

Twenty of one maintainer's own redacted sessions, each scored by Jev
(`task_difficulty`, `resolution`, `record_completeness`, `routineness` as
4-level Scores; `is_substantive` as a Noul) and by Qwen3.8-27B whole-trace
perplexity (chunked scoring). Spearman rho, n = 20:

| Jev dimension | vs perplexity | vs token count |
|---|---|---|
| task_difficulty | +0.04 | +0.59 |
| resolution | **-0.31** | +0.25 |
| record_completeness | +0.15 | -0.01 |
| routineness (higher = rarer work) | +0.34 | -0.05 |
| is_substantive | -0.05 | +0.46 |
| (perplexity itself) | — | -0.33 |

Observations:

- Perplexity is nearly orthogonal to judged difficulty, consistent with the
  Swival result.
- The two highest-perplexity sessions (22.5, 33.2) were short (1.6k and 7.6k
  tokens) and had low resolution; one was effectively abandoned (0.03).
- A 92-token junk session scored perplexity 3.18, unremarkable, and
  `is_substantive` 0.02.
- Jev difficulty has its own length confound (+0.59), in the opposite
  direction from perplexity's.

A blind human-label sheet for difficulty and resolution was prepared and is
**not yet filled**. Every number above is model-vs-model; none is accuracy.

### E2 — work-type categorization (2026-09-21 and 2026-09-22)

2026-09-21: an 11-way `work_type` Choice plus Noul facets on 40 public Swival
sessions. 2026-09-22: a 10-way category Choice, an `is_coding` Noul, five
edit-evidence Nouls, and a completion Score on 50 of the same maintainer's
local agent sessions (606k input tokens, about USD 0.025, zero errors).

Category spread (n = 50): feature 11, review 10, question/research 8,
debugging 7, setup/ops 7, refactor 3, docs 2, non-code 1, unknown 1.
Confidence >= 0.9 on 27, >= 0.7 on 37, >= 0.5 on 46.

Findings:

- **`EpisodeCategory` is too coarse.** Projected onto it, 37 of 50 sessions
  land in `other`. Real sessions are dominated by feature work, review,
  research questions, and setup/ops, none of which the enum names.
- **Edit-evidence Nouls need evidence-bound wording.** A first wording
  ("writing or editing documentation, specs, plans, prose") looked inflated;
  on inspection the sessions had in fact edited specs and plans. Rewording to
  "judge only from `files_edited` and `agent_actions`; explanations in chat do
  not count" produced no label asserting an edit where the digest showed no
  write evidence of any kind. All 20 label-vs-path disagreements were in
  sessions that write through subagents (13 of 50 delegate) or shell
  commands, which the digest did not itemize. Those 20 are unadjudicated.
- **Completion Score is weak.** Median confidence 0.66, skewed toward
  "accomplished", no ground truth.
- **Operational.** A single request carries all questions; the largest
  digest was 79k characters, inside budget. Redaction must handle
  Claude Code's flattened directory names (`-Users-<name>-...`), not only
  `/Users/<name>/`; the first run missed them.

## Combinations

Ordered by expected value against today's known gaps.

### 1. Substance gate beside the perplexity floor

Perplexity refuses structurally empty envelopes; it cannot refuse greetings,
refusals, or setup chatter (E1: 3.18 on junk; floor report: 3.57 on a
greeting). An `is_substantive` Noul catches those. Keep them as **two
independent conditions** (both must pass), not a blended score: they fail on
different inputs and a weighted sum would let one mask the other.

### 2. Resolution guard on perplexity-driven credit

E1's -0.31 suggests part of high perplexity is truncation or failure, not
novelty. Let perplexity raise `credit_quality` only when `resolution` and
`record_completeness` clear a floor. This targets the worst ranking failure:
cut-off sessions scoring as the most novel.

### 3. Category-conditioned normalization

A review session carrying a pasted diff and a research Q&A have different
perplexity baselines. Express perplexity and novelty as **within-category
percentiles** rather than against one global floor. For novelty, restrict the
nearest-neighbour search to same-category traces: "unusual for a debugging
session" is closer to what a data buyer pays for than distance to everything.
This also addresses part of the length/content-mix confound the per-author
shadow columns target, from a different direction; the two can be compared on
the same rows.

### 4. Category as a dedup join guard

The 522-member cluster formed because shared scaffolding shingles made
unrelated sessions collide. Requiring a category match (or top-2 overlap) for
a cluster join is a cheap orthogonal guard, independent of simhash tuning. It
is one candidate for the "escalate the signal" step the recluster runbook
names, alongside author-kind weighting and MinHash.

### 5. Corpus slicing and scarcity

Category x difficulty x resolution x replay sufficiency partitions the corpus
by buyer:

- hard, resolved, replay-complete debugging -> regression-task candidates
  (the need in issue #298);
- hard, failed -> eval and RL material;
- routine, resolved -> SFT material.

Category mix also yields a scarcity signal: weight credit toward
under-represented categories. Contributor insights can show the same breakdown
(credit by category, category mix over time).

### 6. Non-code as a privacy route

E2 marked a personal-document question `non_code` with `is_coding` 0.02. Such
sessions are not agent training data and carry elevated privacy risk; they
should be excluded or routed to review rather than scored. Requires a local
or NEAR-hosted open model (see the constraint above).

### 7. Jev as a second rater for per-author perplexity

The per-author shadow columns may not gate anything until calibrated against
labels. Jev Scores are not labels, but they are a cheap second rater for
disagreement analysis: rows where per-author perplexity and Jev difficulty
disagree sharply are the ones worth a human look first.

## Proposed next steps

1. **Fill the blind labels** (difficulty, resolution) for the E1 sessions.
   This converts every correlation above into an accuracy figure and tests
   combination 2 directly. Cheapest, highest-information step.
2. **Include subagent edits in the digest** and rerun E2, to adjudicate the
   20 unresolved edit labels.
3. **Offline evaluation of combinations 1-3 on public data** with production
   scorers, reporting how rankings move.
4. **Spec an `EpisodeCategory` extension** (feature, review,
   question/research, setup/ops, non-code) with an inferred provenance
   variant stored beside `UserReported`, never overwriting it. Only after
   step 1 shows the categories are reliable.
5. **Stand up the production path** (an open-source System One class model
   on NEAR infrastructure)
   before any combination moves from offline to gate.

## Non-goals

- No hosted third-party call on contributor or pilot traces.
- No change to gate floors, credit constants, or dedup parameters in this
  doc; each would be its own calibrated, era-versioned change.
- No claim that Jev judgments are ground truth. Typed output guarantees the
  interface, not correctness.
