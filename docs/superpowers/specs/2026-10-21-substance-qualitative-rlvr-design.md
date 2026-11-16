# Substance (qualitative): design for the RLVR lab persona

Status: Proposal

Date: 2026-10-21.

This design sets the substance terms, weights, and floors from market research.
It does not fit them to value, because no objective measure of trace value
exists today (section 2). Most parameters are set: they are stated, defensible
assumptions. One, a saturation ceiling, is calibrated against a pinned real
corpus, because it is defined relative to the score distribution. Calibration
also runs statistical tests on that corpus that show how much each assumption
moves the score (section 7). A real buyer signal replaces the assumptions later (section 7).

The design targets one buyer: the reinforcement learning lab that post-trains
agents (the RLVR lab, section 4). It is model-free. It reads the trace
envelope, the stored witness class as the Score step reads it (section 5.3),
and fixed constants. It runs no model and makes no remote call.

## 1. Why: substance is a proxy for value

A trace earns credits. The credit should track what the trace is worth to a
buyer. Substance is the per-trace estimate of that worth.

Substance is not a fabrication detector. A scoring formula cannot separate a
careful fake from real work. A trace that a client builds by hand can set every
field that substance reads. So substance does not try to prove a trace is real.

Three mechanisms divide the work:

- NEAR AI attestation proves that NEAR AI served the final model call over the
  context it was shown. A provider-TEE receipt also proves which pinned model
  ran it; a gateway receipt does not name the model. Attestation does not prove
  earlier turns, tool results, or compacted context, and it does not prove that
  the session made the call. So it defends the final call, not the tool history.
- Contributor caps and delayed settlement limit what a scripted trace can earn.
- Substance estimates what the work is worth to a buyer.

This design follows that division. Substance ranks traces by estimated buyer
value. Attestation enters substance as a light weight, so a trace with an
attested final call ranks slightly higher (section 5.3). The weight is an
incentive to use attested inference, not a measure of proven value.

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
  to look valuable scores as if it were valuable. With one real final call
  through NEAR AI, the same script also earns the attestation weight, because
  the receipt does not cover the tool history. Only contributor caps and delayed
  settlement defend against this. A regression guardrail (section 7) keeps
  substance from rewarding obvious garbage, but it is not a fabrication defense.
- A trace that withholds tool payloads or message text scores at the floors,
  because substance has nothing to read: work and replay need tool payloads, and
  the prompt share of replay needs message text. This is intended for this
  persona, which cannot use content that the trace does not carry. It means
  credit rewards the contributor who shares more content, and that content
  carries more residual risk. The evidence shows when this is the cause of a low
  score (section 8.1).
- Attestation covers a subset of traces today. Receipts exist only for
  inference through NEAR AI, and attested bodies travel only when the
  contributor shares tool payloads. The requirement is off by default, because
  turning it on would refuse every trace from Claude Code, Codex, Gemini, and
  Cline. Pipeline receipts also store no witness evidence yet. So `v` is 0 for
  every pipeline trace now (section 5.3).
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

Substance is a pure function of the envelope and the witness class that the
Score step reads (section 5.3). It returns one number in `[0, 1]`. It reads one
projection of the envelope, not the raw bytes: the deduplicated tool calls. The
work term and the replay shares read this projection. It uses the replay rules
of the protocol crate, so it reads the same payload fields as the protocol's
replay-sufficiency measure. Those rules are private today, and this design
makes them public helpers of the protocol crate (section 8.2):

- Arguments. A call's arguments are the first populated value under one of the
  protocol's argument keys in its `structured_payload` (the new helper
  `tool_call_arguments`).
  The `tool_call_id` and the other payload keys are not arguments. A call with
  no such value carries no arguments.
- Deduplication. Calls with the same `tool_name` and the same arguments,
  serialized as canonical JSON, collapse to one group. So an exact repeat earns
  nothing extra, and a near-duplicate with a varied argument still counts
  (section 2). Withheld calls of one tool carry no arguments, so they collapse
  to one group.
- Results. A `tool_result` carries a result when the protocol's
  `event_carries_result` holds: it has non-empty content, or a populated value
  under one of the protocol's result keys in its payload. It matches a call by
  `tool_call_id` when both carry one, else by its `parent_event_id`. Each result
  matches one call at most. A group carries a result when any of its calls has
  a match. Here substance differs from the protocol's replay measure, which
  pairs a result that has no call id with the next call in order. Substance
  pairs a result that has neither id with no call.

Two helper functions, each used below:

- `sat(x, ceil) = ln(1 + x) / ln(1 + ceil)`, clamped to `[0, 1]`. Diminishing
  returns on a count; it reaches 1 at the ceiling.
- `graded(v, m) = m + (1 - m) * v`. It lifts a value in `[0, 1]` onto `[m, 1]`;
  `m` is a floor above 0.

### 5.1 Work

The work term rewards dense, real, diverse tool use. The RLVR lab values a
machine-observable trajectory of real actions (section 3.2).

From the deduplicated tool-call projection:

- `paired_calls`: deduplicated calls that carry arguments and a result
  (section 5).
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
`share_args` is the share of deduplicated tool calls that carry arguments.
`share_results` is the share of deduplicated tool calls that carry a result
(section 5). When the trace has zero tool calls, `share_args` and
`share_results` are 0, not full credit. So a tool-free trace earns at most the
prompt share, `w_hp`, on replay. This is deliberate: a trace with no tool call
cannot replay a trajectory, so it must not earn full replay.

A share rises when an incomplete call is removed. So a trace whose interrupted
tail is cut scores higher on replay than the uncut trace. This is accepted: the
remaining trajectory is complete, and a complete trajectory replays better. A
paired call carries arguments and a result, so adding one never lowers either
share.

This differs from the protocol crate's `ReplaySufficiency::score()`, which
returns full credit for a tool-free trace that has an initial prompt. Substance
defines replay directly to avoid rewarding a tool-free trace with full replay.

Substance does not read the self-reported `replayable` flag. The flag is a weak
self-report, true on every pilot envelope. No term vetoes the score, so the flag
does not zero it.

### 5.3 Attestation weight

Attestation proves that NEAR AI served the final model call over the context it
was shown. It does not prove earlier turns, tool results, or compacted context.
It enters substance as a light weight, so an attested trace ranks slightly
higher. With this weight, substance measures worth with a
provenance premium, not worth alone. The weight is an incentive to use attested
inference. It is not a measure of proven value, because the receipt covers the
final call only (section 2).

`v` is 1 when the trace's `witness_provenance_class` is
`provider_tee_final_call` or `gateway_final_call`, else 0. The else case covers
`unattested` and `legacy_v1`, a certificate of the older format. Both attested
classes are final-call receipts verified against pinned signers. `v` is 0 for every
pipeline trace today (section 2).

The two classes prove different things. A provider-TEE receipt comes from the
Chat Completions API: it names the model, and that model's key signs it. A
gateway receipt comes from the Responses API: NEAR AI's attested gateway signs
it, but it does not name the model. The account-trust rule counts only the
provider-TEE class, for that reason. The score counts both, because the weight
is an incentive to use attested inference, and Codex reaches NEAR AI only
through the gateway.

The class is not in the envelope, because a contributor could set a field
there. The witness signs a certificate that states the class. The contributor's
client uploads it beside the envelope, and the server verifies it at upload and
stores it with the class.

The input of `v` is the stored class, read at Score time, not at upload.
Before it calls the Score policy, the pipeline's Score step reads the
class for the run's submission with the store read that the credit provenance
label uses (`list_current_verified_witness_evidence`, mapped by
`TraceWitnessProvenanceClass::from_claim`). It passes the class to the policy
in a new `ScoreInput` field, `witness_class`, which is empty when the step does
not read the class. A submission with no stored
evidence reads `unattested`. A read error fails the Score attempt, which
retries; it never falls back to `unattested`. The step reads the class only
for a substance package whose configuration says so (below), so the
compatibility family gets no new query and no new failure mode.

The read gives `unattested` when, at Score time, the submission is not
accepted, is revoked or purged, is past its retention date, or its stored
envelope no longer matches the certificate. Each case takes the trace out of
the live corpus or breaks its link to the certificate, so the weight ends with
it. The certificate itself has no expiry in this read, so a run that waits in
the queue keeps the weight while its submission is live. The evidence records
the class that the policy used (section 8.1), so a score is reproducible from
the envelope, the bundle, and that recorded class.

Pipeline receipts store no witness evidence today. The submit handler verifies
the certificate, but only the legacy upload path stores it, so every
pipeline-routed trace reads `unattested`. The weight can apply only after the
pipeline receipt stores the verified evidence in its own transaction, as the
legacy path does.

So the substance Score configuration holds `witness_source`, which says whether
the Score step reads the class. With `not_read`, the step reads nothing, and
the policy scores every trace as `unattested` and records that class. With
`stored`, the step reads the class as above. The bundle id hashes the choice,
so a package keeps its scores when a later package reads the class. The first
release uses `not_read`. Until pipeline receipts store the evidence, it gives
the same result as the read for every pipeline trace.

```
attest = graded(v, att_floor_mult)
```

`att_floor_mult` is high (section 6), so `v = 0` barely lowers substance and
`v = 1` raises it only slightly. The weight rewards proven provenance without
punishing the common unattested trace.

The score reads the class. This changes the label-only contract of V89 (#1061,
earned-trust decision 5), which says that the provenance class weights no gate,
score, or credit amount. Decision 5 is already reversed for account trust
(2026-10-11), so the contract holds for gates, scores, and credit amounts. This
design removes scores from it. The comments on `witness_provenance_classes` in
the ingest service and on `TraceWitnessProvenanceClass`, and the contract text
in `docs/trace-commons.md`, change with it. The V89 migration header stays as
the record of the original contract.

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

So the pass or fail of an edge case, such as a tool-free trace, is set by the
floor (section 6). It is not a property of the formula.

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

Every set parameter has a starting value and a rationale. The calibrated
parameter has a starting percentile.
Parameters fall into two classes:

- Set. The value is fixed by judgment from the research of section 3 and from
  the baseline work facets already in use. The table gives the value and its
  rationale. Calibration reports how much each set parameter moves the score,
  but does not choose its value (section 7).
- Calibrated. The value is defined relative to the real score distribution, so
  calibration sets it from the pinned corpus (section 7). Only `sub_ceil` is
  calibrated.

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
| `sub_ceil` | calibrated: 90th percentile | Substance saturation ceiling in `q`. At the 90th percentile of substance over the pinned corpus, the top tenth of real traces gets full substance credit, and the rest spread across the curve. The percentile is a judgment. |
| `sub_floor_mult` | 0.25 | Substance floor in `q`. |
| `att_floor_mult` | 0.90 | Attestation weight floor inside substance. An attested trace scores about 11 percent above the same trace unattested (1.0 against 0.9): "slightly higher," as the design intends. |
| `substance_floor` | 0.12 | The pass threshold. Every trace with an initial prompt passes: the lowest such score is 0.1242, a tool-free trace without attestation (`0.30 * 0.46 * 0.90`). A trace with no initial prompt and no paired call fails: it scores at most 0.111 (`0.30 * 0.37 * 1.0`), because with no paired call the argument share and the result share sum to at most 1. So credit, not the gate, ranks a tool-free trace low. Whether a tool-free trace fails is a later decision, made from real pilot traffic. |

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

The calibrated parameter is defined relative to the score distribution, so
calibration sets it from that distribution. `sub_ceil` is the 90th percentile
of substance over the pinned corpus.

Calibration runs offline. It writes the calibrated value into the bundle
configuration and produces a calibration report. The report records the corpus
pin, the calibrated value, and the result of every check below. Two new
`pipeline.py` commands do this (section 8.2). `pipeline.py dataset generate`
builds the guardrail fixture of check 1 from the pinned corpus. Until the
generator exists, check 1 runs on synthetic bases that the test code builds.
`pipeline.py evaluate` scores a dataset with the substance bundle and writes the
report. Neither runs a model, an embedder, or the index. Four checks apply.

1. Fabrication regression guardrail. A synthetic benchmark pairs base
   traces with fabricated variants, one variant per base for each vector. For
   each pair, the variant must meet its expectation, and no variant may score
   above its base. The check is pairwise and per vector, not a pooled
   false-positive rate. This is a guardrail against obvious garbage, not a value
   test, and not a fabrication defense. Section 7.1 gives the recipe.
2. Monotonicity and no veto. More paired calls never lower substance: a paired
   call can only raise work, and it cannot lower either replay share
   (section 5.2). Every floor is above 0, so no term vetoes a trace.
3. Real-corpus distribution. Report the distribution of substance and of each
   term over the pinned corpus. This distribution is the input to `sub_ceil`.
   No fit runs, so it makes no generalization claim.
4. Sensitivity analysis. For each set parameter, score the pinned corpus at half
   and at double its value, holding the others fixed. For each term, score the
   corpus with that term removed. Each variant recalibrates `sub_ceil` at the
   same percentile and keeps `substance_floor`, so the comparison isolates the
   change. Compare each variant with the baseline on three measures: the
   Spearman rank correlation of substance, the gate flip rate (the share of
   traces whose `substance_passed` changes), and the credit shift (the median
   absolute change in the substance half of `q`, `h(substance)`, with novelty
   held at 1, because calibration runs no embedder). A variant is insensitive
   when the Spearman
   correlation is at least 0.95 and the gate flip rate is at most 2 percent. An
   insensitive parameter stays at its stated value. A sensitive parameter is
   listed in the report with its rationale, because its assumption drives the
   score. An insensitive term whose removal still passes check 1 is listed as a
   removal candidate (section 11). An insensitive term that check 1 needs stays,
   and is documented as a guardrail, not a value signal. A parameter or term
   that is constant on the pinned corpus, or that acts only on q, is reported as
   untested, not insensitive. Check 4 does not judge it, and it stays an owner
   decision (section 11).

The first real buyer signal is the trigger to replace judgment with a fit. One
confirmed price, one ranked preference, or one weight from a real buyer lets a
later version fit the set parameters. Until then, every parameter stays
provisional.

### 7.1 The guardrail recipe

A base comes from the pinned corpus, or it is synthetic (section 7). It has at
least one paired call, and every
tool call in it is paired. So a variant that removes events can remove only real
work, and every base scores above the floors, so a variant can score below it.
The generator checks this structure; it never runs the score.

| Vector | Transform | Change | Expect |
|---|---|---|---|
| Noise padding | `append_noise` | 4 KiB of base64 random bytes appended to the last assistant message | equal |
| Template filler | `template_filler` | every assistant message replaced by one fixed paragraph, repeated to the original byte length | equal |
| Empty scaffold | `add_scaffold` | 20 `tool_call` events with no arguments and no results | not above |
| Truncation | `truncate_half` | the second half of the events removed | not above |
| Duplication | `duplicate_events` | the event list repeated once | not above |
| Missing evidence | `strip_results` | every `tool_result` emptied: its content and its payload removed | below |
| Asserted outcome | `assert_success_no_work` | all tool events removed, `task_success = success` | below |

Noise padding and template filler tie their base, because substance reads no
prose. Asserted outcome scores below its base, because it also removes every
tool event. Substance does not read the outcome.

Each pair records the base, the variant, the transform, and the expectation.
The manifest records the recipe parameters, the corpus pin, and the seed, so a
run reproduces. A change to any recipe parameter is a new recipe id. The report
gives, for each transform, the share of pairs that meet the expectation and the
term that moved most.

The regression fixture is synthetic, or it is derived from a licensed public
corpus and passed through the same redaction and residual-risk pass as a real
trace. The manifest records which, and records the source licence. No raw,
unredacted trace is committed.

## 8. Evidence and integration

### 8.1 Evidence

The score evidence records the composite, each term, the tool-call count, and
the provenance class:

```
substance_micros, substance_passed, substance_version,
substance_work_micros, substance_replay_micros,
substance_tool_calls, witness_provenance_class
```

`substance_tool_calls` records the number of deduplicated tool calls, paired or
not. With the work term, it shows why a score is low. A count of 0 means that
the trace had no tool calls. A count above 0 with work at 0 means that no call
was paired: the contributor withheld the tool payloads, or the capture lost
them.

`witness_provenance_class` records the class that set `v`: the stored class
when `witness_source` is `stored`, and `unattested` when it is `not_read`
(section 5.3). Under `stored`, it lets an audit see which traces carried an
attested final call. Under either, it makes a rescore reproducible.

`credit_quality_micros` and `credit_quality_version` are filled as today.

The Settle step writes a row into the legacy gate-decision table,
`trace_gate_decisions`, for every Score policy except the minimal reference
policy, and it fails the run closed when the evidence lacks a required field.
Substance fills the row: the evidence's `quality_passed`, which the row stores
as `perplexity_passed`, is `substance_passed`, and `novelty_score_micros`,
`novelty_passed`, and `nearest_neighbor_hash` come from novelty, as today.

`perplexity_micros` and `tail_fraction_micros` are NULL on a substance row,
because the family measures no perplexity. NULL means "not measured", as it
already does for the per-author perplexity columns and
`qualifying_token_fraction_micros` in the same table, and a reader must not
default it. A migration removes NOT NULL from the two columns. Evidence
validation refuses a partial set of the substance evidence fields, so they are
all present or all absent, and Settle treats evidence that carries them as
substance. The compatibility family still requires both values. Settle
refuses a substance evidence that carries either one, with the label
`pipeline_gate_decision_evidence_inconsistent`.

A reader that uses a perplexity as a value reads only rows that have one:

- The legacy signed score attestation, `trace_commons.score_attestation.v2`,
  and the admin score read, `/v1/admin/scores-by-submission`, list only rows
  with a perplexity, because a v2 entry requires one. A substance trace is
  signed as `trace_commons.pipeline_score_attestation.v1` instead
  (section 8.2).
- The scoped form of the v2 attestation must account for every id that it is
  asked about. It lists an owned submission that has a gate decision, but none
  with a perplexity, in a new list, `scored_without_perplexity`, beside
  `pending` and `unknown`. Without that list it would sign a substance-scored
  trace as pending, and the contributor CLI, which waits until nothing is
  pending, would wait until its timeout.
- The tail-floor calibration and the gate outcome analysis skip NULL. So they
  also skip the row of a substance run whose duplicate verdict withheld its
  credit. That row carries no credit quality version, so a filter on the
  version would miss it.
- The legacy gate's duplicate cache copies the newest row for identical
  content. It never picks a pipeline row, because a pipeline submission row has
  no canonical summary hash. A copy would carry NULL, so it could not pass as a
  perplexity either.

A reader of the pass flag keeps substance rows, because the row's
`perplexity_passed` is `substance_passed` by design. The account-trust rule
counts it, and the public analytics summary counts a substance failure under
its existing label, `perplexity_failed`.

The new evidence fields are optional and are skipped when empty, so a
compatibility outcome serializes byte-identically and stored hashes do not
move.

### 8.2 The bundle

- Family and ids. Substance changes only the Score stage. The substance bundle
  is the compatibility family with one new Score id,
  `trace_commons.score.substance.v1`, and the rule id `substance_novelty_v1`.
  Admission, review, and settle reuse the compatibility ids.
- Configuration. `SubstanceScoreConfig` holds the parameters of section 6,
  `substance_version`, `witness_source` (section 5.3), and the novelty
  configuration, which also gives the novelty half of `q`. The bundle id hashes
  all of it. Calibration runs offline (section 7). The server never fits or
  re-calibrates.
- Score policy. For each trace, in order: parse the reviewed envelope (a parse
  failure is permanent); take the witness class, from `ScoreInput.witness_class`
  when `witness_source` is `stored`, else `unattested` (section 5.3); compute
  the terms, the composite, and `substance_passed`; compute novelty with the
  existing compatibility code, moved behind one shared function; compute `q`;
  apply the compatibility rule for index membership and the award, with
  `substance_passed` in place of `quality_passed`; fill the evidence.
- Credit quality. `q` (section 5.4) is a new credit quality version, `cq4`,
  beside the current `credit_quality` function. Its parameters come from the
  bundle configuration, not from code constants.
- Substance needs no model, GPU, embedder, or remote call. Novelty is computed
  with the embedder and the index, as today, with one change. The novelty
  aggregate weights each chunk by a token count, and today the perplexity
  scorer supplies that count when it scores the chunk. The substance family
  calls no scorer, so the count comes from the chunk's length at 4 characters
  per token, the estimate that the chunker already uses. So the novelty
  weights of this family differ slightly from the compatibility family's. No
  separate novelty version exists: the Score id, the bundle id, and
  `substance_version` carry the change.
- `substance_version` is one label over the formula and the projection. A
  change to any of them bumps the label and the bundle id.
- New code lives in a new module,
  `crates/trace-commons-server/src/versioned_pipeline_substance.rs`, with the
  AGPL header. The two calibration commands are new (section 7). The dataset
  generator is a subcommand of the `trace-commons-pipeline-corpus-export`
  binary, which already holds the pinned corpus loader. The `evaluate` driver
  is a library module run through an ignored test, the same shape as `run`
  and `compare`. Both are driven from `scripts/operator/pipeline.py`. The
  protocol crate makes its replay rules public: `event_carries_arguments`,
  `event_carries_result`, `replay_call_id`, and the new `tool_call_arguments`
  (section 5).
- Contributor points, the contributor cap, and the account-trust rule read the
  legacy row, so they see a substance `q` with no change to their readers. An
  account-trust policy that sets a credit quality minimum must list version 4
  in its allowlist. The credit-estimate fit learns credit amounts from version
  3 rows only (owner decision E9). So a scored substance row stays out of it
  until a separate `cq4` table is fit, and a withheld substance row counts in
  its withheld share, as any withheld row does.
- A pipeline-scored trace is signed as
  `trace_commons.pipeline_score_attestation.v1`, not the legacy
  `trace_commons.score_attestation.v2` (section 8.1). Its entry names the
  submission, the run, the Score outcome, the bundle, the scored award, and the
  Score decision. It carries no Score evidence for any family, so
  `substance_micros` is not signed and the schema does not change. Signing the
  evidence is a separate change.

### 8.3 Tests

- Golden tests: fixed envelopes and witness classes with pinned term values and
  composite, for each `substance_version`. A formula change breaks a pinned
  value and forces a version bump.
- The guardrail contract: on the committed fixture, or on the synthetic bases
  until the fixture exists, in CI, every pair meets its
  expectation (section 7.1).
- The compatibility baseline test
  (`compatibility_baseline_is_executable_and_detects_drift`) stays green, which
  shows that the evidence change is byte-neutral for that family.
- Isolation: the substance-only `evaluate` run builds with no embedder and no
  index, which shows that substance reads neither.
- Settle: the legacy row builds from substance evidence, with NULL in both
  perplexity columns, so a substance run settles and pays. A substance row
  gives no v2 score attestation entry, no admin score entry, and no tail-floor
  input, and the scoped v2 form lists it in `scored_without_perplexity`, not in
  `pending`. Settle refuses compatibility evidence without the perplexity
  fields and substance evidence with them.
- Witness input, for a package whose `witness_source` is `stored`: a substance
  Score of an attested submission gets its stored class, a revoked submission
  gets `unattested`, and a failed read fails the attempt. With `not_read`,
  every trace scores as `unattested`.
- Qualification: the substance package passes `pipeline.py qualify`.

## 9. Learnings and changes from the earlier substance design

This is the only section that refers to the earlier design: the substance v1
bundle design, `docs/superpowers/specs/2026-10-14-substance-v1-bundle-design.md`
on this branch (draft PR #1472). This spec supersedes it in full. Everything
else in this spec stands on its own.

The earlier design framed substance as fabrication detection. It named seven
attack vectors, generated a synthetic benchmark of real and fabricated traces,
and fit four terms (work, density, outcome, replay) offline to separate them.
Acceptance was a pass on that benchmark.

Learnings:

- Attestation defends; scoring estimates value. A formula cannot separate a
  careful fake from real work: a script with no model call maxes every term. So
  substance is a proxy for what a buyer would pay. NEAR AI attestation defends
  the final call, and contributor caps and delayed settlement defend against a
  scripted tool history. A receipt proves only the final call, not earlier
  turns, tool results, or compacted context.
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
  guardrail. A share falls when a bad call is added and rises when one is
  removed, so the guardrail's bases have every tool call paired.
- Two specs that both stay in force drift apart. This spec holds the whole
  design, and the earlier one is kept only as the record of round 1.

Changes:

- One persona, the RLVR lab, instead of a generic composite.
- Two value terms, work and replay. The write-action, outcome, and density terms
  are removed.
- Replay gives no call-share credit to a trace with no tool calls.
- Attestation enters substance as a light binary weight from the stored witness
  provenance class, read at Score time. This removes scores from the V89
  label-only contract. It is an incentive to use attested inference.
- Set parameters are fixed by judgment. Only `sub_ceil` is calibrated on the
  pinned corpus, and a sensitivity analysis shows which
  assumptions move the score.
- The fabrication vectors are a pairwise regression guardrail, not the
  acceptance criterion. The fit, the false-positive gate, the dumb-feature
  guard, the outcome join, and the `correct_failure` control are removed.
- The evidence records the provenance class and the tool-call count. The run
  is signed as `trace_commons.pipeline_score_attestation.v1`.
- Substance has one input beyond the envelope: the stored witness class, when
  its configuration reads it.
- The novelty aggregate weights chunks by the chunker's character estimate,
  not by the scorer's token count, because the family calls no scorer.
- The family writes the legacy gate-decision row, as the compatibility family
  does, so contributor points, the contributor cap, and the account-trust rule
  need no change. The earlier design
  planned a separate Stage 2 for them; it is dropped. The row's perplexity
  columns hold NULL, not a placeholder, so no reader takes a substance row for
  a perplexity.

What stays the same:

- Substance is model-free.
- Substance is a floored multiplicative composite. No term vetoes a trace, and
  one floor gates.
- Credit quality keeps the shape `q = h(substance) * g(novelty)`.
- Every parameter lives in the bundle configuration and is hashed into the
  bundle id. The server never fits.

## 10. Non-goals

- Simulating every kind of buyer. Other personas are documented in the research
  but are out of scope here.
- A fit to value. There is no value ground truth yet (section 2).
- A fabrication defense in the formula. Attestation, contributor caps, and
  delayed settlement own that.
- Scoring the attested share only, or weighting each term by coverage. The
  witness records a final-call class, not a share, so this design uses one light
  binary weight inside substance.
- Any change to novelty, the embedder, the index, or the award amount, beyond
  the chunk weights of section 8.2.
- A conditional scorer dependency. The bundle runtime requires every package to
  name a scorer descriptor, so the substance package names one that it never
  calls. A separate change makes the dependency depend on the family.

## 11. Open owner decisions

These decisions change the model and need an owner ruling before a substance
package is promoted.

- The attestation weight. `att_floor_mult = 0.90` gives an attested trace about
  11 percent more substance. Confirm that this is the intended size, or set
  another.
- Removal of insensitive terms. The sensitivity analysis lists each term whose
  removal keeps the ranking and the gate within tolerance and still passes the
  guardrail (section 7). Decide whether to remove it in the next bundle version.
