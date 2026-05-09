# Trace Commons Roadmap

This roadmap coordinates the multiphase path from the current Trace Commons MVP to a production-ready private corpus. It is a planning companion to `docs/internal/trace-commons.md` and `docs/internal/trace-commons-storage.md`; those documents remain authoritative for the envelope rules, threat model, API surface, storage schema, and migration details.

## Production Gap Queue

These are the next independent production slices that can be staffed in parallel. Each slice should land behind flags where it changes serving behavior and must add caller-level tenant-scope tests before promotion.

### Auth and Keying

- [x] Add EdDSA/Ed25519 signed upload-claim verification through default or `kid`-selected public-key config, JSON/file keysets with optional `not_before`/`not_after` activation windows, safe config-status total/EdDSA active/inactive key counts, issuer/audience/JTI/TTL policy, unsupported-algorithm rejection, and an EdDSA-required production gate that rejects static tokens and HS256 signed claims on authenticated routes.
- [x] Add issuer-managed EdDSA/Ed25519 upload-claim enforcement that accepts only active `kid`-selected keys from the managed JSON/file keyset, requires issuer and audience checks, and rejects static tokens, HS256 claims, default EdDSA keys, ad hoc keyed public-key files, missing `kid`, and unmanaged `kid` values.
- [x] Add guarded remote issuer-managed EdDSA/Ed25519 keyset bootstrap with HTTPS-only fetch, exact host allowlist, no redirects, DNS/private-target validation, DNS pinning, optional bearer fetch credential, timeout, and body-size cap.
- [x] Add autonomous client upload-claim refresh for opted-in queues: scoped policies can point at a guarded HTTPS issuer, request short-lived EdDSA/Ed25519 bearer claims with tenant/audience/consent/use metadata, keep workload credentials in env vars, cache claims only in memory until the refresh margin, and retry submit/status-sync once after 401/403 with a forced refresh.
- [x] Add a standalone EdDSA/Ed25519-only upload-claim issuer MVP with `/v1/trace-upload-claim`, `/health`, and a Trace Commons keyset endpoint that publishes `kid` plus `public_key_pem` in the ingest-compatible keyset shape. The issuer authenticates workload JWTs with EdDSA only, enforces issuer/audience/expiry plus workload scope/use allow-lists, can optionally connect to the configured PostgreSQL DB and require contributor tenant-access grants with issuer/audience/subject binding plus scope/use narrowing, signs short-lived contributor upload claims with `kid`, `iss`, `aud`, `iat`, `exp`, and `jti`, and rejects RSA material by construction.
- [x] Finish live issuer-managed EdDSA/Ed25519 key refresh/sync so long-running deployments can rotate issuer-owned key records after startup without restart: URL keysets refresh in-process on a bounded interval, failures preserve the last good active keyset, managed-required deployments can configure a max-stale fail-closed window, and config-status exposes safe refresh health without URLs, hosts, key ids, PEMs, or bearer fetch credentials.
- [x] Add durable tenant access grant storage for issuer-authorized hosted-agent principals, roles, consent scopes, allowed uses, issuer/audience/subject attribution, expiry, revocation metadata, and safe metadata across PostgreSQL, plus admin create/list/revoke APIs, CLI helpers, and enforcement gates for trace submission, autonomous contributor credit/status readback, reviewer/export/worker/audit/admin read surfaces that require an active exact-role grant, bind signed EdDSA/Ed25519 grants to configured issuer/audience/JWT `sub`, and intersect grant scopes/uses with static or EdDSA claim allow-lists.
- [ ] Promote auth-derived `TenantCtx` into every ingest, review, export, worker, maintenance, and contributor-status path so envelope tenant fields remain attribution only. First guards now fail closed when file-backed metadata, stored envelope bodies with a conflicting server tenant ref, derived rows, credit ledger rows, audit rows, revocation tombstones, replay export manifests, export provenance, or benchmark artifacts are read from an authenticated tenant directory but contain a different embedded tenant id/storage ref; service-local object-ref reads/deletes also verify the tenant key ref, stored envelope body tenant scope when it uses a server tenant ref, encrypted benchmark artifact reads verify the decrypted body tenant, and vector payload deletes verify the encrypted payload body's tenant storage ref before physical deletion. File-backed compatibility writes now also reject embedded tenant drift before persisting credit, settlement, hold, outbox, ranking evidence/calibration, audit, replay manifest, benchmark artifact, or benchmark/ranker provenance rows.
- [ ] Harden PostgreSQL tenant isolation with production roles, transaction-local tenant context coverage, and same-id cross-tenant tests; server migrations now force RLS on Trace Commons tables and centralize every tenant policy behind `trace_current_tenant_id()`, Admin config-status exposes safe PostgreSQL RLS readiness diagnostics for policy coverage, expression drift, disabled RLS, force-RLS count, transaction-local tenant-context probing, role bypass state, and table-owner runtime-role state, operational-summary promotion gates now block on unsafe PostgreSQL RLS diagnostics without exposing table names, a migration-registry regression test keeps diagnostics/policies/FORCE-RLS coverage aligned, and raw PostgreSQL RLS tests now cover tenant rows, tenant policies, tenant grants, same-id submission/object/derived/vector/export rows, audit rows, credit and NEAR settlement control-plane rows, ranking/benchmark control-plane rows, tombstones, retention jobs/items, and revocation propagation rows when the test runtime role is RLS-safe.

### Autonomous Client

- [ ] Turn opted-in post-turn capture into a durable background worker with retry/backoff, idempotency, and network-offline handling. Current runtime capture already skips ineligible current traces instead of leaving new held files, and the agent runtime now also runs a periodic scoped queue flush worker with typed retry/backoff sidecars plus durable scoped flush/status-sync telemetry. Queue writes now use atomic temp-file replacement, queue flush compacts duplicate queued contribution envelopes and orphan held sidecars before submission, malformed queue files are quarantined locally instead of blocking later valid uploads, short-lived EdDSA upload claims can refresh from a guarded issuer, and current telemetry splits sanitized network failures into offline, DNS, timeout, connection-refused, connection-reset/aborted, and generic network buckets from typed provider source chains before string fallback. Remaining work is broader provider-specific request-boundary classification beyond current `reqwest` and `std::io` sources.
- [x] Add periodic contributor credit notices across CLI/web/runtime that summarize accepted, quarantined, rejected, revoked, pending/final credit, delayed-credit deltas, and credit-event counts without exposing trace bodies or central corpus rows. Current notices are backed by a scoped local retry outbox, can be acknowledged until the local credit fingerprint changes, or snoozed for a bounded period through CLI and authenticated web actions.
- [x] Add first local queue diagnostics/status surfaces for CLI and authenticated web: policy readiness, bearer-token environment presence, queued/held counts, sanitized held-reason counts, and local credit summaries where available.
- [x] Extend autonomous diagnostics with policy/version mismatch promotion guidance. Durable last-attempt/success/failure flush telemetry, retryable submission failure counters, status-sync counters, retry/backoff state, last compaction reclaimed count, duplicate-envelope and orphan-hold-sidecar compaction counters, aggregated schema-version/consent-policy/redaction-pipeline/trace-card-redaction-pipeline/malformed-envelope warnings, warning severity, production-promotion blocking flags, safe recommended actions, and sanitized Endpoint/Credential/Network/NetworkOffline/NetworkDns/NetworkTimeout/NetworkConnectionRefused/HttpRejection/Policy/Queue/StatusSync/Submission/Unknown failure classes are now exposed locally through scoped queue diagnostics. Warning aggregates do not include raw envelope bodies or raw observed mismatch values.

### Ingestion Storage

- [ ] Promote DB/object-primary submit, review, replay, benchmark, and ranker paths from pilot flags to per-tenant rollout flags after reconciliation parity is green. Initial tenant allowlist gates now wrap DB contributor/reviewer/replay/audit/tenant-policy reads, object-ref-required modes, and object-primary submit/replay/derived flags; DB contributor credit/status, DB reviewer metadata, DB replay-export selection, DB tenant-policy, DB audit-read, object-primary submit/review, object-primary replay export, and benchmark/ranker derived export caller tests cover tenant A canary rollout while tenant B remains on file fallback.
- [ ] Replace service-local encrypted artifact storage with a service-owned object-store provider abstraction, KMS/key-ref strategy, tenant-hashed object keys, hash/decrypt verification, and migration/backfill tooling. The local encrypted store now implements a `TraceArtifactStore` provider trait for serialized JSON write/read/delete conformance; remote object/KMS providers and migration tooling remain.
- [ ] Add PostgreSQL integration coverage covering the `TraceCorpusStore` slices for submissions, object refs, derived records, vectors, audit, credit, retention jobs, export manifests/items, policies, and tombstones. Retention job/item facade scope, tenant-policy update/scope behavior, review-lease claim/release/audit behavior, raw RLS visibility, and export-manifest mirror rollback atomicity now have PostgreSQL coverage.

### Review and Governance

- [ ] Extend review leases into a fuller assignment/escalation workflow with SLA filters, privacy-review reasons, and central reviewer routing. Reviewer/admin `POST /v1/review/leases/claim-next`, `POST /v1/review/leases/claim-batch`, `ironclaw traces review-lease-claim-next`, and `ironclaw traces review-lease-claim-batch` now add safe tenant-scoped next/batch claim slices for available quarantined traces, using review escalation/SLA ordering before writing DB-backed review lease state and typed audited claim rows. Review finalization now rejects non-quarantined, terminal, expired, and aggregate-only approval records before trace content is read, while PostgreSQL facade tests cover tenant-scoped lease claim/release ownership and typed lease audit metadata.
- [ ] Complete remaining privileged-action ABAC for review override, purge, and tombstone changes using tenant policy plus signed-claim allowed scopes/uses. Manual reviewer/admin delayed utility credit now enforces source allowed-use, tenant-policy, and signed-claim ABAC like worker utility credit. Review decisions now check signed-claim/grant/tenant-policy source scope before trace-body reads, destructive purge preflights selected sources before deletion, and reviewer/admin revocation tombstones are blocked outside source scope while owner self-revocation remains available. Privileged revocation through the body route now requires a non-empty operator reason after source ABAC passes and persists it into file/DB tombstones plus file/DB revoke audit rows.
- [ ] Make audit append/read paths production-grade: DB-primary hash-chain verification, per-source content-read rows, reason enforcement, sampled reconciliation, and no broad corpus download path. Maintenance reconciliation now projects DB audit hash-chain drift into `db_audit_hash_chain_failures`, DB canonical audit-payload drift into `db_audit_canonical_projection_failures`, submitted-audit privacy-risk metadata drift into `db_audit_submission_metadata_mismatches`, and all three into promotion-blocking `blocking_gaps`, while audit reader parity records a safe error hash instead of hiding projection drift behind an unstructured DB read failure. Reconciliation also compares a bounded latest audit-reader sample from files and DB and reports `audit_reader_sample_parity` plus hashed diagnostics when reader-visible projections drift despite matching ids/counts. Reviewer audit-event reads now serve only the requested latest bounded page from the file audit tail or a PostgreSQL `audit_sequence DESC LIMIT` query instead of decoding every tenant audit row before applying `limit`, and they recompute returned row hashes when chain fields are present before serving the page. Aggregate read, trace-content-read, submitted-audit, and revocation audit rows now fail closed or mirror typed safe metadata from code-owned reason fields or matching submission records at the DB mirror/backfill boundary, and canonical reconciliation catches trace-content-read metadata drift, revocation reason-hash drift, plus submitted-audit privacy-risk drift even when the raw audit projection still matches. Contributor/status, reviewer queue/list/audit, replay manifest, tenant policy/grant, export, retention, vector, credit settlement/hold/attestation/outbox, benchmark registry, and ranking evidence/report/admin list reads now use the same aggregate read-audit metadata path.

### Datasets

- [ ] Replace deterministic vector similarity with private vector infrastructure that reads only approved redacted projections, writes tenant-scoped vector metadata, validates private vector-search results against active DB rows, and invalidates entries on revocation/retention. Private embedder/search adapters now exist, operational readiness now blocks stale or cross-profile nearest-neighbor metadata, and deployments can fail closed when the private embedder or search adapter is required but missing; remaining work is deployed vector-store operations and rollout hardening.
- [ ] Promote benchmark conversion and ranker training exports into durable worker jobs with source-list hashes, artifact object refs, lifecycle state, replayability checks, and idempotent delayed utility credit. Export call sites now mirror one-shot durable grant rows and running/complete job rows with safe request metadata for requested/effective limits, status/privacy/consent filters, and hashed external refs; export workers can atomically claim the oldest unexpired queued job for a dataset kind, execute queued replay, benchmark-conversion, ranker-candidate, and ranker-pair exports from that safe metadata while terminalizing the claimed row, drain bounded queued batches across all supported dataset kinds while continuing after per-job failures, admin-retry failed unexpired replayable jobs back to `queued` with hash-only retry metadata, worker-retry due failed jobs with retry-count caps plus exponential-delay backoff, and optionally run that retry/drain loop in-process from a configured export-worker bearer token. Remaining work is remote artifact storage and broader rollout hardening rather than basic queued execution.
- [ ] Add export governance for replay, benchmark, ranker, and training slices: explicit purpose, consent/use filters, item caps, source object refs, manifest invalidation, and time-limited controlled job access. Replay, benchmark, ranker-candidate, and ranker-pair exports now validate and persist tenant/principal/purpose/dataset-kind grant/job slices.

### Observability

- [ ] Add operational dashboards or API summaries for queue throughput, accept/quarantine/reject rates, redaction risk, review SLA, export volume, retention jobs, vector coverage, ranking model/credit readiness, analytics release readiness, and delayed credit settlement. A first admin-only `GET /v1/admin/operational-summary` API and `ironclaw traces operational-summary` CLI helper now return safe tenant-scoped aggregates for submission status/risk, review SLA pressure, DB export manifests/jobs, stale running export-job blockers, retention jobs, analytics broad-release min-cell/noise readiness, safe artifact object-store alias/readiness, vector coverage plus private embedder/searcher/scheduler readiness, PostgreSQL RLS promotion readiness, ranking active-model risk, candidate/active backtest failures, process-evaluator configured readiness, ranking label-adjudication issues, blocked ranking-credit settlement readiness, NEAR credit outbox pending/submitted/confirmed/failed counts with missing submitter/confirmer blockers, ranking worker-run skip totals and reason aggregates with promotion-gate warnings for actionable risk/ineligible skips, revocation-propagation latest checked/completed/failed/skipped/pending counts with skip warnings and failure blockers, object-primary object-store readiness blockers, stale or failed ranking worker-run blockers, delayed credit totals with explicit reversal counts/points, credit-settlement account-cap, configured NEAR contract, NEAR adapter plus required adapter-auth, central-issuer-approval blockers, and credit-settlement scheduler enabled state, and a `rollout_smoke` preflight block that names required canary checks plus recorded/passed/failed/stale/missing rehearsal evidence; the read audit row records safe promotion-gate counts, config-status now exposes safe broad-release analytics noise readiness/max delta plus optional epsilon publication accounting and settlement scheduler readiness without keys or scheduler secrets, and admin ranking reports now include candidate/active model backtests with calibration and pairwise reason codes, a label-adjudication report, and a hashed labeler-reliability report with label counts and issue-rate micros.
- [ ] Emit structured metrics/logs for every promotion gate: DB/file parity, object-ref readability/hash/key-ref failures, RLS/predicate denials, signed-claim failures, worker skips, and revoked-source invalidations. PostgreSQL RLS catalog readiness is now available through safe admin config-status diagnostics, unsafe RLS diagnostics surface as an operational-summary promotion blocker and per-gate metric using aggregate counts only, operational-summary promotion-gate blockers/warnings now emit both an aggregate structured warning log and one structured event per gate with safe tenant ref, severity, gate name, and count, NEAR credit and benchmark registry outbox workers plus the credit-settlement scheduler now emit safe completion/failure logs with tenant refs, row/item counts, hashed errors only, and no raw scheduler config, and core DB dual-write, export scheduler, evaluator adapter, vector payload, backfill, and generic internal-error logs now emit stable error/reason hashes instead of raw error strings. `/v1/admin/operational-metrics` exports a safe Prometheus-text snapshot for promotion readiness, per-gate counts including object-primary object-store blockers, PostgreSQL RLS subcheck/gap gauges, artifact object-store readiness plus safe object-store alias, ranking model/backtest/adjudication/credit readiness gauges plus ranking reason-code counts, ranking evaluator readiness, ranking worker-run lifecycle/totals/pending-after/reason-count gauges, NEAR credit outbox state and adapter/auth-readiness gauges, credit-settlement account-cap, NEAR adapter/auth, central-issuer-approval, and scheduler-enabled gauges, actionable ranking worker skips, latest revocation-propagation worker failed/skipped counts, rollout-smoke recorded/passed/failed/stale/missing evidence plus finite per-check state gauges, submissions, review SLA, export/retention jobs, analytics release readiness, vector entries, vector-infrastructure readiness, benchmark lifecycle including failed registry outbox rows, delayed-credit event totals, and delayed-credit reversal-event totals; broader runtime metrics coverage beyond this admin exporter remains.
- [ ] Build runbooks and smoke checks for per-tenant rollout, rollback, key rotation, object-store migration, retention purge dry runs, and audit-chain verification. A first rehearsal-ready operator draft now lives in `docs/trace-commons-storage.md`, operational summary now exposes the required smoke-check names including `tenant_canary_isolation`, `db_reconciliation_clean`, `rollback_flag_drill`, `key_rotation_drill`, revocation propagation, retention dry-run, vector indexing, analytics release, ranking-model readiness, credit settlement, object-store migration, PostgreSQL RLS readiness, and audit-chain verification plus recorded/passed/failed/stale/missing rehearsal evidence, and admins can inspect or append hash-only rehearsal evidence through `GET`/`POST /v1/admin/rollout-smoke/evidence`, including `latest_only=true` for current per-check state. `GET /v1/admin/rollout-smoke/preflight` now gives operators one admin-only promotion-readiness read that combines the rollout-smoke gate summary with latest hash-only evidence per required check. Dedicated admin drills can now record `submit_status`, `tenant_canary_isolation`, `contributor_credit`, `reviewer_metadata`, `replay_export_selection`, `audit_reads`, `object_primary_reads`, `object_store_migration`, `ranking_model_readiness`, `db_reconciliation_clean`, `rollback_flag_drill`, `key_rotation_drill`, `revocation_propagation`, `delayed_credit_reversal`, `object_deletion_refs`, `retention_dry_run`, `vector_index`, `analytics_release`, `credit_settlement`, `postgres_rls_readiness`, and `audit_chain_verification` evidence without making operators synthesize those rows manually; the credit-settlement drill now blocks readiness on missing configured NEAR contract, missing NEAR submitter/confirmer adapters by default, and missing or unrecorded source-list-bound central issuer approval evidence when issuer approval is required, while the live rollout-smoke gate rejects settlement until all required checks have fresh passing evidence. Keep this unchecked until the smoke pass is rehearsed end to end.
- Centralized early issuer mode is now represented as explicit fail-closed controls rather than a broad actor trust model: `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE=true` validates DB mirror/RLS, managed EdDSA tokens, tenant grants, issuer approvals, caps, NEAR contract pinning, adapters, adapter auth, and rollout-smoke live-settlement gating at startup; drills expose safe missing-control names, operational metrics expose safe central-profile gauges, live settlement re-checks the profile and rollout-smoke readiness before writing batches or NEAR outbox rows, and credit-cycle preflight/worker paths stop before partial cycle side effects.

## Current Gecko-Pass Status

As of the server split, Trace Commons has moved beyond the local-only MVP into a server-owned dark-launch production-storage path:

- Local capture remains opt-in, local-first, and redaction-first. Raw recorded traces still must not leave the client.
- Autonomous clients now have first-pass queue diagnostics/status: CLI `traces queue-status` reports scoped readiness, bearer-token environment presence, queue/hold counts, typed retry/manual-review/policy hold counts, next retry time, durable flush/status-sync telemetry, last compaction reclaimed count, duplicate-envelope, orphan-hold-sidecar, and malformed-envelope quarantine removals, safe warning aggregates for schema-version/consent-policy/redaction-pipeline/trace-card-redaction-pipeline mismatches and malformed envelopes, sanitized failure classes, sanitized held-reason counts, and local credit summary fields; queue writes use atomic temp-file replacement and malformed active queue files are quarantined locally so valid later envelopes still submit. The authenticated web API exposes scoped queue/held counts, durable telemetry, safe warning aggregates, and sanitized held entries without exposing envelope bodies or raw observed mismatch values.
- Periodic credit notices now run through CLI, web, post-turn runtime, and the periodic queue worker path, including delayed ledger deltas and credit-event counts without surfacing trace bodies or central corpus rows. CLI and authenticated web clients can also acknowledge the current local credit fingerprint or snooze notices for a bounded number of hours.
- Signed upload-claim auth supports EdDSA/Ed25519 public-key verification through default or `kid`-selected keys and JSON/file/guarded-HTTPS keysets with optional activation windows and safe active/inactive/managed key-count diagnostics, while HS256 claims and static tokens remain internal bridge paths. A managed-EdDSA-required gate now accepts only active managed-keyset claims with issuer/audience checks; autonomous clients can fetch short-lived EdDSA upload claims from guarded tenant issuers, the repo includes a standalone Ed25519-only issuer MVP for those claims, and ingestion services can refresh guarded remote issuer-managed Ed25519 keysets live after startup with last-good preservation and optional max-stale fail-closed enforcement.
- The private ingestion service still serves file-backed pilot APIs by default, but can dual-write metadata through `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- The PostgreSQL schema slice is owned by this server repo: core corpus rows, object refs, derived records, vector metadata, audit events, credit ledger rows, tombstones, retention/export metadata, compact replay export manifests, and replay export item snapshots.
- `TraceCorpusStore` exists behind the server crate's database facade with the `PgBackend` implementation and PostgreSQL integration coverage.
- Optional DB-backed read flags now cover contributor credit/status, reviewer metadata, replay export selection, and audit event reads.
- Reviewer/admin review ergonomics now include `POST /v1/review/leases/claim-next`, `POST /v1/review/leases/claim-batch`, `ironclaw traces review-lease-claim-next`, and `ironclaw traces review-lease-claim-batch`, which claim the next or a bounded batch of available quarantined traces for the authenticated tenant/principal using escalation/SLA priority ordering, DB-backed lease state, and typed claim audit rows; this remains a reviewer-read dark-launch slice, not a broad production assignment system.
- The encrypted local artifact sidecar stores submitted redacted envelopes, and DB-backed replay export resolves bodies through a shared policy/audit helper that verifies active DB object refs, tenant scope, artifact kind, and content hash for file-backed objects or encrypted artifacts. Production-shaped object-primary modes can now skip plaintext submit/review bodies, replay-export body fallback, and benchmark/ranker derived export files when the matching DB/object-store guards are enabled.
- Local credit visibility now has a reusable report shape that separates local lifecycle state from central accepted/quarantined/rejected status, credit totals, delayed ledger deltas, and last submission/status-sync times.
- Ranking-derived credit now has a bounded worker coordinator that runs calibration, model promotion, prediction-credit issuance, settlement, and NEAR outbox inspection/submission/confirmation for one model/policy/target without granting broad admin settlement access. Pairwise model-risk gates now require distinct preference-label sources and distinct label-writing actor principals whenever the pairwise evidence floor is enabled and the deployment raises the label-source floor above one, stored calibration gates re-check actor diversity when production label-source floors rise, and optional labeler issue-rate/support controls turn the hashed reliability report into model-risk, prediction-credit, readiness, and settlement blockers for the calibration labeler cohort, so centrally operated pilots can keep issuance simple without letting one operator manufacture independent credit evidence. Process-evaluation workers now have an optional in-process scheduler that authenticates as a process-evaluation worker, fails closed by default without a configured external process evaluator, and can append system ranking labels through the existing worker route without exposing scheduler tokens, evaluator refs, raw reasons, or external-ref prefixes in config status. A bounded credit-cycle scheduler route and optional in-process scheduler can preflight or run the next eligible model from a utility-worker token without exposing scheduler tokens, raw reasons, or NEAR contract ids through config status. Dedicated NEAR outbox workers can also submit pending receipt calls and poll submitted rows for confirmation through configured operator adapters, and an optional in-process NEAR outbox scheduler can run that submit/confirm loop from a utility-worker token while exposing only safe readiness fields.
- Centrally run issuance can now pin approved settlement policy versions with `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`; dry-runs and drills still expose source-list hashes for review, operational readiness blocks positive delayed credit until an allowlist is configured, and live settlement rejects unlisted policy versions before writing settlement batches or NEAR outbox rows.
- Maintenance can backfill file-backed pilot records into the DB mirror, mark/purge expired records, prune invalid export caches, index deterministic vector metadata for canonical summaries, and run file-vs-DB reconciliation with reader-projection parity diagnostics. Broad analytics releases can also be guarded by an audit-backed epsilon publication ledger so operators can rehearse fail-closed budget exhaustion before exposing aggregate releases.
- Export audit paths now carry deterministic source-list hashes, replay export manifest metadata can be listed by reviewer/admin tokens, replay/benchmark/ranker export call sites mirror short-lived grant plus export-job lifecycle rows into the DB control plane, replay export jobs terminalize as failed when metadata, retention metadata validation, or required object-ref body reads fail after job start, benchmark/ranker export jobs terminalize as failed for pre-publication retention metadata validation, metadata/source collection, source object-ref revalidation, and source-read audit failures, export-worker automation has dedicated replay/ranker routes plus CLI helpers, and export workers can claim and run queued replay, benchmark-conversion, ranker-candidate, or ranker-pair jobs from persisted safe request metadata without creating a second job row. A bounded run-queued worker surface can drain multiple jobs across all supported dataset kinds while continuing after per-job failures, admins can retry failed unexpired replayable jobs back to `queued` with hash-only retry metadata, workers can retry due failed jobs with bounded attempts and exponential backoff, and deployments can enable an in-process scheduler that runs the same authenticated retry/drain handlers on an interval from a configured export-worker bearer token. Benchmark evaluation/publication workers now have an optional in-process pipeline scheduler that authenticates as a benchmark worker, requires explicit evaluator and registry refs, fails closed by default without a configured external evaluator, and supports dry-run rehearsal without evaluator calls or lifecycle/outbox writes. Benchmark registry outbox submission/confirmation workers now also have an optional in-process scheduler that authenticates as a benchmark worker, drains submit candidates, then confirms submitted receipts with bounded limits and safe config-status exposure. Admins can inspect durable export grants/jobs plus safe operational aggregates through tenant-scoped API/CLI list surfaces and the admin operational summary API/CLI helper. Running export jobs whose grant expiry has passed are reported as stale promotion blockers, and admins can atomically recover those stale rows to `expired` through a hash-only audit path that reads no trace bodies.
- Durable tenant access grant storage now exists for issuer-authorized principals and hosted-agent multitenant permissioning, including role, consent/use allow-lists, issuer/audience/subject attribution, expiry, revocation fields, tenant-scoped admin create/list/revoke APIs and CLI helpers, and tenant-scoped PostgreSQL RLS policies. When `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true`, trace submission, contributor credit/status readback, reviewer/audit reads, review mutations, dataset/export paths, non-revocation worker mutations, maintenance, and admin ledger/observability reads are denied without an active exact-role grant for the authenticated principal. Grant scopes/uses narrow the effective request policy, while revocation/self-delete, revocation propagation, config-status, tenant-policy admin, and grant-management routes remain available for deprovisioning and recovery. Required DB mirror mode now blocks review decisions before body-read/file-side publication when the mirror is absent, mirrors reviewed status/object-ref state before compatibility metadata/audit writes, and DB-mirrors shared aggregate-audit/trace-content-read rows before compatibility file audit append.
- Revocation, retention expiration, and maintenance-discovered file tombstones already invalidate DB-mirrored submission status, object refs, derived records, vector metadata, replay export manifests, and replay export item rows. Required DB mirror mode preflights direct revocation DB tombstone/status/invalidation plus revocation-audit mirrors before writing compatibility file tombstones, status updates, invalidations, or audit rows, preventing file-only revocation state during cutover. Revocation-propagation credit-settlement items now reverse exact delayed-credit ledger rows with deterministic negative settlement events, and an optional in-process revocation-propagation scheduler can run the same bounded worker route from a revocation-worker token while exposing only safe config-status readiness. Non-dry-run physical purge and revocation-propagation object-payload items can now mark only exact physically deleted service-owned submitted/review envelope object refs with `deleted_at`; revocation-propagation object deletes also record durable physical-delete receipt rows with evidence hashes for exact service-owned submitted/review envelope, vector worker-intermediate, benchmark artifact, and ranker export provenance payload targets, including filesystem-remote service-owned artifacts. Vector worker payloads now carry deterministic local redacted-summary feature embeddings in encrypted worker-intermediate objects, and exact vector-entry invalidation is available across PostgreSQL. Disabled cloud remote object deletion and unsupported artifact payload deletion remain future work; disabled remote object-payload rows skip with an explicit disabled-IO reason and no secret-bearing response or stored error text until cloud provider adapters exist.

The main remaining gap is production rollout hardening: file-backed serving is still the compatibility path, encrypted artifacts now have local-service and filesystem-remote service-owned storage plus smoke coverage and filesystem-remote restore rehearsal but not cloud remote providers, PostgreSQL RLS is forced, policy predicates are centralized, readiness now rejects table-owner runtime roles, and the PostgreSQL backend no longer exposes its raw pool as the normal public application API, but deployments still need full production-service-role rehearsal before RLS is the active trust boundary, and vector, benchmark, ranking, retention, and audit systems are not yet complete production workers.

## Operator Finish-Line Checks

These checks describe the production rollout shape, not a completed broad launch. Keep every promotion tenant-scoped until reconciliation, rollback, and smoke evidence is green for that tenant.

- Per-tenant rollout starts with DB dual-write/backfill, active tenant access grants for the exact principals and roles being promoted, and `/v1/admin/db-reconciliation-drill` evidence with no `blocking_gaps`. Promote DB reader flags before object-ref-required modes, then object-primary modes. Treat object-ref readability, hash, or key-ref mismatch diagnostics as rollout blockers.
- Rollback should be a flag and allowlist rollback first: disable the tenant's DB/object/vector/export read gates, keep DB/object dual-write evidence, and preserve audit rows, tombstones, revocation ledgers, and retention job rows. File-backed pilot serving remains the compatibility path during the rollback window, and `/v1/admin/rollback-drill` should be run with evidence recording before and after the flag change.
- Key rotation uses managed EdDSA/Ed25519 keysets with `kid` selection, guarded refresh, last-good preservation, safe config-status counts, and `/v1/admin/key-rotation-drill` smoke evidence. Add new issuer keys before switching claim issuance, keep old keys through the maximum claim lifetime plus refresh window, and keep static-token or HS256 bridge credentials out of production-gated paths.
- Object-store migration now has the service-owned local encrypted backend plus a filesystem-backed `TRACE_COMMONS_OBJECT_STORE=remote_service` rehearsal adapter, optional filesystem deleted-version archives behind `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING`, the admin rollback drill endpoint, `POST /v1/admin/object-store-migration-drill` for hash-only write/read/delete/restore probe evidence plus a hash-only migration manifest reference, and `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING` for startup-level cutover enforcement. Filesystem-remote writes now validate encrypted object records, reject divergent same-key rewrites, and commit JSON records through synced temp-file rename so rehearsal storage does not follow object-path symlinks. Submission metadata records the encrypted artifact object-store alias, DB object-ref backfill preserves that alias across later store cutovers, encrypted object-ref reads fail closed on configured-store mismatches, reconciliation reports store mismatches as their own blocker, and the DB reconciliation drill exposes aggregate object-ref failure counts by class. Operators can require versioning/restore support, see safe requirement/support booleans in config status, operational summary, promotion gates, and metrics, and get explicit fail-closed blockers from adapters that cannot provide it. Do not plan cloud remote object cutover until the production provider, object-version restore story, and rehearsed rollback evidence exist. Local-service and filesystem-remote object-primary canaries must verify tenant key refs, decryptability, and stored hashes.
- Retention purge promotion starts with dry runs, legal-hold review, durable retention job/item inspection, and audit-chain verification. Legal-hold skips, extended expiration timestamps, and export source selection are only trusted when they fit the server-derived retention class for the submission's allowed uses. Destructive purge should stay behind tenant kill switches until a dry-run report, reconciliation report, and rollback plan have been reviewed.
- Audit-chain promotion requires `/v1/admin/audit-chain-drill` evidence, append-only audit/tombstone preservation, and source-list hash checks for exported slices. Projection drift or stale predecessor diagnostics block promotion.
- Smoke checks should include submit/status, `tenant_canary_isolation`, contributor credit, reviewer metadata, replay export selection, audit reads via `/v1/admin/canary-read-drill`, `object_primary_reads` via `/v1/admin/object-primary-read-drill` for one canary tenant while a second tenant remains on fallback, `object_store_migration` via `/v1/admin/object-store-migration-drill`, `ranking_model_readiness` via `/v1/admin/ranking/readiness-drill`, `db_reconciliation_clean` via `/v1/admin/db-reconciliation-drill`, `rollback_flag_drill`, `key_rotation_drill`, audit-chain verification, `revocation_propagation` via `/v1/admin/revocation-propagation-drill`, `delayed_credit_reversal` plus `object_deletion_refs` via `/v1/admin/revocation-effects-drill` after the live worker runs, `retention_dry_run` via `/v1/admin/retention-dry-run-drill`, `vector_index` via `/v1/admin/vector-index-drill`, `analytics_release` via `/v1/admin/analytics-release-drill`, `benchmark_pipeline` via `/v1/admin/benchmark-readiness-drill`, and PostgreSQL RLS readiness via `/v1/admin/postgres-rls-drill`. Rollout-smoke readiness treats latest per-check evidence older than 24 hours as stale, so runbooks should refresh canary evidence immediately before production promotion. Include a revocation case that proves exact delayed-credit reversal and service-owned object deletion for submitted/review, vector, benchmark, and ranker payload refs where the current implementation supports them; disabled cloud remote payload deletes remain future work, but worker rehearsal should also prove they skip closed without leaking bucket, KMS, credential, or object-key material.

## Roadmap Principles

- Keep contribution opt-in and local-first. Uploads are always redacted `ironclaw.trace_contribution.v1` envelopes.
- Treat envelope contributor and tenant fields as attribution only. Authorization comes from request identity, tenant policy, and DB row scope.
- Keep the file-backed MVP available until each DB/object-primary read surface has parity evidence and rollback.
- Store metadata, object refs, hashes, indexes, ledgers, and workflow state in DB. Store trace bodies and large artifacts in encrypted object storage. Store vector payloads in a vector backend or backend-specific index, with relational metadata as the source of truth.
- Version every derived artifact by input hash, worker version, policy version, and output artifact id.
- Test through callers for any side effect: handlers, store facades, maintenance jobs, and export/revoke flows must prove tenant id, actor principal, object ref, and submission id propagation.

## Phase Plan

### Phase 0: MVP and Storage Bridge Baseline

Status: mostly complete on `gecko-pass`.

Scope:

- Local opt-in policy, preview, queue, flush, credit display, list-submission summary visibility, scoped web/runtime policy, and autonomous post-turn contribution.
- Deterministic local redaction, tool-aware payload redaction, stable placeholders, and Privacy Filter safe projection.
- Internal ingestion service with submit, list, revoke, review, credit, analytics, replay export, benchmark candidate, maintenance, and audit surfaces.
- DB dual-write metadata, encrypted local artifact sidecar, DB-backed reader flags, vector metadata indexing, compact replay manifest rows, and maintenance backfill/reconciliation.

Dependencies:

- Existing local redaction envelope contract.
- File-backed pilot store under `TRACE_COMMONS_DATA_DIR`.
- Current server-owned PostgreSQL migration and server-owned `TraceCorpusStore`.

Verification gates:

- Local redaction canary tests prove secrets, paths, raw sidecar spans, and raw text do not survive accepted envelopes or derived summaries.
- PostgreSQL store and caller-level tests prove tenant scoping for submit, review, credit, revoke, export selection, maintenance vector indexing, and audit reads.
- File-backed APIs remain default unless a surface-specific DB read flag is enabled.

Exit criteria:

- Keep this phase shippable as the internal pilot baseline while later phases build behind flags.

### Phase 1: DB Read Cutover Readiness

Status: in progress. Reconciliation now covers submissions, derived records,
object refs, vectors, credit-ledger counts, audit-event counts, replay/export
manifest counts, export item counts, revocation/tombstone counts, and
reader-projection parity for contributor credit, reviewer metadata, analytics,
audit, and replay/export manifest surfaces; remaining cutover work is parity
enforcement, PostgreSQL coverage breadth, and rollout diagnostics.

Scope:

- Use reconciliation parity diagnostics as the promotion gate for contributor, reviewer metadata, analytics, audit, and replay/export manifest DB read flags.
- Add PostgreSQL integration coverage for the same logical store operations covered by the storage contract.
- Make DB-backed reader flags safe to enable per tenant or per deployment surface, with visible diagnostics when parity checks fail.
- Keep file-backed fallback for pilot data and rollback during the cutover window.

Dependencies:

- Phase 0 dual-write/backfill metadata.
- Stable status values, audit actions, credit event kinds, export purposes, object ref kinds, and retention/revocation transitions.

Verification gates:

- For sampled tenants, DB-backed contributor status/credit, reviewer lists, analytics, replay export selection, and audit event reads match file-backed behavior.
- Backfill-only maintenance reports malformed pilot submission/derived metadata as bounded item failures instead of aborting the whole backfill, and keeps valid records moving.
- The retention-maintenance worker route can be owned by the server through the optional in-process scheduler: startup validates retention-worker auth, scheduler ticks call the same narrow worker route, tenant-scoped dry-run/live purge behavior is tested through the caller, and config status exposes only safe scheduler shape.
- The dedicated vector-index worker route is worker-scoped and bounded: it requires a configured DB mirror before doing work, accepts an optional 1-500 item limit, reports checked/indexed/skipped/pending counts, and avoids retention-maintenance side effects. The optional in-process vector-index scheduler now validates vector-worker auth plus DB mirror availability at startup and calls that same bounded route without exposing worker tokens or raw purpose text in config status.
- PostgreSQL tests prove duplicate ids/hashes under separate tenants do not cross-read or cross-mutate.

Exit criteria:

- DB metadata reads can be promoted surface by surface without changing the envelope contract or losing file-backed rollback.

### Phase 2: Service-Owned Object Storage

Status: not production-complete.

Scope:

- Move redacted envelope bodies and large artifacts from local encrypted sidecar semantics to service-owned encrypted object storage.
- Resolve review/export/body reads through `trace_object_refs` first, with file-backed fallback only for migrated pilot records.
- Verify content hashes and tenant/key policy before every trace-content read.
- Add object lifecycle states for invalidated, deleted, and retained artifacts, and connect them to revocation/retention jobs.

Dependencies:

- Phase 1 DB metadata reads for object refs and source eligibility.
- Secrets/KMS or equivalent key reference strategy.
- Tenant-derived object partitioning that never trusts envelope tenant fields.

Verification gates:

- Object keys do not expose raw user ids, local paths, prompts, tenant tokens, or secret values.
- Object reads fail closed when DB source status is revoked, expired, purged, rejected, quarantined, or outside the requested consent/use scope.
- Hash/decrypt checks run before reviewer, replay, benchmark, ranker, or audit-visible content access.

Exit criteria:

- New production tenants can store redacted trace bodies outside file-backed pilot directories while keeping pilot fallback available for migration.

### Phase 3: Tenant Policy, RBAC, ABAC, and Audit Hardening

Status: partially represented by static token roles plus optional ingest-time
tenant submission policies for allowed consent scopes and trace-card uses.
Production-like deployments can require explicit tenant policy entries, ingest
can now read those policies from the TraceCorpusStore behind
`TRACE_COMMONS_DB_TENANT_POLICY_READS`, and worker tokens now have scoped
export, retention, vector, and benchmark route gates; fuller RBAC/ABAC remains
future work.
The PostgreSQL store now sets `tracedao.trace_tenant_id` transaction-locally around
tenant-scoped Trace Commons operations while retaining explicit `tenant_id`
predicates, and server migrations force RLS for the Trace Commons tables and
centralize the tenant policy predicate. The raw pool is restricted to crate
internals plus an explicitly named test/diagnostic hook, so new application paths
are steered through tenant-context store helpers instead of grabbing unscoped
clients. The RLS readiness drill and fail-closed startup gate also probe that
tenant context is transaction-local and clears after commit, and now reject
runtime roles that either bypass RLS or own the Trace Commons tables. This is
still an incremental guardrail until every DB-backed runtime path runs under the
production service role.

Scope:

- Replace static tenant-token assumptions and JSON pilot policies with central tenant context, short-lived credentials or signed upload claims, role grants, allowed scopes, allowed uses, and expiry.
- Enforce PostgreSQL RLS or equivalent query guardrails for every tenant-scoped `trace_*` table.
- Keep PostgreSQL RLS and explicit tenant filters visible in repository methods and caller tests.
- Require reasons and typed safe metadata for privileged actions: review override, delayed credit mutation, bulk export, retention override, purge, and tombstone changes.
- Add tamper-evident or append-only audit behavior for submit, read, review, credit mutate, revoke, export, retain, purge, vector index, and benchmark conversion.

Dependencies:

- Phase 1 DB metadata reads.
- Phase 2 object access through service-owned identities.
- A settled `TenantCtx` or ingest equivalent for handlers and workers.

Verification gates:

- Contributor, reviewer, admin, and worker roles are tested against same-id cross-tenant fixtures.
- Every privileged mutation emits an audit event with tenant id, actor/job id, role, target ids, reason, and decision input hash where applicable.
- PostgreSQL RLS tests prove tenant B rows are invisible under tenant A context.

Exit criteria:

- Authorization no longer depends on envelope fields or static pilot tokens, and audit coverage is sufficient for security review.

### Phase 4: Derived Artifact Workers

Status: vector metadata, private embed/search adapters, vector-backed ranking feature derivation, and benchmark candidate plumbing are partial; production rollout hardening remains future work.

Scope:

- Implement private vector duplicate/novelty workers that embed only approved redacted projections and write `trace_vector_entries` plus derived records. The vector worker can now call a private embedder, fail closed when the private embedder or private search adapter is required but missing, store compatible payloads, query a private vector-search adapter, accept only server-validated active tenant vector-entry neighbors before writing duplicate/novelty metadata, and expose nearest-neighbor policy gaps when stored refs are stale or cross model/store/dimension/version profiles. The ranking feature worker can require active vector metadata before deriving server-provenanced ranking features, and benchmark conversion plus ranker training candidate/pair exports now collapse exact canonical-summary hash duplicates before source-list hashing and delayed utility credit; remaining work is deployed vector-store operations and promotion evidence for broad rollout.
- Promote benchmark conversion into controlled worker jobs that record consent scope, review state, redaction version, replayability requirements, source-list hashes, and artifact refs.
- Add ranker/model-utility jobs as offline analysis that may append delayed credit only with a downstream artifact/job reference.
- Extend item-level export manifest rows beyond replay datasets to benchmark and ranker artifacts once those exports become durable job outputs.

Dependencies:

- Phase 2 object-primary artifact reads for all trace-body surfaces.
- Phase 3 worker roles, audit, and ABAC checks.
- Revocation checks before source read and before artifact publish.

Verification gates:

- No vector entry, benchmark artifact, ranker pair, or export item can be produced for revoked, expired, quarantined, rejected, out-of-scope, or unapproved submissions.
- Worker outputs record input hash, worker version, policy version, source projection, and output object ref or vector id.
- Delayed credit events are append-only, bounded by policy, reasoned, and linked to review/export/worker artifacts.

Exit criteria:

- Derived artifacts become reproducible, tenant-scoped, revocation-aware corpus assets rather than ad hoc pilot outputs.

### Phase 5: Production Retention and Revocation Propagation

Status: partial metadata invalidation, exact delayed-credit reversal, vector/export row invalidation, and service-owned local artifact deletion now exist; cloud remote payload deletion, broader benchmark-settlement coverage, and production hardening remain future work.

Scope:

- Implement resumable retention jobs with dry-run reports, legal-hold checks, policy-change handling, retries, grace periods, and verification.
- Fan out revocation and retention transitions to object refs, object payloads, vectors, benchmark artifacts, ranking/training queues, export manifests/items, credit settlement, and worker queues.
- Keep tombstones long enough to prevent re-ingest or re-export after content deletion.
- Add reconciliation that finds active derived artifacts whose source is revoked, expired, or purged and invalidates them.

Dependencies:

- Phase 2 object lifecycle controls.
- Phase 3 audit and policy enforcement.
- Phase 4 derived artifact source links.

Verification gates:

- Revocation writes or confirms tombstones before content invalidation.
- Existing exports are invalidated or item-marked when a source is revoked after export.
- Destructive object/vector deletes are delayed, audited, resumable, and verified.
- Benchmark, ranking, and credit-settlement invalidation tests cover the gaps called out in the storage plan.

Exit criteria:

- Production deployments can honor contributor revocation and retention policy across every central and derived corpus surface.

### Phase 6: Production Cutover and Tenant Rollout

Status: future.

Scope:

- Disable file-backed writes for production tenants after DB/object-primary reads pass parity and rollback windows.
- Keep file-backed reads for one release window for migrated pilot tenants.
- Add per-tenant rollout flags, dashboards, maintenance reports, and kill switches for DB reads, object reads, vector workers, benchmark exports, and retention deletion.
- Update public/internal docs and `FEATURE_PARITY.md` if user-visible Trace Commons behavior changes.

Dependencies:

- Phases 1 through 5 complete for the target tenant class.

Verification gates:

- Migration manifests prove source file hashes, DB rows, object refs, tombstones, credit totals, audit events, and export manifests converge.
- Rollback drills prove DB-first reads can be disabled without deleting rows and without losing audit/tombstone state, using `/v1/admin/rollback-drill` plus fresh `rollback_flag_drill` smoke evidence for the tenant.
- Security review clears tenant policy, object access, audit, retention, and revocation paths.

Exit criteria:

- Trace Commons can accept production tenants without relying on pilot file storage as the primary serving path.

## Parallelization Lanes

These lanes can proceed in parallel as long as their write scopes stay disjoint and they meet at the verification gates above.

| Lane | Primary ownership | Can start after | Produces | Must coordinate with |
|------|-------------------|-----------------|----------|----------------------|
| A. DB parity and read cutover | Storage/control-plane | Phase 0 | Reconciliation coverage, PostgreSQL tests, safer DB read flags | Lanes B, E, G |
| B. Ingestion/API reader promotion | Ingest service/API | Phase 0 and Lane A contracts | DB-backed contributor, reviewer, analytics, replay, audit behavior by surface | Lanes A, D, E |
| C. Object-primary artifact storage | Artifact service/storage | Object ref contract from Phase 0 | Service-owned encrypted object reads/writes and local-sidecar migration path | Lanes A, B, F |
| D. Tenant policy and audit | Auth/security | Phase 0 role semantics | Tenant context, RBAC/ABAC, RLS policy, typed audit metadata | Lanes B, C, E, F |
| E. Retention and revocation propagation | Lifecycle workers | Phase 0 invalidation semantics | Tombstone-first propagation, retention jobs, reconciliation, rollback safety | Lanes A, C, F |
| F. Derived workers and exports | Vector/benchmark/ranking | Phases 2 and 3 contracts | Vector worker, benchmark worker, ranker utility, item-level manifests | Lanes C, D, E |
| G. Verification and docs | Test/operations/docs | Always | Caller-level tests, migration reports, rollout docs, parity notes | All lanes |

## Dependency Map

- DB-backed reads depend on dual-write or backfill plus reconciliation.
- Object-primary reads depend on DB object refs and tenant/key policy.
- RLS and ABAC depend on a trusted tenant context in handlers and workers.
- Vector and benchmark workers depend on object-primary reads, worker roles, audit, and revocation checks.
- Retention and revocation production gates depend on object/vector/export artifact links.
- Broad rollout depends on parity evidence, rollback drills, and docs updates.

## Verification Gates Summary

- Redaction gate: accepted envelopes and derived summaries never contain raw trace text, raw sidecar spans, secrets, local paths, bearer tokens, or raw tool payloads outside explicit policy.
- Tenant gate: every read/write/mutation/export path is driven by auth-derived tenant and actor context, with same-id cross-tenant tests.
- Parity gate: DB-backed reader-projection diagnostics are green before a surface-specific read flag is promoted.
- Object gate: every trace body read verifies object ref tenant linkage, hash, decryptability, source status, consent scope, and allowed use.
- Audit gate: privileged mutations and content reads emit typed, tenant-scoped, append-only audit events with reason, purpose, and decision input hashes where needed.
- Revocation gate: revoke and retention flows invalidate or block submissions, object refs, derived rows, vectors, benchmarks, exports, worker queues, and credit settlement.
- Rollback gate: disabling DB/object/vector read flags leaves file-backed pilot behavior available and preserves audit/tombstone history.

## Next Build Lanes

The highest-value next work is:

1. Finish DB-read parity and reconciliation so reviewer, analytics, replay, audit, and contributor surfaces can graduate from optional flags with confidence.
2. Introduce service-owned encrypted object storage and route remaining review/export body reads through object refs.
3. Add tenant policy/RLS hardening before broadening reviewer/admin/export access.
4. Complete retention/revocation propagation for benchmark, ranking, worker, and already-published export artifacts.
5. Build the private vector worker and benchmark conversion workers only after object-primary reads and worker authorization are in place.

This ordering keeps the corpus trustworthy before it becomes more useful: metadata parity and object ownership come first, then policy/audit hardening, then derived data products.
