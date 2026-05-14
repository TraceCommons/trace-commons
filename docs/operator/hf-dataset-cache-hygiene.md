# HuggingFace Dataset Cache Hygiene — Operator Runbook

Pilot-bootstrap ([`./pilot-bootstrap.md`](./pilot-bootstrap.md),
[`./pilot-bootstrap-first-100-traces.md`](./pilot-bootstrap-first-100-traces.md)),
the agent-traces corpus builder
(`scripts/operator/build-agent-traces-corpus.py`), and any future
bake-off corpus rebuild ([`./agent-traces-bakeoff-run.md`](./agent-traces-bakeoff-run.md))
all read from `~/.cache/huggingface/`. Sizes are non-trivial — this
runbook is the reference for managing the cache, reclaiming disk
when needed, and avoiding cache-related re-download costs on the
next run.

## Datasets and approximate sizes

| Repo | Files | Approx size |
|---|---|---|
| `jedisct1/agent-traces-swival` | 33,667 JSONL | ~1.5 GB |
| `badlogicgames/pi-mono` | 627 JSONL | TBD |
| `TeichAI/DeepSeek-v4-Pro-Agent` | 4,006 JSONL | TBD |

Model checkpoints share the same root and dominate the disk
footprint on bake-off hosts:

| Model | Approx size |
|---|---|
| Llama-3.1-8B-Instruct | ~16 GB |
| Qwen3-8B-Base | ~16 GB |
| Qwen 3.6 27B Dense | ~54 GB |
| Gemma 4 31B Base | ~62 GB |

---

## Cache layout

```
~/.cache/huggingface/
  hub/
    datasets--<org>--<name>/
      blobs/
      refs/
      snapshots/
    models--<org>--<name>/
      blobs/
      refs/
      snapshots/
  token                  # if HF login was performed
```

Datasets and models are siblings under `hub/`, distinguished by the
`datasets--` vs `models--` prefix. `huggingface-cli scan-cache` is
the canonical reader of this structure.

---

## Disk-space requirements

- **Pilot-bootstrap host (datasets only).** ~5 GB headroom is
  sufficient for swival; ~10 GB if pi-mono and DeepSeek-v4-Pro-Agent
  are also resident.
- **Bake-off host (datasets + all four candidate models).** ~150 GB
  headroom. The 4-way candidate set alone is ~148 GB on disk; the
  corpus datasets are small relative to the models. Lambda H100
  SXM5 80GB instances ship with enough local SSD for this; if the
  shape changes, verify before staging.

Plan disk before you start a run. A mid-run ENOSPC on the cache
silently fails the dataset / model load and is hard to distinguish
from upstream HF errors in the hash-only logs.

---

## Hygiene commands

List cached repos and their sizes:

```bash
huggingface-cli scan-cache
```

Delete a specific repo interactively (preferred):

```bash
huggingface-cli delete-cache --disable-tui
# Then enter the numbered selection for the repo(s) to remove.
```

Delete a specific repo by path (when scripting):

```bash
rm -rf ~/.cache/huggingface/hub/datasets--<org>--<name>
rm -rf ~/.cache/huggingface/hub/models--<org>--<name>
```

The `datasets--`/`models--` prefix on the directory name is part
of the canonical path; do not strip it.

---

## When to clear

- **Between bake-off runs targeting different model sets.** Stale
  models keep occupying disk indefinitely.
- **After teardown of an ephemeral pilot-bootstrap host.** Cache
  is local to the host and dies with it; no explicit clear needed,
  but the host's disk should be sized to not require mid-run
  clearing.
- **Never during a live run.** Concurrent deletion races the
  loader and produces partial-file errors that look identical to
  upstream HF outages.
- **Not for cost reasons.** See note below.

---

## Cost note

Cache clears do not affect billing. The cost driver is instance
runtime — for the GPU cost ledger and bake-off cost-per-run see
[`./gpu-cost-ledger.md`](./gpu-cost-ledger.md). Deleting the cache
to "save money" only adds re-download time on the next run
(several minutes for datasets, tens of minutes for the 4-way model
set). Don't.

If a host needs to free disk mid-flight, prefer clearing model
caches that are not needed for the current run over clearing
dataset caches that the current run depends on.

---

## Hash-only / no-secrets reminder

`~/.cache/huggingface/token` contains the operator's HF access
token if `huggingface-cli login` was used. Never commit this file
or any path under `~/.cache/huggingface/` to the repo, and never
include the token in logs, audit rows, or commit messages.
