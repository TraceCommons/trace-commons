# Substance (qualitative): design for the RLVR lab persona

Status: design. Not implemented.

Date: 2026-10-21.

This design sets the substance terms, weights, and floors from market research.
It does not fit them to data. No objective measure of trace value exists today
(section 2). So the values here are stated, defensible assumptions, not a fitted
result. A real buyer signal replaces them later (section 7).

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

- NEAR AI attestation proves which part of a trace is real. It is the robust
  defense against a falsified trace.
- Substance estimates what the work is worth to a buyer.

This design follows that division. Substance ranks traces by estimated buyer
value. Attestation enters as a weight, so a trace with proven provenance ranks
slightly higher (section 5.3).

The credit quality seam stays `q = h(substance) * g(novelty)`. This design adds
one light attestation weight to it (section 5.4).

## 2. Limitations

State these limits before the model, so a reader does not over-trust it.

- No objective measure of value exists. There is no dataset that maps a trace to
  a price a buyer paid. So the parameters are set by judgment from the market
  research, not fitted. Every parameter is provisional.
- The design targets one persona. The RLVR lab is the dominant buyer today
  (section 3). Other buyers value other things. A single value function cannot
  serve all buyers.
- Substance does not detect fabrication. A coordinated script can max every
  term with no model call: it emits distinct write-named calls with a nonce
  argument and a short result, adds an initial prompt, copies real prose into
  the density band, and asserts success. So an unattested trace that is built to
  look valuable scores as if it were valuable. Only attestation, contributor
  caps, and delayed settlement defend against this. A regression guardrail
  (section 7) keeps substance from rewarding obvious garbage, but it is not a
  fabrication defense.
- The score depends on the contributor's consent class. Work and replay need
  tool payloads, and density needs message text. A contributor who declines tool
  payloads scores at the work and replay floors, and likely below
  `substance_floor`. So credit becomes an incentive to share content classes
  that carry more residual risk. Attested bodies also travel only under the
  tool-payload consent flag. Whether to correct this with per-consent-class
  floors is an open owner decision (section 10). The evidence records the
  consent class (section 8), so the effect is visible either way.
- Attestation covers a subset of traces today. Receipts exist only for inference
  through NEAR AI. The requirement is off by default, because turning it on would
  refuse every trace from Claude Code, Codex, Gemini, and Cline. So the attested
  coverage `v` is 0 for most traces now (section 5.3).
- The market numbers are secondary. No primary rate card exists. The research
  reads trade press and vendor pages (section 3). Treat the figures as
  directional.

## 3. Market research (why these fields)

This section summarizes the research that informs the persona and the
parameters. It is not exhaustive. It gives the rationale for the terms in
section 5. The full report is the Notion page "Buyer personas and value
functions for Trace Commons" (2026-10-20), built from a market scan.

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

Substance is a pure function of the envelope. It returns one number in [0, 1].
It reads a projection of the envelope, not the raw bytes:

- Agent prose. The concatenated `redacted_content` of every `assistant_message`
  and `reasoning` event, in order. The density term reads this. Tool results and
  user messages are excluded, because the agent did not write them.
- Deduplicated tool calls. Each `tool_call`'s arguments serialized as canonical
  JSON, with calls identical by (`tool_name`, arguments) collapsed to one. The
  work term reads this, so a repeated loop earns nothing extra.

Three helper functions, each used below:

- `sat(x, ceil) = ln(1 + x) / ln(1 + ceil)`, clamped to [0, 1]. Diminishing
  returns on a count; it reaches 1 at the ceiling.
- `graded(v, m) = m + (1 - m) * v`. It lifts a value in [0, 1] onto [m, 1]; `m`
  is a floor above 0.
- `band(x; lo, hi, ramp) = 1` inside [lo, hi], a linear ramp down to 0 across
  width `ramp` just outside, else 0. It rewards a middle range.

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
work = graded(work, m_work)
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

### 5.2 Outcome

The RLVR lab needs a checkable terminal label to compute a reward. A clean
failure is as useful as a clean success, because a failure trace carries a
learning signal (section 3.2).

```
outcome = 1.0         if task_success is success or failure
        = o_partial   if task_success is partial
        = o_unknown   if task_success is unknown
```

This term does not read `human_correction`. For this persona the sign of a
correction is ambiguous: a correction can mean an expert improved the work, or
it can mean the agent failed. A clean failure already scores 1.0, so the
correction adds nothing here and its ambiguity is avoided.

The outcome values are fixed design constants, not fitted parameters. The
ordering is a design decision: success equals failure, and both are above
partial, which is above unknown. A fit is not free to reverse this ordering.

`task_success` is a client-asserted field. The capture path sets it to success
by default whenever a turn has a response. So success is the common, cheap
label, and the honest reporter who records a true `unknown` or `failure` scores
lower. This is an accepted limit of a client-asserted field. The field is read
at face value; attestation, caps, and settlement are the defense, not this term.

### 5.3 Density

The density term rewards real reasoning prose and rejects filler and noise
(section 3.2).

The density reads a per-message ratio, not one global ratio. For each
`assistant_message` and `reasoning` event with at least `density_min_bytes` of
content, compute `ratio_i = deflate_len_i / raw_len_i`, with DEFLATE at a fixed
level. Let `ratio` be the median over those events. If no event meets
`density_min_bytes`, the band value is 0 and density is the floor.

```
density = band(ratio; lo, hi, ramp)
density = graded(density, m_dens)
```

A per-message median is deliberate. One global ratio lets a single appended
block move the whole term: 4 KiB of base64 added to a below-band base can push
the global ratio into the band. A median over messages resists one appended
block, because the block is one message among many.

The band is two-sided on purpose. Template filler compresses too well and falls
under `lo`. Base64 or random content does not compress and rises over `hi`.

### 5.4 Replay

The RLVR lab needs a trace to re-run deterministically (section 3.2). Replay is
near-critical, so its floor is low: a non-replayable trace ranks far lower, but
it is not vetoed.

```
replay = w_hp * has_initial_prompt + w_sa * share_args + w_sr * share_results
replay = graded(replay, m_rep)
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

### 5.5 Attestation weight

Attestation proves which share of a trace is real. It does not change what the
work is worth. So it does not enter substance. It enters the credit quality `q`
as a light weight, so an attested trace ranks slightly higher.

`v` is `verified_inference_coverage` in [0, 1], the witness-computed share of the
trace backed by an attested NEAR AI inference receipt. It is 0 for most traces
today (section 2).

```
attest = graded(v, att_floor_mult)
```

`att_floor_mult` is high (section 6), so `v = 0` barely lowers `q` and `v = 1`
raises it only slightly. The weight rewards proven provenance without punishing
the common unattested trace.

### 5.6 Composite, credit quality, and gate

```
substance = graded(work,    m_work)
          * graded(outcome, m_out)
          * graded(density, m_dens)
          * graded(replay,  m_rep)          in [0, 1]

q = graded(sat(substance, sub_ceil), sub_floor_mult)
  * g(novelty)
  * graded(v, att_floor_mult)
```

Every term passes through `graded` with a floor above 0, so no single term
forces substance to 0. The composite only ranks a trace. One threshold gates:

```
substance_passed = substance >= substance_floor
```

So the pass or fail of an edge case, such as a tool-free or a code-heavy trace,
is a floor decision in calibration (section 7). It is not a property of the
formula.

`g(novelty)` is the existing novelty half of credit quality, unchanged.

## 6. Parameters

Every parameter has a starting value and a rationale. The values come from the
research of section 3 and from the baseline work facets already in use. Mark
every value "to revise": a real buyer signal revises it (section 7).

| Parameter | Value | Rationale |
|---|---|---|
| `pc_ceil` | 20 | Saturation ceiling for paired calls. A dense trajectory has many paired calls; returns diminish past ~20. |
| `dt_ceil` | 8 | Ceiling for distinct tools. Breadth of real tools matters, but a handful already shows breadth. |
| `w_pc` | 0.60 | Paired calls are the strongest signal of a machine-observable trajectory, so they carry the most weight. |
| `w_dt` | 0.40 | Tool breadth is valued but secondary to call density. |
| `o_partial` | 0.60 | A partial outcome is a usable but weaker reward label. |
| `o_unknown` | 0.40 | An unknown outcome is the least useful for a verifiable-reward buyer. |
| `lo` | 0.30 | Lower band edge. Prose that compresses below this is likely filler. |
| `hi` | 0.60 | Upper band edge. Prose that compresses above this is likely noise or encoded bytes. |
| `ramp` | 0.10 | Ramp width outside the band, so the edge is not a cliff. |
| `density_min_bytes` | 256 | Minimum agent prose before the band applies. |
| `w_hp` | 0.40 | The initial prompt is the anchor a replay starts from. |
| `w_sa` | 0.30 | Arguments on calls are needed to re-run an action. |
| `w_sr` | 0.30 | Results on calls are needed to verify an action. |
| `m_work` | 0.30 | Work floor. A tool-free trace ranks low, not zero. |
| `m_out` | 0.30 | Outcome floor. An unknown outcome ranks low, not zero. |
| `m_dens` | 0.25 | Density floor. A code-heavy trace ranks low, not zero. |
| `m_rep` | 0.10 | Replay floor, low because replay is near-critical for this buyer. |
| `sub_ceil` | to revise | Substance saturation ceiling in `q`. Set so real traces spread across the curve. |
| `sub_floor_mult` | 0.25 | Substance floor in `q`. |
| `att_floor_mult` | 0.90 | Attestation weight floor. An attested trace ranks about 11 percent higher; an unattested trace loses about 11 percent. "Slightly higher," as the design intends. |
| `substance_floor` | to calibrate | The pass threshold. Set at a chosen percentile of the real corpus, so the floor sits where real prose sits. |

Every parameter lives in the bundle configuration. The bundle id hashes them.
The server never changes them at run time. A change to any parameter produces a
new bundle id.

## 7. How the parameters are set and checked

There is no fit to value, because there is no value ground truth (section 2). So
the design sets the parameters by judgment (section 6) and checks the behavior,
rather than fitting the numbers. Four checks apply.

1. Fabrication regression guardrail. Reuse the synthetic benchmark of real and
   fabricated traces. For each vector, no fabricated variant may score above its
   real base. The check is pairwise and per vector, not a pooled false-positive
   rate. This is a guardrail against obvious garbage, not a value test, and not a
   fabrication defense. The regression set holds the fabrication vectors: noise
   padding, empty scaffold, missing evidence, truncation, duplication, asserted
   outcome, template filler, a below-band base plus noise, an above-band base
   plus filler, a filler-and-noise mix tuned to the band centre, and a
   near-duplicate-calls vector that varies a nonce argument per call.
2. Monotonicity and no veto. More real work never lowers substance. Every floor
   is above 0, so no term vetoes a trace.
3. Real-corpus distribution. Report the distribution of substance and of each
   term over the pinned real corpus (the 10k HF pin). Set `substance_floor` at a
   chosen percentile of that distribution. No fit runs, so this makes no
   generalization claim. It sets the floor where real prose sits.
4. Outcome join, when data exists. Report the AUC of the outcome-free composite,
   `work * density * replay`, against `task_success`, with a length covariate
   beside it. The join excludes the outcome term, because `task_success` is the
   outcome term's own input; a join of the full substance against `task_success`
   would be circular. This is the only empirical value signal. It is reported,
   not gated, until the pilot produces enough labeled data.

The earlier design used this benchmark differently (section 9). It fit the
parameters on a train split, gated a false-positive and false-negative rate on a
test split, and ran a dumb-feature baseline guard. This design removes all three,
because it runs no fit. The benchmark survives only as the pairwise regression
guardrail of check 1.

The regression fixture is synthetic, or it is derived from a licensed public
corpus and passed through the same redaction and residual-risk pass as a real
trace. The manifest records which, and records the source licence. No raw,
unredacted trace is committed.

The first real buyer signal is the trigger to replace judgment with a fit. One
confirmed price, one ranked preference, or one weight from a real buyer lets a
later version fit the parameters this design now guesses. Until then, every
parameter stays provisional.

## 8. Evidence and integration

The score evidence records the composite, each term, and the attested coverage:

```
substance_micros, substance_passed, substance_version,
substance_work_micros, substance_outcome_micros,
substance_density_micros, substance_replay_micros,
verified_inference_coverage_micros, consent_class
```

`verified_inference_coverage_micros` records the witness coverage `v` that
enters `q`. It lets an audit see how much of a trace was attested, and it lets a
later version score the attested share only, without a schema change.

`consent_class` records the contributor's consent class. It makes the
consent-class effect of section 2 visible, and it is the field a per-consent-
class floor would read (section 10).

Integration mechanics:

- The parameters live in the bundle configuration and are hashed into the bundle
  id. The server never fits or re-calibrates.
- The score is a pure function. It needs no model, GPU, embedder, or remote
  call. Novelty is computed as it is today.
- `substance_version` is one label over the formula, the projection, and the
  compressor. A change to any of them bumps the label and the bundle id.
- A pipeline-scored trace is signed as
  `trace_commons.pipeline_score_attestation.v1`, not the legacy
  `trace_commons.score_attestation.v2`, which is built from legacy
  gate-decision rows the substance family never writes. Whether the signed
  `decision` already carries `substance_micros`, or needs a schema bump, is a
  plan-time check.

## 9. Learnings and changes from the earlier substance design

This is the only section that refers to the earlier design (the substance v1
bundle design, 2026-10-14). Everything above stands on its own.

The earlier design framed substance as fabrication detection. It named seven
attack vectors, generated a synthetic benchmark of real and fabricated traces,
and fit the parameters offline to separate them. Acceptance was a pass on that
benchmark.

Two learnings from the maintainer review changed the direction:

- Attestation defends; scoring estimates value. A formula cannot separate a
  careful fake from real work. The maintainer noted that a fake built with one
  model call per trace maxes all four terms. So substance should be judged as a
  proxy for what a buyer would pay, not as a fabrication detector. NEAR AI
  attestation is the robust fabrication defense.
- No objective value measure exists. There is no dataset of trace prices. So an
  offline fit to value is not possible, and fitting to the fabrication benchmark
  optimizes the wrong objective. Parameters are better set by judgment from
  market research, with the assumptions stated.

The concrete changes:

- Target one persona (the RLVR lab), not a generic composite.
- Set parameters qualitatively from market research (section 6), rather than fit
  them offline.
- Add attestation as a light weight on `q` (section 5.5), so an attested trace
  ranks slightly higher. The earlier design stored attestation only as a
  provenance label that did not change `q`.
- Move the seven fabrication vectors from the acceptance criterion to a
  regression guardrail (section 7).

What stays the same:

- Substance is a model-free pure function of the envelope.
- Substance is a floored multiplicative composite; no term vetoes a trace; one
  floor gates.
- Credit quality keeps the shape `q = h(substance) * g(novelty)`, now with the
  light attestation weight.
- Every parameter lives in the bundle configuration and is hashed into the
  bundle id; the server never fits.

## 10. Non-goals

- Serving buyers other than the RLVR lab. Other personas are documented in the
  research but are out of scope here.
- A fit to value. There is no value ground truth yet (section 2).
- A fabrication defense in the formula. Attestation owns that.
- Scoring the attested share only, or weighting each term by coverage. The
  evidence field in section 8 makes this possible later, but this design uses a
  single light weight on `q`.
- Any change to novelty, the embedder, the index, or the award amount.

## 11. Open owner decisions

These decisions change the model and need an owner ruling before the plan.

- Per-consent-class floors. A contributor who declines tool payloads scores at
  the work and replay floors (section 2). Decide whether that is intended. If it
  is not, set a floor per consent class, and read `consent_class` from the
  evidence.
- Whether a tool-free trace passes the gate. A trace with no tool call ranks low
  on work and earns at most the prompt share on replay. Whether such a trace
  passes is the `substance_floor` decision (section 5.6). Decide the floor
  against the real-corpus distribution (section 7).
- The attestation weight. `att_floor_mult = 0.90` gives an attested trace about
  an 11 percent lift. Confirm that this is the intended size, or set another.
