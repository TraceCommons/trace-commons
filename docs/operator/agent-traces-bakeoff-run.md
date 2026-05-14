# A2.6b — Agent-Traces Bake-off Operator Runbook

This runbook documents the operator activity for the A2.6 bake-off:
one bake-off run against an agent-traces novel slice and the existing
A2.4 Wikipedia duplicate slice. It is the post-merge follow-on to
A2.6a (corpus builder, this PR) and produces the A2.6c report.

The bake-off binary, candidate models, paraphrase pipeline, and
duplicate slice are unchanged from A2.3c / A2.4. Only the novel slice
changes. See `docs/superpowers/specs/2026-05-14-agent-traces-bakeoff-design.md`
for the hypothesis and decision rule.

**Expected cost:** ~$25 (~5 hr on a single Lambda H100 SXM5 80GB).

**Predecessors required to have run:**
- A2.3c or A2.4 produced a `corpus-wiki.tar.zst` you can reuse as the
  duplicate-corpus source. The A2.4 run's tarball SHA256 is recorded
  alongside `docs/superpowers/reports/2026-05-14-model-bakeoff-result-a24.{json,md}`.
- The 4-way candidate manifest (`candidates-4way.toml`) and the
  Qwen3-4B-Base paraphrase checkpoint are staged on the operator's
  bake-off host (same setup as A2.3c / A2.4).

---

## Step 1 — Provision the bake-off host

Provision a single Lambda H100 SXM5 80GB instance. `us-southeast-1`
is preferred for cost; any region with capacity is acceptable.
A100-80GB is an acceptable fallback (the bake-off binary supports
both via the `--hardware` flag; runtimes go up ~25%). Smaller GPUs
will not fit Gemma 4 31B Base in the 4-way set.

Boot a recent Ubuntu LTS image; bring up CUDA + the system Python
your previous bake-offs used. No new system packages are required
beyond the existing toolchain.

## Step 2 — Stage models + corpus inputs

Reuse the staging from A2.3c / A2.4:

```bash
# 4 candidate checkpoints + Qwen3-4B-Base paraphrase model.
./scripts/operator/stage-models.sh

# Pull the A2.4 corpus tarball (duplicate + paraphrase slices are reused verbatim).
scp <local>:$REPO/path/to/corpus-wiki.tar.zst   $HOME/bakeoff/corpus-wiki.tar.zst

# Pull the 4-way candidate manifest used in A2.3c / A2.4.
scp <local>:$REPO/path/to/candidates-4way.toml  $HOME/bakeoff/candidates-4way.toml
```

Confirm `sha256sum $HOME/bakeoff/corpus-wiki.tar.zst` matches the
hash recorded in `2026-05-14-model-bakeoff-result-a24.json`. If it
doesn't, refuse to proceed — A2.6 must reuse exactly A2.4's duplicate
slice for the comparison to be valid.

## Step 3 — Install the agent-traces builder dependencies

The corpus builder is pure Python. Install the two non-stdlib
packages it needs:

```bash
pip install --upgrade datasets zstandard
```

Both are widely used and CI-vetted on Lambda hosts; A2.4's
corpus-rebuild script already used `zstandard`. No Cargo changes,
no Rust rebuild.

## Step 4 — Build the A2.6 corpus

```bash
python3 $REPO/scripts/operator/build-agent-traces-corpus.py \
  --source=jedisct1/agent-traces-swival \
  --duplicate-corpus=$HOME/bakeoff/corpus-wiki.tar.zst \
  --count=300 \
  --seed=42 \
  --out=$HOME/bakeoff/corpus-a26.tar.zst
```

The builder emits step lines on stderr (`BakeoffAgentTracesStep: ...`)
and a final `BakeoffAgentTracesOK output_sha256=sha256:...` on stdout.
Record that hash; it goes into the A2.6c report notes.

**Pre-flight check on the source dataset.** Before running, confirm
the dataset's first-upload date post-dates each candidate's training
cutoff (see the spec's Open Question 1). At time of plan writing
swival was created 2026-04-08, comfortably after every candidate's
cutoff. If you swap `--source` to a different dataset, redo the check.

## Step 5 — Run the bake-off

```bash
./target/release/trace-commons-gate-calibrate bake-off \
  --candidates=$HOME/bakeoff/candidates-4way.toml \
  --corpus=$HOME/bakeoff/corpus-a26.tar.zst \
  --hardware=h100 \
  --report-out=$HOME/bakeoff/report-a26.json
```

Same shape as A2.3c / A2.4; expect ~4-5 hr wall-clock for the 4-way
set. The binary writes a Markdown sibling at `report-a26.md`
automatically.

## Step 6 — Pull the report locally and write the notes companion

```bash
scp lambda:$HOME/bakeoff/report-a26.json  docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json
scp lambda:$HOME/bakeoff/report-a26.md    docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.md
```

Then hand-write
`docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26-notes.md`
documenting:

- Per-candidate AUC comparison to A2.3c + A2.4 (table).
- Whether the hypothesis held (at least one candidate's AUC > 0.5).
- Source-dataset SHA256 and corpus tarball SHA256 (from Step 4 stdout).
- Per the spec's "What success looks like" section, the recommended
  next step:
  - **Outcome 1 (any AUC > 0.5):** file A2.7 to re-enable the
    perplexity floor with values calibrated against this run; close
    Phase A.5 (perplexity-replacement metric) as no-longer-needed.
  - **Outcome 2 (all AUCs in 0.4 – 0.5):** document partial
    improvement; Phase A.5 stays parked with reduced urgency; floor
    recommendation stays at A2.5's value (0) as conservative-by-default.
  - **Outcome 3 (all AUCs < 0.4):** A2.5's conclusion is reinforced;
    Phase A.5 stays on the roadmap; agent-traces is no longer a hedge.

Commit the three new files in a single PR titled with the same
`A2.6c` shorthand the spec uses.

## Step 7 — Flip the roadmap entry

After the A2.6c report PR lands, update the Phase A status entry
this PR added to flip "pending run" to "done" with a one-line
summary of the AUC outcome. That's a separate single-line PR; do it
the same way A2.3c / A2.4 entries were finalized.

## Step 8 — Tear down the H100

Lambda H100 capacity is scarce; release the instance promptly:

```bash
lambda-cloud instance terminate <instance-id>
```

Confirm via `lambda-cloud instance list` that no instance is left
running on your account. The data the bake-off needed (model
checkpoints, paraphrase model, source dataset cache) is all
recoverable on the next provisioning, so there is nothing to
preserve before teardown other than the report files already
copied off in Step 6.

---

## Hash-only / no-secrets reminder

The builder, the bake-off binary, and this runbook all conform to
the repo's hash-only audit policy. The notes companion may quote
candidate names, AUC values, and corpus / report SHA256s; do not
include raw trace bodies, raw row content, or any operator-secret
material (HF tokens, model paths on the bake-off host, instance
IDs) in the committed report. Step lines emitted by the builder
are label-only by design.
