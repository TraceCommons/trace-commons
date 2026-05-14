# A2.7 Perplexity Floor Calibration — Operator Runbook (Outcome 1)

> **2026-05-14 BLOCKED ON BAKE-OFF BINARY GAP.** This runbook assumes
> the A2.6 report JSON contains per-trace perplexity scores for the
> calibration candidate. As of A2.6's actual completion the report
> JSON contains ONLY summary statistics (`discrimination_auc`,
> `paraphrase_delta`, etc.) — per-trace score arrays are computed
> internally by the bake-off binary and discarded. Until the binary
> is modified to persist per-trace scores AND a single-candidate
> Qwen 3.6 27B Dense re-run is executed, this runbook cannot
> execute. **Operator action while blocked: ship the pilot with
> `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` per A2.5's
> conservative default; the in-pilot tail-fraction subcommand (PR
> #66) can drive a separate calibration path on real contributor
> data.** Resume here when the per-trace-score binary mod + re-run
> ship.

Phase: A.2.7 post-A2.6. Predecessors:
[`./a26-bakeoff-result-handler.md`](./a26-bakeoff-result-handler.md) (which
routes the operator here once the bake-off report drops). Spec:
`docs/superpowers/specs/2026-05-14-a27-perplexity-floor-update-design.md`
(authoritative for the calibration math — this runbook is the worked
procedure). Plan stub:
`docs/superpowers/plans/2026-05-14-a27-perplexity-floor-update.md`.

This runbook covers **only the Outcome 1 (AUC > 0.5) path**: A2.6 cleared
the bar on at least one candidate, the perplexity gate is fit for purpose,
and the pilot-launch perplexity floor needs to be turned on. For the
Outcome 2 (docs-only) and Outcome 3 (Phase A.5 activation) branches, see
the A2.6 result-handler runbook linked above.

---

## When this fires

Trigger when **all** of the following hold:

- The A2.6 final report is committed under
  `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.{json,md}`.
- At least one candidate's measured AUC on the agent-traces novel slice vs.
  Wikipedia duplicate slice is strictly greater than `0.5`.
- The dataset-recency check from A2.6 Open Question 1 has not flagged a
  failure on any candidate that cleared `0.5`. (If it did, demote the
  outcome one tier per the A2.7 spec § Trigger criteria and stop — this
  runbook does not apply.)
- The operator has the per-trace scoring data from the bake-off report
  locally available. The bake-off binary writes per-row
  `aggregate_perplexity_micros` values for each candidate × slice into the
  `report.json` payload alongside the AUC summary; that is the only input
  this runbook needs.

If any of those is missing, stop. Do not improvise.

---

## Step 1 — Identify the calibration candidate

Per the A2.7 spec § Outcome 1: **the calibration candidate is the
worst-of-passing — the candidate with the LOWEST AUC among those that
crossed `0.5`.** Not the best-AUC candidate. The rationale is robustness
to a future model swap: the floor must still discriminate if the operator
later promotes a smaller / cheaper candidate that also cleared `0.5`.

Two concrete sub-cases, given the partial A2.6 results as of report time
(Llama-3.1-8B-Instruct 0.342, Qwen3-8B-Base 0.243, Qwen 3.6 27B Dense
0.936, Gemma 4 31B Base TBD):

### Case A — Gemma 4 31B Base crosses 0.5

The passing set is `{Qwen 3.6 27B Dense, Gemma 4 31B Base}`. The
calibration candidate is whichever of the two has the **lower** AUC. Read
both AUCs from the report's per-candidate summary block; pick the lower.

### Case B — Gemma 4 31B Base does not cross 0.5

The passing set is `{Qwen 3.6 27B Dense}`. Qwen 3.6 27B Dense is the
calibration candidate by default (only-candidate-of-passing IS
worst-of-passing).

### Tiebreak

If two candidates' reported AUCs tie to the precision the report emits,
pick the one with the **lower 10th-percentile novel-slice perplexity**
(the more conservative anchor downstream). Record the tiebreak in the
calibration log (Step 7).

Record the chosen `candidate_id` before proceeding. Every downstream step
operates on that candidate's per-row data and nothing else.

---

## Step 2 — Compute the Youden's-J optimum

Pull the chosen candidate's per-trace `aggregate_perplexity_micros` from
both the novel slice (label `1`) and the duplicate slice (label `0`) out
of `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json`.
The bake-off binary already emits these in micros; do not unit-convert.

Sweep candidate threshold values across the observed range. At each
threshold compute:

- `TPR = P(perplexity >= threshold | novel)`
- `FPR = P(perplexity >= threshold | duplicate)`
- `J = TPR - FPR`

The threshold that maximises `J` is the Youden's-J optimum. Convert to
real-valued perplexity with `perp = micros / 1_000_000` for the geometric
mean step.

Paste-ready Python (stdlib only):

```python
import json, statistics, math

REPORT = "docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json"
CANDIDATE = "<candidate_id from Step 1>"

report = json.load(open(REPORT))
rows = report["candidates"][CANDIDATE]["per_row"]  # adjust if schema differs
novel = sorted(r["aggregate_perplexity_micros"] for r in rows if r["slice"] == "novel")
dupe  = sorted(r["aggregate_perplexity_micros"] for r in rows if r["slice"] == "duplicate")

def tpr_fpr(thr):
    tpr = sum(1 for v in novel if v >= thr) / len(novel)
    fpr = sum(1 for v in dupe  if v >= thr) / len(dupe)
    return tpr, fpr

candidates = sorted(set(novel + dupe))
best_thr, best_j = max(((t,) + tpr_fpr(t) for t in candidates),
                       key=lambda x: x[1] - x[2])[:2]
# best_thr is in micros
youden_j_micros = best_thr
youden_j = youden_j_micros / 1_000_000.0
print(f"youden_j_optimum_micros = {youden_j_micros}")
print(f"youden_j_optimum        = {youden_j}")
```

Verify `J > 0` at the optimum. A non-positive `J` means the candidate is
not actually discriminating in the right direction — stop and re-check
Step 1 (you may have picked a non-passing candidate by mistake).

---

## Step 3 — Compute the 10th percentile of novel-slice perplexity

From the same per-row data, take the **novel slice only**, sort, and take
the 10th percentile. The 10th percentile is contributor-friendly: it
admits the bottom-decile novel scorers while still rejecting duplicate
content that lies below the novel distribution. (The A2.7 spec § Open
question Q2 commits to 10th percentile for pilot launch — do not
substitute 30th or any other value.)

```python
# continues from Step 2's REPL
import statistics
# stdlib equivalent of np.percentile(novel, 10), linear interpolation
def percentile_linear(sorted_vals, p):
    if not sorted_vals: raise ValueError("empty")
    k = (len(sorted_vals) - 1) * (p / 100.0)
    lo, hi = math.floor(k), math.ceil(k)
    if lo == hi:
        return sorted_vals[int(k)]
    return sorted_vals[lo] + (sorted_vals[hi] - sorted_vals[lo]) * (k - lo)

p10_novel_micros = percentile_linear(novel, 10)
p10_novel = p10_novel_micros / 1_000_000.0
print(f"p10_novel_micros = {p10_novel_micros}")
print(f"p10_novel        = {p10_novel}")
```

---

## Step 4 — Geometric mean

`floor_raw = sqrt(youden_j_optimum * p10_novel)` — both anchors live on
log-scale-comparable perplexity axes, so the geometric mean is the
correct central tendency.

```python
floor_raw = math.sqrt(youden_j * p10_novel)
print(f"floor_raw = {floor_raw}")
```

**Sanity check (spec § Outcome 1 Step 2 reconcile):** if `youden_j` and
`p10_novel` disagree by more than 2× (i.e. `max / min > 2`), do not take
the geometric mean. Stop and file under the spec's Open Question 1:
defer the floor decision to the human operator in the A2.7a findings
report; record both candidate values; launch the pilot with floor `0`
(Outcome 2 posture) while the investigation runs.

```python
ratio = max(youden_j, p10_novel) / min(youden_j, p10_novel)
assert ratio <= 2.0, f"Method A/B diverged by {ratio:.2f}x — see Open Question 1"
```

---

## Step 5 — Apply the 0.5× headroom margin

```python
floor_proposed = floor_raw * 0.5
print(f"floor_proposed = {floor_proposed}")
```

The headroom is conservative-by-default: halving the geometric mean makes
the floor **more permissive** so pilot-day distribution drift does not
shed legitimate contributor work. This 2× headroom matches the
order-of-magnitude conservatism A2.5 applied to
`NOVELTY_FLOOR_MICROS=500000`. The margin is reviewed (not
necessarily re-applied) after the first 1000 pilot traces.

**Median sanity bound (spec § Outcome 1 Step 5):** confirm `floor_proposed`
does not exceed the calibration candidate's *median* novel-slice
perplexity. A floor above the median rejects > 50 % of contributor-grade
novel content under the bake-off distribution and is prima facie too
tight — if this fires, treat the outcome as Outcome 2 and stop.

```python
median_novel = statistics.median(novel) / 1_000_000.0
assert floor_proposed <= median_novel, \
    f"floor_proposed {floor_proposed} > median_novel {median_novel} — demote to Outcome 2"
```

---

## Step 6 — Convert to micros

The gate-service env var `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` is a
`u64` in micros (perplexity × 1 000 000). The inverse helper
`micros_to_f64` lives at
`crates/trace-commons-server/src/bin/gate_calibrate/run_candidate_eval.rs:85`
and is what the gate uses at runtime — the unit must be byte-identical.

Round to the nearest integer (the spec § Outcome 1 Step 4 calls for
`floor(final_floor)`; use it):

```python
floor_micros = int(floor_proposed * 1_000_000)  # truncate (floor toward 0)
print(f"TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS={floor_micros}")
```

Sanity-check `floor_micros >= 0`. Negative micros is malformed and the
gate binary will refuse to start.

---

## Step 7 — Update calibration runbook commentary

Edit `docs/operator/calibration.md`. Locate the "A2.5 pilot-launch floor
recommendations" table near the bottom of the Phase 1 section and rewrite
the `PERPLEXITY_FLOOR_MICROS` row in place. The A2.5 reasoning paragraph
above the table stays put — the change is the value and notes column,
not the history.

Record the following under a new sub-section
`### A2.7 calibration log` immediately after the A2.5 pilot-launch table:

- **Calibration date:** YYYY-MM-DD (UTC).
- **A2.6 report:** sha256 of
  `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json`
  (compute via `sha256sum`).
- **Bake-off run ID:** the run-id field from `report.json` (or "A2.6
  primary" if absent).
- **Calibration candidate:** `candidate_id` chosen in Step 1, with the
  case-A / case-B / tiebreak rationale.
- **Method A — Youden's-J optimum:** value in real units and micros.
- **Method B — p10 novel-slice perplexity:** value in real units and
  micros.
- **Geometric mean:** `floor_raw` value.
- **Post-headroom floor:** `floor_proposed` value.
- **Final micros:** `floor_micros`.
- **Median sanity check:** PASS / FAIL with the median value.
- **One-paragraph rationale:** "Calibrated against worst-of-passing
  candidate `<candidate_id>` from A2.6 (AUC `<value>`). Geometric mean of
  Youden's-J optimum and p10-novel perplexity, halved for pilot-day
  headroom. Median sanity check passed. See A2.7 spec § Outcome 1
  procedure for the recipe."

No raw envelope content, no operator-secret material. Candidate names,
AUCs, percentiles, and SHA256s are within the hash-only audit policy.

---

## Step 8 — Update the deployment template

The production env-var template lives at
`docs/operator/deployment.md:171` (the
`# --- Gate (floors) — A2.5 pilot-launch defaults` block). Search for
collisions with:

```bash
grep -rn "TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS" docs/ scripts/ \
  --include="*.md" --include="*.sh" --include="*.env*"
```

Expect hits in at least:

- `docs/operator/deployment.md` — the `export
  TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0` line. Replace `0` with
  `floor_micros` from Step 6. Update the trailing comment from
  `A2.5: perplexity AUC < 0.5; disabled at pilot launch` to
  `A2.7: calibrated against <candidate_id> @ AUC <value>; see calibration.md`.
- `docs/operator/env-reference.md` — the row for
  `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`. Update the "Pilot-launch
  default" note to the A2.7 value and add a pointer to this runbook.
- `docs/operator/calibration.md` — already covered in Step 7.

The `analyze-calibration.sh` script's `RECOMMENDED ...` echo line stays
as-is — that is bootstrap-driven guidance, not a deployment template.
Note explicitly in the calibration.md A2.7 sub-section that the script's
output is to be overridden by the A2.7 value at pilot launch.

If any other file pins the env-var value (none expected outside the
worktree copies), update it. Do **not** edit files under
`.claude/worktrees/` — those are agent worktrees, not the canonical tree.

---

## Step 9 — File the PR

Single commit. Match the existing short-imperative commit-subject style
(no `feat:` / `fix:` prefix, no emoji). The subject names the calibrated
value so a future operator scanning `git log` sees the change at a
glance:

```
A2.7: enable perplexity floor at calibrated <floor_micros> micros
```

The commit body and PR description carry the calibration math (verbatim
from Step 7's calibration log) plus the report sha256 reference and a
pointer to the A2.7 spec § Outcome 1 procedure. No raw perplexity rows,
no instance IDs, no model paths — calibration constants only.

```bash
gh pr create --title "A2.7: enable perplexity floor at calibrated <floor_micros> micros" \
  --body "$(cat <<'EOF'
## Summary

A2.6 fired Outcome 1 (AUC > 0.5 on at least one candidate). This PR
re-enables `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` at the calibrated
value per the A2.7 spec § Outcome 1 procedure.

- Calibration candidate: `<candidate_id>` (worst-of-passing).
- Youden's-J optimum: `<value>` (micros: `<value>`).
- p10 novel-slice perplexity: `<value>` (micros: `<value>`).
- Geometric mean: `<value>`. Post-headroom (0.5×): `<value>`.
- Final: `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=<floor_micros>`.
- A2.6 report sha256: `<sha256>`.

Median sanity check: PASS (`floor_proposed=<value>` <= median_novel=`<value>`).

## Test plan

- [ ] `cargo check -p trace-commons-server --bins` clean.
- [ ] Replay smoke envelopes per Step 10 of the runbook — high-perplexity
      novel sample passes, low-perplexity duplicate sample rejected.
- [ ] Verify `gate_version_hash` rotates after env-var change.
EOF
)"
```

---

## Step 10 — Smoke-verify after deploy

After the PR merges and the production env is updated, replay a handful
of high-perplexity and low-perplexity sample envelopes against the live
gate and confirm the floor rejects the right ones. The
`pilot-bootstrap` binary is the right harness — see
[`./pilot-bootstrap.md`](./pilot-bootstrap.md) for the replay procedure.

Procedure:

1. Pick a small fixed sample (≤ 10 traces): half from the A2.6 novel
   slice with `aggregate_perplexity_micros` well above `floor_micros`,
   half from the duplicate slice with values well below.
2. Submit each through `pilot-bootstrap` against the deployed gate.
3. Confirm: high-perplexity novel samples → `Decision::Pass` on the
   perplexity channel; low-perplexity duplicate samples →
   `Decision::Fail(reason=perplexity_below_floor)`.
4. Record the result counts in the calibration log (no raw envelope
   bodies — counts and decision-class only).

If any sample on either side of the floor decides the wrong way, **stop
the pilot** and open an A2.8 recalibration ticket. Do not adjust the
floor mid-flight; see Trade-offs below.

After the smoke check passes, update
[`./operational-summary.md`](./operational-summary.md) to note the
expected non-zero perplexity-rejection counter (operators reading the
summary should not see a permanent zero on the perplexity channel
post-A2.7).

---

## Trade-offs the operator must internalize

- **Single value across environments.** A2.7 ships one
  `floor_micros` across staging and prod. The A2.7 plan stub § 4
  explicitly parks staging/prod splits as out-of-scope; the
  hypothetical A2.8 is where environment-specific tuning would land.
- **0.5× headroom is fixed for this calibration run.** Do not retune
  the margin inside Step 5 to "make the floor work." If the geometric
  mean produces an inconvenient value, that is signal — the value is
  what the data says it is. Future calibration runs (A2.8+) may revisit
  the headroom multiplier; this one does not.
- **Calibration is conservative against future model swap, not best-tuned
  to the strongest measured AUC.** Worst-of-passing is deliberate. If
  the operator later swaps to a smaller / cheaper model in the passing
  set, the floor still discriminates.
- **Bias toward contributor acceptance.** 10th-percentile + 0.5×
  headroom together let some duplicates through and catch them at human
  review rather than reject novel contributor work at the gate. This is
  the spec's explicit trade-off.
- **If pilot data shows the floor is too aggressive, file A2.8 — do
  not fix-this-runbook-mid-flight.** The first 1000 pilot traces are
  the recalibration trigger per
  [`./calibration.md`](./calibration.md) Phase 2. A2.7 sets the
  starting value; A2.8 (or successor) carries forward.

## Hash-only / no-secrets reminder

Calibration math (floor value, Youden's-J optimum, percentiles,
geometric mean, headroom result, micros) is **not** operator-secret —
it is calibration constant and may be logged, committed, and discussed
in PR descriptions. Raw envelope content, HF tokens, instance IDs,
model paths on the bake-off host, contributor identities, and trace
bodies are operator-secret and must not appear in any committed file or
commit message. The same rule the A2.6 result-handler runbook uses
applies here verbatim.
