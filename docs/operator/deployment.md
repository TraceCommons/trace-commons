# First-deploy Walkthrough

End-to-end procedure to take a fresh GCP project plus a fresh H100 host to
"`tracedao-ingest` is live, gate is calibrated, smoke-test is green." This
is the authoritative deploy doc; everything else under `docs/operator/`
referrs back here.

## Prerequisites

You must have:

1. **GCP project** with billing enabled.
2. **Cloud KMS key** (symmetric, software-protected or HSM). Note its full
   resource name; you'll set it as `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME`.
3. **Service account** (or Workload Identity binding) with
   `roles/cloudkms.cryptoKeyEncrypterDecrypter` on the key above, and
   `roles/storage.objectAdmin` on the GCS bucket below. Prefer Workload
   Identity over key files.
4. **GCS bucket** with **object versioning enabled** and CMEK pointing at
   the Cloud KMS key. The runbook refuses to start if versioning is off
   when `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true`.
5. **PostgreSQL** instance (Cloud SQL recommended) with the repo's
   migrations applied: `cargo run -p tracedao-server --bin migrate` or the
   equivalent migration command. RLS is forced on every Trace Commons
   table — the runtime role descriptor's SHA256 should match
   `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256` if set.
6. **H100 host** (single-GPU, 80 GB) with CUDA drivers and `nvcc`
   installed. Production builds need `--features local-gpu-models-cuda`.
7. **Rust toolchain** on a build host (can be the H100). Stable channel,
   workspace-pinned.

## Build the binary

On a build host with CUDA available:

```sh
cargo build --release -p tracedao-server \
  --features gcs-client,gcp-kms,local-gpu-models-cuda
```

The two binaries land in `target/release/`:

- `tracedao-ingest` — the main service.
- `tracedao-upload-claim-issuer` — the EdDSA upload-claim signer.

Plus (when built with `local-gpu-models`):

- `tracedao-gate-calibrate` — offline calibration helper. See
  [`calibration.md`](calibration.md).

## Stage models

Use [`scripts/operator/stage-models.sh`](../../scripts/operator/stage-models.sh):

```sh
TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/llama-3.1-8b-instruct \
TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/tracedao-embedder \
HF_TOKEN=hf_xxxxxxxxxxxxxxxx \
./scripts/operator/stage-models.sh
```

The script downloads Llama-3.1-8B-Instruct and BGE-large-en-v1.5, then
verifies SHA256 against `scripts/operator/.model-checksums`. Re-running is
idempotent; already-staged weights are skipped.

## Configure environment

Set env vars in dependency order. The [`env-reference.md`](env-reference.md)
has the full surface; this is a minimum production-shaped configuration.

```sh
# --- Database ---
export DATABASE_URL="postgres://app@/tracedao?host=/cloudsql/.../tracedao"
export TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true
export TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY=true

# --- KEK / GCS ---
export TRACE_COMMONS_KEK_PROVIDER=gcp_kms
export TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME="projects/<proj>/locations/<loc>/keyRings/<ring>/cryptoKeys/<key>"
export TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY=true

export TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=gcs
export TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET=<bucket>
export TRACE_COMMONS_REMOTE_OBJECT_STORE_KMS_KEY_ID="$TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME"
export TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true

# --- Auth ---
export TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS=true
export TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS=true
export TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL="https://issuer.example.com/.well-known/keyset"
export TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS=issuer.example.com
export TRACE_COMMONS_SIGNED_TOKEN_ISSUER=https://issuer.example.com
export TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE=tracedao-ingest
export TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI=true
export TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true

# --- Gate (models) ---
export TRACE_COMMONS_GATE_SERVICE=enclave_local_gpu
export TRACE_COMMONS_GATE_SERVICE_MASTER_KEY=<32B hex>  # generate once, store securely
export TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/llama-3.1-8b-instruct
export TRACE_COMMONS_PERPLEXITY_DEVICE=cuda:0
export TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/tracedao-embedder
export TRACE_COMMONS_VECTOR_INDEX_ROOT=/var/lib/tracedao-vector-index
export TRACE_COMMONS_VECTOR_INDEX_DIM=1024

# --- Gate (floors) — START AT ZERO-CREDIT during Phase 2 of calibration ---
# Values below come from `analyze-calibration.sh` on the HF bootstrap.
# Replace with Phase-2 re-cal numbers after ~1000 real traces.
export TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=0      # placeholder; calibrate first
export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=0   # placeholder
export TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000    # 0.5 cosine novelty
export TRACE_COMMONS_GATE_POLICY_VERSION=pilot-v1
export TRACE_COMMONS_GATE_TOP_K=5

# --- Credit (zero during calibration) ---
export TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0
export TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true
export TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE=true
export TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS=sha256:<central-issuer-principal>
export TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL=true
export TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY=true
```

At least one of the three gate floors must be positive — the binary
refuses to start if all are zero. Setting `novelty_floor_micros=500000`
(corresponds to cosine novelty 0.5) is a reasonable starting point that
will later be replaced by calibrated values.

## Initial start

```sh
./target/release/tracedao-ingest 2>&1 | tee /var/log/tracedao/start.log
```

If the binary refuses to start, the first line of stderr is a hash-only
class name (see [`hash-only-logging.md`](hash-only-logging.md)). The
common first-deploy failures are listed in
[`troubleshooting.md`](troubleshooting.md).

## What to look at in the first hour

1. **Startup phase.** Watch for these `tracing` events in order:
   - `kek_wrapper.ready` — KMS adapter loaded.
   - `gate_service.ready` (with `policy_version` and `gate_version_hash`) —
     candle scorer + fastembed embedder + usearch index loaded.
   - `db.rls_ready` — RLS policies present.
   - `server.listening` — Axum bound.
2. **`GET /v1/admin/config-status`.** Should report no critical config
   warnings. Field `gate_service_status.ready` should be true.
3. **First smoke pass.** Run
   [`scripts/operator/smoke-gate.sh`](../../scripts/operator/smoke-gate.sh)
   in dry-run (default):
   ```sh
   ./scripts/operator/smoke-gate.sh \
     --target=https://ingest.example.com \
     --admin-token=$ADMIN_TOKEN \
     --worker-token=$WORKER_TOKEN
   ```
   The script hits every required drill endpoint and runs a fixture gate
   evaluation. Exit code 0 = ready.
4. **First contributor submission.** Inspect:
   - `trace_audit_events` — chain advances by one row per state transition.
   - `trace_gate_decisions` — one row per gate evaluation, with the
     `gate_version_hash` from step 1.
   - `trace_credit_ledger` — empty (delta is 0 during calibration).

If any of the above is missing or stalls, see
[`troubleshooting.md`](troubleshooting.md).

## Next steps

- Run the HF bootstrap calibration: [`calibration.md`](calibration.md).
- Wire scheduled smoke testing.
- After ~1000 real pilot traces, re-calibrate floors and flip
  `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` from `0` to the
  configured live value.
