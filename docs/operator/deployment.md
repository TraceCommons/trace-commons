# First-deploy Walkthrough

End-to-end procedure to take a fresh GCP project plus a fresh H100 host to
"`trace-commons-ingest` is live, gate is calibrated, smoke-test is green." This
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
   migrations applied: `cargo run -p trace-commons-server --bin migrate` or the
   equivalent migration command. RLS is forced on every Trace Commons
   table — the runtime role descriptor's SHA256 should match
   `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256` if set.
6. **H100 host** (single-GPU, 80 GB) with CUDA drivers and `nvcc`
   installed. Production builds need `--features local-gpu-models-cuda`.
7. **Rust toolchain** on a build host (can be the H100). Stable channel,
   workspace-pinned.

## Build host preflight

Two known build-host issues to clear before invoking cargo. Both were
observed on a fresh Ubuntu 22.04 Lambda Cloud A10 host in the 2026-05
smoke deploy; either will surface as a confusing late-stage build error.

### 1. Compiler must support `avx512fp16`

The `numkong` SIMD crate (transitive dep of `usearch`) uses the
`__attribute__((target("avx512fp16")))` syntax. gcc-11 (the default on
Ubuntu 22.04) does not recognize it; the build fails inside a vendored
C++ source file.

Use gcc-12 or newer:

```sh
sudo apt-get install -y gcc-12 g++-12
sudo update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-12 60 \
  --slave /usr/bin/g++ g++ /usr/bin/g++-12 \
  --slave /usr/bin/cc cc /usr/bin/gcc-12
```

Ubuntu 24.04 ships gcc-13 by default and does not need this step.

### 2. ONNX Runtime prebuilt binary requires glibc 2.38+

The `ort` 2.0.0-rc.12 crate (transitive dep of `fastembed`) downloads a
pre-built ONNX Runtime binary that references C2X glibc aliases
(`__isoc23_strtol`, `__isoc23_strtoll`, `__isoc23_strtoul`,
`__isoc23_strtoull`). These first appear in glibc 2.38. The link will
fail on Ubuntu 22.04 (glibc 2.35) with `rust-lld: undefined symbol`.

**Recommended fix: deploy on Ubuntu 24.04 (glibc 2.39).** This is the
intended target.

**Fallback for Ubuntu 22.04:** link a small shim that aliases the C2X
symbols to the plain `strto*` functions. Only the binary-literal parsing
extension is lost, which the ORT runtime does not exercise on the input
paths the gate uses.

```sh
mkdir -p $HOME/isoc23-shim
cat > $HOME/isoc23-shim/shim.c <<'EOF'
#include <stdlib.h>
long __isoc23_strtol(const char *s, char **e, int b) { return strtol(s,e,b); }
long long __isoc23_strtoll(const char *s, char **e, int b) { return strtoll(s,e,b); }
unsigned long __isoc23_strtoul(const char *s, char **e, int b) { return strtoul(s,e,b); }
unsigned long long __isoc23_strtoull(const char *s, char **e, int b) { return strtoull(s,e,b); }
EOF
gcc -O2 -fPIC -c $HOME/isoc23-shim/shim.c -o $HOME/isoc23-shim/shim.o
ar rcs $HOME/isoc23-shim/libisoc23shim.a $HOME/isoc23-shim/shim.o
export RUSTFLAGS="-L $HOME/isoc23-shim -l static=isoc23shim"
```

This is a deploy-host workaround, not a code change. Track it as a known
constraint until Ubuntu 24.04 (or a newer base) is the operator default.

## Build the binary

On a build host with CUDA available:

```sh
cargo build --release -p trace-commons-server \
  --features gcs-client,gcp-kms,local-gpu-models-cuda
```

The two binaries land in `target/release/`:

- `trace-commons-ingest` — the main service.
- `trace-commons-upload-claim-issuer` — the EdDSA upload-claim signer.

Plus (when built with `local-gpu-models`):

- `trace-commons-gate-calibrate` — offline calibration helper. See
  [`calibration.md`](calibration.md).

## Stage models

Use [`scripts/operator/stage-models.sh`](../../scripts/operator/stage-models.sh):

```sh
TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/qwen3-8b-base \
TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/trace-commons-embedder \
HF_TOKEN=hf_xxxxxxxxxxxxxxxx \
./scripts/operator/stage-models.sh
```

The script downloads the configured perplexity model and
BGE-large-en-v1.5, then verifies SHA256 against
`scripts/operator/.model-checksums`. Re-running is idempotent;
already-staged weights are skipped. A2.5 recommends **Qwen3-8B-Base**
as the operator default (see `calibration.md` Phase 0);
Llama-3.1-8B-Instruct remains a permitted incumbent choice but is no
longer the recommended default.

## Configure environment

Set env vars in dependency order. The [`env-reference.md`](env-reference.md)
has the full surface; this is a minimum production-shaped configuration.

```sh
# --- Database ---
export DATABASE_URL="postgres://app@/trace-commons?host=/cloudsql/.../trace-commons"
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
export TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE=trace-commons-ingest
export TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI=true
export TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true

# --- Gate (models) ---
export TRACE_COMMONS_GATE_SERVICE=enclave_local_gpu
export TRACE_COMMONS_GATE_SERVICE_MASTER_KEY=<32B hex>  # generate once, store securely
export TRACE_COMMONS_PERPLEXITY_MODEL_PATH=/srv/models/qwen3-8b-base  # A2.5 recommendation; arch auto-detected via mistralrs (A2.3)
export TRACE_COMMONS_PERPLEXITY_DEVICE=cuda:0
export TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/trace-commons-embedder
export TRACE_COMMONS_VECTOR_INDEX_ROOT=/var/lib/trace-commons-vector-index
export TRACE_COMMONS_VECTOR_INDEX_DIM=1024

# --- Gate (floors) — A2.5 pilot-launch defaults; see calibration.md Phase 1 ---
# A2.3c + A2.4 measured perplexity-AUC < 0.5 across all candidates and corpora,
# so the perplexity floor ships disabled. Tail-fraction floor is calibrated
# post-first-1000-pilot-traces. Novelty is the active primary gate at launch.
export TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=6246774  # A2.7 (2026-05-15): calibrated from Qwen 3.6 27B per-trace scores; 0.5x headroom on geomean(Youden's-J=13.03, p10_novel=11.98). See docs/superpowers/reports/2026-05-15-a27-calibration-result.json.
export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=0   # A2.5: calibrate post-first-1000-traces
export TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000    # 0.5 cosine novelty; unchanged
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

### Privacy filter backend (pilot)

Pilot builds must include the `near-ai-privacy-filter` Cargo feature to enable
the hosted backend:

```sh
cargo build --release -p trace-commons-server \
  --features gcs-client,gcp-kms,local-gpu-models-cuda,near-ai-privacy-filter
```

Set these additional env vars before starting the binary:

```sh
# --- Privacy filter (NEAR AI hosted backend) ---
export TRACE_PRIVACY_FILTER_BACKEND=near-ai
export TRACE_NEAR_AI_PRIVACY_API_KEY=<near-ai-bearer-token>   # never logged; rotate by restart
# Optional overrides — defaults are production-safe:
# export TRACE_NEAR_AI_PRIVACY_BASE_URL=https://privacy-filter.completions.near.ai/v1
# export TRACE_NEAR_AI_PRIVACY_MODEL=openai/privacy-filter
# export TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS=10000
```

Before admitting real traces, the privacy-filter canary must report healthy.
The canary submits a synthetic PII smoke payload and verifies the filter
removes it; it reuses the existing `run_privacy_filter_canary` path in the
rollout-smoke suite. Check the result via `GET /v1/admin/config-status` —
the `privacy_filter_canary_status.healthy` field must be `true` before you
enable live contributor traffic.

At least one of the three gate floors must be positive — the binary
refuses to start if all are zero. Under the A2.5 pilot-launch defaults
the novelty floor (`TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=500000`,
cosine novelty 0.5) is the floor satisfying that invariant; the
perplexity and tail-fraction floors ship disabled. The tail-fraction
floor is calibrated against the pilot distribution after ~1000 traces;
the perplexity floor stays at zero until Phase A.5 work lands a
replacement metric. See `calibration.md` Phase 1 for the rationale
and `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`
for the underlying data.

## Initial start

```sh
./target/release/trace-commons-ingest 2>&1 | tee /var/log/trace-commons/start.log
```

If the binary refuses to start, the first line of stderr is a hash-only
class name (see [`hash-only-logging.md`](hash-only-logging.md)). The
common first-deploy failures are listed in
[`troubleshooting.md`](troubleshooting.md).

## What to look at in the first hour

1. **Startup phase.** Watch for these `tracing` events in order:
   - `kek_wrapper.ready` — KMS adapter loaded.
   - `gate_service.ready` (with `policy_version` and `gate_version_hash`) —
     mistralrs scorer + fastembed embedder + usearch index loaded
     (A2.3 migrated the perplexity scorer off candle-direct).
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
