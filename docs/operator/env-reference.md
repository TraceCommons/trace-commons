# `TRACE_COMMONS_*` Environment Variable Reference

Every env var the `trace-commons-ingest` binary reads, grouped by surface. The
master list is `grep '^const TRACE_COMMONS_' crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs`.
If you change a default in code, update this table.

The binary parses constants at boot; values that fail to parse refuse the
binary's startup rather than degrading. **Defaults are conservative — when
in doubt, override.** Required envs marked `R`.

## Conventions

- All time fields ending in `_MICROS`, `_SECONDS`, `_HOURS`, `_MS` are
  integers in those units.
- All `_REQUIRE_*` flags accept `true` / `false` (default false unless
  noted).
- Tenant-ID lists are comma-separated string IDs (not UUIDs).
- Bearer tokens are operator-supplied opaque strings; rotated by env
  reload + binary restart.

---

## 1. KEK / encryption surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_KEK_PROVIDER` | R for prod | (none) | One of `local_master_key`, `gcp_kms`. Production v1 is `gcp_kms`. |
| `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME` | R when `gcp_kms` | (none) | Full Cloud KMS resource name (`projects/.../locations/.../keyRings/.../cryptoKeys/...`). |
| `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` | R for prod | `false` | When `true`, refuses `local_master_key` and any downgraded path. |
| `TRACE_COMMONS_GATE_SERVICE_MASTER_KEY` | R when gate=enclave_local_gpu | (none) | Local master key (hex) for wrapping the gate service's per-request DEKs. Separate from the KEK above. |

## 2. Gate orchestrator surface

These are the floors and policy id the orchestrator uses to make pass/fail
decisions. The audit-fixes PR introduced the requirement that they be set
explicitly (no implicit zero defaults).

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_GATE_SERVICE` | R when used | (none) | One of `disabled`, `mock`, `enclave_local_gpu`. Production: `enclave_local_gpu`. |
| `TRACE_COMMONS_GATE_SERVICE_ENCLAVE_ENDPOINT` | Phase B | (none) | Remote enclave endpoint. Unused in v1. |
| `TRACE_COMMONS_GATE_SERVICE_ATTESTATION_VERIFIER_LABEL` | Phase B | (none) | Attestation verifier label. Unused in v1. |
| `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` | R when enclave_local_gpu | (none) | Inclusive lower bound on aggregate perplexity (micros). Pass if measured >= floor. Pilot-launch default: `0` (disabled) — see A2.5 spec / `calibration.md` Phase 1. |
| `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS` | R when enclave_local_gpu | (none) | Inclusive lower bound on tail fraction (micros). Pilot-launch default: `0` — calibrate post-first-1000-pilot-traces per A2.5 spec / `calibration.md` Phase 1. |
| `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` | R when enclave_local_gpu | (none) | Inclusive lower bound on novelty score (micros). Provisional BGE-large-en-v1.5 holding value: `100000` (cosine novelty 0.10); see calibration.md and the October 24 isolated controls. |
| `TRACE_COMMONS_GATE_POLICY_VERSION` | R when enclave_local_gpu | (none) | Human label for the policy version (e.g. `pilot-v1`). Stamps audit rows. |
| `TRACE_COMMONS_GATE_TOP_K` | optional | `5` | Number of nearest neighbors used for novelty. |

The `gate_version_hash` stamped on every audit row and every credit event
is **derived** from the floors + policy version + perplexity model id + max
tokens + tail cutoff + embedder model id + embedder max tokens +
matryoshka dim + vector index dim. Changing any of those rotates the gate
version automatically. See [`model-swap.md`](model-swap.md) for the swap
procedure.

## 3. Perplexity model surface (with `local-gpu-models`)

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_PERPLEXITY_MODEL_ID` | optional | `meta-llama/Llama-3.1-8B-Instruct` | HF repo id. Recorded in the gate version hash. The default is the **incumbent** baseline pending the A2.1 model bake-off (see `docs/operator/calibration.md` § Phase 0); the empirical winner replaces it. |
| `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` | R | (none) | Local directory with HF release layout (`config.json`, `tokenizer.json`, `model.safetensors`*). Set to the path of the bake-off winner after Phase 0 completes. |
| `TRACE_COMMONS_PERPLEXITY_DEVICE` | optional | `cuda` | One of `cuda`, `cuda:N`, `metal`, `cpu`. |
| `TRACE_COMMONS_PERPLEXITY_MAX_TOKENS` | optional | `16384` | Context window cap. |
| `TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF` | optional | `-8.0` | Negative log-probability threshold for the tail fraction. |
| `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` | **deprecated** (A2.3) | n/a | Historical candle-backend selector. As of A2.3 the perplexity scorer is mistralrs-backed and auto-detects the architecture from each model's `config.json`; this env var is ignored. The production binary still grep-spots it at startup and emits a deprecation warn-log when set so operators flip their configs. Slated for hard-error in A2.4. |

## 4. Embedder surface (with `local-gpu-models`)

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_EMBEDDER_MODEL_ID` | optional | `BAAI/bge-large-en-v1.5` | HF repo id. Must match fastembed's known models. |
| `TRACE_COMMONS_EMBEDDER_CACHE_DIR` | optional | `/var/cache/trace-commons-embedder` | Where fastembed caches ONNX weights. |
| `TRACE_COMMONS_EMBEDDER_MAX_TOKENS` | optional | `512` | Max input tokens (truncation applied by fastembed tokenizer). |
| `TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM` | optional | unset (use native dim) | Truncate embedding to this many dimensions and re-normalize. Must be <= native dim and == `TRACE_COMMONS_VECTOR_INDEX_DIM`. |

## 5. Vector index surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_VECTOR_INDEX_ROOT` | optional | `/var/lib/trace-commons-vector-index` | On-disk per-tenant index root. Must be on a fast local disk. |
| `TRACE_COMMONS_VECTOR_INDEX_DIM` | optional | `1024` | Must match embedder output dim. |
| `TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN` | optional | `32` | LRU cap on per-tenant indexes held open. |
| `TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY` | optional | `32` | Inserts between disk syncs. |
| `TRACE_COMMONS_VECTOR_INDEX_FLUSH_INTERVAL_SECONDS` | optional | `60` | Periodic flush cadence. Bounds how much of the novelty corpus a hard kill (SIGKILL, power loss) can lose. `0` disables the background flusher — do not set it to `0` in production. |
| `TRACE_COMMONS_SHUTDOWN_GRACE_SECONDS` | optional | `20` | How long `trace-commons-ingest` drains in-flight requests after SIGTERM before it flushes the vector indexes and exits. Keep it below systemd's `TimeoutStopSec`. |
| `TRACE_COMMONS_VECTOR_INDEX_HNSW_M` | optional | `16` | HNSW M parameter. |
| `TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION` | optional | `200` | HNSW efConstruction. |
| `TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH` | optional | `50` | HNSW efSearch. |

## 6. Object store surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_FILE_OBJECT_STORE` | (label) | `trace_commons_file_store` | Internal label, not an env. |
| `TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE` | (label) | `trace_commons_encrypted_artifact_store` | Internal label. |
| `TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE` | (label) | — | Internal label. |
| `TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE` | (label) | `trace_commons_service_owned_remote` | Internal label. |
| `TRACE_COMMONS_SERVICE_REMOTE_DISABLED_OBJECT_STORE` | (label) | — | Internal label. |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER` | R for prod | (none) | `gcs` in v1. |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET` | R for prod | (none) | GCS bucket name. |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_KMS_KEY_ID` | R for prod | (none) | CMEK key (typically same as `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME`). |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_CREDENTIAL_REF` | optional | (ADC) | Service-account JSON path. Prefer Workload Identity (omit this). |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING` | optional | `false` | File backend only. |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_REGION` | optional | (none) | GCS region. |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_ENDPOINT` | optional | (none) | Override for emulator. |
| `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING` | R for prod | `false` | Refuses to start if bucket lacks object versioning. Set true in prod. |
| `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW` | optional | — | Primary store for submit/review reads. |
| `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT` | optional | — | Primary store for replay/export. |
| `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS` | optional | — | Primary store for derived exports. |
| `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW_TENANT_IDS` | optional | — | Tenant-scoped override. |
| `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT_TENANT_IDS` | optional | — | Tenant-scoped override. |
| `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS_TENANT_IDS` | optional | — | Tenant-scoped override. |
| `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS_TENANT_IDS` | optional | — | Tenant scope for derived-export object-ref enforcement. |

## 7. PostgreSQL surface

| Var | R? | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | R | (none) | Standard Postgres connection string. (Not `TRACE_COMMONS_*` but required.) |
| `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN` | R for prod | `false` | Refuse boot if reconciliation drill shows drift. |
| `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY` | R for prod | `false` | Refuse boot if RLS readiness drill fails. |
| `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256` | optional | — | Expected sha256 of the runtime role descriptor. |
| `TRACE_COMMONS_DB_CONTRIBUTOR_READS_TENANT_IDS` | optional | — | Tenant scope: which tenants the contributor role may read. |
| `TRACE_COMMONS_DB_REVIEWER_READS_TENANT_IDS` | optional | — | Tenant scope for reviewer reads. |
| `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS_TENANT_IDS` | optional | — | Tenants for which reviewer must use object-ref reads. |
| `TRACE_COMMONS_DB_REPLAY_EXPORT_READS_TENANT_IDS` | optional | — | Tenant scope. |
| `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS_TENANT_IDS` | optional | — | Tenant scope. |
| `TRACE_COMMONS_DB_AUDIT_READS_TENANT_IDS` | optional | — | Tenant scope. |
| `TRACE_COMMONS_DB_TENANT_POLICY_READS_TENANT_IDS` | optional | — | Tenant scope. |
| `TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES` | optional | — | Comma-separated retention policy IDs that imply legal hold. |
| `TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST` | optional | — | Page cap for export endpoints. |
| `TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR` | optional | — | Rate-limit budget per tenant. |
| `TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR` | optional | — | Rate-limit budget per principal. |
| `TRACE_COMMONS_SUBMIT_PER_PRINCIPAL_RATE_PER_MIN` | optional | `30` | In-process `POST /v1/traces` limit per authenticated principal per 60 s fixed window, applied before any other submit work. Integer `1`–`600`; anything else refuses startup. Over it, ingest answers `429` `{"error":"submit_rate_limited","retry_after_seconds":N}` with a matching `Retry-After: N` header, where `N` is the whole seconds until the principal's window turns (at least 1). Read once at startup; single-instance only. |
| `TRACE_COMMONS_SUBMIT_PER_PRINCIPAL_CONCURRENCY` | optional | `2` | `POST /v1/traces` requests one principal may have in flight at once. Integer `1`–`4`; anything else refuses startup. The ceiling is the large-body upload middleware's own per-principal cap of 4, which runs first: a higher submit cap could never be reached. Over it, the same `429` `submit_rate_limited` answer with `retry_after_seconds` `2` and `Retry-After: 2`; the large-body middleware's own in-flight refusal (submit and token-bundle uploads) answers the same way. |
| `TRACE_COMMONS_LARGE_BODY_BYTE_BUDGET` | optional | `MAX_INGEST_BODY_BYTES` + 128 MiB (406,847,488) | Deployment-wide budget of upload body bytes in flight (`POST /v1/traces` and the token-bundle upload routes), taken per upload by its declared `Content-Length` (a body without one is weighed at its route's ceiling) before the body is read. An upload that does not fit is refused with the same `429` `submit_rate_limited` answer and `Retry-After: 2`; a declared length over the route's ceiling is refused with `413` at once. Integer from `MAX_INGEST_BODY_BYTES` (272,629,760: one maximal upload) to 4,294,967,295; anything else refuses startup. Ingest holds several copies of a body while it is processed, so resident memory for uploads is a multiple of this budget (measured 2026-10-29 at 200 MB, debug build: about 7.5x through a local encrypted store, 8.7x through the pilot's legacy shape -- remote store plus PostgreSQL mirror -- and about 12x through the pipeline receipt path including its worker's first steps; see `large_envelope_tests`); size it against the host's memory with that multiple. |
| `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS` | optional | `false` | When `true`, accepts medium residual-risk submissions after server-side re-scrub. High residual-risk submissions still quarantine. Intended for tightly scoped pilots where message text/tool payloads are included. Governs both paths: the legacy path accepts every Medium; a tenant routed to the versioned pipeline has Admission admit only a Medium whose server-side basis is `found_and_removed` (alone or with `consent_content_flag`), recorded under the Admission rule `minimal_admission_v1_accept_found_and_removed`; any other Medium is still quarantined `privacy_review_required`. See [`pipeline-activation.md`](pipeline-activation.md#admission-and-trace_commons_accept_medium_risk_submissions). |
| `TRACE_COMMONS_ALLOW_LEGACY_SEGMENT_RESUME` | optional, emergency only | `false` | When `true`, `POST /v1/admin/audit-chain-repair` may resume the DB audit chain across a legacy segment (`"accept_legacy_segment": true`) after a rollback to a pre-#1043 build. Off, a non-dry run that asks for it is refused `409` `legacy_segment_resume_disabled` and writes nothing; the dry run still diagnoses `file_ahead_through_legacy_rows`. Read once at startup. Off by default since 2026-10-02, when the pilot's pre-#1043 rollback target was retired. See [`audit-trail-forensics.md`](audit-trail-forensics.md). |
| `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` | optional | (none, driver fails closed) | Separate connection string for the narrow `trace_gate_driver` role pool used by the perplexity-scoring driver (Task 5/6). Mirrors `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`'s shape: points at a LOGIN role granted membership in `trace_gate_driver` (V36 policies + V42 column-scoped SELECT). See [`perplexity-scoring-driver.md`](perplexity-scoring-driver.md). |
| `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` | R when `TRACE_COMMONS_PII_BACKSTOP_ENABLED=true` | (none, refuses boot) | Separate connection string for the narrow `trace_pii_backstop_driver` role pool (migration V38). Mirrors `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`'s shape: points at a LOGIN role granted membership in `trace_pii_backstop_driver`. See [`pii-backstop.md`](pii-backstop.md). |

## 8. Auth / signed-token surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_SIGNED_TOKEN_SECRET` | legacy | — | HS256 (deprecated). |
| `TRACE_COMMONS_SIGNED_TOKEN_SECRETS` | legacy | — | HS256 keyset (deprecated). |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_PEM` | optional | — | EdDSA pubkey inline. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILE` | optional | — | EdDSA pubkey path. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILES` | optional | — | Multiple pubkey paths. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_JSON` | optional | — | Inline keyset JSON. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_FILE` | optional | — | Keyset JSON path. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL` | optional | — | Remote keyset URL. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS` | optional | — | Allowed hosts for keyset URL. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_BEARER_TOKEN` | optional | — | Bearer for fetching the keyset. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_TIMEOUT_MS` | optional | — | Fetch timeout. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_REFRESH_INTERVAL_SECONDS` | optional | — | Refresh interval. |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_MAX_STALE_SECONDS` | optional | — | Max age before refusing. |
| `TRACE_COMMONS_SIGNED_TOKEN_ISSUER` | R | (none) | Expected `iss` claim. |
| `TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE` | R | (none) | Expected `aud` claim. |
| `TRACE_COMMONS_SIGNED_TOKEN_REVOKED_JTIS` | optional | — | Revocation list. |
| `TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS` | optional | — | Refuse tokens with longer TTL. |
| `TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI` | optional | `false` | Refuse tokens without `jti`. |
| `TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS` | R for prod | `false` | Reject HS256. |
| `TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS` | R for prod | `false` | Require keyset-managed EdDSA (not inline PEM). |
| `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS` | R for prod | `false` | Require grant-row authorization on every read. |

### 8a. Score attestation signing

Set all three or none. With none set, attestation signing is disabled and
both `/.well-known/trace-commons-attestation-keyset.json` and
`/v1/contributors/me/score-attestation` fail closed with
`503 attestation_signing_key_unconfigured` — correct, but indistinguishable
from an outage to a contributor running `trace-commons-contributor attest`,
so configure these wherever contributors attest. A partial configuration is
an operator error and refuses at boot rather than silently disabling.

Generate the keypair with `trace-commons-ingest --generate-attestation-keypair`;
it reads no env and touches no database, so it works before the server is
configured, and prints all three assignments in env-file format.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID` | R when attesting | (none) | Key id published in the keyset and stamped in the JWS header. |
| `TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM` | R when attesting | (none) | PKCS#8 v2 Ed25519 private key PEM. Operator secret; never commit. |
| `TRACE_COMMONS_INGEST_ATTESTATION_PUBLIC_KEY_PEM` | R when attesting | (none) | SPKI Ed25519 public key PEM. Published in the keyset endpoint. |
| `TRACE_COMMONS_INGEST_ATTESTATION_TTL_SECONDS` | optional | `86400` | Attestation lifetime. Must be positive. |

## 9. Analytics surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` | optional | — | k-anon floor. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_KEY` | optional | — | Per-release noise key. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_MAX_DELTA` | optional | — | Max noise delta in micros. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_EPSILON_MICROS` | optional | — | Configured epsilon. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_MAX_EPSILON_MICROS` | optional | — | Refuse epsilons above this. |
| `TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED` | optional | `false` | Enables `/v1/community/*` profile, leaderboard, contributor, and analytics snapshot routes. Requires the DB mirror for real data. When enabled, also starts the coalesced withdrawal→snapshot invalidation drain on `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS`. |
| `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS` | optional | `900` | Interval for the in-process community snapshot invalidation drain (and the published ≤15-minute freshness bound's rebuild cadence). Clamped to `[30, 86400]`. Only consulted when the community surface is enabled. |
| `TRACE_COMMONS_COMMUNITY_TENANT_IDS` | optional | — | Comma-separated tenant ids included in community snapshot recompute. Set this when the runtime DB role is non-bypassing and forced RLS would hide `trace_tenants` enumeration. |
| `TRACE_COMMONS_COMMUNITY_ANALYTICS_PUBLICATION_BASIS` | optional | `approved_noise_mechanism` | What published corpus aggregates rest on. `approved_noise_mechanism` (default) withholds analytics until a calibrated mechanism is approved. `suppression_only` publishes under cell suppression alone — **no noise is applied and totals are not suppressed**, so at small corpus sizes the totals can still describe a handful of contributors closely. The min-cell floor is required either way and is never waived. Any user-facing description of the deployment must state which basis is in force. |
| `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS` | optional | — | Recompute the community snapshot in-process on this interval. Unset, empty or `0` leaves recompute admin-triggered only. Minimum 60s. **Assumes a single writer**: the worker skips a tick when a snapshot is already fresh, but that is coordination, not mutual exclusion. On a multi-replica deployment either leave this unset and drive recompute from one elected scheduler, or accept duplicate aggregation work. Retention keeps the 96 most recent snapshots per window/metric. Without it the published leaderboard is only as fresh as the last manual POST to `/v1/admin/community/snapshots/recompute`. |
| `TRACE_COMMONS_COMMUNITY_CORS_ORIGINS` | optional | `https://tracecommons.ai,http://localhost:8788,http://127.0.0.1:8788` | Comma-separated browser origins allowed to call `/v1/community/*`. Keep this to the public Pages site plus local preview origins. |

## 10. Credit / NEAR surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | R for live | `0` | Per-pass credit points of the gate's scoring award. Set to `0` during calibration; flip to configured value at cutover. The award is settlement-eligible: a non-zero value settles at the next live settlement run and is paid out. A value above `0` refuses to start unless `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` is true (`novelty_utility_credit_requires_production_gate`) and `TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM` is set (`novelty_utility_settle_from_missing`). A submission earns at most one paid award (a positive delta, written at or after the cut-over), whichever gate version scored it, through the gate worker route or the perplexity driver. A $0 award, or one written before the cut-over, never settles and does not stand in the way of a later paid award. |
| `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` | R for live | `false` | Refuse to emit novelty-utility credit unless the gate is a production gate (`enclave_local_gpu`, `dstack` or `enclave_near_ai`); any other gate's decision, including the test gates `in_memory`, `enclave_mock`, `legacy_deterministic` and `dstack_stub`, is recorded with `credit_withheld_reason = non_production_gate`. Required when the delta is above `0`. |
| `TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM` | R when delta > 0 | — | RFC 3339 cut-over for the scoring award, for example `2026-11-01T00:00:00Z`. Settlement, the credit risk summary, withdrawal's `credit_retained` and the legacy drain report count a `novelty_utility` event only when it was written at or after this instant; earlier awards (from a test gate, or at a delta nobody meant to pay) never settle, and do not stop a submission earning its one paid award after it. Unset, no scoring award settles. A value that is not RFC 3339 refuses to start (`novelty_utility_settle_from_invalid`). Set it to the time the paying delta goes live. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` | R for prod | — | Hard cap per account per settlement. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` | R for prod | — | Allowlist of policy versions accepted for settlement. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL` | R for prod | `false` | Require central-issuer approval signature. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_ISSUER_APPROVAL_MAX_AGE_HOURS` | optional | — | Reject stale approvals. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE` | R for prod | `false` | Require central-issuer profile config. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` | R when require | — | Principal refs that count as central issuer. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY` | R for prod | `false` | Block settlement unless rollout-smoke is green. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID` | optional | — | NEAR contract id for settlement. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_NEAR_CONTRACT` | optional | `false` | Require NEAR contract presence. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED` | optional | `false` | Toggles the settlement scheduler. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_SCOPE` | optional | `tenant` | `tenant` uses the configured tenant bearer; explicit `all_tenants` uses the deployment's private internal scheduler and bounded database enumeration. See [automatic credit settlement](automatic-credit-settlement.md). |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN` | R for enabled tenant scope | — | Scheduler bearer token; must be absent for `all_tenants`. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_INTERVAL_SECONDS` | optional | — | Run cadence. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION` | R when enabled | — | Pinned policy version. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_REASON` | optional | — | Reason label written to audit. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run mode. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ISSUER_APPROVAL_EVIDENCE_HASH` | optional | — | Pinned approval-evidence hash. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_NEAR_CONTRACT_ID` | optional | — | Scheduler-side NEAR contract. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_RANKING_MODEL_VERSION` | optional | — | Pinned ranking model. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_RANKING_TARGET_USE` | optional | — | Pinned target use. |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_LIMIT` | optional | — | Per-tick row cap. |
| `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` | optional | — | Adapter submitter URL. |
| `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_BEARER_TOKEN` | optional | — | Adapter bearer. |
| `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` | optional | — | Adapter confirmation URL. |
| `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_BEARER_TOKEN` | optional | — | Confirmation bearer. |
| `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH` | optional | `false` | Require bearer-token auth on adapter. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_ENABLED` | optional | `false` | Outbox scheduler toggle. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_TOKEN` | R when enabled | — | Scheduler bearer. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS` | optional | — | Run cadence. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_SUBMIT_LIMIT` | optional | — | Submit cap. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_CONFIRM_LIMIT` | optional | — | Confirm cap. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_PURPOSE` | optional | — | Audit reason. |

## 11. Benchmark surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_URL` | optional | — | Registry submitter URL. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_URL` | optional | — | Confirmation URL. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_BEARER_TOKEN` | optional | — | Confirmation bearer. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_REQUIRE_ADAPTER_AUTH` | optional | `false` | Require bearer auth. |
| `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` | optional | — | External evaluator URL. |
| `TRACE_COMMONS_BENCHMARK_EVALUATOR_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_BENCHMARK_EVALUATOR_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_ENABLED` | optional | `false` | Registry scheduler toggle. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_SUBMIT_LIMIT` | optional | — | Submit cap. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_CONFIRM_LIMIT` | optional | — | Confirm cap. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_PURPOSE` | optional | — | Audit reason. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_ENABLED` | optional | `false` | Pipeline scheduler toggle. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_EVALUATION_LIMIT` | optional | — | Evaluation cap. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_PUBLICATION_LIMIT` | optional | — | Publication cap. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_REQUIRE_EXTERNAL_EVALUATOR` | optional | `false` | Require external evaluator. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_EVALUATOR_REF` | optional | — | Evaluator ref. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_REGISTRY_REF_PREFIX` | optional | — | Registry prefix. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_MIN_SCORE` | optional | — | Minimum score for publication. |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_REASON` | optional | — | Audit reason. |

## 12. Other scheduler surfaces

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_ENABLED` | optional | `false` | Toggle. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_DATASET_KIND` | optional | — | Dataset kind label. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RUN_QUEUED_MAX_JOBS` | optional | — | Cap. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_JOBS` | optional | — | Retry cap. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_RETRY_COUNT` | optional | — | Per-job retry cap. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_BASE_DELAY_SECONDS` | optional | — | Backoff base. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_MAX_DELAY_SECONDS` | optional | — | Backoff cap. |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_REASON` | optional | — | Audit reason. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_ENABLED` | optional | `false` | Toggle. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_PURPOSE` | optional | — | Audit reason. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_PRUNE_EXPORT_CACHE` | optional | `false` | Prune toggle. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_MAX_EXPORT_AGE_HOURS` | optional | — | Export cache age. |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_PURGE_EXPIRED_BEFORE` | optional | — | Purge horizon. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_ENABLED` | optional | `false` | Vector revocation/rebuild toggle. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_LIMIT` | optional | — | Row cap. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_PURPOSE` | optional | — | Audit reason. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_ENABLED` | optional | `false` | Toggle. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_LIMIT` | optional | — | Row cap. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_REQUIRE_EXTERNAL_EVALUATOR` | optional | `false` | Require external evaluator. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_EVALUATOR_REF` | optional | — | Evaluator ref. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_TARGET_USE` | optional | — | Target use label. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_UTILITY_CATEGORY` | optional | — | Utility category. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_EXTERNAL_REF_PREFIX` | optional | — | External ref prefix. |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_REASON` | optional | — | Audit reason. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_ENABLED` | optional | `false` | Toggle. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_LIMIT` | optional | — | Row cap. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_PURPOSE` | optional | — | Audit reason. |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_MAX_ATTEMPTS` | optional | — | Max attempts before marking terminal-failed. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_ENABLED` | optional | `false` | Credit cycle toggle. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_TOKEN` | R when enabled | — | Bearer. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_INTERVAL_SECONDS` | optional | — | Cadence. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_TARGET_USE` | optional | — | Target use. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_MODEL_VERSION` | optional | — | Ranking model version. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_POLICY_VERSION` | optional | — | Policy version. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_REASON` | optional | — | Audit reason. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_DRY_RUN` | optional | `false` | Dry-run. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_PREFLIGHT_ONLY` | optional | `false` | Run preflight, no settlement. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_SUBMIT_NEAR_OUTBOX` | optional | `false` | Drive NEAR outbox submit. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_CONFIRM_NEAR_OUTBOX` | optional | `false` | Drive NEAR outbox confirm. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_NEAR_CONTRACT_ID` | optional | — | NEAR contract id. |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_LIMIT` | optional | — | Row cap. |

### 12a. Perplexity scoring driver

Unlike the other schedulers above, this driver has no bearer-token gate — it is
driven entirely by `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` plus the separate
`TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` pool (§7). See
[`perplexity-scoring-driver.md`](perplexity-scoring-driver.md) for the full
runbook, including why the floor stays 0.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` | optional | `false` | Toggle. Off by default; existing deployments and CI are unaffected until an operator opts in. A trace the driver scores earns the gate's scoring award as the gate worker route would (cached and skipped-duplicate decisions earn none). The driver issues it as `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`; with `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` set and that issuer unset or not listed, every driver award is withheld as `central_issuer_denied`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_INTERVAL_SECONDS` | optional | `45` | Cadence between enumeration batches. Clamped to `[5, 86400]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_BATCH_SIZE` | optional | `5` | Submissions enumerated per tick. Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS` | optional | `5` | Bounded attempt counter per submission before the driver stops retrying it. Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES` | optional | `true` | Skip-duplicate cache cost control. Falsy values (`0`, `false`, `no`, `off`) disable it. An assembled pipeline runtime reads this and the threshold below even with the driver off, and applies both to compatibility runs' gate decision rows (`docs/operator/pipeline-activation.md`). |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS` | optional | `900000` | Duplicate-score threshold (micros): a submission whose precheck duplicate score, in micros, is at or above it is recorded as `skipped_duplicate` without scoring. Clamped to `[0, 1000000]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_BACKOFF_BASE_SECONDS` | optional | `30` | Base backoff (seconds) applied after a scoring failure, before the bounded attempt counter allows a retry. Clamped to `[0, 86400]`. |

### 12b. Server-side NEAR AI PII backstop driver

Off by default. When `TRACE_COMMONS_PII_BACKSTOP_ENABLED=true`, a Low-risk
trace with `message_text_included` is held in corpus status
`awaiting_pii_backstop` instead of `Accepted` until this driver re-redacts
the message text via the NEAR AI privacy filter (§15) and releases it to
`Accepted`/`Quarantined`. Folded into the perplexity-scoring driver task
family; see [`pii-backstop.md`](pii-backstop.md) for the full enable
checklist and drill.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_PII_BACKSTOP_ENABLED` | optional | `false` | Master toggle. Off by default; existing deployments and CI are unaffected until an operator opts in. |
| `TRACE_COMMONS_PII_BACKSTOP_TICK_INTERVAL_SECONDS` | optional | `45` | Cadence between enumeration batches. Clamped to `[5, 86400]`. |
| `TRACE_COMMONS_PII_BACKSTOP_BATCH_SIZE` | optional | `5` | Held submissions enumerated per tick. Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` | optional | `5` | Bounded attempt counter per submission before the driver stops retrying it (it stays held on `awaiting_pii_backstop`). Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PII_BACKSTOP_PER_SUBMISSION_TIMEOUT_SECONDS` | optional | `900` | Wall-clock ceiling on re-redacting ONE submission. Without it a single large trace monopolises the driver: on the pilot, one submission consumed 50 hours and ~1,250 classify windows while 210 others were never enumerated. A submission that outruns the budget is charged an attempt (being too large is a property of the trace, not the upstream), so backoff applies and it is eventually quarantined for review rather than retried forever. Clamped to `[1, 86400]`. |
| `TRACE_COMMONS_PII_BACKSTOP_BACKOFF_BASE_SECONDS` | optional | `30` | Base backoff (seconds) applied after a re-redaction failure, before the bounded attempt counter allows a retry. Clamped to `[0, 86400]`. |
| `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` | R when enabled | (none, refuses boot) | See §7 above and [`pii-backstop.md`](pii-backstop.md). |

`TRACE_NEAR_AI_PRIVACY_API_KEY` (§15) is also required when this driver is
enabled — the driver reuses the same NEAR AI privacy-filter credentials the
submit-time filter uses. Fail-closed: enabling the backstop without either
the driver database URL or the API key refuses at boot.

## 13. External worker adapter surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_VECTOR_EMBEDDER_URL` | optional | — | Remote embedder URL (only when not in-process). |
| `TRACE_COMMONS_VECTOR_EMBEDDER_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_VECTOR_EMBEDDER_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_VECTOR_EMBEDDER_REQUIRE_EXTERNAL` | optional | `false` | Refuse in-process fallback. |
| `TRACE_COMMONS_VECTOR_SEARCH_URL` | optional | — | Remote search URL. |
| `TRACE_COMMONS_VECTOR_SEARCH_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_VECTOR_SEARCH_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_VECTOR_SEARCH_REQUIRE_EXTERNAL` | optional | `false` | Refuse in-process fallback. |
| `TRACE_COMMONS_PROCESS_EVALUATOR_URL` | optional | — | External evaluator URL. |
| `TRACE_COMMONS_PROCESS_EVALUATOR_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_PROCESS_EVALUATOR_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_URL` | optional | — | Remote deleter URL. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_REQUIRE_EXTERNAL` | optional | `false` | Refuse in-process fallback. |
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_URL` | optional | — | Cache invalidator URL. |
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_BEARER_TOKEN` | optional | — | Bearer. |
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_TIMEOUT_MS` | optional | — | Timeout. |
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_REQUIRE_EXTERNAL` | optional | `false` | Refuse in-process fallback. |

## 14. Ranking / calibration surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` | optional | — | Reject calibrations older than this. |
| `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY` | optional | `false` | Require calibration dataset registry rows. |
| `TRACE_COMMONS_RANKING_REQUIRE_ACTIVE_CALIBRATION_DATASET` | optional | `false` | Require active calibration. |
| `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE` | optional | `false` | Require server-side feature provenance. |
| `TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD` | optional | — | Floor. |
| `TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS` | optional | — | Ceiling. |
| `TRACE_COMMONS_RANKING_MIN_LABEL_COUNT` | optional | — | Floor. |
| `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT` | optional | — | Floor. |
| `TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT` | optional | — | Floor. |
| `TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS` | optional | — | Floor. |
| `TRACE_COMMONS_RANKING_MAX_LABELER_ISSUE_RATE_MICROS` | optional | — | Ceiling. |
| `TRACE_COMMONS_RANKING_MIN_LABELER_RELIABILITY_LABEL_COUNT` | optional | — | Floor. |

## 15. Privacy filter backend

The privacy filter backend is selected explicitly — there is no auto-fallback.
Unknown values for `TRACE_PRIVACY_FILTER_BACKEND` are refused at startup so
misconfigurations surface immediately rather than silently degrading to a
weaker path. A backend named without the configuration it needs (`near-ai`
without its API key, `sidecar` without its command) is refused at startup too,
rather than surfacing on each submission.

`trace-commons-ingest` logs the resolved backend once at startup:

```
Trace Commons privacy filter backend resolved  privacy_filter_backend=near_ai
```

A value of `none` there means deterministic-only redaction. That line is the
only signal distinguishing a filter that ran from one that was never
configured — an unset backend is not an error, and a runtime filter failure
falls back to the unfiltered text with a `privacy_filter:<backend>_failure`
counter. Deployments that require prose-PII filtering should set
`TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` so an absent backend refuses the boot
instead.

The `near-ai` backend requires the `near-ai-privacy-filter` Cargo feature;
pilot builds enable it. When `near-ai` is active, the pipeline-version suffix
appended to envelope records is `+privacy-filter-near-ai-v1`; this value is
audit-relevant and is included in gate version hash derivation.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_PRIVACY_FILTER_BACKEND` | optional | unset | `sidecar` \| `near-ai` \| `self-hosted` \| unset. Unset = deterministic-only redaction. Unknown values refuse startup. |
| `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` | optional | unset | Truthy = refuse to start unless a privacy filter backend is configured. Opt-in; unset keeps existing boot behaviour. Recommended wherever prose-PII filtering is part of the deployment's controls. |
| `TRACE_PRIVACY_FILTER_COMMAND` | when `sidecar` | (none) | Path to sidecar binary. Legacy name `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` still read with a one-shot deprecation warning. |
| `TRACE_PRIVACY_FILTER_ARGS` | optional | empty | Whitespace-separated argv. Legacy: `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. |
| `TRACE_PRIVACY_FILTER_TIMEOUT_MS` | optional | `10000` | Sidecar timeout. Legacy: `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS`. |
| `TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES` | optional | (sidecar default) | Max stdin bytes per call. Legacy: `IRONCLAW_*`. |
| `TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES` | optional | (sidecar default) | Max stdout bytes. Legacy: `IRONCLAW_*`. |
| `TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` | optional | (sidecar default) | Max stderr bytes. Legacy: `IRONCLAW_*`. |
| `TRACE_NEAR_AI_PRIVACY_API_KEY` | when `near-ai` | (none) | NEAR AI Cloud bearer token. Never logged; rotation is restart-only. |
| `TRACE_NEAR_AI_PRIVACY_BASE_URL` | optional | `https://cloud-api.near.ai/v1` | Hosted endpoint; supports the `https://privacy-filter.completions.near.ai/v1` faster path. Must include the scheme and must be `https`, or `http` against loopback (local sidecars, tests) -- the API key rides on every request as a bearer token, so a plaintext non-loopback endpoint is refused at construction. |
| `TRACE_NEAR_AI_PRIVACY_MODEL` | optional | `openai/privacy-filter` | Model slug. |
| `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` | optional | `10000` | HTTP request timeout. |
| `TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES` | optional | (sidecar default) | Refuses inputs above this size. |

---

### Self-hosted privacy filter

Set when `TRACE_PRIVACY_FILTER_BACKEND=self-hosted`. The backend talks to the
`trace-commons-privacy-filter` service on loopback (see
`deploy/pilot-gcp/privacy-filter/`), which serves the same `openai/privacy-filter`
weights the hosted endpoint serves.

Prefer it to `near-ai` for locality: no unredacted prose leaves the host.

Both backends window, but for different reasons, and the difference decides how
you tune them. The hosted endpoint serves a model reporting
`context_length: 512` behind an internal splitter and fails above ~3,000 input
tokens, so it *must* window at 2,000 tokens. The local model has its real 128k
context and would accept a whole field in one request -- but CPU inference is
linear in input length and slow, so a large field in one request runs for
minutes and trips the timeout. The local budget is therefore chosen from the
timeout, not imposed by the model, and windowing keeps large fields covered
rather than failing them closed.

It fails closed on adapter error, alongside `near-ai` rather than `sidecar`.
Running on loopback makes it more reliable, not less required.

| Variable | Required | Default | Notes |
|---|---|---|---|
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL` | R when `self-hosted` | (none, refuses boot) | Base URL of the loopback shim, including the `/v1` suffix, e.g. `http://127.0.0.1:8471/v1`. A configured backend with no endpoint refuses at startup rather than defaulting. |
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_MODEL` | optional | `openai/privacy-filter` | Passed through in the classify request. |
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_TOKENS` | optional | `4000` | Input tokens per request. **Not an upstream limit** -- the local model's context is 128k and would take a whole field. This is a *duration* budget: CPU inference runs at roughly 58 characters/second, so 4,000 tokens (~16 KB of prose) is about five minutes. Raise it only alongside the timeout; a budget that outruns the timeout fails every large field. |
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_CONCURRENT_WINDOWS` | optional | `3` | Windows for one field in flight at once. **Deliberately not 1**, unlike the hosted backend, whose limit is a scar from concurrency collapsing against NEAR AI's service -- a finding about that vendor, not about a local process. One request saturates at ~1.75 of 4 cores because this model's `hidden_size` is 640 and the matmuls are too small to scale across threads; the rest is only reachable by overlapping requests. Measured on the pilot under load: 2 gave 1.32x, 3 gave 1.58x. |
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS` | optional | `600000` | Per-window timeout, 10 minutes. Sized against the token budget above, with headroom for a slower host or a token-dense window. |
| `TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES` | optional | `MAX_TRACE_ENVELOPE_BYTES` | Resource bound on a whole field before windowing. Fields are split to fit the token budget, so this does not need lowering to bound request duration. |

No API key variable exists for this backend by design; the transport never
leaves the machine.

## 16. NEAR AI attestation drill

`POST /v1/admin/near-attestation-drill` reuses the gate's NEAR AI endpoint
credentials and adds two of its own. See
[`near-attestation-drill.md`](near-attestation-drill.md), and note the
`near-attestation-collateral` build feature below.

| Variable | Required | Default | Notes |
|---|---|---|---|
| `TRACE_COMMONS_NEAR_AI_BASE_URL` | R for the drill | (none) | Shared with the gate scorer. Missing, and the drill refuses with `missing_control:near_ai_base_url`. |
| `TRACE_COMMONS_NEAR_AI_MODEL` | R for the drill | (none) | Shared with the gate scorer. |
| `TRACE_COMMONS_NEAR_AI_API_KEY` | R for the drill | (none) | Shared with the gate scorer. Never logged; the drill's evidence carries no part of it. |
| `TRACE_COMMONS_NEAR_AI_SCORER_API_KEY` | optional | (none) | The scorer's own key. Set, the gate scorer and the drill's calls to `TRACE_COMMONS_NEAR_AI_BASE_URL` (report, completion, receipt) use it, and the registry fetch from the attestation gateway keeps `TRACE_COMMONS_NEAR_AI_API_KEY`. Unset, everything uses `TRACE_COMMONS_NEAR_AI_API_KEY` as before. Never logged. Required with the scorer production-host exception below. |
| `TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST` | `near-ai-staging` builds only | (unset) | `true` lets a staging ingest score against a NEAR AI production direct-completions host (`https://<model>.completions.near.ai`, no port or userinfo), with `TRACE_COMMONS_NEAR_AI_SCORER_API_KEY` set (staging design 3.9, owner exception 2026-10-24). Every other NEAR AI host is still checked as staging. Any other value, another scorer host, or a missing scorer key refuses boot with `missing_control:near_ai_scorer_production_host`; a default build refuses it at any value. |
| `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` | **R for the drill to mean anything** | (none) | Comma-separated `key=value` pins over `mrtd`, `rtmr0`..`rtmr3`, each 96 hex characters. Unset, the drill runs and **fails** with `missing_control:near_ai_expected_measurements` -- an unpinned drill proves the endpoint is *an* enclave, not *the* enclave. Take the values from a verified quote, never from the report's own `info.tcb_info` JSON. |
| `TRACE_COMMONS_NEAR_AI_PCCS_URL` | optional | `https://api.trustedservices.intel.com` | Where Intel DCAP collateral is fetched from. Defaults to Intel's own PCS rather than a caching mirror: the collateral is what a quote is verified against. |
| `TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS` | optional | `60` | Shared with the gate scorer; bounds each of the drill's four calls. |

## Account contribution admission (default off)

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED` | optional | `false` | Explicit replacement switch. Off retains legacy evidence behavior. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION` | R when enabled | (none) | Reviewed policy version allowed at startup. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON` | R when enabled | (none) | Explicit bounded allowance, cost bound, period, and `growth_rule`: `none` (flat), `external` (V114), or `tiered-v1` with a `growth` object (earned trust, live). |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS` | R when enabled | (none) | Positive processing lease, at most 86400 seconds. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_PROVIDER_SIGNERS` | See below | (none) | Comma-separated trusted provider-TEE signer keys. At least this or gateway signers must be configured. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_GATEWAY_SIGNERS` | See below | (none) | Comma-separated trusted gateway signer keys; optional when provider signers are configured. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_ACCEPTED_MODELS` | R with provider signers | (none) | Comma-separated accepted provider-TEE models. Not required for gateway-only evidence. |
| `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_MIN_REQUEST_BYTES` | R for independent evidence policy | (none) | Explicit positive minimum request-byte floor. |

If any `TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_*` variable is set, only that
namespace is used: a partial policy fails closed, without borrowing legacy values.
With none set, a complete legacy `TRACE_COMMONS_ADMISSION_*` evidence policy is
required instead. Evidence verification is required whenever account admission
is enabled; the legacy minimum-byte setting does not toggle it on or off.

With account admission enabled, the V59 evidence ledger's own policy is
required too: `TRACE_COMMONS_ADMISSION_ENABLED` with its limits
(`_WINDOW_ATTEMPTS`, `_ACCOUNT_COST_LIMIT`, `_GLOBAL_COST_LIMIT`,
`_PROCESSING_COST_BOUND`, `_LEASE_SECONDS`, `_CHALLENGE_TTL_SECONDS`) and
provider settings, which in turn need the witness bypass. The reason: an upload with verified evidence and no
`source_session` is admitted on that ledger, exactly as with account admission
off. Without it ingest refuses to start with
`account_admission_requires_legacy_evidence_policy`. An upload whose evidence
verifies and that carries `source_session` is admitted without debiting the
bounded allowance.

See [account trust](./account-trust.md) for the activation and response contract.

## Client daily upload allowances (retired)

The contributor clients have no daily upload limit, and
`GET /v1/account/upload-allowance` always answers `{"allowance":null}`.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY` | deprecated | (none) | No longer read. A deployment that still sets it starts normally and ignores it; remove it at the next configuration change. |
| `TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY` | deprecated | (none) | No longer read; as above. |
| `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON` | optional pair | (none) | Independent `tiered-v1` policy for the staging trust evaluator. Still validated at startup and still drives the scheduler's applied evaluations, but nothing reads them since client caps were retired; do not enable anew. Mutually exclusive with account-admission configuration; requires durable forced-RLS DB, external invalidation/read and tiered recorder/writer readiness. |
| `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_VERSION` | R with JSON | (none) | Must equal the independent policy's `version`; staging template uses `staging-earned-upload-v1`. |
| `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL` | optional | main DB pool | Dedicated non-owner trust worker/read connection, available to shadow and independent applied policies. Must name the same endpoint/database as the main DB and satisfy forced-RLS/readiness controls. Staging renderer sets the local `tc_upload_trust_login` URL from its feature-only secret; it leaves `DATABASE_URL` unchanged. |

The staging renderer accepts `TC_CLIENT_UPLOAD_TRUST_MODE=shadow|applied`
(default unset/disabled), plus required live decision labels
`TC_UPLOAD_TRUST_GATE_POLICY_VERSION` and
`TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION`. Each label is 1–64 ASCII
letters/digits or `._+-`; labels without a mode refuse. Only enabled modes
read `tc-staging-account-trust-worker-token` and
`tc-staging-db-upload-trust-password` (16 or more ASCII letters/digits).
The trust login is non-owner, NOSUPERUSER, NOBYPASSRLS and NOCREATEROLE;
grant only `trace_client_upload_trust_runtime`, which carries the reviewed
worker/status-read grants. No `trace_account_admission_runtime`, `app`, guard
or evaluator membership or direct writes. Apply migrations through V138; historical
credits must not be fabricated to qualify recorder history. Shadow uses the existing
shadow policy pair; applied uses the independent client-upload pair. Both
render the scoped scheduler at 300 seconds and limit 10000, leave both
admission switches off and add no witness settings. The renderer validates
JWT shape; ingest validates the scoped `account_trust_worker` token and DB
contracts. See [client allowances](client-upload-allowance.md) for
qualification, rotation and rollback.

## Earned account trust

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON` | optional | (none) | A candidate `tiered-v1` policy evaluated in `shadow`; never read by admission. Set with the version or not at all. |
| `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_VERSION` | R with the JSON | (none) | Must equal the candidate's `version`. |
| `TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED` | optional | `false` | Toggle. `true` without the token refuses startup. |
| `TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN` | R when enabled | (none) | Bearer the scheduler presents to the record and evaluate routes. Must authenticate as role `account_trust_worker` (or `admin`, not recommended); anything else refuses startup. Never logged. |
| `TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS` | optional | `21600` | Cadence, 60 to 86400. Must be shorter than every configured policy's `evaluation_max_age_seconds`, or startup refuses. |
| `TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT` | optional | `10000` | Sources per recorder pass, 1 to 10000. A tick makes at most 10 passes, then evaluates. |

Each scheduler tick records missing facts, then evaluates `applied` when the
admission policy or independent client-upload policy names `tiered-v1`, and
`shadow` when a shadow policy is set.
Startup refuses a scheduler with neither. Its liveness name is
`account_trust_scheduler`. See [account trust](./account-trust.md#earned-growth-live-tiered-v1).

## Legacy invite linking (default off)

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED` | optional | `false` | `true`/`1` opens `POST /v1/account/invites/legacy-link[/challenge]`; anything but `true`/`1`/`false`/`0` fails startup. On requires the `TRACE_COMMONS_INGEST_ATTESTATION_*` key, which countersigns link records. Mark pooled tenants first. |

See [legacy invite migration](./legacy-invite-migration.md).

## Payout rails (default off)

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE` | optional | `disabled` | `disabled`, `dry_run` or `live` for the NEAR AI payout rail. Anything else fails startup, as does `dry_run` or `live` with a rail setting missing. |
| `TRACE_COMMONS_PAYOUT_CONTROL_TOKEN` | R when any rail is `live` | (none) | The operator's payout control token, at least 32 characters; only its hash is kept. It is the only credential `POST /v1/admin/payout-rails/{rail_id}/halt` and `.../resume` and the hold list and clear routes accept; tenant tokens, a tenant admin's included, are refused. Shorter than 32 characters fails startup. Unset fails startup when any rail is `live` (`payout_control_token_unconfigured`), because the kill switch would be unreachable. A `dry_run` rail sends nothing and starts without it, logging the same label; the controls then answer 503 `payout_control_unconfigured`. Rotate by env reload and restart. |

See the [payout design](../superpowers/specs/2026-10-21-nearai-credit-payout-design.md).

## Referral rewards (default off)

Unset means off: with no `TRACE_COMMONS_REFERRAL_PROGRAM_START` no code can
be made or applied, and with `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE` unset or
`disabled` the bonus pass and the referral payout stream do not run. A value
that is present but malformed or out of range (an empty one included) fails
startup with the setting's name and a label, never the value; nothing falls
back to a default. Referrals count as configured when a program start is set
or the mode is not `disabled`; then startup also requires the network mark
key and a login that can assume `trace_referral_resolver`
(`referral_resolver_unprovisioned`, see
[referral-resolver role](./referral-resolver-role.md)). On staging,
`deploy/staging-gcp/render-env.sh` writes these from `TC_REFERRAL_*` and
`TC_NEAR_AI_REFERRAL_*`.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_REFERRAL_PROGRAM_START` | optional | (unset: not started) | RFC 3339. The program accepts codes from this time. Malformed fails startup (`referral_config_invalid`). |
| `TRACE_COMMONS_REFERRAL_PROGRAM_END` | optional | (none) | RFC 3339, after the start. |
| `TRACE_COMMONS_REFERRAL_LATE_DAYS` | optional | `30` | How long after its creation an account may still apply a code, 1 to 365. |
| `TRACE_COMMONS_REFERRAL_EARN_DAYS` | optional | `92` | How long after a referee's account creation the referrer earns, 92 to 3650. Under 92 fails startup (`referral_earn_days_below_promise`). Stamped on each referral when applied; a referral earns until the later of that and the current setting, so raising it extends live referrals and lowering it never shortens one. |
| `TRACE_COMMONS_REFERRAL_BONUS_BPS` | optional | `1000` | The referrer's bonus in basis points of each referee accrual, 1 to 10000. |
| `TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY` | R when referrals are configured | (none) | Base64 of at least 32 bytes (`openssl rand -base64 32`), the HMAC key for network marks; never reaches the database. Missing fails startup (`referral_network_mark_key_missing`), malformed (`referral_network_mark_key_invalid`). Never logged. |
| `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE` | optional | `disabled` | `disabled`, `dry_run` or `live` for the referral payout stream. Anything else fails startup (`near_ai_referral_mode_invalid`). `dry_run` and `live` need `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE` on, and `live` needs it `live` (`near_ai_referral_config_missing`). `dry_run` writes bonus rows and sends nothing. `live` sends with the payout credential unless the two settings below name a dedicated one. Shared, both streams have one lifetime budget and one send window, so referral sends draw down the contributor payout budget. |
| `TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL` | optional, with its id | (the payout credential) | A dedicated NEAR AI org credential (`oc_` and 64 hex) for the referral stream, so a NEAR AI-side cap on it bounds referrals alone. Unset with its id, a `live` stream sends with the payout credential; set to the payout credential with the payout id, the same. Set without its id, or half of the payout pair beside half of another, fails startup (`near_ai_referral_config_missing`). Never logged. |
| `TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL_ID` | optional, with the credential | (the payout credential's) | The credential's UUID. Set without the credential fails startup. |
| `TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO` | optional | `250000000000` | Per-account referral ceiling in nano-USD, above zero; owed referral credit over it is held. |
| `TRACE_COMMONS_REFERRAL_ABUSE_MIN_REFEREES` | optional | `5` | A code is scored once it has this many referees, 1 to 100000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_REFUSED_SHARE_BPS` | optional | `4000` | Refused-referee share that suspends a code, 0 to 10000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_REFUSED_MIN_SUBMISSIONS` | optional | `5` | Submissions before a referee counts toward the refused share, 1 to 100000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_CREDIT_HOLD_SHARE_BPS` | optional | `2000` | Credit-hold share, 0 to 10000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_ADVANCE_FORFEIT_SHARE_BPS` | optional | `2000` | Advance-forfeit share, 0 to 10000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_SHARED_IDENTITY_CLUSTER` | optional | `3` | Referees in one identity cluster, 2 to 100000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_DUPLICATE_SHARE_BPS` | optional | `2000` | Duplicate-trace share, 0 to 10000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_BURST_COUNT` | optional | `20` | More referees than this in 24 hours trips, 1 to 100000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_NEVER_SETTLES_SHARE_BPS` | optional | `6000` | Never-settles share, 0 to 10000. |
| `TRACE_COMMONS_REFERRAL_ABUSE_NEVER_SETTLES_DAYS` | optional | `30` | Days after joining before a referee can count as never settling, 1 to 3650. |
| `TRACE_COMMONS_REFERRAL_ABUSE_TRIP_CODES` | optional | `2` | More codes suspended than this in 24 hours switches the stream to `dry_run`, 1 to 100000. |

The abuse thresholds fail startup with `referral_abuse_setting_invalid` when
set but not a plain decimal in range. See the
[referral rewards design](../superpowers/specs/2026-10-29-referral-rewards-design.md).

## Build-time features (Cargo)

These aren't env vars but they gate which envs even matter at runtime.

| Feature | Description |
|---|---|
| `gcs-client` | Compiles `google-cloud-storage`. Required for `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=gcs`. |
| `gcp-kms` | Compiles `google-cloud-kms`. Required for `TRACE_COMMONS_KEK_PROVIDER=gcp_kms`. |
| `local-gpu-models` | Compiles the mistralrs perplexity scorer + fastembed embedder + usearch vector index. Required for `TRACE_COMMONS_GATE_SERVICE=enclave_local_gpu`. A2.3 migrated the scorer off candle-direct; architecture is auto-detected from `config.json`. |
| `local-gpu-models-cuda` | Implies `local-gpu-models`, adds the mistralrs CUDA backend. Required when `TRACE_COMMONS_PERPLEXITY_DEVICE=cuda*`. |
| `near-attestation-collateral` | Compiles the Intel DCAP collateral client (`dcap-qvl/report`). Required for the collateral fetch in `POST /v1/admin/near-attestation-drill`; without it that drill's `quote_verified` step refuses with `missing_control:near_ai_attestation_collateral_client`. Off by default because it pulls a second async HTTP stack (reqwest 0.13) into the build. |
| `near-ai-privacy-filter` | Compiles the NEAR AI Cloud privacy-filter backend. Required when `TRACE_PRIVACY_FILTER_BACKEND=near-ai`. Pilot builds enable this feature. |
| `self-hosted-privacy-filter` | Compiles the loopback privacy-filter backend. Required when `TRACE_PRIVACY_FILTER_BACKEND=self-hosted`. Pilot builds enable this feature. |

Production build command:

```sh
cargo build --release -p trace-commons-server -p trace-commons-ingest \
  --features gcs-client,gcp-kms,local-gpu-models-cuda
```
