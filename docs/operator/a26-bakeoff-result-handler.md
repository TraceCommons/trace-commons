# A2.6 Bake-off Result Handler — Operator Runbook

Phase: A.2.6 post-run. Predecessors:
[`./agent-traces-bakeoff-run.md`](./agent-traces-bakeoff-run.md) (running
the bake-off itself). Decision-rule reference:
`docs/superpowers/specs/2026-05-14-agent-traces-bakeoff-design.md`
("What success looks like").

The A2.6 bake-off is running on a Lambda H100 SXM5 80GB instance. When
the run completes the operator pulls the report, fills the committed
skeleton, routes to one of three outcome branches, tears down the GPU,
and flips the roadmap entry. This runbook is the single source of
truth for that post-run sequence.

## Known facts (pre-filled)

- **Run launched:** 11:11 UTC 2026-05-14.
- **Hardware:** Lambda H100 SXM5 80GB (single GPU).
- **Corpus tarball sha256:**
  `46e0eef8a52e309ce695ad20d1e242ce43eb210c11e02764beeaf7fa3d341bb5`
- **Candidate manifest sha256:**
  `2e360df9449d81d664caeb0e17ed893ccb28e5998604c4caafd1aa46a13fd0f0`
- **Candidate set:** Llama-3.1-8B-Instruct, Qwen3-8B-Base,
  Qwen 3.6 27B Dense, Gemma 4 31B Base.
- **First completed candidate (as of writing):**
  Llama-3.1-8B-Instruct, AUC 0.342.

---

## When this fires

Trigger on either of:

- A `bakeoff_done` line appears in `~/bakeoff/bakeoff-a26.log` on the
  Lambda host.
- All four `bakeoff_candidate_done` events are present in the same
  log (one per candidate in the 4-way set).

If only some `bakeoff_candidate_done` events are present, the run is
still in progress; do not pull a partial report.

---

## Step 1 — Pull the report

```bash
scp ubuntu@<lambda-ip>:~/bakeoff/report-a26.json \
  docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json
```

The bake-off binary writes a deterministic JSON; the SHA256 should be
stable across same-input reruns. Compute the hash and verify before
committing:

```bash
sha256sum docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json
```

Record the hash in the notes companion (Step 2). If a rerun of the
same `(corpus, manifest, hardware)` tuple produces a different SHA256,
treat it as a determinism regression and STOP — do not route to an
outcome branch; file the divergence in
`docs/operator/pilot-bootstrap-anomaly-2026-05-14.md`-shaped notes
under `docs/operator/`.

---

## Step 2 — Fill the report skeleton

The skeleton at
`docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.md`
(landed in PR #64) has `<TBD>` placeholders. For each of the four
candidates copy these fields straight from the JSON:

- `auc`
- `paraphrase_delta`
- `tail_range`
- `throughput_tps`
- `determinism_stddev`
- `passed_gate`

Write the narrative — hypothesis verdict, per-candidate comparison
to A2.3c / A2.4 AUC, source-dataset SHA256, corpus tarball SHA256,
recommended next step — in the notes companion at
`docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26-notes.md`.

No raw trace bodies, no operator-secret material (HF tokens, instance
IDs, model paths). Candidate names, AUC values, and SHA256s are all
within the hash-only audit policy.

---

## Step 3 — Route per outcome branch

Per A2.6 spec § "What success looks like":

### Outcome 1 — AUC > 0.5 for at least one candidate

A2.7 fires. Open a PR titled exactly:

```
A2.7: re-enable perplexity floor at calibrated value
```

Follow the floor-derivation procedure in
`docs/superpowers/specs/2026-05-14-a27-perplexity-floor-update-design.md`.
The chosen candidate (highest AUC, subject to throughput / VRAM /
determinism gates) becomes the recalibration target. Phase A.5
(perplexity replacement) closes as no-longer-needed; update its
roadmap entry to "closed (A2.6 cleared)".

### Outcome 2 — All candidates 0.4 ≤ AUC < 0.5

A2.7 partial fires as a docs-only PR. Update A2.5's commentary in
its plan + reports to mark the conservative-by-default reasoning
(perplexity floor delta stays at 0). Phase A.5 stays parked with
reduced urgency. The roadmap "Production Gap Queue" entry for the
perplexity floor moves to "deferred pending fresher candidate set."

### Outcome 3 — All candidates AUC < 0.4

Phase A.5 activates. The plan stub at
`docs/superpowers/plans/2026-05-14-a5-perplexity-replacement.md`
becomes the operating doc. Open Question 1 in that plan (metric
choice — per-token rarity vs vocabulary-coverage vs other) must be
resolved before implementation begins; delegate the open question
to the Plan Reviewer expert before any code lands. A2.7 does not
fire.

---

## Step 4 — Teardown

```bash
lambda-cloud instance terminate <instance-id>
lambda-cloud instance list   # confirm none left running
```

Record the run cost in the cost ledger (one append-only row, no
back-edits):

- File: [`./gpu-cost-ledger.md`](./gpu-cost-ledger.md)
- Columns: `date | run-id | hardware | hours | cost | purpose`

Do NOT delete `~/bakeoff/report-a26.json` or the HuggingFace models
cache (`~/.cache/huggingface/`) until after the report JSON is
committed locally (Step 1) and the report PR is merged. If the
report goes missing before the commit, the run is effectively lost —
the determinism guarantee covers reproducing it but at the full
~$25 / ~5 hr re-run cost.

For HF cache hygiene after teardown, see
[`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md).

---

## Step 5 — Roadmap update

After the report PR merges:

- Flip A2.6's entry in the roadmap (`docs/trace-commons-roadmap.md`
  and `README.md`) from "in progress" to "done".
- Record the AUC range (min / max across the 4-way set) in one line.
- Update the "Production Gap Queue":
  - If Outcome 1: replace A2.6 with A2.7 ("re-enable perplexity
    floor at calibrated value").
  - If Outcome 2: replace A2.6 with the deferred-floor note.
  - If Outcome 3: replace A2.6 with Phase A.5 ("perplexity
    replacement").

The roadmap flip lands as a separate single-line PR, same shape as
the A2.3c / A2.4 finalization PRs.

---

## Hash-only / no-secrets reminder

Report JSON, report Markdown, notes companion, and cost-ledger row
are all hash-only and label-only. Candidate names, AUC values, and
corpus / manifest / report SHA256s are in scope. Raw envelope
content, HF tokens, instance IDs, IP addresses, model paths on the
bake-off host, and credentials are NOT — never include them in any
committed file or commit message.
