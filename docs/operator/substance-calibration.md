# Calibrating the substance Score

> **Status: the tooling exists.** `pipeline.py evaluate` runs offline. It
> needs no database, no model, no embedder and no vector index. The
> floor percentile is an open owner decision; a run with any other
> percentile only shows that the command works.

The substance Score (`trace_commons.score.substance.v1`) has two calibrated
values. Each one comes from the distribution of substance over a pinned
corpus:

- `sub_ceil`: the ceiling of the substance half of credit quality. It is the
  90th percentile of substance.
- `substance_floor`: the substance gate. A trace passes at or above it. It is
  the percentile of substance that the owner sets.

`pipeline.py evaluate` measures the distribution and writes the two values.
The design is in the qualitative substance spec,
`docs/superpowers/specs/2026-10-21-substance-qualitative-rlvr-design.md`
(draft PR #1472): the parameters are in its section 6, the checks in its
section 7, and the open owner decisions in its section 11.

## What `evaluate` does

The command exports the pin with its session events, as `pipeline.py compare`
does. It then runs one ignored test of the ingest library,
`substance_evaluate_run`. The test reads each trace, builds its envelope,
projects the envelope onto its deduplicated tool calls, and drops the
envelope. Only the projections go to the calibration, so no trace text can
reach the report.

The calibration runs three checks at the RLVR parameters of the spec and
`sub_floor_mult` 0.25:

1. **Guardrail (check 1).** The seven transforms of the guardrail recipe,
   on eight synthetic bases. Each variant must meet its expectation, and no
   variant may score above its base.
2. **Monotonicity (check 2).** More paired calls never lower substance, from
   0 to 40 calls. The three floors inside substance (`m_work`, `m_rep`,
   `att_floor_mult`) are each above 0, so no term vetoes a trace.
3. **Distribution (check 3).** Substance, work and replay over every trace of
   the pin.

A check that fails refuses the whole run. No value is written.

A trace that cannot be read or built into an envelope also refuses the whole
run. The run never skips a trace, because a skip moves the percentiles with
no record.

## What `evaluate` does not do

- No sensitivity analysis (check 4). The later plan adds it, and the report
  then lists `sensitivity` in `checks_run`.
- No guardrail on corpus bases. Check 1 uses synthetic bases only. The
  report says `"source": "synthetic"`.
- No witness weight. Every trace scores as `unattested`, the class that the
  store holds for every pipeline trace today. The attestation factor is then
  0.90 for every trace, so it scales all traces the same and does not change
  the ranking.
- No choice of the floor. The owner sets the floor percentile.

## The floor percentile is an owner decision

The owner sets the `substance_floor` percentile from the distribution report
(spec section 11, the open owner decisions). A floor at percentile `p` fails
at most `p` percent of the pinned corpus. The floor also decides whether a
tool-free trace passes: such a trace ranks low on work and gets at most the
prompt share on replay.

Give the percentile as `--floor-percentile`, from 0 to 100. The command
refuses a value outside that range (`evaluate_floor_percentile_invalid`).
It has no default.

## Run `evaluate`

```bash
python3 scripts/operator/pipeline.py evaluate \
  --pin docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json \
  --floor-percentile 10
```

This pin is the 10k HF pin: 1,000 bootstrap traces and 9,000 holdout
traces. The export downloads the dataset that the pin names, or uses the
HF cache. `--local-dir DIR` exports from a local JSONL directory instead of
the one that the pin names.

The command uses Cargo's optimized build for the export and for the test.
A pass prints one line:

```text
SubstanceEvaluateOK: report=.local/substance-calibration-report.json values=.local/substance-calibrated-values.json sub_ceil=<micros> floor=<micros>
```

The two files are also in the run directory. A failure prints one
`PipelineFailure:` line with a label.

`.local/` holds the files of the last `evaluate` run only. A trial run with
another percentile replaces the values file of the run that you chose, so
copy that file before a trial run, or run `evaluate` again with the chosen
percentile.

## The two files

`substance-calibration-report.json` is the report
(`trace_commons.substance_calibration_report.v1`). Its bytes are canonical
JSON. Its SHA-256 is the `report_digest` of the values file, so
`sha256sum` of the file gives that digest. It holds labels, digests, counts
and micros only:

| Field | Content |
|---|---|
| `corpus_pin_digest` | The source digest of the pin. |
| `trace_count` | The number of traces. It equals the pin's sample count. |
| `substance_version`, `params`, `sub_floor_mult_micros` | The formula and the set parameters of the run. |
| `checks_run` | `["guardrail", "monotonicity", "distribution"]`. |
| `guardrail` | The recipe id, `source`, and for each transform the pair count, the met count and the term that moved most. |
| `monotonicity` | `sweep_passed` and `floors_above_zero`. |
| `distribution` | For `substance`, `work` and `replay`: min, p10, p25, p50, p75, p90, max and mean, in micros. And `tool_free_count`. |
| `calibrated` | `sub_ceil_micros` at percentile 90, and `substance_floor_micros` at the given percentile. |

`substance-calibrated-values.json` holds the two values and their
provenance: `sub_ceil_micros`, `substance_floor_micros`, and `calibration`
(`corpus_pin_digest`, `report_digest`, `sub_ceil_percentile`,
`substance_floor_percentile`).

### Read the distribution

All values are in micros: 1,000,000 is 1.0.

- Substance is never 0. At the spec values, its lowest possible value is
  27,000 (0.30 x 0.10 x 0.90): a trace with no tool call and no prompt.
- `tool_free_count` is the number of traces with no tool call. A tool-free
  trace has a substance of at most 124,200 at the spec values
  (0.30 x 0.46 x 0.90, a trace with a prompt). A floor above that value
  fails every tool-free trace.
- The `work` and `replay` rows are the raw terms, before their floors. Work
  near 0 for most traces means that few traces have paired calls. Replay
  near 0.40 (400,000) means a prompt with few arguments or results on the
  calls.
- A gap between two percentiles of substance is a place where a small change
  of the floor percentile moves many traces.

The owner reads the substance row to set the floor percentile. Then run
`evaluate` again with that percentile, and keep the values file of that run.

## The values file is the input to the production package

The production substance package takes `sub_ceil`, `substance_floor` and the
calibration provenance from the values file. The bundle id hashes them, so a
new `evaluate` run with other values gives a new bundle id.

- The switch, in order: copy the values into the six variables of the
  deployment env file, deploy, promote the substance package, install the
  new default-routing set, and activate each tenant that is still on the
  compatibility bundle. See "Switch the default package to substance" in
  [pipeline-activation.md](pipeline-activation.md#switch-the-default-package-to-substance).
- How `pipeline.py package --bundle production` builds the substance package
  from the env file, and how the promotion checks run on it: see "The
  production run: `pipeline.py promote`" in
  [pipeline-qualification.md](pipeline-qualification.md#the-production-run-pipelinepy-promote).

## Credit quality version 4

A substance run writes its credit quality as version 4 (`cq4`). No code
refuses an unknown version, but two readers need attention:

- **The account-trust rule.** It reads its allowlist of credit quality
  versions from the policy JSON, and only when `q_min_micros` is set. The
  live policy has `q_min_micros: null`. A policy that sets `q_min_micros`
  must list `4` in `credit_quality_calibration_versions`. If it does not,
  no substance evaluation counts as a passed gate evaluation for account
  trust.
- **The credit-estimate fit.** It trains only on version 3 rows (owner
  decision E9). It skips every `cq4` row. A separate fit for `cq4` is a
  later decision.

## For the removal of the compare harness

The legacy removal (#1330) deletes the compare harness
(`pipeline_compare_pg_tests.rs`). `substance_evaluate_run` lives in that
file and calls `compare_raw` and `compare_envelope` to build each envelope.
Before #1330 deletes the harness, it must move `compare_raw` and
`compare_envelope` to the library, and keep `substance_evaluate_run`. If
they move, the envelope bytes must not change: `evaluate` on one pin must
give the same `report_digest` before and after the move.
