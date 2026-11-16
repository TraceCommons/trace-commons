# Substance v1: design

Status: superseded. Not implemented.

> **Superseded (2026-10-23).**
> [The qualitative substance design](2026-10-21-substance-qualitative-rlvr-design.md)
> replaces this design in full: the model, the integration, the guardrail
> recipe, and the tests. This file is kept only as the record of round 1 of the
> review on PR #1472. Do not implement from it.

This design is a direction, not the final model. The terms, the parameters, and
the floors can change once real data runs through the fit. Treat the specific
terms and values here as a starting point for calibration, not a fixed result.

## 1. Why

A trace earns credits. A fabricated trace that looks like substantive work earns
credits it did not earn. Substance is the gate that separates substantive work
from fabricated work.

The ingest server already defends the submission path. It de-duplicates
submissions, limits the submission rate, and bounds submission time. Substance
does not replace these defenses. Substance adds one thing. It detects specific,
named ways a trace is fabricated to look like substantive work.

Verified inference is the strongest signal of substantive work. The witness
proves one model call was real. It proves only the last call. Every other turn
can be fabricated. It proves the call occurred, not that the work is sound. So
substantive activity can still be low value, and a substance classifier is still
required.

The current metric does not do this. It scores model surprise with a
`PerplexityScorer` and passes a trace when surprise is high enough. The evidence
shows surprise is not substance:

- Short, low-substance sessions score highest. Correlation with length is -0.49
  (per-author shadow spec, 2026-09-18).
- Empty scaffolding dilutes the score. Most tool events in the sample carried no
  content (scaffold analysis, 2026-09-18).
- Random bytes earn the top score. Encoded content peaked at forty times the p90
  (27B re-score, 2026-07-12).
- Perplexity and novelty fail in the same direction, which hands an attacker a
  gradient (red team, 2026-07-12).
- Nothing in the scoring path reads the outcome, success, or replay fields
  (Issue #199).

The gate definition is in `docs/trace-commons.md:8`.

The approach:

1. Name the attack vectors. Each vector is a way to fabricate a trace so it
   looks like substantive work.
2. Generate a synthetic dataset. It pairs a fabricated trace for each vector
   with a real trace.
3. Fit substance offline to separate the fabricated traces from the real ones.
4. Certify the result as evidence (section 7).

The attack vectors substance v1 targets:

- Noise padding: incompressible bytes added to inflate apparent content.
- Empty scaffold: tool calls with no arguments and no results, to fake activity.
- Missing evidence: tool results stripped, so a claim has no support.
- Truncation: part of the trajectory removed.
- Duplication: events repeated to inflate volume.
- Asserted outcome: a success claimed with no work.
- Template filler: one boilerplate paragraph repeated as prose.

Section 7 generates exactly this catalog.

Limitations:

- Substance v1 covers the named vectors only. A vector not in the catalog is not
  caught.
- A pass on the synthetic dataset proves substance avoids these fabrications. It
  does not prove economic value.
- Genuine but low-value work is a separate problem. Substance v1 does not catch
  substantive work of low value. That is a harder problem for a later version.
- Stronger fabrication needs stronger metrics. That is a later version.

## 2. What a genuine trace looks like

A genuine trace is worth money, because a trainer can learn from it. A genuine
trace also holds evidence of substantive work. Substance reads four features of
the envelope. Each feature is present in substantive work and costly to fake.

| Feature | Question | Envelope source |
|---|---|---|
| Work | Did the agent act in a real environment? | `tool_call` arguments, `tool_result` content, tool diversity, side effects |
| Outcome | Does the trace say how it ended, and is a failure explained? | `outcome.task_success`, `outcome.human_correction`, `outcome.user_feedback` |
| Density | Is the agent's own text content, not filler and not noise? | compression ratio of `assistant_message` and `reasoning` content |
| Replay | Can a consumer rebuild the trajectory? | `replay_sufficiency` (protocol crate) |

Substance reads these fields at face value. It assumes the field values are not
forged. The assumption is safe because of the gate. The composite is
multiplicative, so one forged field does not pass a trace. Section 8 shows why
forging a field does not pay.

Scarcity is a fifth property. Novelty owns it. Substance does not measure it.

## 3. Projection

Every score reads a projection of the envelope, not the raw bytes. The
projection is the set of transforms applied before a term or novelty runs.
Substance v1 uses three, and no more.

1. Agent-prose selection. Concatenate the `redacted_content` of every
   `assistant_message` and `reasoning` event, in event order. The density term
   reads this text. `tool_result` and `user_message` content are excluded,
   because the agent did not write them. The bytes are read as-is, because the
   compression ratio must see the real bytes.
2. Tool-call canonicalization and de-duplication. Serialize each `tool_call`'s
   arguments as canonical JSON. Collapse calls that are identical by
   (`tool_name`, canonical arguments) to one. The work term reads this. A
   repeated call is one unit of work.
3. Chunking. Split the render into chunks. Novelty reads these chunks. The
   chunker estimates tokens per chunk with its own tokenizer; the count is a
   guard, not a score.

The substance terms use projections 1 and 2. Novelty uses projection 3. The
substance-only evaluation in section 7 runs projections 1 and 2 and skips
projection 3.

The projection is deterministic. It runs no model and makes no remote call.
`substance_version` covers the projection as well as the formula (section 4.6),
so a change to any transform bumps `substance_version` and changes the bundle
id. v1 keeps one version label, because the projection and the formula move
together; a later version can split them if they change apart. The compatibility
bundle does not use `substance_version` and is unchanged.

## 4. The substance function

Substance v1 is a pure function of the envelope. It runs no model, no GPU, and
no remote call. It reads the projection of section 3 and returns one number in
[0, 1].

All arithmetic is fixed-point micros, like `credit_quality`. Every constant
lives in the bundle configuration and is hashed into the bundle id. Every term
saturates, so no term offers an unbounded gradient.

The parameters are not guessed. The offline fit of section 7 sets them. The
table below names every trained parameter. The term sections that follow give
each formula and name its parameters. They do not print a starting value. A
start value is an input to the fit, not part of this design.

| Parameter | Term | Meaning |
|---|---|---|
| `work_calls_ceil` | work | saturation ceiling for paired calls |
| `work_tools_ceil` | work | saturation ceiling for distinct tools |
| `work_writes_ceil` | work | saturation ceiling for write actions |
| `work_floor_mult` | work | graded floor for the work term (set above 0) |
| `outcome_success` | outcome | score for a success |
| `outcome_partial` | outcome | score for a partial |
| `outcome_corrected_failure` | outcome | score for a corrected failure |
| `outcome_failure` | outcome | score for an uncorrected failure |
| `outcome_unknown` | outcome | score for an unknown outcome |
| `outcome_thumbs_down_mult` | outcome | multiplier for a thumbs-down with no correction |
| `density_lo` | density | lower edge of the compression band |
| `density_hi` | density | upper edge of the compression band |
| `density_ramp` | density | ramp width outside the band |
| `density_min_bytes` | density | minimum agent prose before the band applies |
| `density_floor_mult` | density | graded floor for the density term (set above 0) |
| `replay_floor_mult` | replay | graded floor for the replay term (set above 0) |
| `substance_floor_micros` | composite | the pass floor |
| `sub_ceil` | cq4 | substance saturation ceiling in `q` |
| `sub_floor_mult` | cq4 | substance graded floor in `q` |

The fit sets every parameter in this table. The bundle configuration pins them.
The bundle id hashes them. Qualification certifies them. The server never fits.

`sat(x, ceil) = ln(1 + x) / ln(1 + ceil)`, clamped to [0, 1]. This is the
`saturating_term` curve of `credit_quality.rs`, with the floor at 0.

`graded(v, m) = m + (1 - m) · v`. This is `graded_floor` from the same module.

### 4.1 Work

From the event list, after projection 2:

- `paired_calls`: `tool_call` events that carry arguments (an object under
  `arguments`, `args`, `parameters`, `params`, or `input`, the same detection
  as `replay_sufficiency`) and that have a matching `tool_result` with non-empty
  `redacted_content` or `structured_payload`. The match is by `tool_call_id`,
  else by the result's `parent_event_id`. Projection 2 has already collapsed
  duplicate calls, so a repeated loop earns nothing extra.
- `distinct_tools`: distinct `tool_name` values over the paired calls.
- `write_actions`: events whose `side_effect` is `local_write` or
  `external_write`.

```
work = 0.5 · sat(paired_calls, work_calls_ceil)
     + 0.25 · sat(distinct_tools, work_tools_ceil)
     + 0.25 · sat(write_actions, work_writes_ceil)
work = graded(work, work_floor_mult)
```

The fit sets `work_calls_ceil`, `work_tools_ceil`, `work_writes_ceil`, and
`work_floor_mult`. `work = graded(work, work_floor_mult)`, and the fit sets
`work_floor_mult` above 0. So a trace with no paired call does not score 0 on
work; it scores the work floor. A tool-free trace is ranked low, not vetoed.
Whether it passes is the floor decision of section 4.5.

### 4.2 Outcome

From `outcome`:

| `task_success` | `human_correction` present | outcome |
|---|---|---|
| `success` | any | `outcome_success` |
| `partial` | any | `outcome_partial` |
| `failure` | yes | `outcome_corrected_failure` |
| `failure` | no | `outcome_failure` |
| `unknown` | yes | `outcome_corrected_failure` |
| `unknown` | no | `outcome_unknown` |

`user_feedback = thumbs_down` with no correction multiplies by
`outcome_thumbs_down_mult`. `correction` as feedback counts as a correction
present.

A corrected failure scores as high as a success on purpose. A failure that a
person explained is premium training data, not filler. Every outcome value is
above zero, so this term grades and never gates. All of its inputs are
client-asserted; section 8 explains why that is safe.

### 4.3 Density

The density term reads the agent prose of projection 1.

`ratio = deflate_len / raw_len`, with DEFLATE at a fixed level through the
`flate2` crate. The compressor and its level are part of `substance_version`.

```
d_raw = 1                                   if lo <= ratio <= hi
      = (ratio - (lo - ramp)) / ramp         if lo - ramp < ratio < lo
      = ((hi + ramp) - ratio) / ramp         if hi < ratio < hi + ramp
      = 0                                    otherwise
density = graded(d_raw, density_floor_mult)
```

The fit sets `density_lo`, `density_hi`, `density_ramp`, `density_min_bytes`,
and `density_floor_mult` above 0. Under `density_min_bytes` `d_raw` is 0, so the
term is the density floor, not 0. A code-heavy or log-heavy trace that
compresses below the band is ranked low, not vetoed.

The band is two-sided on purpose. Template filler compresses too well and falls
under `lo`. Base64 or random content does not compress and rises over `hi`.
Perplexity had only one side. Noise padding cannot raise the term; it only
lowers it, so padding gains nothing.

### 4.4 Replay

`replay = ReplaySufficiency::score()` from
`trace_commons_protocol::trace_contribution::replay_sufficiency`: equal thirds
for an initial prompt with content, arguments on calls, and results with
content, with partial coverage earning partial credit. The envelope's
`replayable = false` veto zeroes it, as today.

`replay = graded(replay, replay_floor_mult)`. The fit sets `replay_floor_mult`.
Replay grades; it does not gate.

### 4.5 Composite and gate

```
substance = work · density · outcome · replay          in [0, 1]
substance_passed = substance_micros >= substance_floor_micros
```

Every term is graded with a floor above 0, so no single term forces the
composite to 0. The composite ranks a trace; it does not gate it. One threshold
gates: `substance_floor_micros`. So the pass or fail decision lives in the
floor, not in any one term. A trace with no tool use, or with highly
compressible prose, scores low but not 0. Whether such a trace passes is a floor
decision, set in qualification. It is not a property of the formula, and this
design does not make it.

`substance_floor_micros` has no starting point. The offline fit sets it
(section 7), and the fit sets each term's floor multiplier above 0. A bundle
configuration without `substance_floor_micros` is refused with
`substance_floor_unset`.

### 4.6 Version

`substance_version = "substance.v1+deflate6+events.v1+chunker.v1"`. It is one
label for the whole bundle shape. It is composed from the formula version, the
compressor, the render version (`CANONICAL_RENDER_VERSION`), and the chunker
version (`CHUNKER_VERSION`). A change to a formula, a field mapping, the
compressor, or any projection transform bumps it. v1 keeps one label, because
the projection and the formula move together; a later version can split the
projection out if they change apart. A constant does not bump the version; it
lives in the bundle configuration and changes the bundle id.

## 5. Credit quality `cq4`

`q`, credit quality, stays the economic seam. Its four consumers do not change.
Only its inputs change. The inputs move from (perplexity, peak, novelty) to
(substance, novelty), under a new calibration era, `cq4`.

`credit_quality.rs` gains a second function beside `credit_quality`:

```
credit_quality_v4(substance_micros, nov_rep_micros, k) -> CreditQualityScore
  h = graded(sat_term(substance, k.sub_floor, k.sub_ceil), k.sub_floor_mult)
  g = graded(sat_term(novelty,   k.nov_floor, k.nov_ceil), k.nov_floor_mult)
  q = clamp01(h · g)
```

The parameters `k` are not a code constant. They live in the bundle
configuration, and they are hashed into the bundle id. The offline fit sets
them (section 7). Any code constant is a starting point for the fit, never the
production value. A change to any `cq4` parameter produces a new bundle id and a
new qualification. No code edit can change `q` without one.

`k` for the substance bundle:

- `sub_floor = substance_floor_micros`, the same value as the gate.
- `sub_ceil`, `sub_floor_mult`: the substance-side knobs, set by the fit.
- `nov_floor`, `nov_ceil`, `nov_floor_mult`: the novelty half, taken from the
  bundle's novelty configuration, unchanged.
- `version = 4`.

There is no anomaly term. It existed to catch one inserted garbage chunk under
perplexity, and the density band covers that case. `anomaly_ratio_micros` is
reported as 1_000_000 and `anomaly_withheld` as false, so `CreditQualityScore`
keeps its shape.

`CREDIT_QUALITY_SCHEDULE` is unchanged. It is the legacy path's time-based
schedule of code constants, and the legacy path keeps scoring with it. The
substance bundle does not call `constants_at(now)`. It closes over its decision
function, which is the classifier-audit principle.

The public label is `cq4`. The account-trust rule allowlist and the
credit-estimate table add it (Stage 2).

## 6. The bundle

### 6.1 Family and ids

Substance changes only the Score stage. So the substance bundle is the
compatibility family with one new Score id. It reuses the compatibility
admission, review, and settle implementation ids unchanged. The Score id is new,
`trace_commons.score.substance.v1`, because the classifier-audit principle
requires the id to name the new algorithm. The rule id is `substance_novelty_v1`.

The four ids form a recognized family tuple, so the runtime runs the package.
The guard still holds: the runtime runs a package only when its four stage ids
form one known, qualified-together tuple. Here three of the four ids are the
compatibility ids, and only the Score id is new. A from-scratch family is not
needed, and this design does not add one.

### 6.2 Configuration

The substance score policy holds its configuration, the same way
`CompatibilityScorePolicy` holds `CompatibilityBundleConfig`. The type is
`SubstanceScoreConfig`, in a new module
`crates/trace-commons-server/src/versioned_pipeline_substance.rs` (AGPL header),
`deny_unknown_fields`. It is a score-policy configuration, not a new
bundle-level abstraction.

It holds:

- `novelty: NoveltyConfig`, one named field. The score stage computes novelty as
  well as substance, so it needs the novelty parameters. `NoveltyConfig` is a
  shared type (`projection_id`, `index_id`, `novelty_floor_micros`,
  `embed_insert_novelty_micros`, `top_k`, the four chunk knobs,
  `novelty_utility_microcredits`, and the instrument). The shared novelty
  function of section 11 takes a `NoveltyConfig`. `CompatibilityBundleConfig`
  stays unchanged on the wire and builds a `NoveltyConfig` from its own inline
  fields, so its bytes and bundle id do not move. The two configs share the type
  in code, not the serialized shape.
- The substance constants of section 4.
- The `cq4` parameters of section 5: `sub_ceil`, `sub_floor_mult`. `sub_floor`
  reuses `substance_floor_micros`. The novelty-side `cq4` parameters reuse the
  `NoveltyConfig` above.
- `substance_floor_micros`, `credit_quality_version` (must be 4).
- `substance_version` (section 4.6), which covers the projection too.

No `scorer_model_id`, no perplexity floor, no tail floor.

Every field that shapes a score is here. The offline fit sets the substance
constants, the `cq4` parameters, and the floor. The bundle id hashes them all.
No model parameter lives in code.

### 6.3 Score policy

`SubstanceScorePolicy::evaluate`, in order:

1. Deserialize `reviewed_artifact` as `TraceContributionEnvelope`. A parse
   failure is the permanent `substance_envelope_unreadable`.
2. Compute the four terms, the composite, and `substance_passed`. This reads
   projections 1 and 2 only.
3. Chunk the envelope, embed each chunk, query the index, and aggregate
   novelty. This reads projection 3. It is the compatibility code path, moved
   behind a shared function so the two policies call one implementation.
4. `q = credit_quality_v4(substance, novelty, k)`, with `k` from the bundle
   configuration (section 5).
5. Index membership and the award follow the compatibility rule with
   `substance_passed` in place of `quality_passed`.
6. Fill the evidence.

Step 3 keeps the Score lock and the `UnappliedIndexCommandsReader`; both
protect index consistency and apply to any family that writes the index. The
runtime check at `versioned_pipeline.rs:13089` becomes "the family writes the
index", not "the family is compatibility".

### 6.4 Evidence

Seven flat `Option` fields on `ScoreEvidence`, `ScoreEvidenceFields`, the
`TryFrom`, and `fixed()`, each with `serde(default, skip_serializing_if)`:

```
substance_micros, substance_passed, substance_version,
substance_work_micros, substance_outcome_micros,
substance_density_micros, substance_replay_micros
```

`credit_quality_micros` and `credit_quality_version` are filled as today.
The perplexity fields stay `None`. Compatibility outcomes serialize
byte-identically, so stored hashes and the 10k comparison report do not move.

The loader drops a key the struct lacks. In a mixed-version rollout an old
binary reads a substance row and sees no substance fields. That is accepted:
the old binary also cannot run the substance family, so it never reads such a
row for a decision.

### 6.5 Runtime arms

- `from_package_with_runtime` gains a third arm for the substance id tuple.
  The substance policy needs no scorer, but the signature drops nothing, because
  the compatibility arm still needs its scorer.
- `substance_package()` names the scorer descriptor as a dependency, the same way
  the minimal package does. `from_package_with_runtime` calls
  `require_named_dependency` on the scorer for every package, before the family
  dispatch (`versioned_pipeline_bundle.rs:678`). So the package must name a
  scorer descriptor. The `SubstanceScorePolicy` never holds or calls it. This is
  a known smell, out of scope here; see section 10.
- `package_configuration_is_qualifiable` gains the family.
- `validate_production_package` accepts the substance id tuple.
- The compat-only branches at `versioned_pipeline.rs:13552` (legacy row
  check), `:13616` (duplicate verdict), and `:5785` (legacy row write) are
  not taken for the substance family. The duplicate verdict is a legacy-row
  consumer and returns with Stage 2 on the evidence path.
- `trace_credit_event` is `NoveltyUtility`, as for compatibility.
- `pipeline.py package --bundle substance` and the production assembly learn
  the family.

### 6.6 Chunker ownership of token counts

`aggregate_chunked_novelty(novelty, chunk_tokens, min_chunk_tokens)` takes
token counts. The chunker produces them, with its own tokenizer, as projection
3 defines. The count is a guard, not a score. Novelty depends on no scorer.

`CANONICAL_RENDER_VERSION = "events.v1"` (`chunker.rs:329`) and a new
`CHUNKER_VERSION` constant are two of the parts that compose `substance_version`
(section 4.6). The bundle id changes when the projection changes. The
compatibility bundle is not changed.

## 7. Evaluation

### 7.1 Principle

No ground truth of economic value exists. So substance is not fit to value. It
is fit to a labeled dataset of real and fabricated traces, built from the attack
vectors of section 1. One pinned dataset defines both the fit and the test. The
fit runs offline. Its output is certified evidence. The server trusts the
evidence and never re-fits.

### 7.2 The benchmark dataset

Format `trace_commons.substance_benchmark.v1`:

```json
{
  "schema": "trace_commons.substance_benchmark.v1",
  "recipe": "substance-benchmark-v1",
  "manifest": {
    "params": { "append_noise_bytes": 4096, "scaffold_calls": 20, "truncate_fraction": 0.5 },
    "source": { "hf_pin_digest": "sha256:...", "seed": 42 },
    "split": { "seed": 42, "test_fraction": 0.3, "by": "base" }
  },
  "envelopes": [
    { "id": "...", "base": null, "split": "train", "label": "real", "vector": null, "envelope": { ... } },
    { "id": "...", "base": "...", "split": "train", "label": "fabricated", "vector": "append_noise", "envelope": { ... } }
  ],
  "pairs": [
    { "base": "...", "variant": "...", "transform": "append_noise",
      "expect": "variant_le_base" }
  ],
  "trivial_baseline": {
    "features": { "byte_count": 0.09, "event_count": 0.04, "tool_call_count": 0.02 },
    "passed": true
  }
}
```

Each envelope carries a `label`, `real` or `fabricated`, and the `vector` for a
fabricated one. The labels drive the false-positive and false-negative rates.
Each envelope also carries a `split`, `train` or `test`, and a `base` (the id of
the real trace it comes from; `null` for a base). The split is assigned by base,
so every variant of one base lands on the same side. The fit reads `train` only;
acceptance is measured on `test` (section 7.5, section 7.6). The `pairs` drive
the monotonicity check; `expect` is one of `variant_lt_base`, `variant_le_base`,
`variant_eq_base`, `variant_gt_base`. The `manifest` records the fixed recipe
parameters, the generation inputs, and the split (seed, test fraction, split by
base). `trivial_baseline`
records how well each dumb feature predicts the label, measured as AUC. The
qualification policy sets one AUC bar `T`; this spec does not choose it. `T` is
used twice. Here the generator rejects a dataset when a dumb feature's AUC is
above `T`. In section 7.6 substance must score above `T`. So it is one number,
a single AUC bar, not two. A fixture is committed under
`docs/superpowers/specs/fixtures/` with its digest in the companion `.digest`.

### 7.3 The generator

`pipeline.py dataset generate --recipe substance-benchmark-v1 --source <hf-pin>
--seed N --out <file>`. The generator is Rust (a subcommand of
`trace-commons-pipeline-corpus-export`, which already holds the pinned HF loader
and the envelope builder), and `pipeline.py` drives it. A Python generator would
be the fifth copy of the text extraction.

The recipe is a fixed parameter set. Each transform names its parameters with
values, and the generator writes them into the manifest. A change to any
parameter is a new recipe id. The source pin and the seed are recorded in the
manifest, so a run reproduces.

Base selection: the generator takes a base envelope only when it has at least
one paired tool call, at least 256 bytes of agent prose, and an `outcome` block.
These are structural checks; the generator never runs the score. They keep every
`lt` pair meaningful, because a base at substance 0 cannot be lowered. The base
set must include diverse prose styles, code-heavy and log-heavy among them, so
the false-negative rate sees the styles Finding 1 puts at risk. Each base is
labeled `real`.

Split assignment: the generator assigns each base to `train` or `test`,
deterministically, seeded by base id, at the manifest's `test_fraction`. Every
variant inherits its base's side, so no base straddles the split. The split is
recorded in the manifest, so a run reproduces. The fit never sees a `test` base.

Recipe `substance-benchmark-v1`, one variant per base envelope per transform:

| Transform | Change | `expect` | Label |
|---|---|---|---|
| `append_noise` | 4 KiB of base64 random bytes appended to the last assistant message | `variant_le_base` | fabricated |
| `add_scaffold` | 20 `tool_call` events with no arguments and no results | `variant_le_base` | fabricated |
| `strip_results` | every `tool_result` body removed | `variant_lt_base` | fabricated |
| `truncate_half` | the second half of the events removed | `variant_le_base` | fabricated |
| `duplicate_events` | the event list repeated once | `variant_le_base` | fabricated |
| `assert_success_no_work` | all tool events removed, `task_success = success` | `variant_lt_base` | fabricated |
| `template_filler` | every assistant message replaced by one fixed paragraph, repeated to the original byte length | `variant_lt_base` | fabricated |
| `correct_failure` | base: `task_success = failure`, no correction (a derived envelope); variant: the same plus a `human_correction` | `variant_gt_base` | real |

The first seven transforms are the attack vectors of section 1; each variant is
labeled `fabricated`. `correct_failure` is a positive control: a genuine
improvement must raise the score, so both its traces are labeled `real`.

`add_scaffold` and `truncate_half` are `le`, not `eq` or `lt`: scaffold lowers
replay coverage honestly, and a truncation that removes no tool call leaves work
unchanged. The contract is that neither can raise the score.

The baseline the substance bundle must beat is the dumb-feature battery, not a
scorer. The battery is byte count, event count, and tool-call count. Each is a
trivial predictor of the label. The null hypothesis is that substance separates
real from fabricated no better than the best single dumb feature. The evaluation
reports the best dumb-feature AUC, and substance's AUC must be above the bar `T`
of section 7.6. No scorer runs, so the evaluation stays isolated.

Before it writes, the generator runs the dumb-feature battery (`corpus-validity`,
extended with event count, tool-call count, and byte count) over the envelopes
against the labels. If any feature's AUC is above the bar `T`, the dataset is
refused, and the result is stored as `trivial_baseline`. That is the #204 lesson
as a contract.

### 7.4 The evaluation command

`pipeline.py evaluate --dataset <file> --bundle <family | package>`. For the
substance bundle it:

1. Checks the dataset digest.
2. Builds the bundle from the family or the package.
3. Computes the substance terms, the composite, `substance_passed`, and `q`
   from each envelope. It does not chunk, embed, query the index, or compute
   novelty. It runs projections 1 and 2 only. No HTTP, no Admission, no Settle.
4. Writes per-trace records (every evidence field and the label) to a records
   file as it goes, and resumes from that file when restarted.
5. Writes the report through `pipeline_tooling`: canonical digest, markdown, the
   hash-only `validate_evidence` check, optional `--archive` to the lab catalog.

The isolation holds by construction. The command never runs the index, so it
cannot measure index behavior. The substance run needs no model, GPU, embedder,
or index. The synthetic fixture runs in CI under `pipeline qualification and
restore`, in seconds. The baseline the report compares against is the
dumb-feature battery of section 7.3, not a scorer.

The report holds:

- The separation table: the false-positive rate and the false-negative rate at
  each candidate floor, over the labels. This table is the ROC curve as a set of
  operating points; each candidate floor is one threshold. The floor is the
  operating point that meets the section 7.6 bounds.
- The pairwise table: per transform, the share of pairs that satisfy `expect`,
  with the term that moved most.
- The baseline-dominance table: for each dumb feature, its AUC against the
  `label` and its Spearman correlation with `substance_micros`.
- Percentiles of `substance_micros` and of each term over the real envelopes.
- The outcome join, when the dataset has outcome labels: AUC of
  `substance_micros` against `task_success`, with the length covariate beside it
  (#199's two guards).

### 7.5 The fit

The fit is one offline step. It reads the `train` split of the labeled benchmark
only, and sets the substance constants, the density band, the floor, and the
`cq4` parameters. It never reads the `test` split. So the acceptance numbers of
section 7.6 are a generalization estimate, not a training score.

- Objective: separate `real` from `fabricated`. The floor must meet the
  false-positive and false-negative bounds of section 7.6.
- Constraint: the pairwise monotonicity contract holds. No fabricated variant
  raises the score.
- Method: a bounded, deterministic parameter search over the composite. The
  model class is the composite of section 4; the fit does not change its form.
- Context: the pinned real corpus (the 10k HF pin the comparison uses) gives
  the percentile tables, so the floor sits where real prose sits.

The output is the fit parameters plus the report, with a digest. This is the
certified evidence, in the same form the bundle qualification already uses. The
bundle configuration pins the parameters and the evidence digest. Qualification
verifies the evidence. The server does not re-fit and does not re-calibrate. A
change to any parameter produces a new bundle id, a new fit, and a new
qualification. There is no env var and no code constant for a substance
parameter.

### 7.6 Acceptance

The gated bounds below are measured on the `test` split. The `train` numbers are
reported beside them, so the train-test gap is visible. Each class on the `test`
split must hold at least 125 traces, the #199 minimum, for a gated measure.

Substance v1 is accepted for qualification when, on the `test` split:

1. The floor meets both bounds. The false-positive rate, a fabricated trace that
   passes, is at or below `max_false_positive_rate`. The false-negative rate, a
   real trace that fails, is at or below `max_false_negative_rate`. The
   qualification policy sets both bounds; this spec does not choose their values.
   Both bounds are recorded with the qualification.
2. Every transform row in the pairwise table is at or above `min_pairwise_share`
   for the substance bundle. A row records the share of pairs that satisfy
   `expect`. The qualification policy sets `min_pairwise_share`; this spec does
   not choose its value.
3. Substance's AUC is above the bar `T`; the best dumb feature's AUC is at or
   below `T`. The qualification policy sets `T`; it is the same AUC bar the
   dataset guard of section 7.3 uses. This criterion is weak evidence on its own,
   because the transforms are built to defeat the dumb counts. The honest
   instrument is the `Spearman(dumb feature, substance)` line in the
   baseline-dominance table, which shows whether substance merely tracks a dumb
   count. Read that line first.
4. The outcome join is reported with the length covariate. It is not a gate
   until each class holds at least 125 traces, the #199 minimum.

A pass on the synthetic benchmark proves substance avoids the named
fabrications. It does not prove economic value. Only the outcome join can, and
the pilot has not produced that data yet. This limit is stated so a synthetic
pass is not sold as a value proof, which is what happened with the A2.6 corpus.

The novelty byte-identity check moves to the bundle integration tests
(section 12), because the substance evaluation no longer runs novelty.

## 8. Gaming

Every outcome and feedback field is client-asserted. The composite is
multiplicative, so a weak term pulls the whole score down. An asserted success
with no paired tool call earns only the work floor on the work term, so its
substance is low and falls under `substance_floor_micros`. The floor is the
gate, not a single term. An asserted correction on a trace with substantive work
earns at most the outcome ceiling, 1.0, which a plain success also earns.

Work can be fabricated with an LLM call per trace, as perplexity could. The
cost is the same; the gain is bounded by saturation at `work_calls_ceil`.
Compressibility can be tuned, and that gains at most one term. The witness
proves the last model call; everything else is bounded, graded, and recorded
with its sub-terms, so an audit can see which term carried a suspicious
trace. Contributor caps and delayed settlement remain the economic defense.

## 9. Stage 2: the `q` consumers read the pipeline evidence

The substance family writes no legacy row. Four consumers read `q`
from `trace_gate_decisions` today. Each moves to a read of the pipeline
Score evidence for pipeline-scored traces, keeping the legacy read for
legacy rows:

| Consumer | Today | Stage 2 |
|---|---|---|
| contributor points (`credit_points_from_quality_micros`) | legacy row | pipeline status already reads `evidence->>'credit_quality_micros'`; the points path uses the same read |
| contributor cap increment (`contributor_cap.rs`) | `list_gate_decisions_for_credit_scoring` | a union read over the legacy rows and the Score evidence |
| account-trust growth rule (`account_trust_rule.rs`) | `q` plus version allowlist | the same, with `cq4` allowed and the evidence read |
| credit-estimate fit (`credit_estimate_fit.rs`) | scores by submission | the same union read; label `cq4` |

The duplicate verdict, which is a legacy-row consumer today, moves with the
cap increment. The signed score attestation adds schema v3 with
`substance_micros` beside `credit_quality_micros`; v2 stays readable. The
public analytics gains a `substance_failed` label beside `perplexity_failed`.

The exact read paths are confirmed at plan time; the consumer sweep of
2026-10-14 is the inventory.

### Future direction

Substance v1 is a hand-structured composite. Section 4 fixes its form, and the
fit only sets its parameters. A stronger version replaces the fixed form with a
learned decision model, a judge over the trace. A judge lifts the
fabrication-detection ceiling that section 1 names.

The judge must run inside the operator boundary. A hosted judge is ruled out,
because it sends the trace to a third party, which breaks the fail-closed
privacy model (the tabled Jev design, 2026-09-22). NEAR AI open weights give a
path to a local judge that keeps the trace inside the boundary.

The benchmark built here is the asset that makes a learned judge possible. Its
labels, its pairwise relations, and its trivial-baseline guard are the training
set, the evaluation set, and the acceptance harness. Substance v1 builds the
ground the future model stands on.

## 10. Non-goals

- A model-backed density term (per-author perplexity band or a judge). Later
  version, only if the pairwise table shows filler passing.
- Per-event `success` and `failure_modes` as terms. Sparse today; later.
- Any change to novelty, the embedder, the index, or the award amount.
- A rename or relaxation of any `trace_gate_decisions` column.
- Stage 3: removal of the perplexity driver, the rescore route, the 21 env
  names, and the `calibrate` and bake-off scoring loops.
- A two-bundle `pipeline.py compare`. `evaluate` on one file with two bundles
  covers the Score-stage comparison this design needs.
- Evaluating more pipeline decision points. The harness evaluates one isolated
  component first, the substance score. The index, novelty, and multi-stage
  decisions come later.
- Making the scorer dependency conditional in `from_package_with_runtime`. A
  model-free bundle must still name a scorer descriptor it never uses (section
  6.5). This is a known smell. A separate workstream makes the scorer dependency
  conditional on the family; it touches a shared signature and all callers.

## 11. Components

| Component | Where |
|---|---|
| `SubstanceScoreConfig` (substance constants, `cq4` parameters, floor, `substance_version`, `novelty: NoveltyConfig`), `SubstanceScorePolicy`, the four terms | new `crates/trace-commons-server/src/versioned_pipeline_substance.rs` |
| the projection layer: prose selection, tool-call canonicalization and de-duplication | `versioned_pipeline_substance.rs`, with chunking in the chunker |
| `NoveltyConfig` (shared type) and the shared novelty evaluation (moved out of `CompatibilityScorePolicy`), taking a `NoveltyConfig` | `versioned_pipeline_compat.rs`, called by both policies; `CompatibilityBundleConfig` builds a `NoveltyConfig` from its inline fields |
| `credit_quality_v4`, parameters taken from the bundle config | `credit_quality.rs` |
| seven evidence fields | `crates/trace-commons-gate-api/src/pipeline.rs` |
| chunk token estimate, `CHUNKER_VERSION` | `crates/trace-commons-gate-enclave/src/chunker.rs` |
| `substance_package()`, runtime arm, qualifiable family | `versioned_pipeline_bundle.rs` |
| production acceptance | `versioned_pipeline_qualification.rs` |
| family-generic runtime branches | `versioned_pipeline.rs` |
| benchmark format, generator, recipe | `trace-commons-pipeline-corpus-export` bin, new `substance_benchmark` module |
| `evaluate` driver, metrics, report | library module `versioned_pipeline_evaluation.rs` (dataset loader, pairwise and dominance metrics, records, report), driven by an `#[ignore]` test in a new `pipeline_evaluate_tests.rs` that `pipeline.py evaluate` runs, the same shape as `run` and `compare`; `pipeline_tooling/evaluate.py` |
| `pipeline.py dataset generate`, `pipeline.py evaluate`, `--bundle substance` | `scripts/operator/pipeline.py` |
| committed fixture | `docs/superpowers/specs/fixtures/substance-benchmark-v1.json` |
| runbook | `docs/operator/substance-evaluation.md`, listed in `docs/operator/README.md` |
| Stage 2 consumer reads | `contributor_cap.rs`, `account_trust_rule.rs`, `credit_estimate_fit.rs`, the points path, `trace_score_attestation.rs` |

## 12. Testing

- Golden tests: fixed envelopes with pinned term values and composite, per
  `substance_version`. A formula change breaks a pinned value and forces a
  version bump.
- Contract tests on the committed benchmark fixture, in CI: the pairwise
  table at 1.0 for the substance family on the fixture.
- The existing compatibility baseline test
  (`compatibility_baseline_is_executable_and_detects_drift`) stays green,
  which proves the evidence change is byte-neutral for that family.
- Substance isolation: the `evaluate` run for the substance bundle builds with
  no embedder and no index, which proves the score reads neither.
- Novelty byte-identity across the two families on one file.
- Qualification: `pipeline.py qualify` with `--bundle substance` passes the
  package checks and the corpus runs.
- Stage 2: each consumer has one test with a legacy row and one with a
  pipeline evidence row, both yielding the same consumer output for equal `q`.
