# Substance (qualitative): design for the RLVR lab persona

Status: Proposal

Date: 2026-10-21.

This design sets the substance terms, weights, and floors from market research.
It does not fit them to value, because no objective measure of trace value
exists today (section 2). Most parameters are set: they are stated, defensible
assumptions. Two are calibrated against a pinned real corpus, because they are
defined relative to the score distribution. Calibration also runs statistical
tests on that corpus that show how much each assumption moves the score
(section 7). A real buyer signal replaces the assumptions later (section 7).

The design targets one buyer: the reinforcement learning lab that post-trains
agents (the RLVR lab, section 4). It is model-free. It reads only the trace
envelope and fixed constants. It runs no model and makes no remote call.

## 1. Why: substance is a proxy for value

A trace earns credits. The credit should track what the trace is worth to a
buyer. Substance is the per-trace estimate of that worth.

Substance is not a fabrication detector. A scoring formula cannot separate a
careful fake from real work. A trace that a client builds by hand can set every
field that substance reads. So substance does not try to prove a trace is real.

Two mechanisms divide the work:

- NEAR AI attestation proves that the final model call ran on a pinned model
  over the context it was shown. It does not prove earlier turns, tool results,
  or compacted context. It is the robust defense against a falsified trace.
- Substance estimates what the work is worth to a buyer.

This design follows that division. Substance ranks traces by estimated buyer
value. Attestation enters substance as a light weight, so a trace with proven
provenance ranks slightly higher (section 5.3).

The credit quality seam stays `q = h(substance) * g(novelty)`. The attestation
weight sits inside substance, so `q` keeps its shape.

## 2. Limitations

State these limits before the model, so a reader does not over-trust it.

- Value is subjective. Data has different value to different buyers at
  different times, and none of this is observable at scale from the data
  itself. So this design bases the substance function on qualities that the
  research ties to one large buyer. This rests on many assumptions, but it
  serves as a practical proxy.
- The design targets one persona. The RLVR lab is the dominant buyer today
  (section 3). Other buyers value other things. A single value function cannot
  serve all buyers.
- Substance does not detect fabrication. A coordinated script can max every
  term with no model call: it emits distinct calls with a nonce argument and a
  short result, and adds an initial prompt. So an unattested trace that is built
  to look valuable scores as if it were valuable. Only attestation, contributor
  caps, and delayed settlement defend against this. A regression guardrail
  (section 7) keeps substance from rewarding obvious garbage, but it is not a
  fabrication defense.
- The score depends on the contributor's consent class. Work and replay need
  tool payloads. A contributor who declines tool payloads scores at the work and
  replay floors, and likely below `substance_floor`. So credit becomes an
  incentive to share content classes that carry more residual risk. Attested
  bodies also travel only under the tool-payload consent flag. Whether to
  correct this with per-consent-class floors is an open owner decision
  (section 11). The evidence records the consent class (section 8), so the
  effect is visible either way.
- Attestation covers a subset of traces today. Receipts exist only for
  inference through NEAR AI. The requirement is off by default, because turning
  it on would refuse every trace from Claude Code, Codex, Gemini, and Cline. So
  `v` is 0 for most traces now (section 5.3).
- The market numbers are secondary. No primary rate card exists. The research
  reads trade press and vendor pages (section 3). Treat the figures as
  directional.

## 3. Market Research

This section summarizes the research that informs the persona and the
parameters. It is not exhaustive. It gives the rationale for the terms in
section 5, and each claim names its source.

### 3.1 Who buys this data

The buyers fall into four archetypes:

1. Frontier AI labs that post-train agents with reinforcement learning. The
   dominant buyer today. One analysis calls labs "the only buyers with both
   urgency and budget" (datagravity.dev, 2026-01-29).
2. Evaluation and safety groups that buy benchmarks and red-team data.
3. Enterprises that want agents to work inside real business tools.
4. Regulated-domain teams that buy expert data in finance, legal, and medical
   fields.

This design serves archetype 1, the RLVR lab.

### 3.2 What the RLVR lab pays more for

Several independent sources name the same quality drivers (datagravity.dev,
2026-01-29; troveo.ai, 2026-09-24; aitrainingdatabroker.com, 2026-10-10;
TechCrunch, 2025-09-21):

- Verifiable outcomes. The checkable result is the core product. A clean
  terminal label, success or failure, lets a trainer compute a reward.
- Reproducibility and replay. A trajectory must re-run. Coding and computer-use
  are early markets, because they give dense, machine-observable traces with
  clean verification loops.
- Dense, real tool use. A buyer values recorded actions and their results, not
  prose about actions.
- Difficulty and failure cases. Buyers favor a few hard tasks over many easy
  ones. Research shows failure traces carry signal; failure runs are 12 to 82
  percent longer than success runs (arXiv 2511.00197).
- Real reasoning prose, not filler. Padded or boilerplate prose adds no value.
- Provenance and clean rights. Legal pressure is real; one lab reached a large
  copyright settlement over how data was obtained.

### 3.3 Pricing signals

Prices are opaque and come from secondary sources. One task with a goal and a
verifier reportedly sells for 200 to 2,000 US dollars; an exclusive license runs
4 to 5 times the non-exclusive price (aitrainingdatabroker.com, 2026-10-10;
troveo.ai, 2026-09-24). These figures set no parameter directly. They show the
ranking: a complete, verifiable, replayable task is worth far more than a thin
one.

### 3.4 The mapping to fields is a judgment

The research names what buyers pay for. The link from each driver to a trace
field is a judgment, not a fact. For example, this design treats `paired_calls`
as a proxy for "dense, machine-observable trajectory." The field is ours; the
link is the designer's inference. Section 7 states how to check it.

## 4. The persona: the RLVR lab

- Who they are. A frontier lab that post-trains coding and computer-use agents
  with reinforcement learning. The dominant buyer today (section 3.1).
- Use case. Train and verify agent policies on hard, checkable tasks.
- What they buy. Verifiable-reward tasks and machine-observable trajectories
  that replay deterministically.
- Trace attributes they value. Dense paired tool calls; full replay; a clear
  terminal outcome, success or failure; real reasoning prose; proven provenance.
- Trace attributes they discount. Traces that do not replay; an ambiguous
  outcome; long prose with no tool use.

## 5. The model

Substance is a pure function of the envelope. It returns one number in
`[0, 1]`. It reads one projection of the envelope, not the raw bytes:

- Deduplicated tool calls. Each `tool_call`'s arguments serialized as canonical
  JSON, with calls identical by (`tool_name`, arguments) collapsed to one. The
  work term reads this, so an exact repeat earns nothing extra; a near-duplicate
  with a varied argument still counts (section 2).

Two helper functions, each used below:

- `sat(x, ceil) = ln(1 + x) / ln(1 + ceil)`, clamped to `[0, 1]`. Diminishing
  returns on a count; it reaches 1 at the ceiling.
- `graded(v, m) = m + (1 - m) * v`. It lifts a value in `[0, 1]` onto `[m, 1]`;
  `m` is a floor above 0.

### 5.1 Work

The work term rewards dense, real, diverse tool use. The RLVR lab values a
machine-observable trajectory of real actions (section 3.2).

From the deduplicated tool-call projection:

- `paired_calls`: `tool_call` events that carry arguments and that have a
  matching `tool_result` with non-empty content. The match is by `tool_call_id`,
  else by the result's `parent_event_id`.
- `distinct_tools`: distinct `tool_name` over the paired calls.

```
work = w_pc * sat(paired_calls, pc_ceil)
     + w_dt * sat(distinct_tools, dt_ceil)
```

The work term has no turn-count or length input. A length input would let a
duplicated or padded trace inflate work. That would break the regression
guardrail of section 7. Difficulty is proxied by the volume of real paired work
through the saturation ceilings, not by a raw count of turns.

The work term has no write-action input. The `side_effect` field is derived on
the contributor side by a substring match on `tool_name`. It is unreliable in
both directions. A name that contains "write", "create", or "send" classifies as
a write even with no argument and no result, so a write count is cheap to fake.
Worse, the common edit tools, Claude Code `Edit` and Codex `apply_patch`,
classify as read-only, so real edit work would not count. A write term would
therefore mislabel both fabricated and genuine traces. A reliable write signal
needs a classification that is not a name substring. That is deferred.

### 5.2 Replay

The RLVR lab needs a trace to re-run deterministically (section 3.2). Replay is
near-critical, so its floor is low: a non-replayable trace ranks far lower, but
it is not vetoed.

```
replay = w_hp * has_initial_prompt + w_sa * share_args + w_sr * share_results
```

`has_initial_prompt` is 1 when the trace holds an initial prompt with content.
`share_args` is the share of tool calls that carry arguments. `share_results` is
the share of tool calls that carry results. When the trace has zero tool calls,
`share_args` and `share_results` are 0, not full credit. So a tool-free trace
earns at most the prompt share, `w_hp`, on replay. This is deliberate: a trace
with no tool call cannot replay a trajectory, so it must not earn full replay.

This differs from the server's general `ReplaySufficiency::score()`, which
returns full credit for a trace with zero tool calls. Substance defines replay
directly to avoid rewarding a tool-free trace with full replay.

Substance does not read the self-reported `replayable` flag. The flag is a weak
self-report, true on every pilot envelope. No term vetoes the score, so the flag
does not zero it.

### 5.3 Attestation weight

Attestation proves that the final model call ran on a pinned model over the
context it was shown. It does not prove earlier turns, tool results, or
compacted context. It enters substance as a light weight, so an attested trace
ranks slightly higher. With this weight, substance measures worth with a
provenance premium, not worth alone.

`v` is 1 when the trace's `witness_provenance_class` is
`provider_tee_final_call` or `gateway_final_call`, else 0. Both classes are
final-call receipts verified against pinned signers. `v` is 0 for most traces
today (section 2).

```
attest = graded(v, att_floor_mult)
```

`att_floor_mult` is high (section 6), so `v = 0` barely lowers substance and
`v = 1` raises it only slightly. The weight rewards proven provenance without
punishing the common unattested trace.

This weight amends the label-only contract of V89 (#1061 decision 5), which
says the provenance class weights no gate, score, or credit amount. Zaki to
confirm.

### 5.4 Composite, credit quality, and gate

Each term enters the composite raw, in `[0, 1]`. The composite applies each
term's floor once.

```
substance = graded(work,   m_work)
          * graded(replay, m_rep)
          * graded(v,      att_floor_mult)          in [0, 1]

q = graded(sat(substance, sub_ceil), sub_floor_mult)
  * g(novelty)
```

Every term passes through `graded` with a floor above 0, so no single term
forces substance to 0. The composite only ranks a trace. One threshold gates:

```
substance_passed = substance >= substance_floor
```

So the pass or fail of an edge case, such as a tool-free trace, is a floor
decision in calibration (section 7). It is not a property of the formula.

`g(novelty)` is the existing novelty half of credit quality, unchanged.

### 5.5 Terms not scored in this version

The persona also values a clear outcome and real reasoning prose (section 4).
Substance scores neither in this version, because no reliable field measures
them today.

- Outcome. Substance does not read `task_success`. The field is unverified: it
  is `unknown` unless the contributor answers an optional verdict, and every
  pinned trace is `unknown`. So in practice it records the contributor's
  opinion, not the task's success. A later version adds the term once an
  outcome is verified.
- Density. Substance does not score agent prose. The DEFLATE ratio of prose
  mostly ranks message length: on English prose, the median ratio is 0.70 at
  256 B and 0.39 at 16 KiB. Padded messages also move a per-message median. A
  later version adds credit for real prose when a buyer signal shows that it
  matters.

## 6. Parameters

Every parameter has a starting value and a rationale. Parameters fall into two
classes:

- Set. The value is fixed by judgment from the research of section 3 and from
  the baseline work facets already in use. The table gives the value and its
  rationale. Calibration reports how much each set parameter moves the score,
  but does not choose its value (section 7).
- Calibrated. The value is defined relative to the real score distribution, so
  calibration sets it from the pinned corpus (section 7). Only `sub_ceil` and
  `substance_floor` are calibrated.

Every value is provisional: a real buyer signal revises it (section 7).

| Parameter | Value | Rationale |
|---|---|---|
| `pc_ceil` | 20 | Saturation ceiling for paired calls. A dense trajectory has many paired calls; returns diminish past ~20. |
| `dt_ceil` | 8 | Ceiling for distinct tools. Breadth of real tools matters, but a handful already shows breadth. |
| `w_pc` | 0.60 | Paired calls are the strongest signal of a machine-observable trajectory, so they carry the most weight. |
| `w_dt` | 0.40 | Tool breadth is valued but secondary to call density. |
| `w_hp` | 0.40 | The initial prompt is the anchor a replay starts from. |
| `w_sa` | 0.30 | Arguments on calls are needed to re-run an action. |
| `w_sr` | 0.30 | Results on calls are needed to verify an action. |
| `m_work` | 0.30 | Work floor. A tool-free trace ranks low, not zero. |
| `m_rep` | 0.10 | Replay floor, low because replay is near-critical for this buyer. |
| `sub_ceil` | calibrated | Substance saturation ceiling in `q`. Set at a chosen high percentile of substance over the pinned corpus, so real traces spread across the curve. |
| `sub_floor_mult` | 0.25 | Substance floor in `q`. |
| `att_floor_mult` | 0.90 | Attestation weight floor inside substance. An attested trace scores about 11 percent above the same trace unattested (1.0 against 0.9): "slightly higher," as the design intends. |
| `substance_floor` | calibrated | The pass threshold. Set at a chosen percentile of substance over the pinned corpus, so the floor sits where real traces sit. |

Every parameter lives in the bundle configuration. The bundle id hashes them.
The server never changes them at run time. A change to any parameter produces a
new bundle id.

## 7. How the parameters are set and checked

There is no fit to value, because there is no value ground truth (section 2).
Each parameter is therefore set or calibrated (section 6), and the behavior of
the score is checked against a pinned real corpus, the 10k HF pin.

Set parameters are fixed by judgment. Calibration does not choose their values;
it measures how much each one moves the score, so a reader knows which
assumptions matter.

Calibrated parameters are defined relative to the score distribution, so
calibration sets them from it. `sub_ceil` is a chosen high percentile of
substance over the pinned corpus. `substance_floor` is a chosen percentile of
substance over the same corpus.

Calibration runs offline. It writes the calibrated values into the bundle
configuration and produces a calibration report. The report records the corpus
pin, the calibrated values, and the result of every check below. Four checks
apply.

1. Fabrication regression guardrail. Reuse the synthetic benchmark of real and
   fabricated traces. For each vector, no fabricated variant may score above its
   real base. The check is pairwise and per vector, not a pooled false-positive
   rate. This is a guardrail against obvious garbage, not a value test, and not
   a fabrication defense. The regression set holds the fabrication vectors:
   noise padding, empty scaffold, missing evidence, truncation, duplication,
   asserted outcome, and template filler. Noise padding, template filler and
   asserted outcome now tie their base, because substance reads neither prose
   nor outcome.
2. Monotonicity and no veto. More real work never lowers substance. Every floor
   is above 0, so no term vetoes a trace.
3. Real-corpus distribution. Report the distribution of substance and of each
   term over the pinned corpus. This distribution is the input to the
   calibrated parameters. No fit runs, so it makes no generalization claim.
4. Sensitivity analysis. For each set parameter, score the pinned corpus at half
   and at double its value, holding the others fixed. For each term, score the
   corpus with that term removed. Each variant recalibrates `sub_ceil` and
   `substance_floor` at the same percentiles, so the comparison isolates the
   change. Compare each variant with the baseline on three measures: the
   Spearman rank correlation of substance, the gate flip rate (the share of
   traces whose `substance_passed` changes), and the credit shift (the median
   absolute change in `q`). A variant is insensitive when the Spearman
   correlation is at least 0.95 and the gate flip rate is at most 2 percent. An
   insensitive parameter stays at its stated value. A sensitive parameter is
   listed in the report with its rationale, because its assumption drives the
   score. An insensitive term whose removal still passes check 1 is listed as a
   removal candidate (section 11). An insensitive term that check 1 needs stays,
   and is documented as a guardrail, not a value signal. A parameter or term
   that is constant on the pinned corpus, or that acts only on q, is reported as
   untested, not insensitive. Check 4 does not judge it, and it stays an owner
   decision (section 11).

The regression fixture is synthetic, or it is derived from a licensed public
corpus and passed through the same redaction and residual-risk pass as a real
trace. The manifest records which, and records the source licence. No raw,
unredacted trace is committed.

The first real buyer signal is the trigger to replace judgment with a fit. One
confirmed price, one ranked preference, or one weight from a real buyer lets a
later version fit the set parameters. Until then, every parameter stays
provisional.

## 8. Evidence and integration

The score evidence records the composite, each term, and the provenance class:

```
substance_micros, substance_passed, substance_version,
substance_work_micros, substance_replay_micros,
witness_provenance_class, consent_class
```

`witness_provenance_class` records the witness's class for the final call,
which sets `v` (section 5.3). It lets an audit see which traces carried an
attested final call.

`consent_class` records the contributor's consent class. It makes the
consent-class effect of section 2 visible, and it is the field a
per-consent-class floor would read (section 11).

Integration mechanics:

- The parameters live in the bundle configuration and are hashed into the bundle
  id. Calibration runs offline (section 7). The server never fits or
  re-calibrates.
- The score is a pure function. It needs no model, GPU, embedder, or remote
  call. Novelty is computed as it is today.
- `substance_version` is one label over the formula and the projection. A
  change to any of them bumps the label and the bundle id.
- A pipeline-scored trace is signed as
  `trace_commons.pipeline_score_attestation.v1`, not the legacy
  `trace_commons.score_attestation.v2`, which is built from legacy
  gate-decision rows the substance family never writes. Whether the signed
  `decision` already carries `substance_micros`, or needs a schema bump, is a
  plan-time check.

## 9. Learnings and changes from the earlier substance design

This is the only section that refers to the earlier design: the substance v1
bundle design, `docs/superpowers/specs/2026-10-14-substance-v1-bundle-design.md`
on this branch (draft PR #1472). Everything else in this spec stands on its own.

The earlier design framed substance as fabrication detection. It named seven
attack vectors, generated a synthetic benchmark of real and fabricated traces,
and fit four terms (work, density, outcome, replay) offline to separate them.
Acceptance was a pass on that benchmark.

Learnings:

- Attestation defends; scoring estimates value. A formula cannot separate a
  careful fake from real work: a fake built with one model call per trace maxes
  every term. So substance is a proxy for what a buyer would pay, and NEAR AI
  attestation is the fabrication defense. A receipt proves only the final call,
  not earlier turns, tool results, or compacted context.
- No objective measure of value exists. There is no dataset of trace prices, so
  a fit to value is not possible, and a fit to the fabrication benchmark
  optimizes the wrong objective. Parameters are set by judgment from market
  research, with the assumptions stated.
- A field that the contributor derives or asserts cannot carry a term.
  `side_effect` is a substring match on `tool_name`. `task_success` is the
  contributor's optional verdict, `unknown` by default, so it records an
  opinion, not an outcome.
- The DEFLATE ratio of prose mostly measures message length, and padded
  messages move a per-message median. So compression cannot rank prose quality.
- A regression guardrail must be passable by construction. A vector that no
  projection can tell apart from real work belongs to attestation, not to the
  guardrail.

Changes:

- One persona, the RLVR lab, instead of a generic composite.
- Two value terms, work and replay. The write-action, outcome, and density terms
  are removed.
- Replay gives no call-share credit to a trace with no tool calls.
- Attestation enters substance as a light binary weight from the witness
  provenance class. This amends the V89 label-only contract.
- Set parameters are fixed by judgment. Only `sub_ceil` and `substance_floor`
  are calibrated on the pinned corpus, and a sensitivity analysis shows which
  assumptions move the score.
- The fabrication vectors are a pairwise regression guardrail, not the
  acceptance criterion. The fit, the false-positive gate, the dumb-feature
  guard, and the outcome join are removed.
- The evidence records the provenance class and the consent class. The score is
  signed as `trace_commons.pipeline_score_attestation.v1`.

What stays the same:

- Substance is a model-free pure function of the envelope.
- Substance is a floored multiplicative composite. No term vetoes a trace, and
  one floor gates.
- Credit quality keeps the shape `q = h(substance) * g(novelty)`.
- Every parameter lives in the bundle configuration and is hashed into the
  bundle id. The server never fits.

## 10. Non-goals

- Simulating every kind of buyer. Other personas are documented in the research
  but are out of scope here.
- A fit to value. There is no value ground truth yet (section 2).
- A fabrication defense in the formula. Attestation owns that.
- Scoring the attested share only, or weighting each term by coverage. The
  witness records a final-call class, not a share, so this design uses one light
  binary weight inside substance.
- Any change to novelty, the embedder, the index, or the award amount.

## 11. Open owner decisions

These decisions change the model and need an owner ruling before the plan.

- Per-consent-class floors. A contributor who declines tool payloads scores at
  the work and replay floors (section 2). Decide whether that is intended. If it
  is not, set a floor per consent class, and read `consent_class` from the
  evidence.
- Whether a tool-free trace passes the gate. A trace with no tool call ranks low
  on work and earns at most the prompt share on replay. Whether such a trace
  passes is the `substance_floor` decision (section 5.4). Decide the floor
  against the real-corpus distribution (section 7).
- The attestation weight. `att_floor_mult = 0.90` gives an attested trace about
  11 percent more substance. Confirm that this is the intended size, or set
  another. The weight also needs Zaki's agreement to amend the V89 label-only
  contract (section 5.3).
- Removal of insensitive terms. The sensitivity analysis lists each term whose
  removal keeps the ranking and the gate within tolerance and still passes the
  guardrail (section 7). Decide whether to remove it in the next bundle version.
