# `TRACE_COMMONS_*` Environment Variable Reference

Every env var the `trace-commons-ingest` binary reads, grouped by surface. The
master list is `grep '^const TRACE_COMMONS_' crates/trace-commons-server/src/bin/trace-commons-ingest.rs`.
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
| `TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS` | R when enclave_local_gpu | (none) | Inclusive lower bound on novelty score (micros). Pilot-launch recommendation: `500000` (cosine novelty 0.5); primary active floor at launch under A2.5. |
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
| `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS` | optional | `false` | When `true`, accepts medium residual-risk submissions after server-side re-scrub. High residual-risk submissions still quarantine. Intended for tightly scoped pilots where message text/tool payloads are included. |
| `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` | optional | (none, driver fails closed) | Separate connection string for the narrow `trace_gate_driver` role pool used by the perplexity-scoring driver (Task 5/6). Mirrors `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`'s shape: points at a LOGIN role granted membership in `trace_gate_driver`. See [`perplexity-scoring-driver.md`](perplexity-scoring-driver.md). |
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

## 9. Analytics surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` | optional | — | k-anon floor. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_KEY` | optional | — | Per-release noise key. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_MAX_DELTA` | optional | — | Max noise delta in micros. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_EPSILON_MICROS` | optional | — | Configured epsilon. |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_MAX_EPSILON_MICROS` | optional | — | Refuse epsilons above this. |
| `TRACE_COMMONS_COMMUNITY_LEADERBOARD_ENABLED` | optional | `false` | Enables `/v1/community/*` profile, leaderboard, contributor, and analytics snapshot routes. Requires the DB mirror for real data. |
| `TRACE_COMMONS_COMMUNITY_TENANT_IDS` | optional | — | Comma-separated tenant ids included in community snapshot recompute. Set this when the runtime DB role is non-bypassing and forced RLS would hide `trace_tenants` enumeration. |
| `TRACE_COMMONS_COMMUNITY_ANALYTICS_PUBLICATION_BASIS` | optional | `approved_noise_mechanism` | What published corpus aggregates rest on. `approved_noise_mechanism` (default) withholds analytics until a calibrated mechanism is approved. `suppression_only` publishes under cell suppression alone — **no noise is applied and totals are not suppressed**, so at small corpus sizes the totals can still describe a handful of contributors closely. The min-cell floor is required either way and is never waived. Any user-facing description of the deployment must state which basis is in force. |
| `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS` | optional | — | Recompute the community snapshot in-process on this interval. Unset, empty or `0` leaves recompute admin-triggered only. Minimum 60s. **Assumes a single writer**: the worker skips a tick when a snapshot is already fresh, but that is coordination, not mutual exclusion. On a multi-replica deployment either leave this unset and drive recompute from one elected scheduler, or accept duplicate aggregation work. Retention keeps the 96 most recent snapshots per window/metric. Without it the published leaderboard is only as fresh as the last manual POST to `/v1/admin/community/snapshots/recompute`. |
| `TRACE_COMMONS_COMMUNITY_CORS_ORIGINS` | optional | `https://tracecommons.ai,http://localhost:8788,http://127.0.0.1:8788` | Comma-separated browser origins allowed to call `/v1/community/*`. Keep this to the public Pages site plus local preview origins. |

## 10. Credit / NEAR surface

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | R for live | `0` | Per-pass credit points. Set to `0` during calibration; flip to configured value at cutover. |
| `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` | R for live | `false` | Refuse to emit novelty-utility credit unless gate is `enclave_local_gpu`. |
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
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN` | R when enabled | — | Scheduler bearer token. |
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
| `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` | optional | `false` | Toggle. Off by default; existing deployments and CI are unaffected until an operator opts in. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_INTERVAL_SECONDS` | optional | `45` | Cadence between enumeration batches. Clamped to `[5, 86400]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_BATCH_SIZE` | optional | `5` | Submissions enumerated per tick. Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS` | optional | `5` | Bounded attempt counter per submission before the driver stops retrying it. Clamped to `[1, 1000]`. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES` | optional | `true` | Skip-duplicate cache cost control. Falsy values (`0`, `false`, `no`, `off`) disable it. |
| `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS` | optional | `900000` | Novelty-score threshold (micros) above which a submission is treated as a cache-cost duplicate and skipped. Clamped to `[0, 1000000]`. |
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
weaker path.

The `near-ai` backend requires the `near-ai-privacy-filter` Cargo feature;
pilot builds enable it. When `near-ai` is active, the pipeline-version suffix
appended to envelope records is `+privacy-filter-near-ai-v1`; this value is
audit-relevant and is included in gate version hash derivation.

| Var | R? | Default | Description |
|---|---|---|---|
| `TRACE_PRIVACY_FILTER_BACKEND` | optional | unset | `sidecar` \| `near-ai` \| unset. Unset = deterministic-only redaction. Unknown values refuse startup. |
| `TRACE_PRIVACY_FILTER_COMMAND` | when `sidecar` | (none) | Path to sidecar binary. Legacy name `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` still read with a one-shot deprecation warning. |
| `TRACE_PRIVACY_FILTER_ARGS` | optional | empty | Whitespace-separated argv. Legacy: `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. |
| `TRACE_PRIVACY_FILTER_TIMEOUT_MS` | optional | `10000` | Sidecar timeout. Legacy: `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS`. |
| `TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES` | optional | (sidecar default) | Max stdin bytes per call. Legacy: `IRONCLAW_*`. |
| `TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES` | optional | (sidecar default) | Max stdout bytes. Legacy: `IRONCLAW_*`. |
| `TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` | optional | (sidecar default) | Max stderr bytes. Legacy: `IRONCLAW_*`. |
| `TRACE_NEAR_AI_PRIVACY_API_KEY` | when `near-ai` | (none) | NEAR AI Cloud bearer token. Never logged; rotation is restart-only. |
| `TRACE_NEAR_AI_PRIVACY_BASE_URL` | optional | `https://cloud-api.near.ai/v1` | Hosted endpoint; supports `privacy-filter.completions.near.ai/v1` faster path. |
| `TRACE_NEAR_AI_PRIVACY_MODEL` | optional | `openai/privacy-filter` | Model slug. |
| `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` | optional | `10000` | HTTP request timeout. |
| `TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES` | optional | (sidecar default) | Refuses inputs above this size. |

---

## Build-time features (Cargo)

These aren't env vars but they gate which envs even matter at runtime.

| Feature | Description |
|---|---|
| `gcs-client` | Compiles `google-cloud-storage`. Required for `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=gcs`. |
| `gcp-kms` | Compiles `google-cloud-kms`. Required for `TRACE_COMMONS_KEK_PROVIDER=gcp_kms`. |
| `local-gpu-models` | Compiles the mistralrs perplexity scorer + fastembed embedder + usearch vector index. Required for `TRACE_COMMONS_GATE_SERVICE=enclave_local_gpu`. A2.3 migrated the scorer off candle-direct; architecture is auto-detected from `config.json`. |
| `local-gpu-models-cuda` | Implies `local-gpu-models`, adds the mistralrs CUDA backend. Required when `TRACE_COMMONS_PERPLEXITY_DEVICE=cuda*`. |
| `near-ai-privacy-filter` | Compiles the NEAR AI Cloud privacy-filter backend. Required when `TRACE_PRIVACY_FILTER_BACKEND=near-ai`. Pilot builds enable this feature. |

Production build command:

```sh
cargo build --release -p trace-commons-server \
  --features gcs-client,gcp-kms,local-gpu-models-cuda
```
