# Admission evidence and durable processing ledger

This implementation adds a distinct admission signature profile and a default-disabled processing ledger. It does not provision provider credit, grant permanent membership, or enable production. Ordinary invited submissions retain redaction v1; v1 alone never establishes attested admission.

## Capture and signatures

The authenticated challenge endpoint resolves the provisioned account anchor from the device principal. It creates an unpredictable 32-byte nonce and persists only the digest of this canonical binding, the anchor, and expiry:

```json
{"metadata":{"trace_commons_admission":"tcad1:<anchor-lower-hex64>:<nonce-lower-hex64>:<unix-expiry>"}}
```

This is a string in OpenAI request metadata. It must be inserted into the **final upstream request body** before dispatch. The provider receipt must verify over those exact request bytes and the exact upstream response bytes. The witness chooses the last declared `HttpExchange`, rejects restarted streams and missing/bounded bodies, and independently requires a provider signer from operator-configured trust. A mathematically valid receipt under a self-reported key is insufficient.

`POST /v1/witness/admission` accepts the existing structured contribution request. Its response carries the ordinary artifact and redaction v1 headers plus `x-trace-admission-evidence` and `x-trace-admission-signature`. The additional signature uses the same pinned witness signer and length-delimited `trace_commons_admission_evidence.v1` domain. It binds the account, challenge digest, trusted provider signer, request/response hashes, canonical receipt identity, returned artifact hash, witness measurement, redaction policy, and validity period. Ingest must verify both signatures and its policy allowlist. The schema denies unknown fields and noncanonical hashes. No raw content appears in this evidence.

Receipt identity hashes the decoded signer address and request/response digests, independent of signature spelling or recovery-byte representation. A different wire spelling cannot replenish admission. Expired evidence or mismatched account/artifact cannot fall back to a window attempt.

## Processing state

V59 stores tenant-scoped challenges, account ceilings and immutable submission identities. A narrow `NOLOGIN NOBYPASSRLS` function owner accesses a global receipt-hash set and singleton aggregate ceiling. The global receipt table contains no tenant/account identifier. All tables use forced RLS; reservation functions verify the caller's tenant context and use a fixed qualified search path.

A reservation atomically locks the global ceiling, account and submission, checks budget/window availability, consumes an unused unexpired account-bound challenge and globally unique receipt when attested, and reserves one processing cost bound. SQL transactions roll back partial state on errors. Window submissions consume a window slot; attested submissions still consume account/global processing bounds.

| Transition | Durable effect |
| --- | --- |
| new → reserved | Hold one configured cost bound and a window slot when applicable; fix body/account/evidence identity. |
| reserved → released | Refund latest cost bound and an unused window slot, only before processing. Keep immutable identity for retry. |
| reserved → processing | Mark that work started; the window slot can no longer be refunded. |
| processing → completed | Terminal marker after required durable ingest writes. |
| expired reserved/processing → reserved | Retain prior conservative cost bounds, reserve another bound, reuse the same already-consumed window slot. |
| completed → retry | Authenticate the same owning principal and match stored account/body/submission; return the existing receipt even when original evidence has expired, with no new work or charge. |

Expiry is not evidence that spending did not occur. Costs remain conservative configured bounds, not inferred provider prices, USD conversions, or measured settlement. Quality failures consume the attempt and retain the bound. No automated budget reset or post-processing refund is invented. Account/global configurations are fixed on first reservation and mismatched runtime configuration refuses until an explicit operator migration/reconciliation policy exists.

The handler holds a per-submission PostgreSQL session advisory lock across processing. Acquisition detaches the connection from the pool immediately; it cannot occupy all pool slots while waiting for a second ledger connection. Dropping/cancelling the guard closes its session and releases the lock. Hash collisions only serialize unrelated work. Existing request concurrency/rate limits bound simultaneous detached sessions. Lease expiry cannot permit a second live processor while the first guard survives. Loss of the database connection must remain a launch-review concern: an advisory lock alone cannot fence an external operation already running during a network partition; conservative retained bounds and idempotent downstream work are required.

## Configuration and role grants

Nothing enables admission by default. `TRACE_COMMONS_ADMISSION_ENABLED=true` requires all of `TRACE_COMMONS_ADMISSION_WINDOW_ATTEMPTS`, `TRACE_COMMONS_ADMISSION_ACCOUNT_COST_LIMIT`, `TRACE_COMMONS_ADMISSION_GLOBAL_COST_LIMIT`, `TRACE_COMMONS_ADMISSION_PROCESSING_COST_BOUND`, `TRACE_COMMONS_ADMISSION_LEASE_SECONDS`, and `TRACE_COMMONS_ADMISSION_CHALLENGE_TTL_SECONDS`. These have no economic defaults. Attempts may be zero; costs must be positive; lease/challenge duration must be in 1–86400 seconds. Processing bounds must conservatively cover the configured pipeline before enabling it.

Witness trust uses `TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS`, a comma-separated nonempty set of verified provider signing addresses. Ingest separately freezes `TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS` and its witness pin/measurement/policy allowlist. Missing trust, durable DB requirements or identity mapping must fail closed. Key rotation needs attestation-backed operational review; merely copying a receipt's address is unsafe.

V59 grants function execution to its migration role. A separate runtime role must explicitly receive:

```sql
GRANT USAGE ON SCHEMA public TO runtime_role;
GRANT SELECT, INSERT, UPDATE ON trace_admission_challenges,
  trace_admission_accounts, trace_admission_submissions TO runtime_role;
GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,
  BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),
  trace_transition_admission(TEXT,UUID,UUID,TEXT) TO runtime_role;
```

Do not grant runtime membership in `trace_admission_guard`, access to the global receipt/budget tables, `BYPASSRLS`, or ownership. Enabled startup calls `admission_runtime_ready` on the actual runtime connection. It requires non-owner/NOBYPASSRLS, no guard membership or direct global-table privileges, forced RLS, tenant-table privileges and function EXECUTE. Missing permission refuses startup (`admission_runtime_permissions_not_ready` or `admission_database_unavailable`), never a legacy authorization fallback. Operators must also inspect active mappings and configured limits. Existing V58 mapping and normal trace storage permissions are additionally required.

## External capture gap

Read-only inspection of `/Users/zakimanian/code/ironwire` found the exact insertion boundary in `crates/ironwire_proxy/src/pipeline.rs`: request translation/model substitution at lines 388–417, then `Capture::of_request(request.body.clone())` at 419–425, then `backend.send(request)` at 427. Insert only after translation/model substitution and before capture. Preserve the provider request/response bytes and existing receipt association, and refuse admission capture on unsupported/translated metadata shapes rather than silently drop binding. The native challenge must be acquired before dispatch and associated with that particular capture, including retries and expiry. This external repository has not been modified; synthetic tests do not establish operational native inference readiness.

## Reproducible local checks

Use `RUSTFLAGS='-D warnings' CARGO_TARGET_DIR=/tmp/trace-commons-inference-funding-target` for all Cargo commands. No live provider credentials or inference are used.

- `cargo check -p trace-commons-server --bin trace-commons-witness --locked --offline` passed after service/adapter integration.
- `cargo test -p trace-commons-server --test admission_ledger_pg --locked --offline -- --ignored` uses only explicit `TRACE_COMMONS_ADMISSION_PG_TEST_URL`, a localhost database whose name starts with `admission_test`. The fixture resets only its dedicated V59 tables. It creates a separate non-owner runtime role and exercises actual PostgreSQL concurrency, replay, budget, refunds and RLS. Missing/broken explicit DB fails; no implicit skip.
- `TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL=postgresql://<local-test-admin>@127.0.0.1:55439/admission_test_ingest cargo test -p trace-commons-server --bin trace-commons-ingest actual_postgres_challenge_witness --locked --offline -- --ignored` passed: full migrations, non-owner runtime readiness, real provisioned mapping lookup, challenge, synthetic provider receipt through structured witness, actual router/durable ingest, window exhaustion, expired terminal retry and unchanged cost. The account mapping is a labeled synthetic SQL fixture; B separately tests signed wallet/device provisioning.
- The synthetic witness test `admission_evidence_binds_trusted_final_call_account_and_witness_artifact` covers a real signed provider fixture through the structured witness and both artifact/evidence verifiers. Root's ingest tests provide the route-level continuation.

Production enablement remains separate from passing local tests, including provider signer attestation, final-byte IronWire capture, configured cost ceilings, runtime grants, recovery policy and funding capabilities.
