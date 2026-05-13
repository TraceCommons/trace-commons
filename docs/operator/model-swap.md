# Model Swap

Procedure for upgrading the perplexity model or the embedder. **Any
change here rotates the `gate_version_hash`** — the audit-fixes PR makes
the hash depend on the model IDs plus context windows plus matryoshka
dim. A rotated hash means new gate decisions are stamped with a new
version, and central-issuer approval is required before live credit
emission resumes.

## Three substeps

### 1. Stage the new weights

Update `scripts/operator/.model-checksums` with the new SHA256 (you can
verify against a trusted local copy first), then:

```sh
# Perplexity (Llama) swap
TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/<new-checkpoint> \
HF_TOKEN=hf_xxx \
./scripts/operator/stage-models.sh --perplexity-only

# Embedder swap
TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/tracedao-embedder \
./scripts/operator/stage-models.sh --embedder-only
```

The script refuses if the downloaded SHA256 doesn't match
`.model-checksums`.

### 2. Update env

```sh
export TRACE_COMMONS_PERPLEXITY_MODEL_ID=meta-llama/<new-id>
export TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/<new-checkpoint>
# OR
export TRACE_COMMONS_EMBEDDER_MODEL_ID=BAAI/<new-id>
```

Restart `tracedao-ingest`. On startup, watch for `gate_service.ready`
emitting a new `gate_version_hash`. That value is what gets stamped on
new audit + credit rows.

### 3. Re-calibrate

A new model has a new perplexity / novelty distribution. Re-run the HF
bootstrap then the closed-alpha re-cal per
[`calibration.md`](calibration.md), and update the three floors in env.

**Until re-calibration completes, set
`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0`** so the gate
collects metrics without emitting credit. This is the same pattern as
the initial deploy.

## Grandfather settled credit

`trace_credit_ledger` rows are stamped with the `gate_version_hash` of
the policy that approved them. Rows minted under the previous hash are
unaffected by the swap; only new evaluations are gated by the new
version. This is intentional — never retroactively invalidate credit
that was settled under a previously-approved version.

## Failure modes

| Symptom | Cause | Fix |
|---|---|---|
| Startup fails with `PerplexityScorerInit` error | Model files missing or corrupted | Re-run `stage-models.sh`; check disk space |
| `EmbedderModelIdUnrecognized` | Embedder ID not in fastembed's allowlist | Use the supported list from `fastembed::TextEmbedding::list_supported_models()` |
| Embedder native dim ≠ vector index dim | The new embedder produces a different vector size | Update `TRACE_COMMONS_VECTOR_INDEX_DIM` AND **purge the vector index** (`TRACE_COMMONS_VECTOR_INDEX_ROOT`) before restart — the previous tenant indexes are dim-incompatible |
| `gate_version_hash` did not change | The model id env was not actually updated | Verify with `GET /v1/admin/config-status` |
| Credit row still stamped with old hash after swap | Submission was already in flight when you restarted | Expected — the in-flight evaluation completed under the old hash |
