# Gate validity: a program across eight issues

Program design covering #204, #205, #210, #211, #446, #474, #478 and #199.
This is not a plan for one change. It decomposes the cluster into five
sub-projects, states what blocks what, and specifies the first one in full.
Each sub-project gets its own spec and plan when it is picked up.

## The thesis

Three issues say the same thing in different vocabularies: **the gate has
never been validated against the quantity it claims to measure.**

- **#204**: `discrimination_auc` on the A2.6 corpus measures *source format*,
  not novel-against-duplicate. Paragraph count scores AUC 1.000 on it; the
  selected model scores 0.936. Every one of the 300 duplicate files has
  exactly one paragraph and every novel file has between 7 and 163.
- **#205**: the bake-off optimises a threshold-free AUC while the shipped
  floor sits where most of that discrimination has been given away. Between
  62% and 70% of back-translations of the *same* agent traces pass.
- **#199**: `discrimination_auc` is a statement about redundancy, not
  utility. `task_success` is collected, persisted, rendered in admin
  analytics, and read by no part of the scoring or credit path.

Two more say it about the privacy half:

- **#210**: 0 of 99 real sessions reach Low. All 48 High sessions carry a
  secret finding, and `blocked_secret_detected` is set at detection time
  immediately before the span is redacted -- so the flag records that a
  secret was *found and removed*, not that one survived.
- **#474**: ~80% of the reviewable quarantine queue carries no deterministic
  secret signal.

The two halves are the same defect in different subsystems: a label that does
not measure the thing it is named after.

## The root finding, and how far it reaches

#204 is the root. If class label and source format are entangled in the
corpus, then the A2.7 calibration derived from that corpus inherits the
defect -- which is exactly what #205 observes from the other end, empirically,
without needing the corpus argument at all. The two issues are one finding
seen from opposite sides, and their agreement is the strongest evidence in
this cluster.

Confirmed in the builder rather than inferred from the numbers:
`scripts/operator/build-agent-traces-corpus.py` documents that "the novel
slice is swapped from OASST2 chat" while "the duplicate slice and the
paraphrase slice are reused verbatim from" a separate `corpus-wiki.tar.zst`.
Novel and duplicate come from different sources by construction.

It also reaches the roadmap. `docs/trace-commons-roadmap.md:62` still lists
"**Qwen 3.6 27B Dense AUC 0.936** (crosses the 0.5 threshold)" as the settled
outcome that selected the production scorer. Under #204 that number is a
source-format detector's score. Until the roadmap says the selection is
retained but unvalidated, it will keep being cited as evidence.

## Dependency structure

```
#204 corpus validity ──┬──> #205 floor re-derivation ──┐
                       └──> #446 recalibration ────────┤
                                                       ├──> defensible gate
#211 versioned canonicalisation ──> blocks any change ─┘
      to canonical text, including #478's sampling fix

#199 prospective instrumentation ──── independent; value accrues with time
#210 + #474 risk-rubric policy ─────── independent subsystem
```

## The five sub-projects

| | Sub-project | Issues | Blocked by | Start |
|---|---|---|---|---|
| A | Prospective instrumentation | #199 | nothing | now |
| B | Corpus validity | #204 | nothing | now |
| C | Risk-rubric policy | #210, #474 | decision 1 below | now |
| D | Versioned corpus transition | #211, #325 | nothing | after A/B underway |
| E | Recalibration | #446, #205, #478 | B, D, decision 2 | last |

A, B and C share no code and can run concurrently. A is first among equals:
it is the cheapest item here and the only one whose delay carries a permanent
cost, because rows written before instrumentation can never be recovered.
Recomputing novelty later scores against a fuller index and produces a number
production never used.

## Sub-project B, in full

**Goal.** A corpus on which a model's `discrimination_auc` is a statement
about novelty rather than about source format.

**The defect.** Novel and duplicate slices are drawn from different source
populations, so every property that tracks source -- paragraph structure,
length, vocabulary, formatting -- separates the classes perfectly. #204's
table shows six trivial measures beating the selected model, the best of them
without error.

**The construction.** Both slices must come from the same source population
and differ only in novelty. Concretely: novel is agent traces; duplicate is
*transformed versions of those same traces* -- back-translation or paraphrase
-- so source, format, subject matter and length distribution are held
constant. #205 identifies why this is the right shape: the back-translation
slice is "the one place in this corpus where source and subject matter are
held constant", and it is precisely where the floor barely moves. That is the
discrimination problem stated honestly.

**The acceptance criterion, which is the point of the sub-project.** A corpus
is valid only if the trivial-measure battery *fails* to separate its classes.
Run paragraph count, line count, distinct word count, UTF-8 byte count,
whitespace word count and mean word length, using the repository's own tie
convention -- `discrimination_auc` in
`crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs:18`,
which credits ties 0.5 each under the Mann-Whitney U convention and returns
0.5 for empty inputs. (#204 cites this as `bakeoff_metrics.rs:15-29`, relative
to a different root; the path above is the one that exists on `main` as of
2026-08-28, and there is a same-named test file at
`crates/trace-commons-server/tests/bakeoff_metrics.rs` that is not it.)
Every trivial measure must land near AUC 0.5. Any
structural measure that classifies the corpus is a defect in the corpus, not
a finding about the data.

This battery becomes a permanent, automated gate on corpus construction, not
a one-off audit. #204 was found by hand months after the corpus selected a
model; nothing would have caught it.

**Deliverables.** A corrected builder; the battery as a runnable check wired
into corpus construction; a rebuilt corpus with its sha256 recorded; and a
report stating the selected model's AUC on the corrected corpus beside its
archived 0.936, whatever that comparison shows.

**Explicitly not in B.** Re-running the bake-off. The 27B selection is
retained as unproven-but-inherited. B makes the floors defensible, and floors
are what actually gate traces today; "why this model" is left formally
unanswered and should be recorded as such rather than quietly closed.

## Sub-projects A, C, D and E, scoped

**A -- prospective instrumentation (#199).** Add `composite_score`,
`vector_index_snapshot_id` and `index_cardinality_at_scoring` to the gate
decision row. Prospective only: rows predating instrumentation are not
recoverable. #199 ships a preregistered analysis with a stopping rule of ~125
per class, which is what AUC 0.60 needs at 80% power; an early look at n≈30
can only detect 0.70 or worse and yields a null that means nothing. Honour
the stopping rule -- it exists so the result cannot be reinterpreted after
the fact.

**C -- risk-rubric policy (#210, #474).** The instrumentation half of #474
landed in #483: `residual_risk_basis` now records which conditions held. C is
the policy half, and it needs decision 1 before it can start. #210's two
composing rules -- Medium by any redaction finding, High by any detected
secret -- leave no path to Low for a real coding transcript, because real
coding transcripts contain paths, emails and keys. #210's cheapest ask is
independent of the policy question and worth doing regardless: let
`--dry-run` run unenrolled, so a prospective contributor discovers this in
one command rather than after spending a single-use invite.

**D -- versioned corpus transition (#211, #325).** Changing canonical text
moves the summary hash, the exact-duplicate score, derived novelty and the
gate simhash. Measured in #211, the simhash Hamming distance is 18 against a
clustering threshold of 10, so the same semantic trace forms a new cluster
across the boundary, and the recluster pass reuses stored simhashes rather
than regenerating them, so reclustering cannot heal the split. `summary_model`
does not change, `dedup_simhash` has no version column, and
`gate_version_hash` does not include the canonical renderer version. D must
land before any E change that touches canonicalisation.

**E -- recalibration (#446, #205, #478).** Re-derive floors against the
corrected corpus and the real-session distribution. #478's questions belong
here: whether a cap of 16 is still right when traces average 176 chunks, and
whether strided selection can even see what novelty looks for, since novelty
seeks a distinctive passage and striding is designed to spread across a trace.
E needs decision 2.

## Methodology guardrails

Lifted out of #199 because they govern all of B and E, not one issue. Each
came from a real artifact, not a hypothetical.

**Compute gate signals over content that excludes outcome-bearing fields.**
#199's dry run produced novelty at AUC 0.12 with a confidence interval
excluding 0.5 -- reading as "novel traces are the good ones" -- entirely
because correction phrasing ("no", "wrong", "revert", "instead") is generic
vocabulary shared across sessions, so including those turns raised a corrected
trace's overlap with every other trace. Excluding them moved the number to
0.40 with an interval spanning 0.5.

**Report a length covariate alongside every result.** If the covariate's AUC
matches the gate score's to two decimals, the gate score is measuring the
covariate. Vocabulary entropy and raw token count scored 0.8845 and 0.8840 --
two names for length.

**Treat agreement between encoders as suspect when both read the same input.**
The spurious result had the *tighter* interval, and significance, effect size
and cross-encoder agreement all pointed the wrong way at once, precisely
because both encoders read the same leaked tokens. It reproduced to four
decimal places across two corpora. Reproducibility is not validity.

**Run the trivial-measure battery against any new corpus before trusting a
model number on it.** See sub-project B.

## Decisions required before C and E

Neither is a technical call, and both block work.

**Decision 1, blocking C: is "secret found and removed" meant to be
terminal?** There is a real argument that it should be -- one secret is
evidence a trace is secret-bearing, and no detector is complete, so
distrusting the whole envelope is defensible. The observations are that it
costs 48% of a real corpus permanently, that High carries no operator
override while Medium does (inverted relative to the evidence each carries),
and that the policy is currently emergent rather than stated. C's shape
depends entirely on the answer.

**Decision 2, blocking E: what accept rate are we targeting for genuine
sessions?** #446 asks for "a stated, defensible target accept rate [...]
rather than whatever falls out". Without one, E has no success criterion and
will drift toward whatever the new floors happen to produce, which is how the
present situation arose. The current numbers: 100% of the curated corpus,
25% of real sessions.

## Non-goals

Re-running the bake-off; changing the deployed model; calibrating against
current pilot data, which #474 and #478 both warn is contaminated by the
classifier outage; and closing #446, which is a tracking issue and should
close only when A through E have.

## What this program does not establish

It does not show the gate is wrong. #204 shows the *measurement* cannot
support the claim made from it, which is a different and weaker statement: a
corpus that cannot discriminate is consistent with a model that can. B exists
to find out which. Stating the distinction matters, because the temptation on
reading this cluster is to conclude the gate is broken and start retuning --
and retuning against an invalid measurement is what produced the current
floors.
