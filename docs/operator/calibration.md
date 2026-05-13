# Gate Calibration

How to choose the three gate floors —
`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` — and when to flip credit
emission on.

Three phases, in order. **Phase 0** (model bake-off) happens once per
production deployment, before Phase 1 runs.

## Phase 0 — Model bake-off (A2.1)

Goal: empirically pick the perplexity scorer model from a candidate set,
rather than carrying the incumbent on tooling-maturity grounds. Run once
per deployment, before any floor calibration — floors are scaled to the
winning model's perplexity distribution, so picking the model first is
the prerequisite.

The bake-off binary is the `bake-off` subcommand of
`tracedao-gate-calibrate`. The decision rule (`0.6 * AUC + 0.3 *
stability + 0.1 * tail_range`, with determinism gate, throughput floor,
and license / size / recency tiebreakers) is committed before the run;
the winner is determined by formula, not by inspection.

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
```

Supported `arch` tokens (one per candle backend):

| Token    | Backend                       | Notes                                  |
|----------|-------------------------------|----------------------------------------|
| `llama`  | `candle_transformers::llama`  | Llama 1/2/3 family                     |
| `qwen3`  | `candle_transformers::qwen3`  | Includes QK-Norm (use for Qwen3-Base)  |
| `qwen2`  | (deprecated alias for `qwen3`)| Resolves to `qwen3`; emits a warning   |
| `gemma3` | `candle_transformers::gemma3` | Gemma 2 / Gemma 3 dense                |
| `gemma4` | `candle_transformers::gemma4` | Gemma 4 multimodal (text-only loader)  |

Qwen 3.6 27B Dense (`qwen3_5`) and earlier `gemma` / `gemma2` are not
in the supported set today; candle ships no compatible loader. Track
the spec roadmap for any future addition.

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
feature so the real `CandlePerplexityScorer` is compiled in
(`cargo build --release -p tracedao-server --features local-gpu-models`).
Default-features builds refuse the real-scorer path with
`BakeoffRealScorerRequiresFeature`.

```sh
./target/release/tracedao-gate-calibrate bake-off \
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
./target/debug/tracedao-gate-calibrate bake-off \
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
`tracedao-gate-calibrate`, metrics come out on stdout, and that's all
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
3. Pipes JSONL (`{"plaintext":...}`) through `tracedao-gate-calibrate`.
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

Set the printed values in your env and proceed to Phase 2.

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
