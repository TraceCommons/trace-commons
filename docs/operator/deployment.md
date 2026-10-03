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

### Login-resolver role (contributor accounts)

The contributor-account redeem path uses a dedicated, least-privilege
PostgreSQL role (`trace_login_resolver`) on a **separate pool** configured by
`TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`. The `V30` migration creates that
role as `NOLOGIN NOBYPASSRLS`, so it is **not directly connectable as shipped**
— you must provision a connectable, NOBYPASSRLS role before first traffic or
redeem fails closed (every redeem 400s).

```sh
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL="postgres://<login-role>@/trace-commons?host=/cloudsql/.../trace-commons"
```

Follow [`login-resolver-role.md`](login-resolver-role.md) for the exact
provisioning SQL (recommended: a dedicated LOGIN role with membership in
`trace_login_resolver`). The role MUST remain NOBYPASSRLS — the role-scoped
permissive policy is what authorizes the cross-tenant `code_hash -> tenant_id`
read.

The Slice 2 discoverable **passkey login** path reuses this **same** resolver
role and pool for its `credential_id -> tenant_id` bootstrap (V32 extends the
role with a `(tenant_id, credential_id)` grant on `trace_webauthn_credentials`
plus its own permissive policy). No additional login role is needed — the same
provisioning above covers it. If the resolver pool is unconfigured, passkey login
fails closed (every assertion collapses to the uniform deny; no session minted),
while passkey enrollment/management on the authenticated runtime pool are
unaffected.

### WebAuthn relying party (contributor passkeys, Slice 2)

Passkey enrollment and login require the WebAuthn relying-party identity. All
**three** of these env vars are **required together**:

```sh
export TRACE_COMMONS_WEBAUTHN_RP_ID="tracecommons.ai"            # effective domain; cannot change without invalidating every passkey
export TRACE_COMMONS_WEBAUTHN_RP_ORIGIN="https://app.tracecommons.ai"  # full origin URL
export TRACE_COMMONS_WEBAUTHN_RP_NAME="TraceCommons"            # shown in authenticator prompts
```

- **All-or-nothing.** Setting only some of the three is a misconfiguration: the
  passkey surface stays **disabled** (fail-closed), and the server emits a startup
  `WARN` naming which of the three are set vs unset (names only, never values).
  Setting **none** of them is the normal "passkeys disabled" state and is silent.
- **Origin must match the browser.** `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN` must be the
  exact origin the browser sees (scheme + host + port). A mismatch makes every
  ceremony fail verification at the authenticator. `RP_ID` must be a registrable
  suffix of that origin's host.
- **Several origins.** `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN` may be a
  comma-separated list, e.g. `https://tracecommons.ai,https://ingest.tracecommons.ai`.
  The first entry is the primary origin; every entry is accepted. A single value
  means what it always did. Every entry must be the `RP_ID` host or a subdomain
  of it, or startup fails. Subdomains are never implied: list each origin.
- The `webauthn-authenticator-rs` crate is a **DEV-dependency only** (it backs the
  in-process soft-authenticator used by the passkey tests). It is **not** compiled
  into or shipped with the production binaries; no production env var enables it.

### Native passkey creation (Z2 S2)

The native app creates a passkey, and with it an `unbound` account, through the
unauthenticated `POST /v1/account/native/passkey/create/{start,finish}`. Because
anyone can call it, and attestation is `none`, creation is capped by:

```sh
export TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING=5000   # the pilot's value; there is no default
```

- **Unset disables creation.** Every `create` request gets the uniform deny.
  A value that is not a non-negative integer fails startup.
- The cap is on passkey accounts in state `unbound` or `closed`, counted
  across every tenant. It is checked at `create/start` and again inside the
  `create/finish` transaction. A `closed` account (a bind refused because the
  NEAR AI account already had one, see S3 below) keeps its slot until the
  reaper deletes it, 30 days after the close, so creating and closing accounts
  in a loop cannot get past the cap (V102; before V102 only `unbound` counted).
  `bound` accounts never count.
- When the count reaches the cap, ingest logs the label
  `unbound_account_ceiling_reached` once (target `trace_commons::passkey`), and
  again only after the count has dropped below and reached it a second time.
  Alert on it.
- Each client IP (as the per-IP rate limiter reads it) may make at most
  `TRACE_COMMONS_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY` successful
  `create/finish` calls in a rolling 24 hours; the next gets the uniform deny.
  Unset means **10**; `0` refuses every creation; a value that is not a
  non-negative integer fails startup. The count is held in process, like the
  per-minute limits, and holds only a salted hash of each IP: nothing about
  the caller's address is written to the database. A restart clears it, and
  with more than one ingest instance each keeps its own count.
- Native passkey **sign-in** (`/v1/account/native/passkey/login/*`) is not
  capped and needs no new setting; like the browser sign-in it needs the
  login-resolver pool above.

V98 grants `trace_ingest_runtime` `INSERT` on `trace_account_bindings`, and
`EXECUTE` on `trace_unbound_passkey_account_count()`, a `SECURITY DEFINER`
function owned by the new NOLOGIN role `trace_unbound_account_count_guard`.
An ingest login that holds its grants some other way than through
`trace_ingest_runtime` needs both, or every `create` is refused:

```sql
SELECT has_table_privilege('<ingest runtime login>', 'public.trace_account_bindings', 'INSERT'),
       has_function_privilege('<ingest runtime login>', 'public.trace_unbound_passkey_account_count()', 'EXECUTE');
```

### Connect near.ai: binding a passkey account (Z2 S3)

An unbound account attaches its NEAR AI identity through
`POST /v1/account/near-ai/provision/bind/{start,finish}`, behind the account
middleware with a native (`tcn1_`) session. It runs the NEAR AI login
provisioning ceremony and needs exactly what that path needs (the provisioning
switch, the admission gate, the NEAR account identity, the published issuer,
and the login-resolver pool); there is no new setting. It uses the v2
readiness, so no witness JSON is required.

V100 grants `trace_ingest_runtime` `UPDATE (state, bound_at)` on
`trace_account_bindings` and nothing else. An ingest login that holds its
grants some other way needs it, or every bind fails and leaves the account
`unbound`:

```sql
SELECT has_column_privilege('<ingest runtime login>', 'public.trace_account_bindings', 'state', 'UPDATE'),
       has_column_privilege('<ingest runtime login>', 'public.trace_account_bindings', 'bound_at', 'UPDATE');
```

When the NEAR AI account already belongs to another commons account, the bind
is refused: the passkey account is closed (its sessions and passkey revoked)
and the response carries the existing account's session. Nothing moves between
the two accounts; folding the passkey into the existing account is not built.

When the daemon's device key is already registered to another account (the
machine ran NEAR AI or wallet provisioning for a different account first),
bind finish answers `409 {"error":"device_key_registered_elsewhere"}` rather
than the uniform deny, and writes an `account_binding_failed` audit row with
that stage. Nothing else is written: the passkey account stays `unbound`, and
no fresh device key is minted. The label names no tenant or account.

### Browser passkey step-up page (Z2 S7)

`GET /account/step-up` is where the native app sends a person to add or remove
a passkey or change the payout, which a weak native session cannot do. It runs
the browser passkey sign-in on the ingest origin, so that origin must be on the
origin list above, e.g.
`TRACE_COMMONS_WEBAUTHN_RP_ORIGIN=https://tracecommons.ai,https://ingest.tracecommons.ai`.
Without it the page loads but every sign-in is refused. There is no other
setting; with the relying party or the account database unset, the page is a
scriptless 503. The URL contract, headers and log labels are in
[`native-step-up-page.md`](./native-step-up-page.md).

### Login-with-NEAR (contributor NEAR sign-in, Slice 3a)

NEAR enrollment and login require the NEAR configuration. All **three** of these
env vars are **required together**:

```sh
export TRACE_COMMONS_NEAR_RPC_URL="https://rpc.mainnet.near.org"   # pin a TRUSTED endpoint; used ONLY at enroll
export TRACE_COMMONS_NEAR_NETWORK="mainnet"                        # network label (mainnet|testnet)
export TRACE_COMMONS_NEAR_LOGIN_RECIPIENT="app.tracecommons.ai"    # NEP-413 recipient the signed challenge binds to
```

- **All-or-nothing.** Setting only some of the three is a misconfiguration: the
  NEAR surface stays **disabled** (fail-closed), and the server emits a startup
  `WARN` naming which of the three are set vs unset (names only, never values —
  an rpc_url/recipient may be sensitive). Setting **none** of them is the normal
  "NEAR disabled" state and is silent.
- **RPC is used ONLY at enroll.** The `view_access_key_list` JSON-RPC call that
  proves the signing key is a FullAccess key on the named NEAR account runs
  exclusively during enroll-finish. A malicious/compromised RPC could falsely
  confirm that binding, so **pin a trusted endpoint**. **Login is fully offline**:
  it verifies the NEP-413 signature and resolves the stored `public_key -> tenant`
  binding without any network call.
- **The login `accountId` field is informational only.** The `accountId` in the
  `login/finish` assertion body is NOT verified at login — authentication is by the
  NEP-413 signature over the challenge plus the `public_key -> tenant` resolution;
  the account binding was established and RPC-verified at enroll time.
- **NEAR login depends on the login-resolver pool.** The unauthenticated
  `public_key -> tenant` bootstrap runs on the `trace_login_resolver` pool
  (`TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`, above). If that pool is
  unconfigured, NEAR login fails closed (uniform deny, no session) — same as
  redeem and passkey login. See `docs/operator/login-resolver-role.md`.
- **Encoding deps.** NEP-413 message encoding uses `borsh` (canonical struct
  serialization) and `bs58` (the `ed25519:<base58>` public-key form); both are
  compiled into the production binary and need no env vars.

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
export TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1                 # refuse to boot without a backend
# Optional overrides — defaults are production-safe:
# export TRACE_NEAR_AI_PRIVACY_BASE_URL=https://privacy-filter.completions.near.ai/v1
# export TRACE_NEAR_AI_PRIVACY_MODEL=openai/privacy-filter
# export TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS=10000
```

Set `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` on any deployment where prose-PII
filtering is part of the controls. Without it an unset backend is not an
error: the service starts, submissions succeed, and redaction quietly falls
back to deterministic-only. With it, a missing backend refuses the boot.

Confirm which backend actually resolved — the service announces it once at
startup:

```sh
sudo grep "privacy filter backend resolved" /var/log/tracecommons/ingest.log | tail -1
# Trace Commons privacy filter backend resolved privacy_filter_backend="near_ai"
```

`near_ai` (or `sidecar`) means the filter constructed. `none` means
deterministic-only redaction, whatever the config file says.

Two traps when checking this on the pilot host. Application logs go to
`/var/log/tracecommons/ingest.log`, **not** the journal — `journalctl -u
trace-commons-ingest` shows only systemd lifecycle lines, so a clean journal
is not evidence of anything. And the backend resolving at boot proves the
adapter was built, not that it successfully scrubs; for that, see the canary
check below.

Before admitting real traces, confirm a filter backend actually resolved.
`GET /v1/admin/config-status` reports it:

```json
{
  "privacy_filter_backend": "near_ai",
  "require_privacy_filter": true
}
```

`none` means deterministic-only redaction, whatever the config file says —
that is the state to catch before enabling live contributor traffic, and it
is what this deployment ran in, undetected, from launch until 2026-08-18.

This reports that the adapter **resolved**, which is not the same as proving
it scrubs. A synthetic round-trip is what proves that, and
`run_privacy_filter_canary` implements it — it gates every PII-backstop tick
— but it is not yet exposed on an admin route. Until it is, verify the
filter end to end by classifying a synthetic payload directly:

```sh
curl -s -X POST "${TRACE_NEAR_AI_PRIVACY_BASE_URL:-https://cloud-api.near.ai/v1}/privacy/classify" \
  -H "Authorization: Bearer $TRACE_NEAR_AI_PRIVACY_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model":"openai/privacy-filter","input":"My name is Dana Whitfield, my email is dana.whitfield@example.com."}'
```

Expect `private_person` and `private_email` spans. An empty `data` array or a
4xx means the adapter will construct at boot and then fail on every real
call.

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

## Redeploying the binary

### First: does this build carry a migration the database does not have?

Check before you build, not after you install:

```sh
git diff --name-only <running build_commit> <commit to ship> -- migrations
```

`<running build_commit>` is what `/health` reports. **If that prints anything,
a plain install will take the service down**, on any deployment that runs the
least-privilege role split the pilot does.

Ingest applies migrations at boot and treats a failure as fatal. On the pilot,
ingest connects as a runtime role that is not the owner of any table — every
table is owned by a separate migrator role.
[`invite-free-admission.md`](invite-free-admission.md) §1.3 requires that split
for the admission tables; the pilot applies it to all of them. A runtime
role cannot run DDL, so the first new migration fails, ingest exits, and
systemd restarts it in a loop. This happened on 2026-09-21: a build carrying
V63 through V73 died on `ERROR: must be owner of table device_keys`, restarted
58 times, and was down for about ten minutes until it was rolled back. Nothing
was applied, because the first statement of the first new migration is what
failed.

So apply new migrations **as the migrator, before installing the binary**, by
either route in `invite-free-admission.md` §1.3: point `DATABASE_URL` at the
migrator role for one boot, or apply the files with `psql` as the migrator and
record them in `_trace_commons_migrations`. Then grant the runtime role what
the new tables need; the features that add tables document their own grant
blocks (for example [`native-admission-session.md`](native-admission-session.md)
and [`mission-insight-rewards.md`](mission-insight-rewards.md)). Only then
install. An older binary ignores migration versions it does not know, so
applying them ahead of the binary is safe for the build still running.

Two more things this incident showed:

- **The build publishes both binaries and moves both `latest.txt` pointers**,
  ingest and issuer, even when you mean to deploy one. After a rollback, a bare
  `pull-and-install.sh` would reinstall the build you just backed out. Point
  each `latest.txt` back at the running build, or always pass the tag.
- **"The issuer's source did not change" is not "the issuer binary did not
  change."** It links the same library crate as ingest; its published sha256
  differed across a range in which its own `bin` file was untouched.

### V74: the public-run functions move to a runtime role

V64 meant to close its four public-run definer functions
(`trace_public_run_page`, `trace_resolve_public_run_source`,
`trace_public_run_would_cycle`, `trace_public_run_retained_source`) to PUBLIC
and grant EXECUTE to the migrator, but it did so after leaving the roles that
own them. A non-superuser migrator may not change the ACL of a function it does
not own, and PostgreSQL warns rather than fails there, so on every deployment
migrated by its own owner V64 recorded as applied with PUBLIC still holding
EXECUTE on all four and nobody holding an explicit grant. Every role could call
them; they were reachable because nothing had been closed.

V74 repairs that from inside the owner roles, and the runtime's EXECUTE now
comes from membership in a new `NOLOGIN NOBYPASSRLS` role,
`trace_public_run_runtime`, the way `trace_reward_runtime` works. The migrator
keeps EXECUTE directly, so a deployment that migrates and serves as one role
(CI, local development) needs nothing further.

**A least-privilege deployment must grant the runtime role in the same step as
V74.** V74 takes PUBLIC's EXECUTE away, so from the moment it commits an ingest
login that is not the migrator loses the public-run pages -- every
`/v1/community/runs/{slug}` read and every publication returns a permission
error -- until the grant exists. Apply V74 as the migrator and, in the same session or
the same `psql` script, run:

```sql
GRANT trace_public_run_runtime TO <ingest runtime login>;
```

On the pilot that is, as the migrator:

```sql
GRANT trace_public_run_runtime TO trace_ingest_runtime;
```

V74 also makes the unpublish trigger
(`trace_unpublish_run_when_submission_leaves_accepted`, fired when a
submission's status leaves `accepted`) a `SECURITY DEFINER` function owned by
`trace_public_run_unpublisher`, a role that holds only the columns its one
UPDATE touches. Under V64 the trigger ran as the caller and needed UPDATE on
`trace_public_runs`, which the ingest runtime has no other reason to hold; a
deployment that added `GRANT SELECT, UPDATE ON trace_public_runs TO <ingest
runtime login>` by hand to get past `permission denied for table
trace_public_runs` no longer needs it and may revoke it. The caller's tenant
setting carries into the definer function, so the forced tenant policy still
scopes the update to the caller's own tenant.

### V75: account invite trust runtime grant

V75 adds tenant-scoped account trust, invite grant, and event tables. Its
`trace_account_invite_runtime` role has column-scoped access to the existing
account, verified-anchor, and durable invite rows and access to the three new
tables. The role is `NOLOGIN NOBYPASSRLS`; the ingest login must inherit it for
`POST /v1/account/invites/redeem` to work under the restricted runtime role:

```sql
GRANT trace_account_invite_runtime TO <ingest runtime login>;
```

The invite remains issued by the separate registry role. This grant does not
allow the ingest login to mint or revoke invites.

### V90: the ingest runtime role

Migrations since V62 created tables on paths every client uses and granted
the ingest runtime nothing on them. On a deployment whose runtime login owns
no tables, that fails with `permission denied`:

- every new submission, and every status write, on `trace_submission_sessions`
  (V78);
- every idempotent re-POST of an existing submission, on
  `trace_witness_certificate_evidence` (V76);
- every withdrawal, on `trace_submission_sessions` and then on
  `trace_token_bundles`, whose V65-V68 trigger runs as the caller.

V90 names the pilot's runtime group, `trace_ingest_runtime`, as the schema's
ingest runtime role. It creates the role `NOLOGIN NOBYPASSRLS` if it does not
exist, and refuses to apply if an existing one is `SUPERUSER` or `BYPASSRLS`.
It then grants the role exactly this:

| Object | Grant | Why |
|---|---|---|
| `trace_submission_sessions` | `SELECT` | the source-session lock on every submission; withdrawal's mapping read |
| `trace_source_sessions` | `SELECT, UPDATE (withdrawn_at)` | the `FOR UPDATE` row lock; withdrawal's stamp |
| `trace_witness_evidence_runtime` | membership | V76's role for the evidence table |
| `trace_token_bundles` | `SELECT, UPDATE (state, processing_state, processing_summary)` | the revocation trigger and withdrawal's pending-deletion sweep |
| `trace_token_attachments` | `SELECT, UPDATE (deleted, prepared)` | withdrawal marking a bundle's objects deleted |
| `trace_accounts` | `UPDATE (created_at, closed_at)`, replacing any table-wide `UPDATE` | an account merge closes the absorbed account; `created_at` is the row-lock column |

V90 also grants `INSERT` on both source-session tables to
`trace_account_admission_runtime`. Claiming a source session is the only
writer of those rows, and only account admission claims one.

The `trace_accounts` change matters when account admission is switched on.
Its readiness check refuses a runtime that can update
`trace_accounts.account_id`, and a table-wide `UPDATE` grant allows that.
V90 revokes the group's table-wide `UPDATE` and checks that none remains.

Opt-in token-bundle creation (`TRACE_COMMONS_BUNDLE_SERVER_ID`) still needs its
own grants: `INSERT` on both token tables, and `UPDATE` on the receipt,
expiry and processing columns. V90 does not grant them.

On the pilot the role already exists and holds the ingest login's grants, so
V90 fixes the grants itself and nothing is left to do by hand. On any other
deployment whose ingest login is not the migrator, grant the role once, in the
same session that applies V90:

```sql
GRANT trace_ingest_runtime TO <ingest runtime login>;
```

A deployment that migrates and serves as one role needs nothing.

### V92 to V95: the pipeline tables

V92 to V95 create the versioned pipeline's tables. Each of them grants
`trace_ingest_runtime`, the group V90 names, what the pipeline code reads and
writes on the tables it creates, and nothing broader. Each refuses to apply if
the group does not exist; V90 creates it. The grants are these:

| Table | Grant | Why |
|---|---|---|
| `pipeline_runs` | `SELECT, INSERT`; `UPDATE` on `next_phase`, `state`, `last_error_label`, `updated_at`, `lease_token`, `lease_expires_at`, `attempt_count`, `next_attempt_at`, `phase_started_at`, `index_membership`, `index_command_ref`, `index_command_hash`, `index_write_state`, `score_neighbor_ref`, `score_neighbor_hash`, `settle_selection`, `settle_selection_hash`, `approved_revision_id`, `approved_object_ref_id`, `approved_content_hash` | the receipt inserts the run; claims, phase commits, retries, failures and the lease sweep lock and update it. Nothing updates its identity, `created_at`, `max_attempts`, or its admission decision |
| `phase_outcomes` | `SELECT, INSERT` | each phase commit appends its outcome, and later phases read it |
| `pipeline_bundle_packages` | `SELECT, INSERT` | registering a bundle appends its package, and every phase reads it |
| `pipeline_active_bundles` | `SELECT, INSERT` here; V112 adds `UPDATE (bundle_id, selected_at)` | startup selects the default bundle for a tenant that has none; the receipt reads it. The activation gate switches a tenant to a qualified bundle with the V112 `UPDATE` ([V110 to V113](#v110-to-v113-activation-policy-interventions-the-activation-gate-and-the-rebuild-fence)); no other statement of the runtime updates the row, and the runtime holds no `DELETE` |
| `pipeline_bundle_policy_status` | `SELECT, INSERT` here; V111 adds `UPDATE (runnable, operational_status, error_label, updated_at)` | registering a bundle adds one row per phase; the receipt and the worker read whether a phase is runnable, and an operator's policy intervention updates the row ([V110 to V113](#v110-to-v113-activation-policy-interventions-the-activation-gate-and-the-rebuild-fence)) |
| `pipeline_receipt_artifacts` | `SELECT, INSERT, DELETE, UPDATE (state, committed_at, cleanup_after)` | receipt staging, its final commit, a refused attempt's clean-up, and the orphan sweep |
| `pipeline_run_settlements` | `SELECT, INSERT`; `UPDATE` on `operation_state`, `result_ref_hash`, `external_receipt_hash`, `credit_event_id`, `settlement_batch_id`, `payout_state`, `lease_token`, `lease_expires_at`, `dispatched_at`, `attempt_count`, `last_error_label`, `updated_at` | Score adds one leg per award; Settle, reconciliation, and a failed run advance each leg. Nothing updates a leg's identity, its payout rail, or `created_at` |
| `pipeline_admission_usage` | `SELECT, INSERT` | the receipt counts each key once and reads the counts for its quota |

No grant allows `DELETE` on runs, outcomes, or legs. They go only with their
submission or tenant, through foreign-key cascades, which run as the table
owner. Outcomes and bundle packages also refuse `UPDATE` and a direct `DELETE`
by trigger.

The pipeline also uses tables older than V62:

| Table | What the pipeline needs |
|---|---|
| `trace_tenants` | `INSERT` |
| `trace_submissions` | `SELECT, INSERT`; `UPDATE` on `status`, `reviewed_at`, `updated_at`; row locks (`FOR UPDATE`, `FOR SHARE`) |
| `trace_object_refs` | `SELECT, INSERT`; a row lock (`FOR SHARE`) |
| `trace_derived_records` | `INSERT` |
| `trace_tombstones` | `SELECT` |
| `trace_withdrawals` | `SELECT` |
| `trace_credit_holds` | `SELECT` |
| `trace_credit_ledger` | `SELECT, INSERT`; `UPDATE` on `settlement_state` |
| `trace_credit_settlement_batches` | `SELECT, INSERT`; `UPDATE` on `instrument_id` |

V92 to V95 grant nothing on these tables: the pipeline needs them at V1 or
V2, long before any pipeline migration runs. V90 does not grant them either
-- its own table, above, covers only the tables V63 to V89 added. The pilot's
group holds these as table-wide privileges taken by hand when its schema was
at V62, not by any migration. A deployment whose ingest runtime group holds
V90's grants but never took the pilot's V62-era table grants by hand is still
missing them, and the pipeline -- like the legacy path -- fails closed with
`permission denied` until it does. Only a deployment carrying both, the
pilot's V62-era grants and V90's own, has nothing left to do by hand for the
pipeline.

### Account cookies take the `__Host-` prefix: a one-time browser sign-out

The browser cookies ingest sets for contributor accounts are bound to the
exact host that set them:

| Cookie | Was | Now |
|---|---|---|
| account session | `tc_account_session` | `__Host-tc_account_session` |
| passkey ceremony | `tc_passkey_ceremony` | `__Host-tc_passkey_ceremony` |
| NEAR ceremony | `tc_near_ceremony` | `__Host-tc_near_ceremony` |
| sign-in link ceremony | `tc_login_ceremony` (`Path=/account/login`) | `__Host-tc_login_ceremony` (`Path=/`) |

A browser accepts a `__Host-` cookie only with `Secure`, `Path=/` and no
`Domain`, which every one of these already carried except the sign-in link
ceremony's path. Nothing changes for native clients: the desktop apps
authenticate with a `tcn1_` bearer, not a cookie.

**The first deploy of this build signs every browser out once.** The server
does not read the old session cookie name, so a browser that presents only
`tc_account_session` gets a `401` from `/v1/account/*` and has to sign in
again. There is deliberately no period in which both names are accepted.
Server-side, the old sessions stay valid rows until they expire (seven days)
or are revoked; only the browser's handle to them is dropped.

The old cookie is also cleaned out of browsers. Every response that sets the
new session cookie (sign-in by link, passkey or NEAR, and session rotation),
and a browser logout, carries a second `Set-Cookie` that expires
`tc_account_session` (`Max-Age=0`, `Path=/`, same attributes). The in-flight
ceremony cookies need no cleanup: they live three to ten minutes, and a
ceremony started before the deploy simply has to be started again.

Nothing needs configuring. If a contributor reports being signed out after the
deploy, that is this change; signing in again is the fix.

Signing in again does not end the old session, and the contributor cannot log
it out: logout identifies the session by the new cookie, and the browser no
longer presents the old one. That row stays valid until it expires, up to
seven days. A contributor who wants it gone now should sign in again and call
`POST /v1/account/sessions/revoke-all`, which revokes every session on the
account, the old one and the current one alike, and then sign in once more.

### V97: account bindings

V97 (`trace_account_bindings`, native passkey identity) grants
`trace_ingest_runtime` `SELECT` on the new table and nothing else. Session
validation joins it on every authenticated `/v1/account/*` request, so an
ingest login that holds its grants some other way than through
`trace_ingest_runtime` fails those requests with a 500 until it can read the
table. Check before deploying:

```sql
SELECT has_table_privilege('<ingest runtime login>', 'public.trace_account_bindings', 'SELECT');
```

### V105 and V106: review, invalidation, and export tables

V105 adds the human review claims and assessments, the index invalidation
queue, two columns on `pipeline_run_settlements`
(`payout_eligible`, set when Score inserts a leg, which the runtime's
table-wide `INSERT` from V94 covers; and `credit_audited_at`, set when
ingest's worker has appended `main`'s `CreditMutate` audit event for the
leg's credit event), and indexes for the payout pass and the audit work
list. It
also widens V94's `pipeline_run_settlements_dispatch_shape` check to allow a
Trace Credit leg that `main`'s `NoveltyUtility` credit checks withheld before
its adapter was called: complete, never dispatched, no credit event, with its
withholding label. V106
adds the export snapshots and their items. Like V92 to V95, each grants
`trace_ingest_runtime` what the pipeline code reads and writes there, and
nothing broader, and each refuses to apply if the group does not exist. V105
also grants `main`'s gate driver role, `trace_gate_driver`, two columns of
`pipeline_runs`, and refuses to apply if that role (V36) does not exist:

| Object | Grant | Why |
|---|---|---|
| `pipeline_review_claims` | `SELECT, INSERT, DELETE`; `UPDATE` on `reviewer_principal_ref`, `lease_token`, `lease_expires_at`, `claimed_at` | a reviewer's claim inserts the row, or takes over an expired claim or renews its own; the assessment deletes the spent claim |
| `pipeline_review_assessments` | `SELECT, INSERT` | an assessment inserts its row; the claim, the review queue, and each Review attempt read it |
| `pipeline_index_invalidations` | `SELECT, INSERT`; `UPDATE` on `state`, `completed_at`, `attempt_count`, `next_attempt_at`, `last_error_label` | a withdrawal or a cancelled index write queues the revision's removal; the worker claims, completes, retries, or fails it; the summaries count it |
| `pipeline_run_settlements` | `UPDATE (credit_audited_at)`, the column V105 adds | the worker marks a leg's credit event audited once it appended the `CreditMutate` audit event |
| `pipeline_runs` (to `trace_gate_driver`) | `SELECT (tenant_id, submission_id)`, and a cross-tenant `SELECT` policy for that role only, as V36 gives it on `main`'s tables | `main`'s gate driver leaves every submission with a pipeline run out of its work list and backlog count; the pipeline's own Score scores it |
| `pipeline_export_snapshots` | `SELECT, INSERT`; `UPDATE` on `state`, `export_manifest_id`, `completed_at`, `invalidated_at` | export creation and delivery, a withdrawal's invalidation, and the summaries |
| `pipeline_export_snapshot_items` | `SELECT, INSERT`; `UPDATE` on `invalidated_at`, `invalidation_reason` | export creation, and a withdrawal's invalidation |

No grant allows `DELETE` on assessments, snapshots, or items. A trigger
refuses a direct `DELETE` and an `UPDATE` of their identity; they go only with
their submission or tenant, through foreign-key cascades.

These routes also write tables older than V62, which no pipeline migration
grants anything on. The pilot's V62-era table-wide grants cover them:

| Table | What the pipeline needs |
|---|---|
| `trace_export_manifests` | `INSERT` when a snapshot is delivered; `UPDATE` on `invalidated_at`, `updated_at` when a submission in it is withdrawn |
| `trace_export_manifest_items` | `INSERT` when a snapshot is delivered; `UPDATE` on `source_invalidated_at`, `source_invalidation_reason`, `updated_at` on withdrawal |
| `trace_tombstones` | `INSERT` on withdrawal |
| `trace_object_refs` | `UPDATE` on `invalidated_at`, `updated_at` on withdrawal |
| `trace_derived_records` | `UPDATE` on `status`, `updated_at` on withdrawal |
| `trace_vector_entries` | `UPDATE` on `status`, `invalidated_at`, `updated_at` on withdrawal |
| `trace_revocation_propagation_items` | `SELECT, INSERT` on withdrawal, one item per object to delete |
| `trace_near_credit_outbox` | `SELECT, INSERT`; `UPDATE` on `status`, `near_transaction_hash`, `submitted_at`, `confirmed_at`, `last_error_hash`, `near_call_json` -- only when the runtime enables NEAR payout |

The withdrawal routes -- the pipeline's and `main`'s -- read
`trace_account_admission_submissions` (V77) when the submission belongs to a
source session. V77 grants that table only to
`trace_account_admission_runtime`, and no pipeline migration grants it
again. The ingest login therefore needs membership in that role, which
`main`'s withdrawal route already needs. Without it, withdrawing a
submission of a source session fails with `permission denied` on either
route:

```sql
GRANT trace_account_admission_runtime TO <ingest runtime login>;
```

### V107 and V108: qualification and attempt artifact tables

V107 adds `pipeline_bundle_qualifications`, the immutable record that the
activation gate reads to decide whether a signed package is production
qualified. V108 adds `pipeline_attempt_artifacts`, which stages the object
each phase attempt writes -- Review's approved revision, and Score's index
command and neighbour set -- before its phase commit, so the attempt sweep
can delete the ones that never commit. Like V92 to V95, each grants
`trace_ingest_runtime` what the pipeline code reads and writes there, and
nothing broader, and each refuses to apply if the group does not exist:

| Table | Grant | Why |
|---|---|---|
| `pipeline_bundle_qualifications` | `SELECT, INSERT` | a qualification inserts one row for each bundle and code revision (V112 widens the key); the qualification route inserts it, and the activation gate and the worker and API paths read it |
| `pipeline_attempt_artifacts` | `SELECT, INSERT, DELETE`; `UPDATE` on `state`, `committed_at`, `ciphertext_sha256` | the phase write stages a row (INSERT), the phase commit moves it to `committed` and sets a hash a compatibility Score staged the row without (UPDATE), and the Score commit (for an artifact it did not write) and the attempt sweep delete a `staged` row (DELETE) |

`pipeline_bundle_qualifications` is append-only: a trigger refuses a direct
`UPDATE` or any `DELETE` that is not a cascade, and a row leaves only when
its package does, through the foreign key, which runs as the table owner.
`pipeline_attempt_artifacts`'s guard trigger allows an `UPDATE` only from
`staged` to `committed`, which may set a missing `ciphertext_sha256` to a
64-character lowercase hex value but never change one already set; every
other change to a `committed` row, or to a `staged` row but its commit, is
refused regardless of grant. A `committed` row must have its hash.

Check before deploying:

```sql
SELECT has_table_privilege('<ingest runtime login>', 'public.pipeline_bundle_qualifications', 'INSERT');
SELECT has_table_privilege('<ingest runtime login>', 'public.pipeline_attempt_artifacts', 'INSERT');
SELECT has_column_privilege('<ingest runtime login>', 'public.pipeline_attempt_artifacts', 'ciphertext_sha256', 'UPDATE');
```

### V110 to V113: activation, policy interventions, the activation gate, and the rebuild fence

V110 adds `pipeline_tenant_routing` (one row for each tenant: its routing state),
`pipeline_activation_events` (the history of routing changes), and
`pipeline_receipt_ownership` (the permanent owner of each submission id).
V111 adds `operational_status` to `pipeline_bundle_policy_status` and the table
`pipeline_policy_interventions`. V112 widens the primary key of
`pipeline_bundle_qualifications` to `(tenant_id, bundle_id, code_revision_hash)`
and grants the `UPDATE` on two columns of `pipeline_active_bundles` (`bundle_id`
and `selected_at`) that the activation gate needs. V113 adds
`pipeline_index_rebuild_fences`. Like V92 to V95, each grants
`trace_ingest_runtime` what the pipeline code reads and writes there, and
nothing broader, and each refuses to apply if the group does not exist. Each new
table enables and forces row-level security with the tenant policy
`trace_corpus_tenant_isolation`, and is in `TRACE_COMMONS_RLS_TABLES`. The
grants are these:

| Table | Grant | Why |
|---|---|---|
| `pipeline_tenant_routing` | `SELECT, INSERT`; `UPDATE` on `routing_state`, `activation_record_id`, `actor_principal_ref`, `reason_code`, `evidence_hash`, `recorded_at` | each new upload reads the row; an activation, a rollback, a containment, or a deactivation inserts it or updates every column but the key |
| `pipeline_activation_events` | `SELECT, INSERT` | each routing change appends its event; `GET /v1/admin/pipeline/routing` reads them |
| `pipeline_receipt_ownership` | `SELECT, INSERT` | the legacy path claims a submission id, and the pipeline's receipt commits its own row |
| `pipeline_bundle_policy_status` | `UPDATE (runnable, operational_status, error_label, updated_at)`, added to V93's `SELECT, INSERT` | a policy intervention updates the row, and each phase commit locks it `FOR SHARE`, which needs `UPDATE` on a column |
| `pipeline_policy_interventions` | `SELECT, INSERT` | an intervention appends its record; the list route reads them |
| `pipeline_active_bundles` | `UPDATE (bundle_id, selected_at)`, added to V93's `SELECT, INSERT` | the activation gate switches a tenant's bundle, in a statement that runs only after a qualification on the deployed revision and four runnable policies were found |
| `pipeline_index_rebuild_fences` | `SELECT, INSERT, DELETE`; `UPDATE (fenced_until)` | a rebuild inserts its row and extends it (only `fenced_until` changes), deletes it when it ends, and the invalidation claim reads the rows |

`pipeline_activation_events`, `pipeline_receipt_ownership`, and
`pipeline_policy_interventions` are append-only: a trigger refuses an `UPDATE`
and a direct `DELETE`. A row leaves only through a foreign-key cascade, which
runs as the table owner: with its tenant, and also with its run (an ownership
row) or with its bundle's policy rows (an intervention).
`pipeline_tenant_routing` and
`pipeline_index_rebuild_fences` are mutable, in the columns above. No grant
allows `DELETE` on the routing, event, ownership, or intervention tables. The
runtime login still has no `DELETE` on `pipeline_active_bundles`.

The grants do not make the routes the only way to change routing. V110 lets the
ingest login `INSERT` and `UPDATE` `pipeline_tenant_routing` and `INSERT` events,
and V112 lets it `UPDATE (bundle_id, selected_at)` on `pipeline_active_bundles`.
Any statement under that login can use them, skip the activation gate, and write
no event. Only the code limits this: it changes routing through the routes
(`POST /v1/admin/pipeline/...`, see
[pipeline-activation.md](pipeline-activation.md)), and the active bundle only
through the gate. Change routing only through the routes. Do not run a direct
statement of the ingest login against these tables.

V111 adds a check that requires `runnable` to equal `operational_status =
'runnable'`, and every existing row gets the status `runnable`. V93 gave the
runtime no `UPDATE` on the table and nothing wrote `runnable`, so a row has
`runnable` false only if someone set it by hand. A row like that makes V111 fail
to apply. Check before you apply it.

The table forces row-level security, also for its owner. A count by the
migrator with no tenant set sees no row and answers 0, whatever the table
holds. So do the count in one of these two ways. As a superuser, or as a role
with `BYPASSRLS`:

```sql
SELECT COUNT(*) FROM pipeline_bundle_policy_status WHERE NOT runnable;
```

Or as the migrator, one time for each tenant that has a pipeline bundle (each
tenant that is, or was, on a pipeline list):

```sql
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '<tenant id>', true);
SELECT COUNT(*) FROM pipeline_bundle_policy_status WHERE NOT runnable;
COMMIT;
```

Each count must be 0. V111 itself reads every row when it adds the check, so it
fails on a row that the plain count did not show. Apply V110 to V113 as the
migrator before you install the binary, as for every migration above ("First:
does this build carry a migration the database does not have?").

**Binary rollback to an older build.** An older binary ignores these
migrations. What it does with a tenant that has a routing row depends on the
build:

- A build with no pipeline runtime (the repository binary) reads no routing
  row. It serves every tenant on the legacy path, a tenant whose row says
  `pipeline` or `contained` included.
- A build that has a pipeline runtime and is from before these migrations'
  code also reads no routing row. It routes by its receipts list alone. Every
  new upload of a tenant on its receipts list goes to the pipeline: a tenant
  whose row says `contained` or `legacy`, and a listed tenant with no row, too.
  Its receipt transaction checks no routing and writes no ownership row. It
  also has no policy guard at a phase commit and none at the payout dispatch.
  A suspended policy still stops a receipt and the start of a phase, because
  that build reads `runnable` there. It does not stop a phase that already
  runs, and it does not stop a payout dispatch under a suspended Settle
  policy.

Before you install a build of the second kind, do these steps:

1. Read each listed tenant's routing (`GET /v1/admin/pipeline/routing`) and
   suspended policies (`suspended_policy_count` in `GET
   /v1/admin/pipeline/operational-summary`) on the current build.
2. In the older build's configuration, keep on the receipts list only the
   tenants whose row says `pipeline`. Move every other tenant (contained,
   `legacy`, or with no row) to the drain list, or off both lists. A `contain`
   or a `deactivate` does not protect a tenant on that build: only the lists
   do.

   On that build, a tenant that is off the receipts list uploads on the legacy
   path. This includes a contained tenant. That build does not read the row, so
   nothing there holds the tenant's intake: its uploads are neither refused
   nor sent to the pipeline, and the legacy path takes them. If a contained
   tenant's uploads must stay stopped, do not install that build.
3. Do not rely on a suspension: on that build it does not hold for a phase
   that already runs or for a payout. For a tenant with a suspended Settle
   policy, keep the tenant off both lists (its pipeline work then waits), or
   set the NEAR settlement mode of the older build to `disabled`
   (`TRACE_COMMONS_NEAR_SETTLEMENT_MODE`; this stops every NEAR payout of the
   process, `main`'s too). Keep that until a build with the guards runs again.

See also "Run one build and one configuration" in "Scope lists and the routing
row" of [pipeline-activation.md](pipeline-activation.md).

The drain report (`GET /v1/admin/pipeline/legacy-drain`) reads 16 tables through
the ingest login. Two are pipeline tables that V92 and V110 grant
(`pipeline_runs`, `pipeline_tenant_routing`). Three are covered by V90's grants
(`trace_submission_sessions`, `trace_source_sessions`,
`trace_token_attachments`). The other eleven are older tables of `main`, which
no pipeline migration grants anything on: the pilot's V62-era table grants cover
them, as they cover the older tables in "V92 to V95". A deployment without those
grants answers the report with a `500` (`permission denied`), not with a zero.
The tables are listed in [pipeline-activation.md](pipeline-activation.md),
"Legacy drain report".

The three routes that qualify and activate bundles need three settings that no
earlier build read:

| Variable | Set | What it does |
|---|---|---|
| `TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH` | at run time | a JSON file, an array of trusted keys (`{"key_id", "public_key_base64url"}`), whose signatures make a bundle package trusted |
| `TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH` | at run time | the same format, for the keys whose signatures make a check result count; no key may also be a package key |
| `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` | when you build | the output of `python3 scripts/operator/pipeline.py revision` for the tree you build |

A variable that is set to a file that cannot be read, or whose file is not valid,
refuses the start (`pipeline_trust_store_invalid`), and so does a key that is in
both files (`pipeline_trust_store_overlap`). A trust store variable that is set
to the empty string counts as unset, and so does a build revision that is set
to the empty string. Any other build revision that is not `sha256:` and 64
lowercase hex digits refuses the start (`pipeline_code_revision_invalid`). With a trust store variable unset, or
no revision in the build, the routes refuse (`503`
`pipeline_trust_store_missing`, `409` `bundle_runtime_revision_unknown`). `GET
/v1/admin/config-status` reports the three as booleans
(`pipeline_package_trust_store_loaded`, `pipeline_check_trust_store_loaded`,
`pipeline_code_revision_configured`). The details are in
[pipeline-activation.md](pipeline-activation.md), "What the process needs".

Check before deploying:

```sql
SELECT has_table_privilege('<ingest runtime login>', 'public.pipeline_tenant_routing', 'INSERT');
SELECT has_table_privilege('<ingest runtime login>', 'public.pipeline_receipt_ownership', 'INSERT');
SELECT has_column_privilege('<ingest runtime login>', 'public.pipeline_active_bundles', 'bundle_id', 'UPDATE');
SELECT has_column_privilege('<ingest runtime login>', 'public.pipeline_bundle_policy_status', 'operational_status', 'UPDATE');
SELECT has_table_privilege('<ingest runtime login>', 'public.pipeline_index_rebuild_fences', 'DELETE');
```

### Build and install

The pilot host has no Rust toolchain; binaries are built by Cloud Build and
pulled from GCS. From a clean checkout at the commit you intend to ship:

```sh
gcloud builds submit --config cloudbuild.yaml \
  --project tracecommons-pilot-2026 \
  --substitutions _TAG=$(git rev-parse --short HEAD)
```

Roughly 6 minutes on `E2_HIGHCPU_32`. **Wait for `SUCCESS` before installing:**

```sh
gcloud builds list --project tracecommons-pilot-2026 --limit 3
```

Then on the host, naming the tag you expect:

```sh
deploy/pilot-gcp/pull-and-install.sh ingest <short-sha>
```

Pass the tag. `latest.txt` points at the last build that *published*, so
installing while your build is still in flight silently reinstalls the previous
binary, restarts the service, and prints `done.` — a successful-looking no-op
that has happened in practice. With the tag, a mismatch refuses instead.

Deploy `ingest` alone unless the issuer also changed: ingest fetches the
issuer's JWKS at boot and fail-closes without it, so leaving the issuer running
keeps that dependency out of the deploy. `both` installs the issuer first for
exactly this reason.

The script backs the running binary up to `<path>.bak-<timestamp>` and prints
its own rollback command. Keep that line — the backup from a *premature* run is
a copy of the old binary, not a pre-deploy snapshot, so identify the rollback
point by the run that actually installed the tag you wanted.

Verify all three after the restart:

```sh
curl -s http://127.0.0.1:3907/health                     # build_commit == your tag
sudo grep "privacy filter backend resolved" /var/log/tracecommons/ingest.log | tail -1
systemctl is-active trace-commons-ingest
```

With `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1` set, `active` also proves a
privacy backend resolved — the service refuses to start otherwise.

### Log rotation

The units append to `/var/log/tracecommons/*.log`; nothing rotates them by
default, and `ingest.log` reached ~174MB before this was noticed. Install the
provided config once per host:

```sh
sudo install -o root -g root -m 0644 \
  deploy/pilot-gcp/logrotate-tracecommons.conf /etc/logrotate.d/tracecommons
sudo logrotate --debug /etc/logrotate.d/tracecommons   # dry run
```

It uses `copytruncate` deliberately; see the comments in that file.

## Identifying what is deployed

Every binary carries the commit it was built from, and both services report it
on `/health`. Ask the running service rather than the host's git checkout: the
checkout on the pilot host is routinely weeks behind the binary that is
actually running.

```sh
curl -s https://ingest.example.com/health | jq .
```

```json
{
  "status": "ok",
  "schema_version": "trace_contribution.v1",
  "build_commit": "6f160d43",
  "build_time": "2026-08-17T18:42:11Z",
  "build_version": "0.1.0"
}
```

The issuer answers the same way, alongside the `checks` object it already
reported — on its degraded response too, which is when the question is most
often being asked:

```sh
curl -s https://issuer.tracecommons.ai/health | jq '{status, build_commit, build_time}'
```

The same identity is on the command line, for a binary that is on disk but not
running:

```sh
./target/release/trace-commons-ingest --version
# trace-commons-ingest 0.1.0 (commit 6f160d43, built 2026-08-17T18:42:11Z)
```

Read `build_commit`, not `build_version`. The crate version does not move when
a deploy does — it stayed at `0.1.0` across every change that has ever shipped
to the pilot — so it identifies nothing on its own. `build_commit` is the value
to paste into `git show`.

A `build_commit` of `unknown` means the build could not resolve a commit. The
binary still runs, but it cannot be traced back to source. Cloud Build compiles
from a source tarball with no `.git/` (see `.gcloudignore`), so it passes the
commit in through the `TRACE_COMMONS_BUILD_COMMIT` environment variable;
`cloudbuild.yaml` sets it from the same value it names the GCS object with. A
local `cargo build` inside a git checkout picks the commit up from git instead.
So `unknown` on a deployed binary points at the build step, not at the host.

## Next steps

- Run the HF bootstrap calibration: [`calibration.md`](calibration.md).
- Wire scheduled smoke testing.
- After ~1000 real pilot traces, re-calibrate floors and flip
  `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` from `0` to the
  configured live value.
