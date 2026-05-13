# Gate Calibration

How to choose the three gate floors —
`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` — and when to flip credit
emission on.

Three phases, in order:

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
