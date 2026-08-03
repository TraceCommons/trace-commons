# Gate Calibration

How to choose the three gate floors —
`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` — and when to flip credit
emission on.

Three phases, in order. **Phase 0** (model bake-off) happens once per
production deployment, before Phase 1 runs.

## Phase 0 — Model bake-off (A2.1)

> **A2.5 callout — the bake-off picks a model, not a discriminator.**
> The A2.3c + A2.4 bake-offs measured aggregate-perplexity AUC < 0.5
> across all four candidates and both corpus variants. The metric is
> inverted on the modern aligned-LLM ecosystem — models find OASST2
> reasoning *less* surprising than duplicate content. The bake-off's
> `winner_id` is therefore *not* a "this model has good perplexity
> discrimination" signal; it is "this model has the best
> operationally-acceptable score under the committed decision rule."
> Phase 1's perplexity floor ships at 0 for pilot launch regardless
> of which model the bake-off picks. See
> `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`
> for the data and reasoning. Phase 0 is still worth running — it
> still picks model identity, license, throughput, and the model
> against which the tail-fraction floor will eventually be
> calibrated.

Goal: empirically pick the perplexity scorer model from a candidate set,
rather than carrying the incumbent on tooling-maturity grounds. Run once
per deployment, before any floor calibration — floors are scaled to the
winning model's perplexity distribution, so picking the model first is
the prerequisite.

The bake-off binary is the `bake-off` subcommand of
`trace-commons-gate-calibrate`. The decision rule (`0.6 * AUC + 0.3 *
stability + 0.1 * tail_range`, with determinism gate, discrimination
floor, throughput floor, and license / size / recency tiebreakers) is
committed before the run; the winner is determined by formula, not by
inspection.

The discrimination floor (rule version 2) drops any candidate at or below
`AUC = 0.5` before the throughput floor is computed. AUC 0.5 is chance, so
a candidate at or under it carries no usable signal at any speed, and
letting one into the throughput comparison lets it set a floor that
eliminates a slower candidate that does discriminate. A run where nothing
clears the floor has no winner rather than a fast one.

Authoritative design and rollout sequence:

- Spec: `docs/superpowers/specs/2026-05-13-model-bakeoff-retrofit-design.md`
- Plan: `docs/superpowers/plans/2026-05-13-model-bakeoff-retrofit.md`

### Build the corpus

```sh
HF_TOKEN=hf_xxxx \
BAKEOFF_PARAPHRASE_MODEL_PATH=/srv/models/qwen3-4b-base \
./scripts/operator/build-bakeoff-corpus.sh /srv/bakeoff/corpus.tar.zst
```

The script downloads OASST2 + GAIA for the novel slice, samples from
`scripts/operator/bakeoff-duplicate-seeds.txt` for the duplicate slice,
and runs Qwen3-4B-Base back-translation for the paraphrase slice.
Outputs a `.tar.zst` tarball plus its SHA256. **Append the SHA256 to
`scripts/operator/.bakeoff-corpus-checksums`** so the bake-off is
reproducible.

For CI / smoke without HF auth, the dry-run path emits a 6-entry
synthetic corpus:

```sh
BAKEOFF_CORPUS_DRY_RUN=1 ./scripts/operator/build-bakeoff-corpus.sh /tmp/dry.tar.zst
```

### Write the candidate manifest

`candidates.toml`:

```toml
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"
params_b = 8
release_date_unix = 1721260800

[[candidate]]
id = "qwen3-8b-base"
path = "/srv/models/qwen3-8b-base"
arch = "qwen3"
license = "apache-2.0"
params_b = 8
release_date_unix = 1745798400

[[candidate]]
id = "gemma-4-31b-base"
path = "/srv/models/gemma-4-31b"
arch = "gemma4"
license = "apache-2.0"
params_b = 31
release_date_unix = 1743552000

[[candidate]]
id = "qwen3.6-27b-dense"
path = "/srv/models/qwen3.6-27b"
arch = "qwen3_5"
license = "apache-2.0"
params_b = 27
release_date_unix = 1776470400
```

Supported `arch` tokens (informational; mistralrs auto-detects the
architecture from each candidate's `config.json`):

| Token     | Notes                                                        |
|-----------|--------------------------------------------------------------|
| `llama`   | Llama 1/2/3 family.                                          |
| `qwen3`   | Qwen3 dense (QK-Norm; use for Qwen3-Base).                   |
| `qwen3_5` | Qwen 3.5 / 3.6 dense (the family id under which 3.6 ships).  |
| `qwen2`   | Deprecated alias for `qwen3`; resolves to qwen3 with a warn. |
| `gemma3`  | Gemma 2 / Gemma 3 dense.                                     |
| `gemma4`  | Gemma 4 (text-only path; multimodal heads are ignored).      |

A2.3 dropped per-arch dispatch on our side. The bake-off binary
forwards each candidate's local path to mistralrs, which reads
`config.json` and selects the pipeline internally. The `arch` field
in the manifest is retained for `ctx_for` lookup and operator
ergonomics but does NOT drive backend selection.

When picking the Gemma 4 candidate, verify the base (not instruct)
variant is staged. A2.1 confirmed that instruct-tuning distorts
perplexity calibration. Run:

```sh
python3 -c "import json; print(json.load(open('/srv/models/gemma-4-31b/config.json'))['architectures'])"
```

The expected output is `["Gemma4ForCausalLM"]` (the base architecture).
A variant ending in `ForConditionalGeneration` or `InstructForCausalLM`
is the instruct tuning and should not be used as the base candidate.

The manifest emits a warning (not an error) for non-incumbent
`llama-community` candidates — the spec restricts new picks to
Apache-2.0 or MIT, but Llama-3.1-8B-Instruct is grandfathered as the
incumbent.

### Run the bake-off

On the H100 host the binary must be built with the `local-gpu-models`
feature so the real `LocalPerplexityScorer` (mistralrs-backed) is
compiled in
(`cargo build --release -p trace-commons-server --features local-gpu-models`,
or `--features local-gpu-models-cuda` to activate mistralrs's CUDA
kernels).
Default-features builds refuse the real-scorer path with
`BakeoffRealScorerRequiresFeature`. CPU-only `local-gpu-models` builds
that select `--hardware=a10` or `--hardware=h100` are refused with
`BakeoffCudaHardwareRequiresCudaFeature` at startup, before any model
load — this guard exists because mistralrs otherwise silently falls
back to CPU inference on CUDA hosts, which on 2026-05-14 burned ~63
minutes of Lambda H100 time at 0 MiB VRAM before being aborted. If you
see that error class, rebuild with `--features local-gpu-models-cuda`.

The mistralrs backend is git-pinned to master SHA
`2d4ba4f16f61e5e18be085d0dd137bc95cba038a` (2026-04-15). Slice 0 of the
A2.3 migration validated the pin on Lambda A100 — full release build
in ~5m30s, ~177 MB binary, raw-logits perplexity finite over an
82-token English input on Gemma 4 E4B. Update the pin in
`crates/trace-commons-gate-enclave/Cargo.toml` if upstream lands a fix the
operator needs; record the new SHA + validation date alongside.

```sh
./target/release/trace-commons-gate-calibrate bake-off \
  --candidates=/srv/bakeoff/candidates.toml \
  --corpus=/srv/bakeoff/corpus.tar.zst \
  --hardware=h100 \
  --report-out=/srv/bakeoff/report.json
```

The binary loads each candidate sequentially (they don't all fit
simultaneously), scores the three corpus slices, runs the determinism
replay, captures `nvidia-smi` VRAM, and writes a JSON report plus a
companion markdown file. Total runtime is `sum(load + score)` across
candidates; budget ~9 hr GPU at ~$35 (single H100).

For dry-run validation without GPU weights:

```sh
./target/debug/trace-commons-gate-calibrate bake-off \
  --candidates=/tmp/dry-candidates.toml \
  --corpus=/tmp/dry.tar.zst \
  --hardware=cpu \
  --report-out=/tmp/dry-report.json \
  --mock-scorer
```

Reports emitted with `--mock-scorer` set carry `mock_scorer: true` and
a `[MOCK SCORER - NOT VALID FOR PRODUCTION DECISIONS]` markdown banner
so they cannot be confused with a real bake-off.

### Apply the decision

The `winner_id` field in `report.json` is the empirically-chosen
production model. Commit the report under
`docs/superpowers/reports/YYYY-MM-DD-model-bakeoff-result.md`
(alongside the JSON), then flip the
`TRACE_COMMONS_PERPLEXITY_MODEL_ID` and
`TRACE_COMMONS_PERPLEXITY_MODEL_PATH` defaults in a one-line PR. After
the swap, **re-run Phase 1** below against the winning model — floors
must be re-derived because they're scaled to the model's perplexity
distribution.

A model swap is operationally expensive (vector replay, audit
grandfathering); the spec's 2% tolerance band on the decision rule
exists precisely to avoid swapping for marginal gains. "No change" is
a valid bake-off outcome — if the incumbent is within 2% of the leader
on the weighted score, the incumbent wins on the license tiebreaker
(or keeps the win outright).

Phase 0 is complete when:

- [ ] The bake-off ran end-to-end without aborted candidates.
- [ ] `report.json` has a populated `winner_id`.
- [ ] The report's SHA256 is recorded somewhere durable.
- [ ] The corresponding env-var defaults are flipped (or the no-change decision is documented).

## Phase 0.1 — Alternative corpus shapes (A2.6)

Phase 0 above describes the canonical bake-off corpus: OASST2 +
GAIA novel slice, curated boilerplate or Wikipedia-introductions
duplicate slice, Qwen3-4B back-translation paraphrase slice. A2.5
concluded that across both duplicate-slice variants we measured
(A2.3c boilerplate, A2.4 Wikipedia) **every candidate's AUC stayed
below 0.5** — so the pilot-launch perplexity floor ships at 0.

A2.6 tests whether the bug is the *novel* slice, not the gate.

### Why this exists

The OASST2 novel slice is exactly what the candidate models were
RLHF'd on, so they find it predictable. The duplicate-slice rotations
in A2.3c and A2.4 each gave the models *less*-trained-on material as
the "duplicate" — backwards from intent. A 2026-05-14 HF survey of
`format:agent-traces` datasets surfaced multi-turn tool-using sessions
captured from real OSS work — material structurally closer to Trace
Commons's intended input shape *and* less in-distribution for the
candidate models. If swapping the novel slice flips at least one
candidate AUC above 0.5, the gate-as-designed isn't broken — A2's
*novel-slice choice* was. See
`docs/superpowers/specs/2026-05-14-agent-traces-bakeoff-design.md`
for the full hypothesis.

### Three corpus variants measured so far

| Variant | Novel slice | Duplicate slice | Result |
|---------|-------------|-----------------|--------|
| A2.3c boilerplate-duplicate | OASST2 chat | Curated boilerplate | AUC 0.054 – 0.276; all four candidates < 0.5 |
| A2.4 Wikipedia-duplicate | OASST2 chat | Wikipedia article intros | AUC 0.185 – 0.264; all four candidates < 0.5 |
| A2.6 agent-traces-novel + Wikipedia-duplicate (pending run) | swival security-audit traces | Wikipedia article intros (reused from A2.4) | Hypothesis: at least one AUC > 0.5 |

A2.6 holds the candidate set, paraphrase pipeline, and duplicate
slice fixed; only the novel slice changes. Direct A2.4 comparability
is the point.

### How to build the A2.6 corpus

Use the dedicated builder
(`scripts/operator/build-agent-traces-corpus.py`). It streams the
source dataset from the HuggingFace hub, joins each row's narrative
fields into a single prose body (see the script's module docstring
for the swival row-to-text mapping), length-filters to 200–2000 words,
deterministically samples `--count` entries, and reuses the duplicate
+ paraphrase slices from an existing A2.4 `corpus-wiki.tar.zst`. The
Rust loader is unchanged: the new tarball satisfies the same
`manifest.json` + slice-directory contract as the canonical builder.

```bash
pip install datasets zstandard  # one-time on the bake-off host
python3 scripts/operator/build-agent-traces-corpus.py \
  --source=jedisct1/agent-traces-swival \
  --duplicate-corpus=$HOME/bakeoff/corpus-wiki.tar.zst \
  --count=300 \
  --seed=42 \
  --out=$HOME/bakeoff/corpus-a26.tar.zst
```

The bake-off binary itself takes the new tarball without modification:

```bash
./target/release/trace-commons-gate-calibrate bake-off \
  --candidates=$HOME/bakeoff/candidates-4way.toml \
  --corpus=$HOME/bakeoff/corpus-a26.tar.zst \
  --hardware=h100 \
  --report-out=$HOME/bakeoff/report-a26.json
```

The full operator procedure (provisioning, model staging, teardown)
lives at `docs/operator/agent-traces-bakeoff-run.md`.

### When to use which corpus

- **A2.6 corpus** — the experiment. Run this once before pilot
  launch to resolve the open question A2.5 left parked. If at least
  one candidate AUC > 0.5, file A2.7 to update the floor recommendations.
- **A2.4 (Wikipedia-duplicate) corpus** — the baseline. If A2.6
  invalidates the hypothesis (all AUCs still < 0.5), A2.5's
  recommendation stands and the pilot launches with the perplexity
  floor disabled. The A2.4 corpus is what a re-bake against the
  canonical OASST2 + Wikipedia shape uses.
- **A2.3c (boilerplate-duplicate) corpus** — retired. Boilerplate
  proved structurally inferior to Wikipedia intros across all four
  candidates.

## Phase 1 — Offline HF bootstrap

Goal: get order-of-magnitude floor values before any contributor traffic.

### What we use

- **OASST2** (`OpenAssistant/oasst2`): ~75k multi-turn instruction
  conversations covering broad assistant-style tasks.
- **GAIA** (`gaia-benchmark/GAIA`): ~500 hard reasoning tasks. Sparser
  but heavier-tail; included to populate the right tail of the
  distribution that OASST2 alone misses.

**HF data never enters the corpus.** Only the per-trace numeric metrics
emerge from the calibration run; the plaintext is read on stdin into
`trace-commons-gate-calibrate`, metrics come out on stdout, and that's all
that's retained.

### Run

On the H100 host with `local-gpu-models` built:

```sh
./scripts/operator/calibrate-from-hf.sh \
  --output=/var/tmp/gate-bootstrap.csv \
  --sample-size=10000   # OASST2; GAIA defaults to 500
```

Internally this:

1. Downloads OASST2 + GAIA via `huggingface-cli`.
2. Flattens each row to a single plaintext string (conversation turns
   concatenated for OASST2; task text for GAIA).
3. Pipes JSONL (`{"plaintext":...}`) through `trace-commons-gate-calibrate`.
4. Writes CSV: `dataset,row_idx,perplexity_micros,tail_fraction_micros,novelty_score_micros`.

Runtime on a single H100: roughly 1 hour for 10k OASST2 + 500 GAIA at
~3 traces/sec.

### Analyze

```sh
./scripts/operator/analyze-calibration.sh \
  --input=/var/tmp/gate-bootstrap.csv \
  --target-pass-rate=0.30
```

Output:

```
RECOMMENDED TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=<N>
RECOMMENDED TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=<N>
RECOMMENDED TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=<N>
```

Floors are picked at the 70th percentile when targeting a 30% pass rate
(i.e. "novel enough" = "higher than 70% of bootstrap traces").

### Adopt — with eyes open

**These values are order-of-magnitude guidance, not final settings.** HF
data is not pilot data. Distribution shift is real:

- Real contributors will be optimizing for a specific task; OASST2's
  general-assistant mix may have different perplexity tails.
- GAIA's hard-reasoning examples skew toward the high-perplexity end;
  including them moves the 70th-percentile floor up.
- Novelty floors are most sensitive to corpus drift because the index
  starts empty at deploy and fills up over time.

### A2.5 pilot-launch floor recommendations

A2.3c + A2.4 invalidated the assumption that aggregate perplexity
discriminates novel reasoning from duplicate content. The
`analyze-calibration.sh` output above will still emit a recommended
perplexity floor, but **do not adopt it for pilot launch**. Override
with the values below.

| Floor                                 | Pilot-launch value           | Notes                                                                                                                  |
|---------------------------------------|------------------------------|------------------------------------------------------------------------------------------------------------------------|
| `PERPLEXITY_FLOOR_MICROS`             | `0` (disabled)               | All measured AUCs < 0.5 across both bake-off corpora and all four candidates. A positive floor would reject in the wrong direction. |
| `TAIL_FRACTION_FLOOR_MICROS`          | `0` at launch                | Calibrate post-first-1000-pilot-traces using only the tail-fraction column from `analyze-calibration.sh` output. The aggregate-perplexity column is misleading. |
| `NOVELTY_FLOOR_MICROS`                | `500000` (cosine novelty 0.5) | Unchanged from A2 deployment guidance. Embedder + vector-index path was not part of the A2.3c/A2.4 invalidation; primary active gate at launch. |

The deployment runbook's "at least one of the three floors must be
positive" invariant is satisfied by `NOVELTY_FLOOR_MICROS=500000`.

Driver report:
`docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`.
Spec: `docs/superpowers/specs/2026-05-14-gate-floor-recalibration-design.md`.

### When the perplexity gate becomes useful

Phase A.5 will revisit whether a perplexity-shaped floor can do real
work once we have ~1000 pilot traces labeled novel/duplicate. Three
candidate approaches, all parked until pilot data lands:

- **Contrastive perplexity.** Compute the delta in logprobs between
  two model checkpoints (one well-trained, one less so). The
  *difference* may be more novelty-indicative than either absolute
  perplexity. No schema change; one extra model load. See A2.5 spec
  §3 for the open-question discussion.

- **Per-token rarity.** Explicitly gather the lowest-N logprobs
  across the trace and gate on "any genuinely surprising tokens
  exist." This is a tighter version of `tail_fraction` and may
  collapse into it once tail-fraction is pilot-calibrated. Cheapest
  to implement; smallest design surface.

- **Learned discriminator.** Train a small classifier on labeled
  novel/duplicate exemplars from the pilot. Requires labeled pilot
  data we don't have yet; first ~1000 traces are the prerequisite.
  Highest ceiling, highest design cost.

Choosing among these is Phase A.5 work. The findings report
captures the rationale for parking it; pick after pilot data
arrives, not before.

Set the novelty floor printed above (`NOVELTY_FLOOR_MICROS=500000`)
and the two zero overrides in your env, and proceed to Phase 2.

## Phase 2 — Closed alpha (zero-credit gating)

Goal: re-derive the floors from **actual** pilot traces.

### Configuration

```sh
export TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0
```

The gate evaluates every submission, populates `trace_gate_decisions`,
but does not emit credit. Contributors see normal accept/reject behavior
based on the gate's pass/fail; they do not see credit because no credit
is minted.

### Collect

Target ~1000 real traces. At low pilot volume this can take days.
Monitor with:

```sql
SELECT count(*) FROM trace_gate_decisions
 WHERE gate_version_hash = (
   SELECT gate_version_hash FROM trace_gate_decisions
    ORDER BY occurred_at DESC LIMIT 1);
```

### Re-analyze

Dump per-trace metrics from the live table to a CSV with the same
schema as the bootstrap CSV, then re-run `analyze-calibration.sh`:

```sh
psql "$DATABASE_URL" -At -F, -c "
  SELECT 'pilot',
         row_number() OVER (ORDER BY occurred_at),
         perplexity_micros,
         tail_fraction_micros,
         novelty_score_micros
    FROM trace_gate_decisions
   WHERE gate_version_hash = current_setting('app.gate_version_hash')
" > /var/tmp/gate-pilot.csv

./scripts/operator/analyze-calibration.sh \
  --input=/var/tmp/gate-pilot.csv \
  --target-pass-rate=0.30
```

The pilot-derived floors are the **real** values. Update env, restart,
verify `gate_version_hash` rotates (because the floors are inputs to
the hash), and decide via central-issuer review whether the new policy
is approved.

## Phase 3 — Live cutover

Goal: emit credit on novel traces under the calibrated policy.

```sh
export TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=1.0   # or configured value
export TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true
export TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY=true
```

Restart. Verify by running the [`smoke-test.md`](smoke-test.md) with
`--enable-credit` and inspecting `trace_credit_ledger` for fresh rows.

## On-going re-calibration

Re-run Phase 2's pull + analyze on a cadence (monthly during early
pilot; quarterly once stable). Calibration drift is real — the corpus
fills, novelty floors that worked at 1k traces stop discriminating at
100k. Watch the pass-rate trend; when it drifts more than ~10 points
from the target, re-calibrate.

A model swap ([`model-swap.md`](model-swap.md)) forces a full Phase 2
re-cal because the perplexity / novelty distributions change.

## Why this matters

Floors set too low → every trace passes → the gate doesn't gate. Credit
emission becomes a function of submission volume, not novelty.

Floors set too high → almost nothing passes → contributors see rejection
without explanation, pilot stalls.

The 30% target is a starting heuristic; tune to whatever pass rate the
pilot economics actually want.
