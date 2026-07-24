# Corpus map and trace triage — design

## Goal

Build a human-judgment layer over the trace corpus, and expose its output on two
surfaces that share one clustering substrate:

1. **Trace triage console** (internal, tenant-scoped, full content) — a
   sampling + clustering + in-situ annotation tool that lets a human build a
   *derived* taxonomy of what is actually in the corpus and label a sample
   against it.
2. **Public corpus map** (community surface, aggregate, content-free) — a
   labelled coverage map of the corpus published under the existing
   `public_attribution` / min-cell / noise regime, so contributors can see
   which regions are dense and which are sparse.

The load-bearing deliverable is neither console nor map. It is the **labelled
sample**: the first ground-truth data against which the credit pipeline's
automated proxies (`q`, perplexity, novelty, `dup_pen`) can be validated.

## Background: the unvalidated premise

The credit pipeline composes `q` (perplexity + novelty, PR #168), `dup_pen`
(cross-trace dedup, PR #169), and per-contributor concave caps (PR #171) into a
shadow credit score. Every one of those signals was calibrated against
*distributional* statistics — percentiles, tail floors, AUC against synthetic
corpora, red-team farming scenarios. None was ever validated against a human
judgment that a given trace is worth paying for.

This matters because the failure history of the gate is a history of proxies
that separated distributions without measuring value:

- A2.6: perplexity-as-novelty flunks at 8B-class models across every corpus and
  only works at 27B — a proxy whose validity was an artifact of model scale.
- Tail-fraction: 81% of pilot traces score exactly 0; the signal does not
  discriminate and the floor stays disabled.
- The 2026-07-12 switch to a dense 27B scorer plus `PERPLEXITY_FLOOR_MICROS=6M`
  reframed perplexity as a *content-complexity* floor rather than a novelty
  signal — a reinterpretation, not a validation.

Shreya Shankar's evals framework (analyze → measure → improve) names the
underlying problem directly ([talk](https://www.youtube.com/watch?v=tqUDjc1HzO4)):
quality is subjective, it lives in the judgment of
the team building the product, and pointing a model at raw data to "find the
issues" yields surface-level or hallucinated findings. The correct use of
automation is to build the interface where a human applies judgment efficiently,
then distil that judgment into a measurable automated proxy with *known error
rates*.

Trace Commons differs from her setting in one way that must not be lost: this is
an open, paid, adversarial contributor market. Human judgment does not scale to
it and cannot be trustlessly attested. So the conclusion is **not** "put humans
in the gate." It is:

> Humans define the taxonomy and label a sample. The automated gate is a
> distillation of that judgment, with measured true/false-positive rates,
> recalibrated on a schedule.

### Why the envelope's existing taxonomy is not enough

The envelope already carries substantial eval machinery — `TraceFailureMode`
(a 16-variant enum plus `other(String)`), `outcome.task_success`,
`outcome.error_taxonomy`, the server-authored `process_evaluation` block with
per-axis ratings, and `training_dynamics.cartography_bucket`.

These do not substitute for this work, for three reasons:

- **The taxonomy is a priori.** `TraceFailureMode` was enumerated before the
  corpus existed. Deriving a taxonomy *from* the data by open-coding a sample is
  the entire point of the error-analysis stage; a guessed enum is the mistake
  the method exists to correct. Expect the derived taxonomy to disagree with the
  enum, and treat that disagreement as a finding.
- **The labels are client-asserted.** `outcome.failure_modes` and
  `events[].failure_modes` are set by the submitting client. In a paid market
  they are an attacker-controlled field, not evidence.
- **They feed nothing.** Failure modes reach the value-scorecard *explanation
  text* and nothing else. No gate, no `q`, no credit.

`process_evaluation` is the correct home for server-side evaluator labels and
should be the eventual write target — but it is populated by an evaluator, and
we have no validated evaluator to populate it with. That is what this work
produces.

## Non-goals

- **No change to gating, scoring, credit, or settlement.** Nothing in this spec
  multiplies into anything that pays. Human labels are recorded alongside gate
  decisions, never mutated into them.
- **No LLM judge in the gate.** A judge may be *proposed* by Phase 3 work, but
  it cannot ship without the measured alignment numbers Phase 2 produces.
- **No public exposure of trace content**, in any form, at any cluster size,
  including summarised or paraphrased form. See "Label disclosure" below.
- **No new consent scope** if the existing `public_attribution` regime covers
  the surface. It largely does; the one genuinely new disclosure class is
  content-derived *labels*, addressed explicitly below.
- **No annotation UI framework decision here.** The console's shape is
  specified; its implementation stack is a plan-level choice.
- **No re-litigation of the `TraceFailureMode` enum.** The derived taxonomy
  lives in its own versioned table. Reconciling the two is follow-up work.

## Prerequisite: semantic clustering

The public map and the console's cluster view both need *semantic* clustering.
PR #169 shipped simhash-only inline clustering, with embedding clustering
deferred as sub-project #2b. Simhash clusters near-duplicates — literal
reduplication and light rewording — which is exactly right for `dup_pen` and
useless as a topic map. A map built on simhash would show paraphrase families,
not regions of the task space.

**#2b (trace-level bge-large embedding clustering, in the separate dedup vector
index) is a hard dependency of Phases 1 and 3.** It should be re-scoped from
"dedup improvement" to "clustering substrate," since it now carries two
consumers.

Phase 2 (validation) does *not* depend on #2b and can proceed against a
stratified random sample as soon as Phase 1's annotation store exists.

## Architecture

### Three phases, sequenced by dependency

```
Phase 1  annotation store + triage console  →  labelled sample + derived taxonomy
Phase 2  validation                         →  do our proxies correlate with judgment?
Phase 3  public corpus map                  →  legible coverage, gated by review
```

Phase 2 is the decision point. If human value ratings show near-zero correlation
with `q`, that changes the credit roadmap more than any further scorer work
would, and Phase 3 should not ship a map whose regions are labelled by a metric
we have just learned is meaningless.

### Phase 1 — annotation store and triage console

**Storage.** A new append-only annotation table, tenant-scoped under forced RLS
like every other Trace Commons table, keyed by gate-decision id. Append-only and
versioned for the same reason gate decisions are: re-review is a first-class
operation, and Shankar's "one-shot evaluation is a mistake" point applies
directly — returning to already-reviewed traces routinely surfaces failure modes
missed the first time. A revised label is a new row, never an update.

Each annotation row carries: decision id, taxonomy version, assigned labels, a
human value rating, free-text notes, annotator ref, and created-at. The
free-text notes field is content-adjacent (an annotator may quote a trace) and
must therefore inherit the same retention, revocation, and export restrictions
as trace content. It is never eligible for any public surface.

**Taxonomy.** A versioned taxonomy table — label id, version, name, definition,
parent (for axial coding), and lifecycle state. Open coding produces a flat
label set; axial coding groups it. The version pin on every annotation row is
what makes taxonomy evolution non-destructive: re-coding an old sample against a
new taxonomy version is a new set of annotation rows, and prevalence measured
under v1 remains interpretable.

**Sampling.** The console must sample *stratified across score bands*, not
uniformly and not top-scoring-first. The question Phase 2 asks is whether the
gate separates anything a human agrees with, and that question is unanswerable
from a sample drawn by the gate's own ranking. Strata: `q` deciles, gate status
(accepted / rejected / held), and — once #2b lands — cluster.

**Console.** Sample → cluster view → annotate in situ. Full trace content,
tenant-scoped, behind an existing worker-credential-style bearer gate following
the established route pattern. The console is a judgment accelerator, not a
finder: it must not lead with model-generated findings, because pointing a model
at raw data and asking it to find the issues produces exactly the surface-level
and hallucinated output the method is designed to avoid. Model assistance is
appropriate for *navigation* — clustering, sampling, similarity, "show me more
like this" — and inappropriate for proposing labels before a human has coded.

**Inter-annotator agreement.** A designated overlap subset must be labelled by
at least two annotators, with agreement reported. A taxonomy that one person
can apply and a second cannot reproduce is not a measurement instrument, and
every downstream number in Phase 2 inherits its unreliability.

### Phase 2 — validation

Given the labelled sample, report:

- Correlation of human value rating against `q`, and against each of its
  components (perplexity, novelty) separately. Components may correlate in
  opposite directions; the composite would hide that.
- Correlation against `dup_pen` and against the concave-cap effective rate.
- Prevalence of each derived taxonomy label, with confidence intervals.
- Confusion between the derived taxonomy and the client-asserted
  `TraceFailureMode` labels on the same traces — a direct measure of how far
  client-supplied labels can be trusted.
- Gate-decision agreement: of traces a human rates valuable, what fraction did
  the gate accept, and vice versa.

Output is a written finding, not a code change. It feeds the roadmap.

### Phase 3 — public corpus map

Extends the existing community analytics surface rather than creating a new
boundary. `/v1/community/analytics/summary` already publishes corpus-wide
aggregates (submission volume, accept rate, gate-decision distribution, novelty
histogram) under `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` and the broad-release
noise config, gated by `public_attribution`. The map is a new aggregate in that
family: per-region trace counts, density, and a published human-reviewed label,
computed by the same worker-on-a-schedule pattern, subject to the same min-cell
suppression and count noise.

**Why publish it.** Not marketing. Today a contributor receives a score and an
opaque explanation string; the novelty term is a black box that rejects them. A
coverage map converts it into a directive: *these regions are sparse.* That is
the single most actionable thing we can tell a contributor, and it makes the
economics of the credit function legible in a way no per-trace explanation can.
The devfolio public demo page is the natural host, and per-project scoping
(PR #172) gives a natural filter axis.

**Label disclosure is the new risk class.** Aggregate *counts* are already
authorized and already shipped. Content-derived *labels* are not. A descriptive
label attached to a small cluster is a trace disclosure in summary form — at
cluster size 1 it is straightforwardly a content leak, and no amount of count
noise fixes it. Controls:

- Clusters below the min-cell floor are suppressed entirely — not merged into
  an "other" bucket that could be differenced against the total.
- Every published label is drawn from the reviewed taxonomy, pitched at an
  abstraction level that cannot reconstruct a trace, and passes an explicit
  human publication review. Labels carry a lifecycle state (draft → reviewed →
  published); only `published` renders.
- No free-text annotation notes, no cluster exemplars, no representative
  snippets, ever.
- Because published labels are content-derived, the consent policy needs an
  explicit versioned statement covering aggregate content-derived labels. This
  is narrower than a new scope but is not a render-time judgment call.

**Publishing the map publishes the gate's decision boundary.** Contributors can
farm sparse regions for novelty credit. This is a real adversarial concern here
in a way it is not in Shankar's setting. Partial mitigations: coarse region
resolution, lagged refresh, never publishing thresholds. But the honest position
is that *directed contribution into underrepresented regions is the behaviour we
want* — the map is working when it causes that. The failure case is synthetic
junk aimed at empty space, which is precisely what the perplexity floor and
`dup_pen` exist to catch, and which Phase 2 will tell us whether they actually
do.

## Conventions and constraints

- **Hash-only audit and logging.** Annotation activity is audited by decision
  hash and label id. Never log note text, label rationale, or trace content.
- **Forced RLS.** Annotation and taxonomy tables are Trace Commons tables and
  get forced RLS with `trace_current_tenant_id()` predicates. The public map's
  aggregation worker follows the cross-tenant read pattern already established
  for novelty and dedup — content-derived representations are already compared
  globally; region membership is the same class of derived signal.
- **Fail-closed.** If the taxonomy publication-review state cannot be read, the
  public map serves nothing. If min-cell config is absent, the map endpoint
  refuses rather than serving unsuppressed counts.
- **Migrations.** New tables need migration numbers above the applied range on
  the shared test DB, and `run_migrations` is hand-rolled — new migrations must
  be wired in explicitly.

## Open questions

1. **Who annotates?** Phase 1 assumes an internal annotator. Whether
   contributors can ever annotate their own traces (cheap, scales, obviously
   gameable) or peer-annotate others' (expensive, needs its own reputation
   model) is unresolved and deliberately out of scope.
2. **Value rating scale.** A scalar 1–5 is easy to correlate but compresses
   distinct reasons for value. Pairwise comparison is more reliable per
   judgment and more expensive. Recommend starting scalar, since Phase 2 needs
   correlation and the sample is small.
3. **Sample size.** 50–100 traces is enough to open-code a taxonomy. It is
   probably not enough for tight confidence intervals on per-label prevalence.
   Phase 2 should report intervals and let them argue for more labelling.
4. **Region definition for the map.** Flat k-means over the dedup embedding
   index is the simple answer; hierarchical regions would let the map zoom.
   Deferred to the plan.
5. **Does the derived taxonomy replace `TraceFailureMode`?** Not decided.
   Reconciliation is follow-up work and may conclude the enum should be
   deprecated in favour of a server-authored `process_evaluation` write.

## Verification

- `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
- `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
- `cargo clippy -p trace-commons-server --all-targets` with the repo allow-list
- Storage-contract and pg-store tests for the new tables
- Public-surface tests must include: min-cell suppression of small clusters,
  refusal when min-cell config is absent, and non-rendering of draft labels
