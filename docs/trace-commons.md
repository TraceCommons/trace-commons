# IronClaw Trace Commons

Trace Commons is an opt-in pipeline for contributing locally redacted IronClaw traces to a private corpus. It is separate from replay trace fixtures: replay traces support deterministic tests, while contribution envelopes carry consent, redaction metadata, replayability metadata, scoring, revocation, and contributor credit.

## Local-First Rules

- Trace contribution is off by default.
- Raw traces stay local.
- Uploads require a standing opt-in policy with an explicit ingestion endpoint.
- The client submits only `ironclaw.trace_contribution.v1` envelopes after deterministic local redaction.
- Message text and tool payloads remain excluded unless the user opts into those fields.
- Medium/high privacy risk traces can be held for manual review by policy.
- OpenAI Privacy Filter or other PII sidecars must only contribute safe summaries: redacted text, allow-listed label counts, and warnings. Do not serialize original text or `detected_spans[*].text`.
- `safe_privacy_filter_redaction_from_output` converts Privacy Filter-style output to redacted text plus `SafePrivacyFilterSummary`, dropping raw `text`, raw span contents, raw offsets, and unsafe span labels.
- Tool-specific structured redaction treats email, calendar, messaging, browser, filesystem, and database payload fields as sensitive before generic secret/path scrubbing.
- Deterministic text redaction preserves safe within-trace structure with stable placeholders such as `<PRIVATE_EMAIL_1>` and `<PRIVATE_LOCAL_PATH_1>` instead of flattening every entity to the same token.
- A local Privacy Filter sidecar can be enabled with `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` and optional whitespace-split `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. The sidecar receives `{"text":"..."}` on stdin and must return Privacy Filter-style JSON on stdout. IronClaw keeps only the safe `redacted_text` and aggregate summary. The sidecar is launched with a cleared environment except `PATH`, `LANG`, and `LC_ALL`; `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS`, `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES`, `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES`, and `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` tune local guardrails.

## CLI MVP

```bash
# Enable autonomous submission after local redaction.
ironclaw traces opt-in \
  --endpoint https://trace-ingest.internal/v1/traces \
  --scope debugging-evaluation

# Hosted tenants can refresh short-lived EdDSA upload claims from an issuer.
ironclaw traces opt-in \
  --endpoint https://trace-ingest.internal/v1/traces \
  --upload-token-issuer-url https://issuer.near.com/v1/trace-upload-claim \
  --upload-token-issuer-allowed-hosts issuer.near.com \
  --upload-token-audience trace-commons \
  --upload-token-tenant-id tenant-a \
  --upload-token-workload-token-env IRONCLAW_TRACE_WORKLOAD_TOKEN

# Create a local redacted envelope from an existing recorded trace.
ironclaw traces preview \
  --recorded-trace trace.json \
  --output contribution.json

# Queue a redacted envelope for autonomous submission.
ironclaw traces enqueue --envelope contribution.json

# Or preview and queue in one step.
ironclaw traces preview \
  --recorded-trace trace.json \
  --enqueue

# Submit eligible queued envelopes using the standing policy.
ironclaw traces flush-queue

# See local credit totals and recent explanations.
ironclaw traces credit

# Acknowledge or snooze a due periodic credit notice.
ironclaw traces credit --notice --ack
ironclaw traces credit --notice --snooze-hours 24

# Disable autonomous contribution.
ironclaw traces opt-out
```

The static submit token is read from `IRONCLAW_TRACE_SUBMIT_TOKEN` by default. The token is not stored in the policy file. Hosted deployments can instead configure an upload-claim issuer in the standing policy. Queue flush, explicit `traces submit --envelope`, remote status sync, and remote revoke then request short-lived bearer claims from the HTTPS issuer, require exact issuer-host allowlisting, reject embedded URL credentials/query/fragment/internal targets, require the returned claim to be an EdDSA/Ed25519 JWT with a `kid`, cache it only in process memory until its refresh margin, and retry once with a forced refresh after a 401/403 from submit, status-sync, or revoke calls. Optional workload credentials for the issuer stay in the configured environment variable and are never written to the policy. `preview --enqueue` and `enqueue --envelope` use the same standing-policy gate as autonomous flush: the policy must be enabled, must have an ingestion endpoint, and must allow any message text or tool payloads already present in the redacted envelope. Plain `preview` remains local and does not require opt-in.

## Private Ingestion Service MVP

The repository includes a local private-corpus ingestion binary for development and internal deployments:

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2026-04-27T00:00:00Z,tenant-a:reviewer:review-token-a,tenant-a:export_worker:export-token-a,tenant-b:dev-token-b' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin tracedao-ingest
```

Token entries may use `tenant_id:token` for contributor access or
`tenant_id:role:token` for scoped roles. Either form may append a short-lived
credential expiry attribute, for example
`tenant_id:role:token;expires_at=2026-04-27T00:00:00Z` or
`tenant_id:token;expires=2026-04-27T00:00:00Z`. Expired bearer tokens are
rejected before tenant attribution, and token-principal hashes are computed from
the secret token value only, not the expiry metadata. The ingest service recognizes
`contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`,
`revocation_worker`, `vector_worker`, `benchmark_worker`, `utility_worker`, and
`process_eval_worker` (also accepted as `process_evaluation_worker` for token
configuration). Worker roles do not inherit reviewer
visibility: export workers can build replay/ranker exports, benchmark workers
can run benchmark conversion through either the reviewer-compatible conversion
route or a dedicated worker route, can run deterministic benchmark
evaluation batches through the benchmark-evaluations worker route, and can run
registry publication batches through the benchmark-registry-publications route, retention workers can run the dedicated
retention worker route or retention-scoped maintenance, revocation workers can
run only the dedicated revocation-propagation route, and vector workers can run
the dedicated vector-index worker route or vector-index maintenance. Utility
credit workers can append idempotent delayed utility credit through their
dedicated route for accepted traces only and can run the bounded credit-cycle
coordinator for one model/policy/target, or the scheduler route that selects the
next eligible candidate/active model for one target/policy and skips existing
live claims, without access to reviewer bonus, abuse penalty, review queues,
audit logs, or tenant policy administration.
As an alternative to configured static bearer tokens, internal deployments can
set `TRACE_COMMONS_SIGNED_TOKEN_SECRET` to accept HS256 signed tenant claims.
This HS256 path is an internal bridge for controlled pilots, not the production
asymmetric-token design. The current service also accepts EdDSA/Ed25519 signed
claims when configured with `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_PEM`
or `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILE`; keyed EdDSA public-key
rotation can use `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILES` as
comma-separated `kid:path` entries. Production upload claims should be treated
as EdDSA/Ed25519-only: static tokens and HS256 signed claims remain bridge paths
for controlled pilots, not the production claim mechanism. Set
`TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS=true` to make every authenticated
route reject static tokens and HS256 signed claims, even if bridge credentials
are still configured; this gate requires at least one EdDSA/Ed25519 public key
at startup. For stricter issuer-governed deployments, set
`TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS=true`; this accepts only
active `kid`-selected EdDSA keys loaded from
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_JSON`,
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_FILE`, or
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL`, requires issuer and audience
checks, and rejects static tokens, HS256 claims, default EdDSA keys, ad hoc
keyed EdDSA public-key files, missing `kid`, and unmanaged `kid` values. The
URL source is also refreshed in the background for long-running deployments,
must use HTTPS, requires an exact
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS` allowlist, rejects
embedded credentials, query strings, fragments, localhost/internal/private
targets, disables redirects, pins validated DNS results, and size-caps the JSON
response. Use `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_BEARER_TOKEN` only when
the issuer endpoint requires a separate fetch credential, and
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_TIMEOUT_MS` to lower the startup
fetch and refresh timeout. `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_REFRESH_INTERVAL_SECONDS`
controls the live refresh cadence, defaulting to five minutes. Set
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_MAX_STALE_SECONDS` with managed
EdDSA-required mode to fail closed once the last successful remote keyset refresh
is older than the configured stale window; failed refreshes preserve the last
good active keyset. The verifier rejects unsupported JWT algorithms and can run
with only EdDSA keys configured.
For HMAC bridge rotation, `TRACE_COMMONS_SIGNED_TOKEN_SECRETS` can also provide
comma-separated `kid:secret` entries; signed tokens with a JWT header `kid`
must match one of those configured key ids, while tokens without `kid` use the
single default secret when present. EdDSA keyed public-key files use the same
JWT `kid` selection behavior. For activation-window rotation,
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_JSON`,
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_FILE`, or the guarded
`TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL` can load a JSON keyset with entries
shaped as `{ "kid": "...", "public_key_pem": "...", "not_before": "<RFC3339>",
"not_after": "<RFC3339>" }`; `not_before` and `not_after` are optional, but
inactive keys are rejected before token verification.
Config status reports the total signed-token key count plus aggregate EdDSA key
counts for configured, active, inactive, managed, active-managed, and
inactive-managed keys, plus whether remote keyset refresh is enabled, refresh
interval/max-stale seconds, last success/failure timestamps, and stale state.
It does not return key material, key ids, keyset URLs, host allowlists, bearer
fetch credentials, or activation timestamps.
Signed tokens must include `tenant_id`, `exp`, and either `principal_ref` or
`sub`; `role` defaults to `contributor` and may use the same role names as
static tokens. Claims may also include `allowed_consent_scopes` and
`allowed_uses` arrays to restrict which submitted trace consent scopes and
trace-card uses the bearer token can upload or later use for replay exports,
benchmark/ranker dataset generation, process-evaluation labeling, and utility
credit. Set
`TRACE_COMMONS_SIGNED_TOKEN_ISSUER` and/or
`TRACE_COMMONS_SIGNED_TOKEN_AUDIENCE` to require matching `iss` and `aud`
claims. Set `TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS` to require `iat` and
reject tokens whose `exp - iat` lifetime exceeds the configured bound. Set
`TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI=true` to require every signed claim to
carry a JWT ID, and `TRACE_COMMONS_SIGNED_TOKEN_REVOKED_JTIS` to a
comma-separated emergency denylist of JWT `jti` values. The config-status route
reports only whether signed-token auth and issuer/audience checks are enabled
plus safe key-count aggregates, the revoked-`jti` count, max TTL,
require-`jti` flag, and whether EdDSA-only or managed-EdDSA signed-token auth
is required; it never returns the secret or configured claim values.
Submitted-trace audit rows record only the safe auth method (`static_token` or
`signed_claim`) plus the hashed actor principal.
Local/file/HTTPS managed EdDSA keysets with activation windows are available for
controlled deployments and can be made the only accepted upload-claim rotation
path with `TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS=true`. HTTPS
issuer keysets now refresh live after startup so issuer-owned Ed25519 key records
can rotate without restarting the ingestion service.
`TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true` adds a DB-backed hosted-tenant
permissioning gate on trace submission, contributor credit/status readback,
reviewer/audit reads, review mutations, dataset/export paths, non-revocation
worker mutations, maintenance, and admin ledger/observability reads: the
authenticated tenant/principal must have an active, unrevoked, unexpired
`trace_tenant_access_grants` row with the same role as the request token. Grant
consent-scope and allowed-use allow-lists are additional constraints; they
intersect with any static-token or EdDSA claim allow-lists and cannot upgrade
the token's role. For signed EdDSA/Ed25519 upload claims, any issuer, audience,
or subject stored on the grant must also match the verified claim, so an
issuer-authorized hosted tenant grant cannot be replayed across issuer,
audience, or subject boundaries. Static-token bridge grants ignore those signed
claim binding fields. Revocation/self-delete, revocation propagation,
config-status, tenant-policy admin, and tenant-access-grant admin routes stay
available for deprovisioning and recovery. Admin tokens can manage the current
tenant's grant rows through `GET|POST /v1/admin/tenant-access-grants` and
`POST /v1/admin/tenant-access-grants/{grant_id}/revoke`, or the matching
`ironclaw traces tenant-access-grants-list`, `tenant-access-grant-create`, and
`tenant-access-grant-revoke` CLI helpers. The local-only
`ironclaw traces tenant-principal-ref` helper derives the stored
`principal_sha256:...` value for either static-token or signed-claim actors
without printing raw credentials. Those routes are tenant-scoped, DB-backed, and
audited with safe hash/count-only grant metadata.
For autonomous clients, the standing policy can point at a separate guarded
HTTPS upload-claim issuer. The queue worker posts a bounded claim request with
tenant/audience/consent/use metadata, sends optional workload credentials only
as an Authorization header, accepts only EdDSA JWT responses with `kid`, and
keeps the resulting bearer token in memory until its refresh margin.
Process-evaluation workers can submit bounded process quality metadata through
`POST /v1/workers/process-evaluation` using the CLI helper:

```bash
ironclaw traces process-evaluation-submit \
  --endpoint https://trace-ingest.internal/v1/traces \
  --submission-id 018f2b7b-0c11-72fd-95c4-1f9f98feac01 \
  --reason "nightly evaluator pass" \
  --evaluator-version process-eval-2026-04-26 \
  --label proper_verification \
  --tool-selection pass \
  --verification partial \
  --utility-credit-points-delta 0.5 \
  --utility-external-ref process-eval:nightly:2026-04-26
```

The same worker request can optionally include a `ranking_label` object for
accepted traces that are also allowed for the requested ranking target use. The
server validates that target before reading the trace body, stores only a
deterministic process-evaluation evidence hash plus the external-ref hash in the
ranking label row, requires the label source to remain `system`, and treats
repeated external refs as idempotent retries when the evidence and label fields
match.

Schedulers can also use `POST /v1/workers/process-evaluations/run` for bounded
process-evaluation batches. When `TRACE_COMMONS_PROCESS_EVALUATOR_URL` is
configured, the worker sends only derived candidate summaries, summary hashes,
hashed submission/trace ids, purpose hashes, evaluator refs, and safe aggregate
metadata to the adapter. It omits raw trace bodies, contributor principals, raw
submission ids, and raw external refs. Requests can set
`require_external_evaluator=true` to fail closed when the adapter is absent, and
can provide a ranking target plus `external_ref_prefix` to append idempotent
system ranking labels from the evaluator response.

When `--utility-credit-points-delta` is set, the helper also sends a trimmed
`utility_external_ref`; the service uses that external reference to append an
idempotent `training_utility` delayed credit event for the evaluated accepted
submission. Non-JSON output prints appended/skipped credit counts when the
server returns them.

Internal deployments can also add a fail-closed tenant submission policy. When
`TRACE_COMMONS_TENANT_POLICIES` contains an entry for the authenticated tenant,
new submissions must use only the listed consent scopes and trace-card allowed
uses before the server re-scrubs and stores them:

```bash
TRACE_COMMONS_TENANT_POLICIES='{
  "tenant-a": {
    "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
    "allowed_uses": ["debugging", "evaluation", "benchmark_generation", "aggregate_analytics"]
  }
}' \
cargo run --bin tracedao-ingest
```

Tenants without an explicit entry keep the development default so existing local
pilots continue to work. When a policy exists, it is also used as downstream
ABAC: replay exports require the `evaluation` use, benchmark conversion requires
`benchmark_generation`, ranker candidate/pair exports require
`ranking_model_training`, and vector indexing requires at least one derived-use
permission (`debugging`, `evaluation`, `benchmark_generation`,
`ranking_model_training`, or `model_training`). Matching source traces must
carry an allowed consent scope and the required trace-card allowed use, so
pre-policy records without matching allowed-use metadata are skipped. The
aggregate-only use is intentionally insufficient for vector indexing because
that retention class does not permit derived artifacts. Production deployments
should configure this policy for every tenant and treat envelope contributor
fields as attribution only. Set
`TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY=true` to reject new submissions
and export requests from tenants that do not have an explicit policy entry.

Set `TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS=true` in production-like ingestion
deployments to require explicit low-risk, accepted-status, consent-scoped replay
and benchmark export filters. Ranker training exports also require explicit
accepted-status, low-risk, ranking/model-training consent filters when this
guard is enabled.
Set `TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST` to lower the per-request item
cap for replay, benchmark, and ranker exports; the default remains 500 for
compatibility, and requests above the configured cap are clamped.

Set `TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR` and/or
`TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR` to bound autonomous
contributor uploads. Quotas are disabled by default, apply only to contributor
tokens, count active accepted/quarantined submissions in the last hour, and do
not block idempotent retries of an existing submission id. Revoked, expired, and
purged submissions stop consuming quota.

Optional dark-launch storage can be enabled for internal pilots:

```bash
# Mirror corpus metadata into the configured TraceDAO PostgreSQL database.
TRACE_COMMONS_DB_DUAL_WRITE=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Optionally serve contributor credit/status endpoints from that DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_CONTRIBUTOR_READS=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Optionally serve reviewer metadata views from that DB mirror.
# Production-like rollouts can add TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Optionally select replay exports from DB metadata.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Fail closed when benchmark/ranker export sources lack active submitted-envelope object refs.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Optionally serve reviewer audit reads from the DB mirror.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_DB_AUDIT_READS=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Fail maintenance closed when DB/file reconciliation reports promotion-blocking gaps.
# Use with admin maintenance requests that set reconcile_db_mirror: true.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Pause retention expiration/purge for selected central retention policy IDs.
TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES=private_corpus_revocable,benchmark_revocable \
cargo run --bin tracedao-ingest

# Store submitted redacted envelopes in the encrypted local artifact sidecar.
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_ARTIFACT_DIR=/var/lib/ironclaw/trace-artifacts \
cargo run --bin tracedao-ingest

# Prefer the service-owned local object-store backend for production-shaped pilots.
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
cargo run --bin tracedao-ingest

# Skip plaintext submitted/reviewed envelope body files for submit/review pilots.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Require replay export body reads through service-owned DB object refs.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true \
TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest

# Skip plaintext benchmark/ranker artifact and provenance files.
TRACE_COMMONS_DB_DUAL_WRITE=true \
TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true \
TRACE_COMMONS_DB_REVIEWER_READS=true \
TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true \
TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS=true \
TRACE_COMMONS_OBJECT_STORE=local_service \
TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true \
TRACE_COMMONS_ARTIFACT_KEY_HEX=<ironclaw-secrets-compatible-hex-key> \
TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR=/var/lib/ironclaw/trace-object-store \
DATABASE_URL=postgres://tracedao:tracedao@localhost/tracedao \
cargo run --bin tracedao-ingest
```

`TRACE_COMMONS_DB_DUAL_WRITE=true` builds a PostgreSQL-backed `TraceCorpusStore` mirror from `DATABASE_URL`. The mirror writes tenant-scoped submissions, tenant policies, tenant access grants, object refs, derived precheck records, export manifest metadata, export manifest item snapshots, audit events, credit events, utility attestations, credit settlement batches, credit holds, NEAR credit outbox rows/status, review state, revocation tombstones, retention maintenance job/item ledger rows, ranking calibration dataset registry rows, ranking model/feature/prediction/label evidence, ranking calibration runs, and ranking worker-run ledger rows, including redaction-count aggregates and derived summary/tool/coverage metadata needed for DB-backed reviewer/export/analytics/ranking paths. By default, pilot API reads still use the file-backed store. `TRACE_COMMONS_DB_TENANT_POLICY_READS=true` switches submission and export policy lookup to DB-backed `trace_tenant_policies`; combine it with `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY=true` to fail closed when no tenant policy exists. `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true` requires DB dual-write and makes trace submission, contributor credit reads, contributor credit-event reads, and contributor submission-status sync fail closed unless `trace_tenant_access_grants` contains an active exact-role row for the authenticated tenant/principal; any grant allow-lists narrow the effective consent/use policy before envelope validation proceeds. Admin-token reads and writes through `/v1/admin/tenant-policy` append hash-chained file audit events and mirror safe DB audit metadata with policy version, allow-list counts, and a policy projection hash. Admin-token reads, creates, and revocations through `/v1/admin/tenant-access-grants` are tenant-scoped and mirror safe DB audit metadata with the action, role/status, allow-list counts, and a grant projection hash. Admin-token reads through `/v1/admin/config-status` expose only safe cutover booleans, schema version, DB/object-store configured status, configured legal-hold policy IDs, ranking calibration max-age hours, ranking minimum label-source count, the max export item cap, submission quota limits, export-job scheduler enablement plus bounded interval/dataset/limit fields, the object-store provider alias, object-store I/O enabled status, object-primary object-store eligibility, tenant rollout allowlist counts, and PostgreSQL Trace Commons RLS readiness counts when the DB backend can report them; the RLS status now separates policy readiness from `FORCE ROW LEVEL SECURITY` readiness and reports only table-count/name diagnostics, not row data. Set `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY=true` with `TRACE_COMMONS_DB_DUAL_WRITE=true` and `DATABASE_URL` to fail startup unless every Trace Commons table has the tenant policy installed, RLS enabled, FORCE RLS enabled, matching policy expressions, and a runtime role that does not bypass RLS. The response omits roots, tokens, paths, secrets, tenant ids, raw scheduler retry notes, row contents, and tenant policy contents while still writing a read audit event. `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true` switches `/v1/contributors/me/credit`, `/v1/contributors/me/credit-events`, and `/v1/contributors/me/submission-status` to the DB mirror; it requires DB dual-write/backfill to be configured and preserves tenant plus principal filtering. `TRACE_COMMONS_DB_REVIEWER_READS=true` switches reviewer/admin metadata reads for analytics, trace listing, quarantine queue, active-learning queue, benchmark candidate conversion, ranker candidate/pair exports, credit settlement/hold/attestation/outbox lists, ranking evidence/calibration, review decisions, and review lease claim/release state to the DB mirror. Review leases are tenant-scoped, bound to the authenticated reviewer/admin principal, may be reclaimed by the same principal or after expiration, and are cleared automatically when a trace leaves quarantine. Review decisions are allowed only for live quarantined submissions: accepted/rejected/revoked/expired/purged submissions are rejected before any envelope body is read, and approvals are blocked for aggregate-only retention classes that do not permit derived corpus artifacts. Review decisions resolve envelope bodies through active DB object refs first; compatibility mode may fall back to a file-backed body only when file metadata is still present, while DB-sourced submissions with no file metadata require an active submitted-envelope object ref and do not recreate the missing file metadata row. Set `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` with DB reviewer reads to make all DB-backed review decisions fail closed when no active submitted-envelope object ref exists. `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true` switches replay export eligibility and derived metadata selection to the DB mirror, then attempts to resolve submitted envelope bodies through a shared replay body-read policy/audit helper that verifies tenant scope, object ref ownership, artifact kind, and content hash for DB object refs, including the encrypted local artifact sidecar. Compatibility mode falls back to the file-backed envelope body if no active DB object ref exists. Set `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` with DB replay export reads to fail closed instead. `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` requires DB dual-write and makes benchmark conversion plus ranker candidate/pair exports fail closed unless every selected source has an active submitted-envelope object ref that can be tenant/hash verified before artifacts, provenance, or utility credit are published. `TRACE_COMMONS_DB_AUDIT_READS=true` switches `/v1/audit/events` to the DB mirror. Each global DB read/object-ref gate has a comma-separated tenant allowlist companion for canary promotion: `TRACE_COMMONS_DB_CONTRIBUTOR_READS_TENANT_IDS`, `TRACE_COMMONS_DB_REVIEWER_READS_TENANT_IDS`, `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS_TENANT_IDS`, `TRACE_COMMONS_DB_REPLAY_EXPORT_READS_TENANT_IDS`, `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS_TENANT_IDS`, `TRACE_COMMONS_DB_AUDIT_READS_TENANT_IDS`, `TRACE_COMMONS_DB_TENANT_POLICY_READS_TENANT_IDS`, and `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS_TENANT_IDS`; dependency gates must cover the same tenants before fail-closed object-ref or object-primary modes can be enabled. Maintenance reconciliation reports reader-projection parity for contributor credit/status/events, reviewer metadata, analytics, audit event counts, and replay/export manifest summaries so operators can check each read flag before promotion; it also reports file/DB credit-ledger and canonical audit-event ID gaps so operators can distinguish count parity from missing mirrored rows. For credit-bearing ranking paths, reconciliation now reports file/DB counts, missing rows, model status drift, calibration dataset registry drift, calibration report-hash drift, and worker-run status drift for ranking model versions, calibration dataset registry rows, feature/prediction/label evidence, calibration runs, and ranking worker runs. Reconciliation responses include `blocking_gaps`, a compact machine-readable list of promotion blockers. Set `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` after dual-write/backfill parity looks stable to reject maintenance requests that omit `reconcile_db_mirror: true` with `400 Bad Request` and to fail closed with `409 Conflict` when reconciliation still reports promotion-blocking gaps. When that clean-reconciliation gate is enabled, global DB reader flags and their tenant allowlists also require `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` so a clean reconciliation cannot drift immediately through best-effort mirror writes. Requests that ask for reconciliation without a configured DB mirror return `503 Service Unavailable`. Audit-chain verification also includes a DB mirror report that checks previous-hash continuity, recomputes hashes for canonical mirrored payloads, and compares DB action/metadata projections against those payloads; derived diagnostics compare file/DB presence, status, canonical-summary hashes, and active derived rows attached to invalid sources; export diagnostics split DB manifests into replay, benchmark, ranker, and other counts, flag manifest items missing source object refs, and report active export manifests/items still referencing invalid sources; object-ref diagnostics distinguish missing/unreadable bodies from content-hash integrity mismatches; and vector diagnostics flag accepted/current canonical summaries that still need active vector entries.

Maintenance reconciliation also reports
`ranking_calibration_dataset_manifest_conflict_keys` for legacy file-backed
holdout registries whose history contains conflicting manifest/count metadata
for one `(calibration_dataset_hash, target_use, policy_version)` key.
Admin dataset-readiness, ranking-credit readiness, and operational-summary
routes project the same legacy conflict class into safe aggregate
`calibration_dataset_manifest_conflict_count` fields and
`ranking_calibration_dataset_manifest_conflicts` promotion-gate blockers, so
operators can hold credit issuance without exposing the underlying manifest
history. Operational summary also projects PostgreSQL RLS production readiness
into aggregate-only promotion-gate fields, ranking backtest pass/fail counts,
label-adjudication issue counts, and reason-code totals into the ranking block
and promotion gates, and it includes a `rollout_smoke` preflight block that
lists required canary smoke checks, including `tenant_canary_isolation`,
`db_reconciliation_clean`, `rollback_flag_drill`, `key_rotation_drill`,
revocation propagation, retention dry-run, PostgreSQL RLS readiness, and
audit-chain verification, while reporting recorded, passed, failed, stale, and
missing rehearsal evidence separately from promotion-gate readiness.
Operators inspect and capture that evidence with admin-only `GET` and `POST`
`/v1/admin/rollout-smoke/evidence`; `GET` can collapse history to the latest
record per check with `latest_only=true`, and writes name one required check, a
`passed` or `failed` status, a sha256-prefixed evidence hash, and an optional
external reference that is stored only as a hash in the tenant audit chain.
`POST /v1/admin/canary-read-drill` turns an existing canary submission into
hash-only evidence for `submit_status`, `tenant_canary_isolation`,
`contributor_credit`, `reviewer_metadata`, `replay_export_selection`, and
`audit_reads`. Operators provide the canary `submission_id` plus a fallback
tenant id for isolation; the response exposes only aggregate booleans, counts,
blocker codes, and hashes before optionally appending one rollout-smoke evidence
row per check.
`POST /v1/admin/object-primary-read-drill` proves the object-primary read path
for an existing canary submission: submitted-envelope object refs must be
service-owned and tenant/hash readable, review and replay body reads must resolve
through object refs, plaintext submitted body files must be absent, and a
fallback tenant must remain outside object-primary rollout. The response returns
only readiness booleans, blocker codes, hashes, and optional `object_primary_reads`
rollout-smoke evidence.
`POST /v1/admin/db-reconciliation-drill` runs the clean-reconciliation smoke
check without maintenance side effects: it requires a DB mirror, reuses the
file-vs-DB reconciliation engine, returns safe aggregate counts plus compact
blocker codes, and can append `db_reconciliation_clean` rollout-smoke evidence.
`POST /v1/admin/rollback-drill` runs the concrete rollback smoke check against
the current tenant: it requires a DB mirror, compares file-backed and DB-backed
submission, audit, and tombstone ids without deleting or rewriting either side,
returns safe aggregate counts plus compact blocker codes, and can append the
`rollback_flag_drill` rollout-smoke evidence row with only hashes.
`POST /v1/admin/key-rotation-drill` runs the matching managed-EdDSA rotation
smoke check: it reports only safe keyset counts and refresh-health booleans,
requires production-shaped managed EdDSA enforcement with at least two active
managed keys, issuer/audience/JTI/TTL policy, and a fresh guarded refresh
window, and can append `key_rotation_drill` evidence without exposing key ids,
PEMs, hosts, URLs, or bearer fetch credentials.
`POST /v1/admin/postgres-rls-drill` runs the PostgreSQL RLS readiness smoke
check from the same safe diagnostics exposed by config status. It returns only
aggregate policy/RLS/FORCE RLS counts, role-bypass state, and compact blocker
codes, and can append `postgres_rls_readiness` evidence.
`POST /v1/admin/retention-dry-run-drill` runs the retention/cache selection
smoke check through the real maintenance dry-run path with backfill, vector
indexing, reconciliation, and audit-chain verification disabled. It returns
tenant-scoped aggregate candidate counts, dry-run deletion guards, compact
blocker codes, and can append `retention_dry_run` evidence without changing
trace status or deleting objects.
`POST /v1/admin/revocation-propagation-drill` runs the revocation-propagation
smoke check through the real worker dry-run path. It requires a DB mirror,
returns only aggregate due/completed/failed/skipped/pending counts plus compact
dry-run blocker codes, leaves propagation rows unclaimed, and can append
`revocation_propagation` evidence.
`POST /v1/admin/revocation-effects-drill` is the post-live canary proof for the
same revoked submission: it reads DB propagation rows, reversed credit events,
NEAR reversal outbox rows, deleted service-owned object refs, and physical-delete
receipt rows, then can append hash-only `delayed_credit_reversal` and
`object_deletion_refs` evidence without exposing trace bodies, credit-account
refs, object keys, or raw operator reasons.
`POST /v1/admin/audit-chain-drill` runs audit-chain verification without
maintenance side effects and can append `audit_chain_verification` evidence;
responses expose counts, last hashes, blocker codes, and hashes of verifier
failures rather than raw audit failure text.
`GET /v1/admin/ranking/calibration-dataset-conflicts` provides the
operator drill-down: exact conflict keys, latest projected hash-only registry
metadata, whether the latest row still blocks credit issuance, and a remediation
hint to register a new calibration dataset hash or policy version instead of
rewriting the conflicted key. `POST
/v1/admin/ranking/calibration-dataset-conflicts/quarantine` appends an
`archived` status row using the latest projected manifest metadata for a
conflicted key; that retires the key from active manifest-conflict blockers
without rewriting the old rows, while any model still depending on the retired
holdout remains blocked by `calibration_dataset_retired`. When strict DB mirror
writes are enabled and the DB already has older immutable manifest metadata for
that key, quarantine mirrors a status-only archive update so the DB preserves its
existing manifest fields instead of accepting a manifest rewrite. The route also
appends a `ranking_calibration_dataset_quarantine` audit event with hash-only
conflict-key and operator-reason metadata.

Ranking worker-run ledger rows are part of the ranking DB mirror, the
DB-backed reviewer ranking read surface, and the maintenance reconciliation
promotion gate.

`GET /v1/admin/config-status` also reports the server-owned ranking calibration
and automation gates: `ranking_min_label_count`,
`ranking_require_calibration_dataset_registry`,
`ranking_min_confidence_threshold`,
`ranking_max_average_absolute_error_micros`, and
`ranking_worker_run_stale_after_hours`. It also reports safe NEAR settlement
readiness fields: `near_credit_submitter_configured`,
`near_credit_submitter_timeout_ms`, `near_credit_outbox_submit_default_limit`,
`near_credit_outbox_submit_max_limit`,
`near_credit_confirmer_configured`,
`near_credit_confirmer_timeout_ms`,
`near_credit_outbox_confirm_default_limit`,
`near_credit_outbox_confirm_max_limit`,
`benchmark_registry_submitter_configured`,
`benchmark_registry_submitter_timeout_ms`,
`benchmark_registry_outbox_submit_default_limit`,
`benchmark_registry_outbox_submit_max_limit`,
`benchmark_registry_confirmer_configured`,
`benchmark_registry_confirmer_timeout_ms`,
`benchmark_registry_outbox_confirm_default_limit`,
`benchmark_registry_outbox_confirm_max_limit`,
`benchmark_evaluator_configured`,
`benchmark_evaluator_timeout_ms`, and
`credit_cycle_worker_step_count`.
This safe projection lets operators confirm workers cannot lower the
sample-size or quality requirements below production credit policy and can see
whether live NEAR submission/confirmation, benchmark evaluator, or benchmark-registry submission is wired without exposing the
relayer/adapter URL, bearer token, hosts, or account identities.

Stored envelope body reads now also validate any decoded contributor tenant scope that uses the server `tenant_sha256:<hash>` shape against the authenticated tenant for both file-backed and object-ref-backed reads, so a tampered server tenant ref cannot ride behind otherwise valid metadata while legacy client attribution strings remain non-authoritative.

Maintenance parity for credit settlement is first-class: backfill counts and mirrors file-backed utility attestations, credit settlement batches, credit holds, and NEAR credit outbox rows/status, while reconciliation reports file/DB counts, missing-id gaps, settlement/outbox status drift, and hold-release drift for each family before credit settlement read flags or required mirror writes are promoted. Ranking parity uses the same maintenance channel: backfill counts and mirrors file-backed ranking model versions, calibration dataset registry rows, feature/prediction/label evidence, calibration runs, and worker-run rows, while reconciliation reports file/DB counts, missing rows, model status drift, calibration dataset status drift, calibration report-hash drift, and worker-run status drift before DB reviewer ranking reads or credit-bearing ranking paths are promoted. Benchmark registry parity follows the same operator contract: published and revoked benchmark artifacts enqueue hash-only registry publish/revoke outbox rows, backfill mirrors those rows/status into PostgreSQL, and reconciliation treats missing registry outbox rows or status/receipt drift as blockers before external registry adapter readiness is promoted.

Set `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` during production cutover after DB dual-write parity checks pass. It requires `TRACE_COMMONS_DB_DUAL_WRITE=true` and makes submission, revocation, review decision, credit, utility attestation, credit settlement, credit hold, NEAR credit outbox, benchmark registry outbox, ranking evidence/calibration, replay export manifest, benchmark/ranker provenance, audit, and trace-content-read mirror failures return an internal error instead of silently continuing with file-only state. Submission, delayed-credit, credit settlement control-plane, benchmark registry outbox, ranking evidence/calibration, replay export manifest, benchmark provenance, benchmark lifecycle, and ranker provenance mirror failures also avoid publishing local file-side metadata/object, ledger, settlement/outbox JSONL records, ranking JSONL evidence, replay-manifest, artifact/provenance, or staged service-local encrypted artifact objects for the failed operation. If a final export audit mirror fails after export metadata was staged, required mode removes the replay/benchmark/ranker file artifacts and deletes the DB export manifest/items/object refs for that export. Replay, benchmark, and ranker export DB mirrors write manifest metadata, export artifact object refs, and manifest item snapshots through one backend transaction so an item/reference validation failure does not leave a partial DB export.

Required mirror mode also applies to ranking worker-run ledger writes, so failed
DB run accounting blocks the worker response instead of leaving automation
history file-only.

`TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` optionally sets a
per-account live-settlement cap in credit points. It defaults to unset for pilot
compatibility; `0` also disables the cap. When configured, dry-run and live
settlement responses report the cap in micros plus aggregate
`settlement_policy_excluded_*` counts. Any account whose currently pending
settlement-eligible positive credit exceeds the cap is excluded from that run
without creating settlement batches or NEAR outbox rows, leaving the pending
ledger visible for hold/review or a higher governed cap.
Admins can inspect `GET /v1/admin/credit-risk-summary` before issuing credit to
see tenant-scoped pending, held, and over-cap totals grouped by deterministic
credit-account hash. The response is bounded by `limit` (default 100, max 500)
and returns `account_count`, `returned_account_count`, and `truncated` so
operators can tell whether more hashed accounts remain. It omits raw account
refs, source event ids, external refs, hold reason details, and trace bodies.

`TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES` is a comma-separated list of validated central retention policy IDs. Matching submissions are not newly expired or purged by maintenance even when `expires_at` or `purge_expired_before` would otherwise match, but the hold is honored only when the stored policy id matches the server-derived retention class for the submission's allowed uses. Review and maintenance also fail closed if file-backed metadata extends `expires_at` beyond that server-derived retention window. `/v1/admin/config-status` exposes the configured IDs for admin verification.

`TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` optionally sets a fail-closed
freshness window for model-derived ranking credit. When set, ranking model
promotion, `/v1/workers/ranking/prediction-credit`, and ranking-utility
settlement all require the latest matching promotable calibration run to be no
older than the configured hour count. This lets production credit issuers stop
minting or settling credits from stale ranking models while preserving the
file-backed pilot default when unset.

`TRACE_COMMONS_RANKING_MIN_LABEL_COUNT` sets the server-owned minimum joined
prediction/label count that a calibration run must meet before it can promote a
model or back ranking-derived credit. Calibration workers may request a higher
run-specific floor, but they cannot lower the deployment floor. It defaults to
`1` for pilot compatibility and accepts `1` through `1000000`; production credit
issuers should raise it to match the smallest calibration cohort they are
willing to treat as credit-bearing evidence.

`TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD` sets a server-owned floor for
the per-prediction confidence threshold used by calibration runs. Calibration
workers may request a higher threshold, but not a lower one. It defaults to `0`
for pilot compatibility and accepts `0` through `1`.

`TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS` sets a server-owned
ceiling for acceptable aggregate and per-label-source calibration error. The
effective calibration threshold is the lower of the worker request and this
deployment ceiling. It defaults to `1000000` micros and accepts `0` through
`1000000000000`.

`TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT` sets the minimum distinct label
source count that a joined calibration run needs before it can promote a model
or back ranking-derived credit. It defaults to `1` for pilot compatibility and
accepts `1` through `4`, matching the current `frontier_lab`, `reviewer`,
`benchmark`, and `system` label source taxonomy. Production credit issuers
should raise it, typically to `2`, so one lab or reviewer source cannot
unilaterally calibrate a credit-bearing ranking model.

`TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT` sets the minimum joined
pairwise preference-label count required by the active model-risk report before
ranking-derived credit is considered risk-clear. It defaults to `0` for pilot
compatibility and accepts `0` through `1000000`; production issuers can raise it
once pairwise reviewer/frontier-lab feedback is part of the credit-safety bar.

`TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS` sets the minimum fraction
of joined pairwise preferences that an active model must order correctly,
encoded in micros. It defaults to `500000` (chance for binary preferences) and
accepts `0` through `1000000`; active model-risk reports emit
`pairwise_accuracy_below_threshold` when joined pairwise evidence exists but the
model falls below this deployment threshold.

`TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` optionally suppresses aggregate analytics cells whose count is below the configured threshold. The endpoint still returns content-free totals and reports `min_cell_count` plus `suppressed_cell_count` for compatibility, and also returns a `privacy_budget` object with the `k_anonymity_min_cell` strategy, released/suppressed cell counts, whether suppression was applied, and conservative broad-release blocker reasons such as `min_cell_count_disabled` or `small_cells_suppressed`. Reviewers/admins can request `GET /v1/analytics/summary?release_scope=broad` as a publication preflight; the route fails closed with safe blocker reason codes when the privacy budget is not broad-release ready.

`TRACE_COMMONS_ARTIFACT_KEY_HEX` enables encrypted trace object storage. `TRACE_COMMONS_ENCRYPTED_ARTIFACTS=true` can be used as an explicit guard for the legacy encrypted artifact sidecar, but still requires the key. `TRACE_COMMONS_OBJECT_STORE=local_service` selects the production-shaped service-owned local backend and records DB object refs with the `trace_commons_service_local_encrypted` provider alias. That mode uses `TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR` when set, otherwise `TRACE_COMMONS_ARTIFACT_DIR`, otherwise `TRACE_COMMONS_DATA_DIR/service_object_store`. `TRACE_COMMONS_OBJECT_STORE=remote_service` with `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=file_system` selects the filesystem-backed service-owned remote adapter, records DB object refs with the `trace_commons_service_owned_remote` provider alias, and uses an absolute `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET` path as the remote object root; AWS/GCS/Azure selections remain fail-closed behind `trace_commons_service_owned_remote_disabled`. In encrypted service-owned modes, submitted redacted envelopes, benchmark conversion artifacts, and ranker export provenance manifests are encrypted with TraceDAO secrets crypto, stored under tenant-hashed or tenant/submission-hashed artifact paths, and referenced by DB object refs. File-backed submission records retain envelope receipts so envelope reads resolve through encrypted object storage when present; benchmark/ranker export manifest items carry per-source object refs to the shared export artifact or provenance object. `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` is a production-shaped submit/review cutover guard: it requires DB dual-write, required DB mirror writes, DB reviewer reads, reviewer object-ref reads, and an enabled service-owned object store (`local_service` or filesystem `remote_service`), then omits the plaintext submitted/reviewed envelope body files while still writing compatibility metadata, derived records, and file audit rows. `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true` is the replay-export companion guard: it requires DB dual-write, required DB mirror writes, DB replay export reads, replay object-ref-required reads, and an enabled service-owned object store, then makes replay body exports use the existing DB object-ref path without file fallback. `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` is the benchmark/ranker companion guard: it requires DB dual-write, required DB mirror writes, DB reviewer reads, required derived source object refs, export guardrails, and an enabled service-owned object store, then stores benchmark artifacts and ranker provenance only in encrypted object storage while keeping DB manifest/items as the durable index for purpose filters and lifecycle invalidation. The object-primary guards also accept `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW_TENANT_IDS`, `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT_TENANT_IDS`, and `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS_TENANT_IDS` for tenant-by-tenant promotion behind the same dependency checks.

Then opt a client into that endpoint:

```bash
export IRONCLAW_TRACE_SUBMIT_TOKEN='dev-token-a'

ironclaw traces opt-in \
  --endpoint http://127.0.0.1:3907/v1/traces \
  --scope debugging-evaluation
```

The service exposes:

- `GET /health`
- `POST /v1/traces`
- `GET /v1/traces` with reviewer filters for status, privacy risk, consent scope, derived tool/tag metadata, and export/provenance `purpose`
- `DELETE /v1/traces` with `{ "submission_id": "..." }`
- `DELETE /v1/traces/{submission_id}`
- `POST /v1/traces/{submission_id}/revoke`
- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `POST /v1/contributors/me/submission-status`
- `GET /v1/analytics/summary` with optional `release_scope=operator|broad`
- `GET /v1/review/quarantine`
- `GET /v1/review/active-learning`
- `GET /v1/review/routing-summary`
- `POST /v1/review/batch-decisions`
- `POST /v1/review/{submission_id}/decision`
- `POST /v1/review/{submission_id}/lease`
- `DELETE /v1/review/{submission_id}/lease`
- `POST /v1/review/{submission_id}/credit-events`
- `GET /v1/datasets/replay`
- `GET /v1/datasets/replay/manifests`
- `POST /v1/benchmarks/convert`
- `POST /v1/benchmarks/{conversion_id}/lifecycle`
- `POST /v1/workers/benchmark-convert`
- `POST /v1/workers/benchmark-evaluations/run`
- `POST /v1/workers/benchmark-registry-publications/run`
- `GET /v1/admin/benchmark-registry-outbox`
- `POST /v1/workers/benchmark-registry-outbox/submit`
- `POST /v1/workers/benchmark-registry-outbox/confirm`
- `POST /v1/workers/benchmark-registry-outbox/mark-status`
- `GET|POST /v1/workers/replay-export`
- `GET|POST /v1/workers/ranker/training-candidates`
- `GET|POST /v1/workers/ranker/training-pairs`
- `POST /v1/workers/utility-credit`
- `POST /v1/workers/utility-attestations`
- `GET /v1/admin/credit-attestations`
- `GET|POST /v1/admin/credit-holds`
- `GET /v1/admin/credit-risk-summary` with optional `limit`
- `GET|POST /v1/admin/credit-settlements`
- `POST /v1/workers/credit-settlements/run`
- `POST /v1/workers/credit-cycle/run`
- `POST /v1/workers/credit-cycle/scheduler/run`
- `GET /v1/admin/near-credit-outbox`
- `POST /v1/workers/near-credit-outbox/submit`
- `POST /v1/workers/near-credit-outbox/confirm`
- `POST /v1/workers/near-credit-outbox/mark-status`
- `GET|POST /v1/admin/ranking/model-versions`
- `GET|POST /v1/admin/ranking/calibration-datasets`
- `POST /v1/admin/ranking/model-promotions`
- `GET /v1/admin/ranking/features`
- `GET /v1/admin/ranking/predictions`
- `GET /v1/admin/ranking/labels`
- `GET /v1/admin/ranking/preference-labels`
- `GET /v1/admin/ranking/adjudication-report`
- `GET /v1/admin/ranking/labeler-reliability-report`
- `GET /v1/admin/ranking/calibration-report`
- `GET /v1/admin/ranking/pairwise-evaluation-report`
- `GET /v1/admin/ranking/model-backtest-report`
- `GET /v1/admin/ranking/model-risk-report`
- `GET /v1/admin/ranking/dataset-readiness-report`
- `GET /v1/admin/ranking/credit-readiness-report`
- `GET /v1/admin/ranking/worker-runs`
- `POST /v1/admin/ranking/worker-runs/{ranking_worker_run_id}/recover-stale`
- `GET /v1/admin/ranking/calibration-runs`
- `POST /v1/workers/ranking/features`
- `POST /v1/workers/ranking/predictions`
- `POST /v1/workers/ranking/prediction-credit`
- `POST /v1/workers/ranking/prediction-credit/run`
- `POST /v1/workers/ranking/model-promotions/run`
- `POST /v1/workers/ranking/labels`
- `POST /v1/workers/ranking/preference-labels`
- `POST /v1/workers/ranking/calibration-runs`
- `POST /v1/workers/ranking/calibration-runs/run`
- `GET /v1/ranker/training-candidates`
- `GET /v1/ranker/training-pairs`
- `GET|POST|PUT /v1/admin/tenant-policy`
- `GET /v1/admin/export/access-grants`
- `GET /v1/admin/export/jobs`
- `POST /v1/admin/export/jobs/{export_job_id}/recover-stale`
- `POST /v1/admin/export/jobs/{export_job_id}/retry`
- `POST /v1/workers/export/jobs/claim-next`
- `POST /v1/workers/export/jobs/claim-and-run`
- `POST /v1/workers/export/jobs/run-queued`
- `POST /v1/workers/export/jobs/retry-failed`
- `GET /v1/admin/config-status`
- `GET /v1/admin/vector-entries`
- `GET|POST /v1/admin/rollout-smoke/evidence`
- `POST /v1/admin/canary-read-drill`
- `POST /v1/admin/object-primary-read-drill`
- `POST /v1/admin/db-reconciliation-drill`
- `POST /v1/admin/rollback-drill`
- `POST /v1/admin/key-rotation-drill`
- `POST /v1/admin/postgres-rls-drill`
- `POST /v1/admin/retention-dry-run-drill`
- `POST /v1/admin/revocation-propagation-drill`
- `POST /v1/admin/revocation-effects-drill`
- `POST /v1/admin/audit-chain-drill`
- `POST /v1/admin/maintenance`
- `POST /v1/workers/retention-maintenance`
- `POST /v1/workers/revocation-propagation`
- `POST /v1/workers/vector-index`
- `GET /v1/audit/events`

`GET /v1/audit/events` accepts an optional `limit` query parameter, defaults to 100 events, and caps responses at 500 events. Reviewer audit reads use that limit at the storage boundary: file-backed reads parse only the latest audit-log tail needed for the response, while DB-backed reads use a tenant-scoped latest-events query ordered by audit sequence.

The ingestion service treats every upload as untrusted. It validates the schema and revocable consent, re-runs deterministic redaction on the submitted envelope, recomputes privacy hashes and credit estimates, enforces optional per-tenant/per-principal hourly contributor submission quotas for autonomous upload safety, stores accepted low-risk traces under the authenticated tenant, and quarantines medium/high-risk traces with zero immediate credit. Reviewer quarantine and active-learning queues are prioritized work queues: queue items expose `review_age_hours`, `review_escalation_state`, `review_escalation_reasons`, and optional DB-backed review lease metadata so operators can filter by `lease_filter=all|mine|available|active|expired`, then sort by reviewer SLA pressure, assignment, and escalation causes without opening trace bodies. `GET /v1/review/routing-summary` gives reviewers/admins aggregate routing pressure, available/active/expired/mine counts, privacy-risk and escalation buckets, and hash-only assignee load counts without exposing trace bodies or raw reviewer principals. Reviewers/admins can apply a bounded common approve/reject decision to up to 50 submission ids through `POST /v1/review/batch-decisions`; each item still runs the same single-submission eligibility, lease, ABAC, body-read, object-ref, mirror, and audit path as `POST /v1/review/{submission_id}/decision`, and the response reports per-submission success or safe error metadata. Revocation writes a tenant-scoped, first-writer-wins tombstone with redaction and canonical-summary hashes when available, marks local or DB-backed metadata revoked only for authorized owners/reviewers, lifecycle-revokes any published benchmark artifact derived from the source, and blocks later uploads in the same tenant that match a retained file-backed or DB-mirrored tombstone redaction hash, canonical-summary hash, or submission id.

Replay dataset exports, benchmark conversion artifacts, and ranker training exports include a deterministic `sha256:` hash of their source item list. The same hash is mirrored into audit `decision_inputs_hash` for the export event, giving reviewers a stable tenant-scoped proof of which submissions fed an exported dataset without exposing trace content in the audit row. Benchmark conversion artifacts also carry a schema version plus registry/evaluation lifecycle metadata; new conversions start as registry `candidate` and evaluation `not_run`, and benchmark workers or reviewers can update registry/evaluator state through the lifecycle endpoint without changing the export contract. The lifecycle endpoint rejects `published` registry status unless the artifact also has `passed` evaluator status, registry ref, published timestamp, evaluator ref, evaluated timestamp, and evaluator score, so registry publication cannot outrun the server-side evaluator record. `POST /v1/workers/benchmark-evaluations/run` uses the configured `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` adapter when present; schedulers can set `require_external_evaluator=true` to fail closed instead of falling back to the in-process structural evaluator. The evaluator request contains bounded derived candidate summaries, artifact/source-list hashes, purpose hash, evaluator ref, and scoring threshold, but omits raw trace bodies, contributor principals, and raw source submission ids. When a lifecycle update moves an artifact to `published`, the service also writes a durable benchmark registry publish outbox row containing only ids, registry refs, source-list hashes, artifact-payload hashes, evaluator refs, scores, and status timestamps; when source revocation or retention invalidation lifecycle-revokes a previously published artifact, the service writes a matching hash-only revoke outbox row for the same registry ref. Admins can inspect these rows through `GET /v1/admin/benchmark-registry-outbox`, benchmark workers can submit pending/failed rows to a configured registry adapter through `POST /v1/workers/benchmark-registry-outbox/submit`, poll submitted rows for confirmation through `POST /v1/workers/benchmark-registry-outbox/confirm`, and use `POST /v1/workers/benchmark-registry-outbox/mark-status` as a manual fallback using external receipt refs and hashed error details. Set `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` to point at a trusted evaluator adapter; `TRACE_COMMONS_BENCHMARK_EVALUATOR_BEARER_TOKEN` adds bearer auth, and `TRACE_COMMONS_BENCHMARK_EVALUATOR_TIMEOUT_MS` bounds evaluation calls. Set `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_URL` to point at a trusted registry adapter; `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_BEARER_TOKEN` adds bearer auth, and `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_TIMEOUT_MS` bounds submit calls. Set `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_URL` to point at the adapter's confirmation endpoint; `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_BEARER_TOKEN` adds bearer auth, and `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_TIMEOUT_MS` bounds confirmation calls. Registry submitter and confirmation requests are hash-only and contain no raw benchmark artifact or trace body. The `ironclaw traces benchmark-lifecycle-update` helper posts the same registry/evaluator patch shape for worker automation and manual reviewer operations. Lifecycle updates rewrite the tenant-scoped file artifact in compatibility mode or the encrypted service-local object in object-primary mode, refresh DB object refs when the DB mirror is configured, and append an audit event with the registry/evaluation statuses; in required mirror mode the rewrite happens only after the DB mirror succeeds. Benchmark and ranker exports also write tenant-local provenance manifests with source ids, caller-supplied purpose, and invalidation fields; CLI ranker candidate/pair export commands accept `--purpose` so guarded services can require it. Export workers should prefer the dedicated `/v1/workers/replay-export`, `/v1/workers/ranker/training-candidates`, and `/v1/workers/ranker/training-pairs` routes, which reuse the same tenant policy, consent/use filtering, access-grant, export-job, audit, and source-hash behavior as the reviewer/admin export routes while keeping automation on worker-scoped endpoints. Admins can inspect the durable grant/job control-plane rows through `GET /v1/admin/export/access-grants` and `GET /v1/admin/export/jobs`, with bounded status and dataset-kind filters, or the matching `ironclaw traces export-access-grants-list` and `export-jobs-list` helpers. Revocation and retention maintenance mark those provenance manifests invalid instead of deleting them. Benchmark conversion plus ranker candidate and pair exports revalidate DB source status immediately before publishing artifacts and append idempotent delayed utility credit events for included accepted sources, keyed by tenant plus submission so rerunning the same worker surface does not double-credit. Trusted offline utility jobs can use `POST /v1/workers/utility-credit` with `regression_catch`, `training_utility`, or carefully scoped ranking backfills, a concrete `external_ref`, and one or more accepted submission ids to append idempotent delayed credit without exposing reviewer-only credit mutation. Model-derived ranking credit should prefer `POST /v1/workers/ranking/prediction-credit`, which takes a single `ranking_prediction_id` plus reason, requires the referenced prediction to come from the latest active matching model with a positive settlement score, and appends one idempotent `ranking_utility` credit event bound to `ranking_prediction:<uuid>`. Frontier-lab or trusted-worker evidence should prefer `POST /v1/workers/utility-attestations`, which stores a hash-only attestation record with policy version, evidence hash, external-ref hash, and source ids before appending the same idempotent pending utility credit. Ranking workers can separately write hash-only feature records, model predictions, frontier/reviewer labels, and aggregate calibration runs for accepted submissions; prediction writes are accepted only when they bind to a registered active/candidate model, that model's policy and feature schema, and an existing feature vector hash for the same source. Admin calibration reports join predictions to labels, while persisted calibration runs record model version, target use, policy version, evaluation dataset hash, error metrics, confidence thresholds, promotion thresholds, joined label-source diversity counts, max per-source calibration error, deterministic joined-evidence hash, reason codes, and a report hash before model-derived credit is trusted; calibration runs must use the registered model calibration dataset hash, ignore mismatched legacy prediction policy/schema rows, and can fail with `insufficient_label_source_diversity` when `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT` is higher than the joined source count or `label_source_error_above_threshold` when any label-source cohort exceeds the calibration error threshold. Registered model manifests are immutable for a `model_version`: changing the feature schema, policy, training dataset, calibration dataset, or artifact hash requires a new model version, while status transitions reuse the same manifest. Admins can stage models as `candidate` before calibration, and `POST /v1/admin/ranking/model-promotions` promotes an eligible candidate to `active` only when the requested model/policy/target-use/calibration-dataset run is promotable and the latest matching evidence is still promotable; legacy mutable records with a different calibration dataset hash require a fresh matching run before promotion. Ranking prediction-credit and utility settlement use the same current-model calibration dataset gate; ranking utility settlement also requires the named model's latest version to be `active` and to match the settlement policy, with a latest promotable calibration run for the requested target use; each ranking utility credit event must reference a matching `ranking_prediction:<uuid>` whose source, model, policy, target use, and settlement score match the credit event. Admins then run `POST /v1/admin/credit-settlements` in dry-run or final mode to convert eligible pending utility events into non-transferable settled account credit; utility workers can use the narrower `POST /v1/workers/credit-settlements/run` route with the same explicit policy version, optional ranking model/target-use gate, optional NEAR contract id, and idempotent retry behavior without gaining generic admin settlement access. Active credit holds exclude accounts from settlement and move eligible pending points into the held projection. Contributor credit summaries keep settlement and hold projections scoped to the contributor principal, while reviewer/admin credit summaries include tenant-wide settlement line items they are authorized to read. When a settlement request includes a NEAR contract id, the service writes a durable NEAR outbox item for each settled account, using only batch ids, account hashes, source-list hashes, policy versions, amounts, and issuer-signature hashes; if a retry finds a finalized batch whose outbox item is missing, it reconstructs the same hash-only pending outbox item before skipping already-settled source events. Set `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` to point at a trusted NEAR relayer; utility workers can then call `POST /v1/workers/near-credit-outbox/submit` to submit pending outbox payloads, record public transaction hashes, mark relayer failures with hashed error details, and append a bounded audit event. `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_BEARER_TOKEN` adds relayer bearer auth, and `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_TIMEOUT_MS` bounds submit calls. The manual `POST /v1/workers/near-credit-outbox/mark-status` route remains available for confirmation, recovery, and operator fallback. Tenant policy allowed-use ABAC is applied before exports and process-evaluation labeling, then again while selecting sources, so replay exports, benchmark conversion, ranker exports, process-evaluation workers, utility-credit workers, utility-attestation workers, ranking-evidence workers, ranking prediction-credit workers, and settlement workers cannot publish, label, rank, credit, or settle sources outside the tenant's consent/use allow-list. Signed-token allow-lists are enforced on the same downstream paths: replay exports, benchmark/ranker dataset generation, process-evaluation workers, utility-credit workers, utility-attestation workers, ranking-evidence workers, and ranking prediction-credit workers cannot consume sources outside the caller claim's allowed scopes/uses. Production-like deployments can also require active submitted-envelope object refs for every benchmark/ranker source so stale or unreadable trace bodies fail before publishing derived artifacts. Replay dataset exports mirror compact DB manifest rows plus per-source item snapshots with source status, hash, and source object ref at export time; benchmark and ranker item rows name the derived summary artifact version used and now carry per-source object refs to the benchmark artifact or ranker provenance payload. In file-backed mode those refs point at the local JSON artifact; in service-local encrypted mode they point at the tenant-checked encrypted object-store payload. Manifest metadata, export artifact object refs, and item snapshots are mirrored transactionally in PostgreSQL. Manifest and item rows are invalidated when any source submission is revoked, expired, or purged. Reviewer/admin tokens can inspect replay export manifest metadata through `GET /v1/datasets/replay/manifests`; DB-backed listing is scoped to replay dataset manifests and filters out benchmark/ranker provenance rows.

`GET /v1/admin/credit-risk-summary` gives admins a safe bounded pre-issuance
view of pending, held, and over-cap credit grouped by account hash, so operators
can review capped or held accounts without exposing raw account refs, event ids,
external refs, or hold details. The optional `limit` defaults to 100 accounts
and is capped at 500; aggregate totals are computed before account-list
truncation.

Export job rows preserve a `trace_export_job_request.v1` metadata snapshot
across start, completion, and failure with requested/effective limits,
status/privacy/consent filters, and only hashed external refs for later
replayable worker execution. `POST /v1/workers/export/jobs/claim-next` lets an
export worker atomically claim the oldest unexpired queued job for the current
tenant and optional dataset-kind filter, marking it `running` without reading
trace bodies. `POST /v1/workers/export/jobs/claim-and-run` supports queued
replay, benchmark-conversion, ranker-candidate, and ranker-pair jobs: it claims
the next queued job for that dataset kind, reconstructs
status/privacy/consent/limit filters from the safe metadata snapshot, runs the
existing export/artifact/provenance path, and marks the same job row complete or
failed instead of creating a second job. `POST /v1/workers/export/jobs/run-queued`
is the bounded scheduler route: it claims up to `max_jobs` queued rows,
optionally filtered by dataset kind, continues after per-job execution failures
by terminalizing failed rows, and returns safe completed/failed/pending counts
plus job summaries without embedding exported trace bodies. The
`/v1/workers/export/jobs/retry-failed` worker route is the bounded retry pass: it
scans failed unexpired jobs with replayable request metadata, applies retry-count
and exponential-delay bounds, requeues due rows with hash-only retry metadata,
and reports retried/not-due/ineligible/remaining-failed counts. Deployments that
want the server to own that loop can configure
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_TOKEN` with an export-worker bearer token;
the optional in-process scheduler sleeps for
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_INTERVAL_SECONDS` (default 60), runs the same
`retry-failed` pass, then runs the same `run-queued` pass. The scheduler accepts
the same optional dataset-kind narrowing and bounded controls through
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_DATASET_KIND`,
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RUN_QUEUED_MAX_JOBS`,
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_JOBS`,
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_RETRY_COUNT`,
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_BASE_DELAY_SECONDS`, and
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_MAX_DELAY_SECONDS`; the raw token and
retry note are never logged, and retry audit rows keep only reason hashes.

Deployments that want the server to own vector-index catch-up can configure
`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` with a vector-worker bearer token.
Startup validates that the token can call the vector worker route and that the
DB mirror is configured, then the in-process scheduler sleeps for
`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_INTERVAL_SECONDS` (default 60) before
calling `POST /v1/workers/vector-index` with the same bounded worker semantics.
`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_LIMIT` caps each pass from 1 to 500,
`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_DRY_RUN=true` keeps the scheduler in
count-only mode, and `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_PURPOSE` sets the
audited worker purpose. Config status exposes only scheduler enablement,
interval, limit, and dry-run status; it never returns the worker token or raw
purpose text.

Stale export-job recovery is intentionally narrow. If a `running` export job is
still open after its grant expiry, an admin can call
`POST /v1/admin/export/jobs/{export_job_id}/recover-stale` with a non-empty
reason to atomically mark only that stale running row `expired`. The response,
DB row, and audit metadata store only the job id, recovered status, reason hash,
and safe recovery markers, never the raw operator note or trace bodies. The admin
retry route (`POST /v1/admin/export/jobs/{export_job_id}/retry`) only requeues
failed jobs while the job is still unexpired and still carries replayable
`trace_export_job_request.v1` metadata. Retry clears terminal execution fields,
returns the row to `queued`, increments a safe retry counter, records only reason
hashes and admin principal refs, and then lets worker schedulers claim the job
normally.

Ranking model manifests must keep training and calibration dataset hashes
disjoint. New overlapping manifests are rejected, and legacy overlapping
manifests are blocked by model promotion, prediction-credit, settlement, and
active-model risk gates under `training_calibration_dataset_overlap`, so the
registered calibration dataset acts as holdout evidence rather than a reused
training split.

Admins can manage the holdout registry through
`GET|POST /v1/admin/ranking/calibration-datasets`. Registry rows are hash-only:
they store calibration dataset hash, target use, policy version, source-list
manifest hash, source count, label-source count, label-actor count, lifecycle
status, and a hashed actor principal. They do not store trace bodies, reviewer
notes, raw lab refs, or source ids.
For an existing `(calibration_dataset_hash, target_use, policy_version)` holdout
key, status-only lifecycle updates are append-only but must keep the source
manifest hash, source count, label-source count, and label-actor count unchanged.
Manifest or count changes require a new dataset hash or policy version, and both
file-backed writes and PostgreSQL mirror writes reject those rewrites.
File-backed readers also fail closed if legacy JSONL history contains conflicting
manifest rows for one holdout key.
Calibration runs may use matching registry rows in `candidate` or `active`
status, but reject matching `deprecated` or `archived` rows so retired holdout
sets cannot keep model evidence alive after stewardship review.

Calibration sample counts use effective labels, not raw label rows. If the same
label source writes multiple labels for the same submission and target use, the
latest `(submission_id, target_use, label_source)` row replaces older rows for
joined evidence, aggregate error, label-source diversity, and promotion
thresholds. A latest `disputed` label removes that source from joined
calibration evidence until a newer non-disputed label arrives, so challenge
labels cannot keep a model promotable by carrying the older score forward.
Different label sources on the same submission still count as distinct evidence,
but the write path binds each label-source enum to an authority role: utility
workers may write `frontier_lab`, reviewer/admin tokens may write `reviewer`,
benchmark workers may write `benchmark`, process-evaluation workers may write
`system`, and admins may override for controlled repairs.
However, when `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT` requires multiple
joined sources, those sources must also be written by at least that many distinct
actor principals; otherwise the run records
`insufficient_label_actor_diversity` and remains non-promotable.

Process-evaluation-derived ranking labels are intended as auxiliary evaluator
evidence, not direct credit authority. They participate in the same calibration,
model-risk, readiness, and settlement gates as other ranking labels.

`GET /v1/admin/ranking/adjudication-report` groups the latest absolute labels
and pairwise preferences into unresolved issue buckets for disputed labels,
cross-source absolute-label outcome conflicts, and reversed pairwise
preferences. Calibration runs record `ranking_adjudication_issues_present` and
remain non-promotable while any unresolved issue exists for the target use, so
stored model-quality evidence cannot advance credit issuance ahead of label
review. `GET /v1/admin/ranking/labeler-reliability-report` projects the same
issue participation into source-level and hashed-actor rows, including
absolute-label, preference-label, dispute, conflict, and total issue counts
without exposing raw actor principals or external refs.

Set `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` to point at the relayer's confirmation endpoint; utility workers can then call `POST /v1/workers/near-credit-outbox/confirm` to poll submitted NEAR rows, mark confirmed transactions, or record hashed terminal failures without sending raw NEAR call args. `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_BEARER_TOKEN` adds confirmation bearer auth, and `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_TIMEOUT_MS` bounds confirmation calls.

Pairwise ranking preference labels are available through `/v1/workers/ranking/preference-labels` and `/v1/admin/ranking/preference-labels`. They bind a preferred accepted submission to a rejected accepted submission for the same target use and store only evidence/external-ref hashes plus positive preference strength; they are training evidence for rankers or reward models, not a substitute for the absolute utility labels required by calibration and settlement gates. Ranker training-pair exports consume eligible preference labels before score-order heuristics, include the preference label id/source/category/strength/evidence hash, and still require both sides to pass the existing tenant policy, consent, status, privacy-risk, and revocation filters. Admin pairwise-evaluation reports compare latest candidate/active model settlement scores against those preference labels and return joined-pair counts, correct/reversed/tied ordering counts, accuracy in micros, and preferred-score margin aggregates without raw external refs or trace bodies. Active model-risk reports also expose those pairwise metrics and add `pairwise_accuracy_below_threshold` when joined pairwise evidence exists but the model orders less than half of it correctly, letting credit readiness and settlement treat preference drift as an active-model risk.

`POST /v1/workers/ranking/calibration-runs/run` lets production schedulers scan
active or candidate ranking models with a bounded `limit`, required target use,
non-empty reason, optional model/policy filters, dry-run support, and optional
calibration threshold overrides that are still capped by server-owned floors.
Live runs persist calibration records and append a hash-only worker-run ledger
row with result refs to the generated calibration ids.

`POST /v1/workers/ranking/model-promotions/run` lets production schedulers scan
latest candidate ranking models with a bounded `limit`, required target use,
non-empty reason, optional model/policy filters, and dry-run support. It reuses
the same overlapping live-run guard, calibration/freshness/diversity gates as
the admin promotion endpoint, rechecks current joined prediction/label evidence
against the latest promotable calibration run, promotes only eligible
candidates, skips ineligible candidates with safe reason counts, and reports
remaining candidate count without granting utility workers generic admin
promotion access.

Direct `active` writes to `POST /v1/admin/ranking/model-versions` are rejected:
activation is target-use scoped, so admins should register `candidate` model
manifests and activate them through `/v1/admin/ranking/model-promotions`.

`POST /v1/admin/ranking/model-promotions` can also be used as a hash-only
preflight by setting `dry_run: true`. The response includes the registered
holdout calibration dataset hash, the stored calibration run/report/evidence
hashes and joined-evidence counts, and a freshly recomputed current-evidence
snapshot with the effective server-owned thresholds, error metrics,
low-confidence prediction count, promotability flag, and reason codes. Operators
can compare these fields before activation without exposing trace bodies or raw
frontier-lab references. The promotion gate also requires the current
candidate/target backtest to pass, so pairwise evidence, ordering failures, or
unresolved label-adjudication issues from
`/v1/admin/ranking/model-backtest-report` block activation.

`GET /v1/admin/ranking/dataset-readiness-report` groups the latest ranking model
manifests by registered holdout calibration dataset hash. Each dataset row
reports candidate/active/deprecated/archived model counts, models without target
evidence, target-use readiness rows, current joined-evidence/report hashes,
effective calibration thresholds, current error metrics, and blocker reason
counts. The report is derived from the existing model, prediction, label, and
calibration ledgers, so it works in file-backed mode and DB reviewer-read mode
without exposing trace bodies or raw lab references.

Promotion and active-model risk recomputation re-apply the current server-owned
calibration floors to stored calibration evidence. Raising production thresholds
therefore blocks old calibration runs from activating models, minting prediction
credit, or settling ranking utility until fresh evidence clears the new policy.

`GET /v1/admin/ranking/worker-runs` exposes the hash-only worker-run ledger for
bounded ranking calibration, prediction-credit, model-promotion, and full
credit-cycle automation. Rows include the run id, running/completed/failed
status, dry-run flag, filters, limits,
checked/succeeded/skipped counts, result refs, and machine-readable skip reason
counts, but store only the reason hash rather than raw operator notes.
Calibration worker runs also add non-promotable persisted calibration run
reasons, such as `ranking_adjudication_issues_present`, to those counts so a
successful artifact write still surfaces credit blockers. The same ledger
coordinates scheduler retries: live non-dry-run calibration,
prediction-credit, model-promotion, and credit-cycle runs refuse active
overlapping non-stale rows before appending a new `running` entry. Admins can call
`POST /v1/admin/ranking/worker-runs/{ranking_worker_run_id}/recover-stale` with
a non-empty recovery reason to append-finalize a stale running row as `failed`;
the raw reason is hashed, a `ranking_worker_run_recovery` audit event is
appended with only the run id, run kind, recovered status, and reason hash,
fresh active runs are rejected, and the recovered row no longer contributes to
the stale-run promotion blocker.

`GET /v1/admin/ranking/model-backtest-report` recomputes the same current calibration, pairwise, and label-adjudication checks for latest candidate and active model/target-use pairs. Each row reports current joined-evidence hashes, latest calibration run/report hashes, joined label counts, aggregate and per-source error metrics, low-confidence counts, pairwise evidence and accuracy, pass/fail status, and machine-readable reason codes so operators can evaluate a candidate before promotion or credit issuance without exposing trace bodies or raw lab references.

`GET /v1/admin/ranking/model-risk-report` recomputes the current joined-evidence hash for each active model/target-use pair and reports post-calibration prediction/label counts, current joined-label source diversity, current calibration thresholds, current aggregate/per-label-source error metrics, pairwise preference evidence counts, pairwise policy thresholds, pairwise ordering accuracy, unresolved label-adjudication blockers, low-confidence fresh predictions, stale or non-promotable calibration status, training/calibration dataset overlap, evidence-hash drift, aggregate risk-code counts, and per-model machine-readable risk codes without exposing trace bodies or raw lab references.

`GET /v1/admin/ranking/credit-readiness-report` lists pending positive `ranking_utility` credit events that have not already settled and explains whether each can settle under the referenced active-model prediction. Blocked rows include machine-readable reasons such as missing prediction refs, missing or inactive models, missing/stale/non-promotable/under-diverse calibration, score mismatches, held credit accounts, low-confidence predictions, and uncleared active-model risk codes such as current evidence drift, plus the calibration run/report/joined-evidence hashes when available.

Model-derived ranking credit also applies the latest calibration run's confidence threshold and active-model risk report to each active-model prediction at issuance, readiness, and settlement time. Low-confidence or uncleared-risk predictions remain visible in admin evidence and risk reports, but `/v1/workers/ranking/prediction-credit` rejects them and settlement excludes manually appended ranking utility events that reference them.

Ranking calibration runs apply both caller-supplied thresholds and
deployment-owned floors. In production, set
`TRACE_COMMONS_RANKING_MIN_LABEL_COUNT` high enough that a small ad hoc label
set cannot promote a credit-bearing model, then layer
`TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD`,
`TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS`,
`TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT`,
`TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT`,
`TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS`, and per-source cohort
error gates on top so the sample is high-quality, broad enough across
reviewers/labs, and still agrees with pairwise preference evidence. Direct
registration of an `active` model uses the same calibration freshness and
diversity gates as the explicit model-promotion route.

Vector indexing now follows the same downstream ABAC model: tenant policy and
signed-claim allowed-use filters are enforced before the worker runs, then both
the indexed source set and nearest-neighbor pool are reduced to traces that
permit a derived vector use. Aggregate-only traces are skipped because that
retention class does not permit derived artifacts. Indexed vector worker
payloads now include a deterministic local redacted-summary feature embedding
inside encrypted `WorkerIntermediate` object payloads, while relational vector
rows retain only metadata, hashes, scores, and provenance. Revocation
propagation can invalidate a specific vector entry by id, and service-owned
vector payload objects are verified as vector artifacts before physical
deletion and receipt recording.

Vector workers should use `POST /v1/workers/vector-index` for scheduled
indexing. The worker route requires the DB mirror before it starts, accepts
`purpose`, `dry_run`, and an optional `limit` from 1 to 500, and returns
`checked_count`, `vector_entries_indexed`, `skipped_existing_count`, and
`pending_after_count`. Unlike broad admin maintenance, this route does not mark
expired/revoked records, prune export caches, write retention ledgers, or run
DB reconciliation; it only indexes eligible DB-mirrored derived summaries and
audits the vector-index pass. Deployments can use
`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` to run that same bounded worker
route in-process on an interval without granting the scheduler generic
maintenance authority.

Admin tokens can inspect safe tenant-scoped vector metadata through
`GET /v1/admin/vector-entries`. The route requires the DB mirror, supports
bounded `limit`, `status`, `source_projection`, and `submission_id` filters, and
returns vector ids, source hashes, embedding model/dimension/version,
nearest-neighbor ids, scores, cluster refs, and lifecycle timestamps. It never
returns raw trace bodies or embedding values and appends an aggregate read audit
row for the returned item count.

Shared reviewer/admin list-style reads use the same operator page limit policy:
omitted `limit` defaults to 100, and explicit limits outside 1..=500 are rejected
with a client error instead of being silently clamped.

`POST /v1/admin/maintenance` can also be used by reviewers/admins to bridge file-backed pilot data into the optional DB mirror. It marks submissions expired when their retention-policy `expires_at` has passed, mirrors expiration status plus artifact/export-manifest invalidation to the DB mirror when configured, writes a durable DB retention job row plus per-submission lifecycle item rows for mirrored expire/purge/revoke actions, repairs DB revocation/artifact invalidation for submissions that are already file-marked revoked, lifecycle-revokes published benchmark artifacts whose sources are revoked or expired and enqueues matching registry revoke outbox rows, prunes cached export payloads whose manifest references revoked or expired sources, and keeps expired traces out of replay, benchmark, and ranker exports.

Admin tokens can inspect the durable tenant-scoped retention ledger through `GET /v1/admin/retention/jobs` and `GET /v1/admin/retention/jobs/{retention_job_id}/items`; both routes require the configured DB mirror, support bounded filtered reads, and append read-audit breadcrumbs without exposing trace bodies. The matching CLI helpers are `ironclaw traces retention-jobs-list` and `ironclaw traces retention-job-items`, and the web Trace Commons operator panel exposes the same read-only job/item lookups with its session-only admin token.

Retention workers should prefer the narrower `POST /v1/workers/retention-maintenance` route, which exposes only `purpose`, `dry_run`, `prune_export_cache`, `max_export_age_hours`, and `purge_expired_before`. Revocation workers should use `POST /v1/workers/revocation-propagation`, which requires the DB mirror, lists due tenant-scoped revocation-propagation ledger items, claims them with retry attempts, applies metadata/vector/export invalidation actions idempotently, appends deterministic negative credit-ledger reversal events for settled revoked sources, enqueues `reverse_credit_receipt` NEAR outbox calls for settled sources that had a NEAR contract, verifies and deletes service-owned encrypted submitted/review envelope payloads for exact tenant/submission/object-ref targets, records a durable physical-delete receipt item with an evidence hash when service-owned deletion succeeds or was already recorded, marks unsupported physical-delete stores or artifact kinds as skipped, and audits dry-run or completed counts without reading trace bodies unless the action is a bounded object-payload delete.

Set `purge_expired_before` to an explicit RFC3339 cutoff plus a non-empty `purpose` to mark already-expired submissions purged and delete their file-backed and encrypted local artifact copies; dry-run purge previews may omit purpose. When a non-dry-run purge or revocation-propagation object-payload delete physically removes a service-owned encrypted trace body, the DB mirror marks only the matching tenant/submission/object-store/object-key object ref with `deleted_at`, while expiration and revocation continue to invalidate refs without claiming physical deletion. Revocation-propagation object deletes also upsert a tenant-scoped `PhysicalDeleteReceipt` propagation row so retries can backfill the receipt after a successful delete and reconciliation can distinguish invalidation from proven payload removal.

Set `backfill_db_mirror: true` to validate tenant-local file-backed submissions, envelopes, and derived precheck records, then mirror submissions that are not already present in the configured DB. Backfill also covers hash-only utility attestations, settlement control-plane rows, NEAR credit outbox rows/status, benchmark registry outbox rows/status, ranking evidence/calibration rows, audit rows, and replay export manifests. Backfill isolates per-submission and per-export failures so one corrupt envelope, missing derived precheck, unreadable replay manifest, or malformed control-plane row does not block valid records; the maintenance response and audit metadata include `benchmark_artifacts_invalidated` and `db_mirror_backfill_failed` plus bounded failure details.

Set `index_vectors: true` to publish deterministic canonical-summary vector metadata rows from accepted DB-mirrored derived records during a broader admin maintenance run. Non-dry-run vector indexing requires an active submitted-envelope object ref that can be read and hash-checked before the vector row is written, and mirrors the source object ref into the per-source content-read audit row.

Set `reconcile_db_mirror: true` to return a tenant-scoped report comparing file-backed metadata counts, DB object/vector/export/tombstone counts, DB retention job/item counts, the just-written retention maintenance ledger row/item count, credit-ledger and canonical audit-event ID gaps, DB canonical audit-payload projection failures, credit-settlement/NEAR/benchmark-registry/ranking control-plane drift, active submitted-envelope object ref presence/readability/hash integrity, export manifest item object-ref gaps, active derived/export rows that still point at invalid sources, reader-projection parity for contributor, reviewer metadata, analytics, audit, and replay/export manifest surfaces, and compact `blocking_gaps`. Canonical DB audit projection drift is reported in `db_audit_canonical_projection_failures`, blocks clean reconciliation, and causes audit reader parity to record a safe DB error hash rather than exposing raw row contents. With `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true`, maintenance requests that omit `reconcile_db_mirror: true` fail before maintenance side effects with `400 Bad Request`, reconciliation requests fail with `409 Conflict` until `blocking_gaps` is empty, and DB reader promotion cannot be configured without required mirror writes. Failed dirty reconciliation attempts still append the normal maintenance audit event after any completed maintenance work. Set `verify_audit_chain: true` to include a file-backed audit hash-chain integrity report plus, when a DB mirror is configured, a DB audit report with canonical-payload hash recomputation and projection checks for rows that carry `canonical_event_json`. Use `dry_run: true` to count valid backfill, benchmark artifact invalidation, or vector-index candidates without writing rows or artifacts.

DB audit hash-chain drift is reported separately in `db_audit_hash_chain_failures` for invalid hash format, genesis/predecessor mismatch, and canonical-payload hash mismatch. It is also included in `blocking_gaps`, so clean reconciliation cannot pass when DB audit rows have matching IDs but broken hash continuity or payload hashes. Ordinary reviewer audit reads also recompute the hash-chained payload for each returned bounded row when chain fields are present, and reject incomplete or mismatched hash fields before returning file-backed or DB-backed audit pages.

Reconciliation also compares a latest bounded audit-reader sample from the file log and DB mirror. Projection drift is reported as `audit_reader_sample_parity=failed` with only counts, event ids, and projection/error hashes in `audit_reader_sample_failures`, so operators can catch legacy DB audit rows whose ids and counts line up but whose reader-visible kind, status, actor, reason hash, export, or hash-chain fields do not.

For credit settlement and benchmark registry cutover, the same maintenance run also backfills and reconciles utility attestations, credit settlement batches, credit holds, NEAR credit outbox rows/status, and benchmark registry outbox rows/status. Dirty gaps or status/release/receipt drift in any of those families are promotion blockers when `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` is enabled.

Retention legal holds are service-configured by policy ID, not by trace submitters. A trace envelope can suggest a retention policy, but only the authenticated service configuration decides whether that policy is under legal hold. Review, maintenance, replay export, benchmark conversion, and ranker training source selection cross-check current server retention policy ids plus expiration timestamps against the server-derived retention class before reading trace bodies, skipping expiration, purging, or publishing derived/export outputs.

Tenant tokens can be configured as either `tenant_id:token` for contributor access or `tenant_id:role:token` where role is `contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`, `vector_worker`, `benchmark_worker`, `utility_worker`, or `process_eval_worker`. Add `;expires_at=<RFC3339>` or `;expires=<RFC3339>` to either syntax for a short-lived static-token bridge while production identity claims are still being built. The service can also accept signed HS256 tenant claims when `TRACE_COMMONS_SIGNED_TOKEN_SECRET` is set, and EdDSA/Ed25519 signed claims when an EdDSA public key is configured; these claims bind tenant id, actor/principal, role, allowed consent scopes/uses, issuer, audience, JWT `sub`, and expiry in the bearer token instead of enumerating every token in service configuration. `TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS=true` turns static tokens and HS256 signed claims into rejected bridge credentials for all authenticated routes, including admin and worker routes. `TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS=true` is stricter: it also rejects default/ad hoc public-key config and accepts only active managed-keyset EdDSA claims with recognized `kid` headers plus issuer and audience checks. When tenant access grants are required, signed-claim issuer/audience/JWT `sub` values must match any corresponding grant fields before scopes/uses are applied; static-token bridge grants ignore these signed-claim-only binding fields. The claim allow-lists gate submission plus downstream exports, benchmark/ranker generation, process-evaluation labeling, worker utility-credit jobs, and utility-shaped manual delayed-credit mutations. Each static or signed token is treated as its own pseudonymous auth principal inside the tenant, and the principal hash excludes static-token expiry metadata. Reviewer/admin tokens can list tenant-local quarantine, approve or reject live quarantined submissions, append delayed credit events, view tenant analytics, and export approved replay dataset slices. Review decisions and delayed credit mutations require non-empty operator reasons; benchmark/regression/training/ranking delayed credit also requires the source trace and caller policy to permit the credited use. Review decisions fail closed for terminal, expired, non-quarantined, or aggregate-only-approval records before trace content is read. Contributor tokens can submit, revoke, read their own token-principal credit/events, and sync status for their known submission ids, but cannot review, view tenant-wide analytics, append credit events, or export datasets.

Signed claim allow-lists also gate vector-index workers. A vector worker token
that carries only `aggregate_analytics` is rejected because vector payloads and
nearest-neighbor metadata are derived artifacts.

Production identity should replace static-token enumeration and HS256 bridge
claims with issuer-managed EdDSA/Ed25519 signed tenant upload claims. The
current EdDSA verifier supports default or `kid`-selected public keys, JSON,
file-backed, or guarded HTTPS EdDSA keysets with optional `not_before`/`not_after`
activation windows, issuer and audience checks, max-TTL policy, required JWT
IDs, emergency `jti` denylists, a deployment flag that rejects non-EdDSA bridge
credentials, and a stricter managed-keyset flag that accepts only active
issuer/keyset EdDSA keys with `kid` headers. Guarded HTTPS issuer-managed
keysets refresh live after startup with last-good preservation and optional
max-stale fail-closed enforcement.
The standalone `trace_commons_upload_claim_issuer` binary is the first
production-shaped issuer service for hosted tenants. It exposes `POST
/v1/trace-upload-claim`, `GET /health`, and `GET
/.well-known/trace-commons-ed25519-keyset.json`; authenticates workload tokens
with EdDSA/Ed25519 only; rejects RSA key material; signs short-lived contributor
claims with `kid`, `iss`, `aud`, `iat`, `exp`, and `jti`; and publishes the same
`kid` plus `public_key_pem` keyset shape consumed by the ingest service. The
issuer enforces workload tenant, scope, and allowed-use narrowing. Deployments
that wire a Trace Commons DB into the issuer can also require an active
contributor tenant-access grant before a claim is minted; the issuer uses the
same signed-principal hash shape as ingest for grant lookup, binds optional
issuer/audience/subject grant fields to the outgoing claim issuer, audience, and
workload actor, and intersects grant consent/use allow-lists without replacing
the raw actor stored in the signed upload claim.
Configure it with `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_*` environment variables
for bind address, signing key PEM or file, signing public key PEM or file,
signing `kid`, issuer, audience, max TTL, workload public key PEM or file, and
optional workload issuer/audience checks. Set
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS=true` to make
the standalone issuer connect through `DATABASE_URL`
configuration and fail closed unless the workload actor has an active
contributor grant for that tenant.
Production claims must bind the tenant id, actor or job principal, role,
expiry, optional JWT ID, and allowed consent scopes/uses; the service still
derives the storage partition from the
authenticated claim after verification, not from envelope metadata supplied by
the contributor.

On submit, the service also writes a derived redacted-only record with:

- canonical summary and hash
- hash-based duplicate precheck
- deterministic redacted-summary novelty score for worker-side feature provenance
- coverage tags for channel, tool, tool category, outcome, failure mode, and privacy risk
- aggregate analytics by status, privacy risk, task success, tool, tool category, and coverage tag

The current API remains intentionally file-backed under `TRACE_COMMONS_DATA_DIR`
for compatibility and easy local operation, with optional DB-backed read flags
for contributor, reviewer metadata, replay/export selection, and audit surfaces.
This server repo owns the production storage path: optional DB dual-write
metadata, optional encrypted local artifact storage, object-primary
submit/review mode that avoids plaintext envelope body files while retaining
file-backed metadata/audit compatibility records, a durable DB
revocation-propagation ledger for downstream invalidation/retry work, Trace
Credits settlement/hold/attestation/NEAR-outbox tables, ranking
evidence/calibration tables, and a fail-closed reconciliation gate for promotion
jobs. `TRACE_COMMONS_OBJECT_STORE=remote_service` now has a service-owned
filesystem-backed remote adapter (`TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=file_system`)
for production-like object I/O rehearsals while AWS/GCS/Azure providers remain
fail-closed behind the disabled remote alias. Enabled remote service storage
requires provider/bucket-or-root/KMS/credential-reference configuration plus
`TRACE_COMMONS_ARTIFACT_KEY_HEX`, refuses plaintext compatibility fallback, and
can satisfy object-primary startup guards; disabled cloud-provider scaffolds
still cannot. The server-owned
`V1__trace_commons_schema.sql` through
`V10__trace_credit_settlement_joined_evidence_hash.sql` migrations cover the
tenant-scoped Trace Commons metadata, credit-settlement control plane, ranking
evidence substrate, persisted model-promotion calibration runs, settlement
ranking gates, default `FORCE ROW LEVEL SECURITY` hardening for every Trace
Commons RLS table, persisted label-source diversity evidence, per-source
calibration error evidence for promotion gates, calibration joined-evidence
hashes, and settlement calibration evidence-hash binding; config-status can
report catalog-only RLS readiness, including policy counts, expression
mismatches, disabled tables, force-RLS counts, force-RLS missing tables, whether
the current role bypasses RLS, and a stricter production-ready boolean. The
admin operational summary and metrics promote unsafe RLS diagnostics into a
`postgres_trace_rls_not_ready` promotion blocker while exposing only readiness
booleans and aggregate counts, not table names.
Production deployments can set `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY=true`
to fail startup unless the configured PostgreSQL database is fully ready for RLS
as an active tenant boundary. Production deployments still need a non-bypassing
runtime role plus transaction-local tenant context through every DB-backed
runtime path before RLS can become the active trust boundary. Production
deployments should finish promoting reviewer/export/analytics paths into
DB/object-primary reads and move encrypted artifacts behind remote service-owned
object storage before broad rollout.

## Production Hardening Roadmap

The current implementation is a usable MVP for local development and controlled internal pilots. A production Trace Commons deployment needs the following before broad tenant rollout:

### DB and Object Storage

- Promote the current dual-write mirror into relational metadata reads for all API surfaces and service-owned encrypted object storage for redacted trace bodies. Contributor credit/status, reviewer metadata, replay export selection, and audit reads already have opt-in DB-backed rollout gates.
- Keep metadata and object keys tenant-scoped from the auth-derived tenant id. Do not trust tenant fields in the envelope as storage partition keys.
- File-backed compatibility reads now also fail closed if a metadata or derived row stored under one tenant directory claims a different embedded tenant id or tenant storage ref. Continue moving mutating paths to `TenantCtx` so authenticated tenant identity stays the trust boundary after rehydration.
- Store immutable submission records, append-only credit events, revocation tombstones, review decisions, export job manifests, and processing job state as separate records.
- Use row-level tenant policies or an equivalent authorization layer for every metadata query.
- Encrypt object storage at rest, require TLS in transit, and keep object bucket access behind service identities rather than reviewer/user tokens.
- Do not expose raw corpus bucket access or broad downloadable corpus snapshots. Individual trace reads, dataset builds, and training/evaluation exports should go through API-mediated, tenant-scoped jobs with per-source audit events and explicit purpose metadata.
- Version all derived artifacts. A redaction, vector, ranking, benchmark, or export worker must record input envelope hash, worker version, policy version, and output artifact id.

### Tenant RBAC and ABAC

- Move beyond static tenant tokens before production. Static tenant tokens can carry expiry attributes and the ingest service can accept HS256 signed tenant claims as interim bridges, with claim allow-lists already constraining submission and downstream export/worker use. The service can verify EdDSA/Ed25519 signed claims through default or `kid`-selected public keys and local/file/guarded-HTTPS keysets with activation windows; production can enable `TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS=true` to reject static and HS256 bridge credentials, or `TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS=true` to accept only issuer/keyset EdDSA claims while refreshing guarded remote keysets live after startup.
- Enforce RBAC for contributor, reviewer, admin, trainer/export job, and service-worker roles.
- Add ABAC checks for consent scope, allowed use, privacy risk, review state, retention policy, revocation state, export purpose, and tenant data residency. Current tenant policy ABAC covers submission allowed scopes/uses plus replay, benchmark, and ranker export required uses, and signed-claim allow-lists should continue to constrain both submission and downstream worker/export consumption.
- Keep vector workers under the same ABAC model as export and utility workers: both the worker claim/policy and the source trace must permit a derived vector use, and aggregate-only traces must not enter derived vector indexes.
- Require privileged operations such as tombstone deletion to carry an explicit reason. Review decisions, delayed credit mutation, and destructive retention purges already require non-empty reasons or purposes, and export guardrails can require explicit replay, benchmark, and ranker export purposes.
- Treat envelope contributor ids as pseudonymous attribution only. Authorization must come from request identity and central policy.

### Audit and Reviewability

- Add append-only audit events for every trace read, write, review decision, credit mutation, revocation, export, retention purge, and worker-derived artifact.
- Include tenant id, actor or job id, role, submission id, action, reason, request id, decision inputs, and output artifact ids.
- Make audit logs tamper-evident and queryable by tenant/security reviewers without exposing raw trace content.
- Add sampled audit reconciliation jobs that compare object storage, metadata rows, vector ids, export manifests, credit ledger rows, and revocation tombstones.

### Retention and Deletion

- Define retention windows by consent scope and allowed use. The envelope's `trace_card.retention_policy` should map to central policy, not directly drive deletion behavior.
- Implement retention jobs that remove or tombstone metadata, redacted trace objects, derived vectors, benchmark artifacts, export cache entries, and queued worker outputs.
- Keep revocation tombstones long enough to prevent re-ingest/re-export of the same submission hash after content deletion.
- Block new processing and export for revoked or expired submissions. Existing derived artifacts must be marked invalid before any downstream job consumes them.

### Revocation Propagation

- Treat revocation as a state transition that fans out to object storage, review queues, vector indexes, benchmark sets, ranking/training queues, export jobs, and credit ledgers.
- Make revocation idempotent and tenant-scoped. Repeated requests should preserve the first revocation reason/time unless an admin appends audit context.
- Require downstream workers to check central revocation state immediately before reading trace content and immediately before publishing a derived artifact.
- Add reconciliation that finds derived artifacts whose source submission is revoked and marks or removes them.

### Vector Index, Ranking, and Benchmark Conversion

- Generate embeddings only from redacted summaries and approved redacted trace fields. Never embed raw traces, sidecar raw text, or unreviewed high-risk content.
- Keep vector ids tenant-scoped and source-linked so index entries can be deleted or invalidated on revocation/retention.
- Promote deterministic novelty/duplicate scoring into private vector-backed feature workers. The first ranking feature worker already reserves `feature_provenance:server_derived`, can require active vector metadata for vector-backed duplicate/novelty inputs, and can be required for prediction-credit, readiness, and settlement with `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE=true`; production still needs the full embedding backend with nearest neighbors, cluster id, and coverage contribution.
- Add ranking/model-utility jobs as offline analysis. Their outputs may append delayed credit events, but should not become immediate automatic payment signals.
- Convert approved traces into benchmark/replay datasets through a controlled job that records consent scope, review state, redaction version, deterministic replay requirements, and export manifest id.
- Require benchmark conversion to fail closed when the trace is revoked, expired, not approved for the target use, or missing replayability metadata.

### Privacy Filter Sidecar Operations

- Run Privacy Filter sidecars as untrusted local subprocesses or containers with timeouts, output size limits, and no access to Trace Commons credentials.
- Pass only the minimum text required for local redaction. Do not pass bearer tokens, full policy files, queue files, or raw tool payloads unless the local policy explicitly includes those fields.
- Accept only the safe projection: redacted text, allow-listed labels, counts, warnings, and summary metadata. Strip `text`, raw span strings, raw offsets, and unknown high-risk fields; unsupported span labels are mapped to `unknown` so malformed sidecars cannot smuggle emails, paths, or tokens through label names.
- Treat sidecar failures as non-fatal redaction warnings and fall back to deterministic local redaction rather than uploading raw content.
- Add canary-secret tests that feed synthetic credentials, local paths, tenant ids, and user ids through the sidecar path and assert they do not appear in envelopes, logs, or derived summaries.

## Autonomous Submission Policy

The local policy is stored under `~/.ironclaw/trace_contributions/policy.json` and controls:

- endpoint and bearer token environment variable
- default consent scope
- whether redacted message text or tool payloads may be included
- selected tool filters
- minimum local submission score
- whether medium-risk traces require manual review
- periodic credit notice interval for CLI/web/runtime notifications

The runtime can call the same queue and flush behavior later after a task completes. Queue writes use same-directory temporary files, file sync, atomic rename, and best-effort parent-directory sync. When `flush-queue` runs under an enabled policy, it compacts duplicate queued contribution envelopes and orphan held sidecars, quarantines malformed queued envelope files into a local `queue_malformed` directory instead of blocking later valid submissions, submits eligible traces autonomously, honors typed retry backoff for transient submission failures, and prints a credit update when the configured notice interval has elapsed.

`ironclaw traces queue-status` reports local autonomous queue readiness without exposing trace bodies: opt-in state, endpoint presence, bearer-token environment availability, capture toggles, selected-tool count, queued and held counts, typed retry/manual-review/policy hold counts, the next scheduled retry time, durable flush telemetry, retry/status-sync failure counters, last compaction reclaimed count, duplicate envelopes removed, orphan hold sidecars removed, malformed envelopes quarantined, sanitized held-reason counts, sanitized Endpoint/Credential/Network/NetworkOffline/NetworkDns/NetworkTimeout/NetworkConnectionRefused/HttpRejection/Policy/Queue/StatusSync/Submission/Unknown failure classes, and the same local credit summary used by `credit`. Diagnostics also expose aggregate warnings for schema version mismatch, consent policy mismatch, redaction pipeline mismatch, trace-card redaction pipeline mismatch, and malformed envelopes; those warnings include severity, production-promotion blocking flags, and safe recommended actions, but never include raw bodies or raw observed mismatch values. The authenticated web API exposes a narrower scoped queue status through `/api/traces/queue-status`, returning queued/held counts, durable telemetry, safe warning aggregates, and sanitized held queue entries for the current user only.

`ironclaw traces credit --notice` and `GET /api/traces/credit-notice` mark a due periodic credit notice without exposing central corpus rows. Notice summaries include pending/final totals, delayed ledger deltas, credit-event counts, and recent safe explanations when the local notice interval is due. Due notices are also written to a scoped local `credit_notice_outbox.json` retry outbox keyed by the local credit fingerprint, so a failed runtime channel delivery does not consume the notice permanently. The outbox stores the safe summary/message, delivery status, attempt count, next retry time, and delivery attempts with sanitized channel names plus error kind/hash only. Opted-in clients can acknowledge the current notice until the local credit fingerprint changes (`ironclaw traces credit --notice --ack` or `POST /api/traces/credit-notice` with `{"action":"acknowledge"}`), or snooze it for a bounded period (`--snooze-hours N` or `{"action":"snooze","snooze_hours":N}`); both actions suppress matching pending outbox items. The local fingerprint is a hash over submission ids, lifecycle/status, credit totals, and credit-event metadata, not over explanation text or trace bodies.

The agent runtime also schedules an autonomous post-turn contribution pass after a response is persisted or a turn fails. It reads the authenticated user's scoped policy, verifies the thread still belongs to that user, captures the most recent turns from durable conversation history, locally redacts the envelope, queues it, and flushes eligible queued envelopes. After flush, the agent drains due local credit-notice outbox items through the originating channel and records delivery success or sanitized failure. Independently, the long-running agent loop starts a periodic Trace Commons queue worker that scans the owner and active DB users, flushes opted-in scoped queues, honors typed retry backoff, drains pending credit-notice outbox items across channels, and records delivery success or safe retry state instead of silently consuming notice state.

During each queue flush and before web credit/submission responses, the client asks the ingestion API for status updates for locally known submitted ids. The status endpoint is tenant-bound by the bearer token and returns only records from that tenant's namespace, so delayed reviewer credit can be reflected locally without allowing broad corpus enumeration. Contributor credit, credit-event, and status-sync reads append safe aggregate read-audit rows with item counts only, not raw requested submission id lists. The authenticated web settings response also surfaces persisted held-queue reasons and the richer local credit report for the current user scope only; held queue responses contain submission ids and sanitized hold reasons, not queued envelope bodies.

In the authenticated web gateway, policy, queue, ledger, and revocation state are scoped under a hashed user/tenant directory rather than the global CLI policy. Envelopes carry a pseudonymous contributor id and a separate pseudonymous tenant scope reference, so the private ingestion service can attribute credit and enforce tenant boundaries without storing raw user ids in the trace body.

## Multitenant Permissioning

Trace contribution authorization must be derived from the authenticated request or runtime identity, not from fields inside a submitted envelope. Envelope fields such as `contributor.pseudonymous_contributor_id` and `contributor.tenant_scope_ref` are attribution/provenance metadata only.

For local capture:

- Web preview and autonomous runtime capture use the authenticated `user_id` as the trace scope.
- Conversation history is read through tenant ownership checks before a contribution envelope is built.
- Local policy, queue, submission history, revocation state, and credit records live under `trace_contributions/users/<hash>` for the authenticated user scope.
- The envelope includes a stable pseudonymous contributor id and a separate stable pseudonymous tenant scope reference. Neither includes the raw user id.

For the private ingestion service:

- The service should bind every request to a tenant from AuthN/AuthZ credentials, such as a tenant-scoped token, mTLS identity, or EdDSA/Ed25519 signed upload claim.
- It should reject requests where the authenticated tenant is not allowed to submit for the claimed tenant scope.
- Central metadata, credit ledger rows, revocation tombstones, privacy review queues, trace objects, and export jobs should all be keyed by the auth-derived tenant id plus the authenticated principal or contributor pseudonym. The auth-derived tenant id is the storage partition; envelope tenant references are never partition keys.
- RBAC/ABAC policies should allow contributors to see only their own submissions and credit, reviewers to see quarantined/redacted traces for permitted tenants, and trainer jobs to read approved slices through controlled jobs.
- Signed upload claims should carry allowed consent scopes and allowed uses so the same verified claim can limit submission, replay export, benchmark/ranker generation, process-evaluation labeling, and utility-credit jobs.
- Audit logs should record tenant id, actor id/job id, submission id, access reason, and export target for every individual trace read or mutation.
- The corpus should not be exposed as a raw bulk download. Researchers, trainers, and reviewers should access approved slices through scoped API routes or controlled jobs that write tenant-scoped manifests and read-audit rows.

With these rules, trace contributions can be correctly permissioned and attributed in a multitenant deployment: the trusted tenant binding comes from authentication and database row policy, while pseudonymous envelope metadata supports corpus analytics and credit assignment without becoming a trust boundary.

## Trace Commons Threat-Model Checklist

Use this checklist for any change touching trace capture, redaction, ingestion, review, export, credit, or derived datasets.

- Raw trace non-upload: verify raw recorded traces never leave the client; only `ironclaw.trace_contribution.v1` envelopes produced after local redaction may be submitted.
- Frontend untrusted: treat gateway UI requests as user-controlled input. Re-check auth, tenant ownership, policy scope, and conversation ownership on the server before previewing, queueing, submitting, listing, or revoking traces.
- Sidecar output stripping: reject or strip Privacy Filter sidecar fields that can carry original text, raw detected span text, raw offsets that are unnecessary downstream, or unknown nested payloads.
- Token isolation: submit/review/admin tokens must not be stored in policy files, trace envelopes, queue files, sidecar stdin, logs, or exported datasets.
- Tenant isolation: every ingestion read/write must bind to the auth-derived tenant and actor. Contributor-supplied `tenant_scope_ref`, `pseudonymous_contributor_id`, `submission_id`, and `revocation_handle` are not authorization inputs.
- Role isolation: contributors cannot list quarantine, append delayed credit, read analytics, export datasets, or probe other contributors' submissions. Reviewers/admins cannot bypass tenant scope.
- Bulk export controls: dataset export must require an authorized role, explicit purpose, consent/use filter, privacy-risk filter, review state filter, output manifest, and audit event per source trace.
- Delayed credit abuse: delayed credit append must be privileged, append-only, audited, bounded by policy, and linked to a concrete downstream artifact or review decision.
- Revocation propagation: revocation must block future status changes, review approval, vector indexing, benchmark conversion, ranking/training use, and export. Existing derived artifacts need invalidation or removal.
- Retention bypass: retention jobs must cover central metadata, object storage, vector entries, benchmark artifacts, export caches, worker queues, and local references where applicable.
- Canary secret tests: include synthetic API keys, bearer tokens, local paths, emails, tenant ids, user ids, and tool payload secrets in regression fixtures and assert none survive in accepted envelopes or sidecar-derived summaries.
- Audit completeness: any path that reads or mutates central trace content, credit, review state, export state, or revocation state must emit a tenant-scoped audit event.

Protected web API endpoints:

- `GET /api/traces/policy`
- `PUT /api/traces/policy`
- `POST /api/traces/preview`
- `POST /api/traces/submit`
- `POST /api/traces/flush`
- `GET /api/traces/credit`
- `GET /api/traces/credit-notice`
- `POST /api/traces/credit-notice`
- `GET /api/traces/queue-status`
- `GET /api/traces/submissions`
- `POST /api/traces/submissions/{submission_id}/revoke`

The web settings panel includes a Trace Commons tab for standing opt-in, autonomous submission controls, queue flushing, persisted held queue reasons, recent submissions, revocation, and richer credit/review totals. The chat composer also has a Trace button that previews the current thread's redacted envelope and can queue it for the same autonomous submission path. Local preview remains available without opt-in, but enqueue/manual-submit/autonomous acceptance now preflights the scoped standing policy and requires both opt-in and an ingestion endpoint before capture/redaction work is queued. Web enqueue and submit requests may not widen capture beyond the standing policy: if the policy disallows message text or tool payloads, crafted requests with those flags are rejected before a queue file is written.

## Implementation Status Matrix

| Area | Status | Maintainer notes |
|------|--------|------------------|
| Local opt-in policy and opt-out | Implemented MVP | CLI and scoped web/runtime policy files exist; static submit tokens and issuer workload credentials stay in environment. Hosted tenants can configure a guarded HTTPS upload-claim issuer for autonomous short-lived EdDSA bearer refresh. |
| Local preview, queue, flush, and credit display | Implemented MVP | CLI and web paths use local redacted envelopes and local submission metadata; queue writes use atomic temp-file replacement and malformed queued envelopes are quarantined locally so later valid envelopes still flush; `ironclaw traces queue-status` reports scoped policy readiness, bearer-token environment presence, upload-claim issuer readiness without issuer URLs/tokens/tenant ids, queued/held counts, typed retry/manual-review/policy hold counts, next retry time, durable flush/status-sync telemetry, last compaction reclaimed count, duplicate-envelope, orphan-hold-sidecar, and malformed-envelope quarantine removals, safe warning aggregates, sanitized held-reason counts, sanitized failure classes, and the local credit summary, while authenticated web activity exposes scoped queue/held counts and reloads persisted held queue reasons plus accepted/quarantined/rejected and delayed-credit report fields for the current user scope. Periodic credit notices are backed by a scoped local retry outbox, can be acknowledged until credit changes, or snoozed for a bounded number of hours through both CLI and authenticated web actions. |
| Deterministic local redaction | Implemented MVP | Includes generic secret/path scrubbing, stable placeholders, tool-aware payload handling, and Privacy Filter safe projection. |
| Privacy Filter sidecar integration | Implemented MVP | Local command/stdin/stdout path exists with safe output projection, non-fatal fallback, minimal child environment, stderr hashing, IO limits, and canary tests. Production container sandboxing and stricter output contracts remain. |
| Autonomous post-turn and periodic contribution | Implemented MVP | Runtime queues/flushed scoped envelopes after persisted or failed turns only when the scoped standing policy is enabled, has an ingestion endpoint, and the current redacted envelope is eligible for autonomous submission. Ineligible current traces are skipped instead of written as held queue files, while existing queue flushes and periodic credit-notice sync still run. A periodic agent-loop worker also flushes opted-in owner/active-user queues, fetches in-memory short-lived EdDSA upload claims from a guarded issuer when configured, retries once after auth rejection with a forced refresh, stores retryable submission failures as typed redacted sidecars with capped exponential backoff, skips retry-held envelopes until due, compacts duplicate queued contribution envelopes and orphan held sidecars, quarantines malformed queue files locally, classifies failures into sanitized local diagnostic buckets including offline/DNS/timeout/connection-refused network subtypes, and drains durable local credit-notice outbox items with delivery success/failure recording. |
| Web Trace Commons settings and preview endpoints | Implemented MVP | Authenticated gateway endpoints and UI controls exist; server-side tenant/user checks remain the trust boundary, and queue/manual-submit paths preflight scoped opt-in policy before enqueueing. |
| Private ingestion service | Implemented MVP | Development/internal binary validates schema, reruns redaction, computes hashes/credit, enforces optional contributor-only tenant/principal hourly submission quotas for autonomous uploads, stores accepted/quarantined records, and serves review/status/export routes, including reviewer trace-list filtering by export/provenance purpose. It can now dark-launch DB dual-write metadata and encrypted envelope artifacts. |
| Tenant token roles | Partial | Static tenant tokens support contributor/reviewer/admin plus scoped export, retention, vector, and benchmark worker behavior; benchmark, retention, and vector workers now have dedicated `/v1/workers/benchmark-convert`, `/v1/workers/retention-maintenance`, and `/v1/workers/vector-index` routes as well as CLI helpers for those scoped routes and scoped compatibility routes where needed; optional `expires_at`/`expires` RFC3339 attributes reject stale bearer tokens before tenant attribution while preserving principal hashes over the token secret only; optional HS256 signed tenant claims bind tenant id, actor principal, role, issuer/audience when configured, allowed submission consent scopes/uses, expiry, optionally bounded `exp - iat` lifetime, and optional required `jti` claim without enumerating every bearer token; optional EdDSA/Ed25519 signed claims can be verified with default, `kid`-selected public keys, or local/file/guarded-HTTPS keysets with activation windows, config status exposes safe total/EdDSA active/inactive/managed key-count aggregates plus guarded remote keyset refresh health without key material, production-like deployments can require EdDSA signed tokens so static and HS256 bridge credentials are rejected on every authenticated route, and stricter deployments can require managed EdDSA keysets so only active `kid`-selected issuer/keyset claims are accepted while refreshing guarded HTTPS keysets live after startup; required tenant access grants can bind signed EdDSA claims to issuer, audience, and JWT `sub` without affecting static-token bridge grants; autonomous and explicit-submit clients can refresh short-lived EdDSA upload claims from a guarded HTTPS issuer instead of storing long-lived Trace Commons bearer tokens; optional tenant submission policies can restrict allowed consent scopes and trace-card uses at ingest and export time, DB-backed tenant policy records can drive submission and export policy reads behind `TRACE_COMMONS_DB_TENANT_POLICY_READS`, admin tokens can manage the current tenant's DB-backed contribution policy via `/v1/admin/tenant-policy` or the `ironclaw traces tenant-policy-get/set` CLI helpers, policy admin reads/writes are audited with safe hash-only policy metadata, and production-like deployments can require every submitting/exporting tenant to have a policy entry. Fuller central policy, RBAC/ABAC, and row-level tenant enforcement hardening remain. |
| Contributor credit ledger and delayed credit sync | Partial | Append-only local and central credit events exist, pending submission estimates are kept separate from settled final/ledger credit, reviewer/admin delayed credit mutation requires a reason and can resolve submission metadata through the DB reviewer read path when file metadata has been removed, utility-shaped manual delayed credit for benchmark conversion, regression catches, model training, and ranking training now enforces source allowed-use plus tenant-policy and signed-claim ABAC before writing ledger rows, benchmark conversion plus ranker candidate and pair exports append idempotent delayed utility credit with external artifact/job refs, DB credit events preserve training-vs-ranking utility types, terminal traces retain ledger events for audit but exclude those deltas from contributor totals, DB audit rows include typed credit-mutation metadata with hashed reason/reference fields, maintenance reconciliation reports file/DB ledger event ID gaps, settlement can enforce a configured per-account credit cap before final batches or NEAR outbox rows are created, and autonomous clients periodically notify opted-in users when submitted or later-revoked records receive ledger changes, including delayed ledger deltas and credit-event counts. Production still needs broader fraud-review operations and issuer policy governance. |
| Quarantine/review workflow | Partial | Reviewer/admin routes can list and decide on quarantined redacted traces; quarantine and active-learning queue items expose reviewer SLA/escalation metadata (`review_age_hours`, `review_escalation_state`, and `review_escalation_reasons`) plus optional DB-backed review lease assignment fields for prioritized triage; queue reads can filter lease state with `all`, `mine`, `available`, `active`, or `expired`; the routing summary exposes safe aggregate pressure, lease state, escalation, privacy-risk, and hash-only assignee load counts for central triage; review decisions require a non-empty reason, cannot bypass another active reviewer lease, and are rejected for non-quarantined, terminal, expired, or aggregate-only approval records before trace content is read; bounded batch decisions can apply one common approve/reject action to up to 50 explicit submission ids while preserving the same per-item lease, ABAC, body-read, object-ref, mirror, and audit path; with DB reviewer reads enabled, reviewers/admins can claim or release durable tenant/principal-scoped leases, review decisions resolve submitted-envelope bodies through active DB object refs when available, can be configured to require active object refs, emit content-read audit rows, and mirror the reviewed envelope body as a fresh `review_snapshot` object ref without reclassifying the original submitted envelope; object-primary submit/review mode can also skip plaintext submitted/reviewed envelope body files. Production still needs richer assignment policy and external reviewer-router integrations. |
| Replay dataset export | Partial | Approved redacted slices can be exported by reviewer/admin tokens, production-like deployments can require explicit accepted/low-risk/consent-scoped export guardrails for replay, benchmark, and ranker-training exports plus caller-supplied export purposes and active DB object refs for body reads, tenant policy allowed-use ABAC gates replay/benchmark/ranker requests and source selection, and each export call site now goes through a short-lived tenant/principal/purpose/dataset-kind access-grant check before building replay, benchmark, or ranker slices. DB metadata can drive replay export selection, benchmark/ranker exports can fail closed when selected sources lack active submitted-envelope object refs, submitted envelope bodies can resolve through active DB object refs for file or enabled service-owned artifact stores, object-primary replay export mode can require service-owned DB object refs with no file fallback, manifests carry source-list hashes mirrored into audit `decision_inputs_hash`, replay/benchmark/ranker exports mirror durable one-shot grant rows plus running/complete export job rows, stale running export jobs surface as operational promotion blockers once their grant expiry has passed, replay exports mirror compact DB manifest rows and per-source item snapshots with source object refs plus invalidation timestamps, benchmark/ranker item rows link derived refs, active vector refs when indexed, and per-source object refs to file-backed or service-owned encrypted benchmark/ranker artifacts, reviewer/admins can list replay manifest metadata, and each exported trace body read emits a tenant-scoped audit event. Production needs persistent background export workers, cloud remote object storage, broader bulk export controls, and execution of revocation propagation for already-published artifacts. |
| Analytics summary | Partial | Aggregate counts by status/risk/tool/coverage exist, including content-free process-evaluation aggregates for evaluated trace count, labels, rubric ratings, and score bands. Process-evaluation workers can also attach idempotent hash-only ranking labels for ranking-allowed traces, while still blocking disallowed target uses before trace-body reads. Deployments can set a minimum cell-count threshold to suppress rare aggregate buckets before responses leave the service, analytics responses include privacy-budget accounting with released/suppressed cell counts, suppression status, and broad-release blocker reasons, and `release_scope=broad` fails closed unless the privacy budget is broad-release ready. Production still needs differential-privacy/noise calibration if exposed beyond reviewer/admin operators. |
| Production relational DB and encrypted object storage | Partial | Server-owned V1 PostgreSQL schema, server-owned `TraceCorpusStore`, the `PgBackend` implementation, optional ingest DB mirror with contributor, tenant policy, reviewer metadata, durable review lease fields, replay selection reads, policy-gated DB object-ref-backed replay envelope reads, vector-entry metadata, compact replay export manifest metadata, replay export item snapshots, durable retention job/item ledger rows, durable revocation-propagation item rows, durable hash-only benchmark registry outbox rows, backend-enforced same-tenant/submission checks for derived object refs, vector-entry derived refs, and export item derived/object/vector refs, benchmark/ranker export artifact object refs, atomic export manifest/object-ref/item mirror writes inside PostgreSQL with rollback coverage for invalid item references, canonical audit payloads for DB verifier recomputation, DB-native audit append ordering, encrypted local artifact sidecar, `TraceArtifactStore` provider trait, service-owned local encrypted object-store mode, filesystem-backed remote-service object-store mode plus disabled cloud-provider config scaffold, tenant-allowlisted rollout gates for DB reads/object-ref requirements/object-primary modes, object-primary submit/review mode for plaintext-free envelope body storage, object-primary replay export object-ref read mode, service-owned submitted/review envelope physical-delete execution from revocation-propagation items, maintenance-triggered DB mirror backfill for submissions plus existing file-side credit/audit/replay-manifest/registry-outbox rows with isolated per-item failure reporting, derived presence/status/hash/invalid-source diagnostics, file/DB credit-ledger and canonical audit-event ID gap diagnostics, split export-manifest kind diagnostics, export item object-ref and invalid-source diagnostics, benchmark registry outbox gap/status diagnostics, separate object-ref presence/readability/hash-mismatch diagnostics, vector index gap diagnostics, reader-projection parity diagnostics, initial PostgreSQL tenant RLS policies, default `FORCE RLS` migration coverage, and safe PostgreSQL RLS readiness diagnostics exist. Cloud remote object storage implementation, parity enforcement, non-bypassing service-role/central-policy hardening, and broader object-primary reads remain. |
| Central audit log | Partial | File-backed audit rows now include optional hash-chain fields plus a maintenance verifier while preserving legacy JSONL compatibility, DB audit rows mirror those chain hashes and canonical hash payloads for file-backed events, and PostgreSQL audit rows now carry tenant-scoped `audit_sequence` values assigned inside serialized append transactions that reject stale hash-chain predecessors. Maintenance emits a DB mirror report that checks hash-field format, previous-hash linkage, canonical-payload hash recomputation, DB action/metadata projection drift, file/DB canonical audit-event ID gaps, DB canonical-payload projection failures, and submitted-audit privacy-risk metadata drift for mirrored rows; projection drift is a named blocking reconciliation gap and audit-reader parity records only a safe error hash. Audit routes and optional DB audit reads cover core submit/review/credit/revoke mutations, contributor credit/status reads with aggregate item counts, retention/purge artifact invalidations, tenant policy admin reads/writes, reviewer analytics/list/review-queue/audit-log reads, dataset/benchmark/ranker exports, per-trace replay export content reads, process-evaluation writes, and per-source derived-summary reads for benchmark conversion, ranker candidate/pair exports, and vector indexing. Reviewer audit-log reads apply the response limit at the storage boundary, using a bounded file tail or tenant-scoped PostgreSQL `audit_sequence DESC LIMIT` query instead of decoding every audit row for ordinary pages. Aggregate read rows now mirror typed safe metadata with only a code-owned surface and bounded item count; `object_ref_id` is mirrored when the body is read through a DB object ref; trace-content-read rows now mirror typed safe metadata with a code-owned surface and optional purpose hash instead of raw purpose text, and the DB mirror boundary derives or validates that metadata from the hash-only reason fields before write. Submitted audit backfill now preserves typed privacy-risk metadata from matching submission records, and reconciliation reports stale submitted-audit privacy-risk metadata as `db_audit_submission_metadata_mismatches`. Revocation audit rows now carry typed metadata with only the revocation reason hash, while preserving the existing audit reason string for operator review. Canonical reconciliation also compares trace-content-read metadata back to its canonical reason and revocation metadata back to the canonical reason hash so stale typed metadata cannot hide behind a matching raw reason string. Derived-summary read rows carry only safe surface/purpose/source identifiers. Privileged delayed credit, process-evaluation writes, review lease claim/release operations, and tenant policy mutations now mirror typed safe metadata with bounded values and hashes rather than raw request bodies. |
| Retention enforcement | Partial | Submit records persist server-derived retention policy ids and expiry timestamps; review, maintenance, replay export, benchmark conversion, and ranker source selection reject current server retention policy rows that claim a mismatched policy id or extend `expires_at` beyond the allowed-use-derived retention window; maintenance and the dedicated retention worker route mark expired submissions and derived records, mirror DB expiration/artifact/export-manifest invalidation with typed action-count audit rows and durable retention job/item ledger rows when configured, prune cached exports that reference expired sources, skip expiration/purge for operator-configured legal-hold retention policy IDs only after the stored policy id matches the server-derived allowed-use retention class, and can explicitly purge expired file/encrypted local artifact copies by cutoff only when non-dry-run purge requests carry a non-empty purpose. Production still needs service-owned object storage deletion workflows. |
| Revocation propagation to derived artifacts | Partial | Current revocation marks local/file status, mirrors DB status, writes tenant-scoped first-writer-wins tombstones with redaction/canonical-summary hashes when available, authorizes DB-only revocation against the original contributor or reviewer/admin principal, rejects same-tenant re-ingest matching retained file-backed or DB-mirrored tombstone submission ids, redaction hashes, or canonical-summary hashes, invalidates DB-mirrored object refs, derived precheck rows, vector metadata entries, replay export manifest rows, replay export item rows, file-backed benchmark/ranker provenance manifests, and published benchmark artifacts by moving their registry state to revoked and evaluation state to inconclusive plus enqueueing a hash-only external-registry revoke row, blocks file-backed replay export, applies the same DB invalidation path when maintenance discovers an existing file-backed revocation tombstone or already file-marked revoked submission, and can physically delete service-owned encrypted submitted/review envelope payloads from exact tenant-scoped object-ref revocation items while marking unsupported stores/artifact kinds as skipped. Production must still invalidate worker caches. |
| Vector duplicate/novelty index | Partial | DB schema, storage contract, dedicated vector worker route, maintenance-triggered metadata indexer, object-ref/readability gating before non-dry-run vector writes, per-source vector-index content-read audits with source object refs, admin-safe vector-entry metadata listing, and reconciliation gap diagnostics now persist/vector-check vector-entry metadata, nearest trace ids, cluster id, duplicate score, novelty score, and invalidation state for accepted canonical summaries. Duplicate/novelty scoring now uses deterministic redacted-summary similarity with exact-hash matches as the strongest signal, and encrypted artifact storage can persist a redacted canonical-summary vector payload as a worker-intermediate object ref for later embedding-worker promotion. Real embedding generation, vector payload search, and model-backed duplicate/novelty workers are still future work. |
| Ranking/model utility pipeline | Partial | A trusted offline utility-credit worker route and CLI helper append idempotent delayed credit for accepted traces using `regression_catch`, `training_utility`, or `ranking_utility` plus an external job/artifact reference. Ranking workers can write hash-only immutable model manifests, feature hashes, predictions, frontier/reviewer/system evaluator labels, and persisted calibration runs with aggregate quality gates, registered calibration-dataset binding, optional fail-closed holdout registry enforcement through `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY`, deterministic joined-evidence hashes, server-owned joined-label/confidence/error quality gates, configurable label-source diversity, configurable pairwise evidence/accuracy thresholds, and per-source cohort error gates before model-derived credit is trusted; admins can stage `candidate` models freely, and the admin promotion endpoint activates candidates only when the requested model/policy/target-use/calibration-dataset run is promotable, the latest matching evidence is still promotable, the holdout registry gate is satisfied, and any configured calibration freshness window is satisfied; dry-run promotion responses expose hash-only holdout readiness fields for stored calibration evidence, current recomputed evidence, effective thresholds, errors, low-confidence counts, and reason codes before activation. Dataset-readiness reports group latest model manifests by registered holdout calibration dataset hash and expose candidate/active status counts, target-use readiness, current evidence hashes, thresholds, holdout-registry blockers, errors, and blocker reason counts. Model manifests must use disjoint training/calibration dataset hashes, and active-model risk reports block legacy overlaps under `training_calibration_dataset_overlap`, missing required holdout registrations under `calibration_dataset_not_registered`, and retired holdouts under `calibration_dataset_retired`. Process-evaluation workers can emit idempotent system ranking labels from rubric output only for sources that are allowed for the requested ranking target, storing evidence/external-ref hashes rather than raw evaluator notes; the batch run route can call a configured process-evaluator adapter over derived summaries and hashes and fail closed when the adapter is required but absent. The dedicated ranking prediction-credit worker converts a positive active-model prediction settlement score into one idempotent `ranking_utility` event bound to `ranking_prediction:<uuid>` only while the model's matching calibration dataset remains promotable and fresh under `TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` when configured and the active model/target has no current model-risk report codes; the batch run endpoint lets schedulers scan stored predictions with bounded forward progress and idempotent retries while blocking uncleared active-model risk by default and reporting risk-code skip counts. Scoped calibration-run, model-promotion, prediction-credit, and credit-cycle batch runs now persist hash-only worker-run lifecycle rows with running/completed/failed status, limits, counts, result refs, skip reason aggregates, and safe fatal-error hashes for admin review. The credit-cycle worker sequences those ranking automation steps with credit settlement and NEAR outbox dry-run/live submit/confirm checks for one model/policy/target and rejects overlapping live non-stale cycles for the same filters. Ranking utility settlement requires the named model to be active with a latest promotable calibration run for the settlement policy/target use and the active model's registered calibration dataset, with no uncleared active-model risk codes, and only fresh prediction-bound ranking credit events can settle. Admin model-risk, credit-readiness, worker-run, operational-summary, and operational-metrics surfaces now expose active-model risk, blocked ranking-credit counts, automation history, worker skip totals and reason aggregates, promotion-gate warnings for actionable worker skips, stale running and failed worker-run blockers, blocker reason aggregates, safe promotion-gate read-audit counts, safe Prometheus-text promotion and operational dashboard gauges, aggregate and per-gate structured warning logs for non-clean promotion gates, and a rollout-smoke preflight evidence-gap block without raw trace or lab evidence. Production still needs deployed evaluator operations, curated gold/holdout dataset stewardship, and broader evaluator cohort management. |
| Benchmark conversion pipeline | Partial | Reviewer/admin conversion and the dedicated benchmark worker route can produce tenant-scoped benchmark candidate artifacts with schema-versioned registry/evaluation lifecycle metadata, consent/status/risk filters, source-list hashes, immediate DB source-status revalidation, audit events, per-source derived-summary read audits, derived artifact refs, durable provenance manifests that revocation/maintenance can invalidate, idempotent utility credit events, audited lifecycle updates for registry/evaluator state in file-backed and object-primary modes, a benchmark-worker evaluation batch route that records deterministic passed/failed evaluator metadata or calls a configured external evaluator adapter without publishing, a benchmark-worker registry publication batch route that publishes only passed evaluator artifacts through the same lifecycle guard, server-enforced publication only after passed evaluator metadata with registry/evaluator refs and score, durable hash-only benchmark registry publish/revoke outbox rows with admin listing, configured adapter submission and confirmation, and worker/manual status marking, maintenance backfill/reconciliation coverage for registry outbox drift, operational-summary readiness counts for candidate, evaluated, publishable, published, revoked, registry outbox status, external-registry-adapter-gap, and external-registry-invalidation-gap states, and source-invalidation propagation that lifecycle-revokes published benchmark artifacts and enqueues external-registry revoke rows. The CLI now has `benchmark-lifecycle-update` for worker/reviewer automation against that lifecycle endpoint. Production still needs deployed external evaluator and registry adapter operations. |
| Production sidecar operations | Partial | Sidecar launches now use timeout/IO limits, minimal environment inheritance, stderr hashing, non-fatal deterministic fallback, safe output projection with allow-listed span labels, and canary-secret regression coverage. Production still needs container sandboxing and deployment-specific sidecar isolation. |

## Credit

The client computes a local pending credit estimate from a trace value scorecard. The scorecard keeps privacy risk, quality, replayability, capped novelty, duplicate penalty, coverage, difficulty, dependability, and correction value as separate components before producing the online score.

Each local submission record can store append-only credit events. The initial event records the accepted submission estimate as pending credit; it is not treated as settled final credit unless a later review or utility process explicitly finalizes it. Benchmark conversion plus ranker candidate and pair exports currently append idempotent delayed utility events for accepted included sources. Trusted offline utility jobs can append regression, training, or ranking utility credit for accepted traces through the dedicated worker surface; reviewer value and abuse penalties stay on reviewer/admin mutation paths. Shapley-style or influence estimates can inform offline analysis, but should not be exposed as direct immediate payment logic.

The ingestion API can return a receipt with updated pending/final credit and explanations; those values are stored locally in `submissions.json`.

Delayed credit/status refresh uses:

```http
POST /v1/contributors/me/submission-status
Authorization: Bearer <tenant-token>

{ "submission_ids": ["..."] }
```

The response is an array of records visible to that authenticated principal. Missing ids are omitted, which keeps cross-tenant and same-tenant cross-principal probes indistinguishable from unknown submissions.

Status records keep pending submission estimates separate from settled credit. `credit_points_pending` remains the online estimate, `credit_points_final` is present only when explicit final settlement exists, and delayed ledger fields are included when review or downstream jobs have awarded later utility credit: `credit_points_ledger`, `credit_points_total`, and `delayed_credit_explanations`. When delayed ledger events exist, `credit_points_total` is computed as explicit final credit plus the delayed ledger delta, not as pending estimate plus ledger. If a trace is later revoked, expired, or purged, status sync reports a zero delayed ledger and a safe explanation that retained ledger events are excluded, while the contributor credit-events endpoint still hides those terminal-trace rows. Local autonomous clients store the total as the effective settled credit and reset the credit-notice timer, so opted-in users can be periodically informed about benchmark, regression, ranking/training, reviewer, or abuse-penalty adjustments without seeing other contributors' ledger rows. Periodic notice summaries include the delayed ledger delta and credit-event count alongside pending and final confirmed credit.

Reviewers/admins can append delayed credit after downstream utility is known:

```http
POST /v1/review/{submission_id}/credit-events
Authorization: Bearer <reviewer-token>

{
  "event_type": "benchmark_conversion",
  "credit_points_delta": 2.5,
  "reason": "Converted into replay benchmark run 2026-04-25",
  "external_ref": "benchmark-run:2026-04-25:trace-commons"
}
```

Model-derived ranking utility credit uses the prediction-bound route:

```http
POST /v1/workers/ranking/prediction-credit
Authorization: Bearer <utility-credit-worker-token>

{
  "ranking_prediction_id": "018f2b7b-0c11-72fd-95c4-1f9f98feac01",
  "reason": "active model prediction selected for credit"
}
```

The route reads the stored prediction, verifies that its source is still an
accepted trace allowed for the prediction target use, requires the latest
registered model version to be `active` with matching policy and feature schema,
requires the latest matching calibration run to be promotable and fresh when
`TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` is set, requires a positive
settlement score, requires the active model/target pair to have no current
model-risk report codes, and appends a single idempotent `ranking_utility` event
whose external reference is `ranking_prediction:<uuid>`.

Production schedulers can call `POST /v1/workers/ranking/prediction-credit/run`
with a non-empty `reason`, optional `limit`, and optional `model_version`,
`target_use`, or `policy_version` filters. Live non-dry-run requests first
reject overlapping active prediction-credit runs for the same tenant and broad
or narrow matching filters with `409 Conflict`; runs older than
`ranking_worker_run_stale_after_hours` no longer block retry, but still surface
as operational blockers until recovered. The run endpoint scans stored
predictions in creation order, skips already-credited predictions without
consuming the batch limit, blocks uncredited predictions for active model/target
pairs with uncleared model-risk report codes unless `allow_at_risk_models` is
explicitly set, applies the same active-model/calibration/source checks as the
single-prediction route, and reports checked, credited, already-existing,
model-risk-skipped, ineligible, still-pending, and blocked-risk reason counts so
a retrying scheduler can make bounded forward progress without double-crediting
or minting credits from stale/drifted ranking evidence.

Production credit schedulers that want the full ranking-to-settlement sequence
can call `POST /v1/workers/credit-cycle/run` with a non-empty `reason`, one
`model_version`, one `policy_version`, one `target_use`, optional per-step
limits, optional calibration thresholds, and optional `near_contract_id`. The
coordinator requires a utility worker, then delegates to the existing bounded
calibration-run, model-promotion, prediction-credit, credit-settlement, and NEAR
outbox submit/confirm workers in that order. It records its own `credit_cycle`
worker-run lifecycle row, rejects overlapping live non-stale cycles for the same
model/policy/target, and result-refs the delegated ranking worker runs plus the
settlement batch. `submit_near_outbox` and `confirm_near_outbox` both default to false, so the final NEAR steps
inspects pending NEAR payloads as a dry-run unless the operator explicitly asks
the trusted relayer submission or confirmation step to run live.

For unattended cron-style operation, utility workers can call
`POST /v1/workers/credit-cycle/scheduler/run` instead. The scheduler takes one
`target_use`, optional `model_version` and `policy_version` filters, the same
per-step limits and NEAR options as the direct coordinator, and a bounded
`limit` for candidate selection. It scans latest candidate and active models,
prefers candidates before active models, skips any model/policy/target that
already has a live non-stale `credit_cycle` worker-run claim, and runs a
read-only calibration preflight before creating a direct cycle claim. Candidates
with no matching prediction evidence, no target labels, no labels that join to
the model's matching predictions, non-promotable current calibration evidence,
or uncleared pairwise evidence/accuracy policy risk are reported as scheduler
skips without creating credit-cycle worker rows, credit events, settlement
batches, or NEAR outbox items. The response reports
checked, started, skipped, active-claim skipped, still-pending, and skip-reason
counts plus a per-candidate decision list and the nested cycle responses.
Requests with `preflight_only: true` stop after eligibility checks and return
`eligible`/`skipped` decisions without invoking the direct coordinator or
creating worker rows, credit events, settlement batches, or NEAR outbox rows.
This gives external schedulers a safe retry surface without granting generic
admin settlement access.

Trusted offline utility workers use a narrower bulk route for accepted traces:

```http
POST /v1/workers/utility-credit
Authorization: Bearer <utility-credit-worker-token>

{
  "event_type": "ranking_utility",
  "credit_points_delta": 1.25,
  "reason": "ranking eval utility",
  "external_ref": "ranking_prediction:018f2b7b-0c11-72fd-95c4-1f9f98feac01",
  "submission_ids": ["..."]
}
```

The CLI wrapper is:

```bash
ironclaw traces worker-utility-credit \
  --endpoint https://trace-ingest.internal \
  --bearer-token-env TRACE_COMMONS_UTILITY_CREDIT_WORKER_TOKEN \
  --event-type ranking-utility \
  --credit-points-delta 1.25 \
  --reason "ranking eval utility" \
  --external-ref ranking_prediction:018f2b7b-0c11-72fd-95c4-1f9f98feac01 \
  018f2b7b-0c11-72fd-95c4-1f9f98feac01
```

The bulk worker route is intentionally limited to `regression_catch`, `training_utility`, and `ranking_utility`; it is not for `reviewer_bonus` or `abuse_penalty`. For model-derived ranking credit, prefer `/v1/workers/ranking/prediction-credit` so the service derives the amount and external reference from the active-model prediction instead of accepting hand-crafted worker inputs. Ranking utility events are credit-bearing only when the external reference is a single `ranking_prediction:<uuid>` for the same source.

Settlement treats `ranking_utility` as model-derived credit. A settlement run
without `ranking_model_version` excludes pending ranking utility events from the
source list. A settlement run that provides `ranking_model_version` uses
`ranking_target_use` or defaults it to `ranking_model_training`, then requires
the latest registered model version to be `active`, the active model policy to
match the request `policy_version`, and the latest matching calibration run to
be `promotable` and fresh when
`TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` is configured. The same
active model/target/policy risk report must be clear at settlement time; if
current evidence drift, non-promotable current evidence, or other model-risk
codes are present, ranking utility events for that gate are excluded. Each
selected ranking utility credit event must also reference
`ranking_prediction:<uuid>` for a prediction with the same submission, model,
target use, policy, and settlement-score micros as the credit delta; otherwise
the event is excluded from the settlement source list. Settlement responses
include aggregate `ranking_credit_events_excluded_reason_counts` for dry-runs
and live runs, with reason codes such as `missing_ranking_model_gate`,
`missing_prediction_ref`, `low_confidence_prediction`, and active-model risk
codes. Finalized batches record the calibration run id plus the calibration
report hash and joined-evidence hash used for the gate.
When a settlement request includes `near_contract_id`, the NEAR payload builder
validates it as a lowercase NEAR account id before any settlement batch or
outbox row is persisted.

Production settlement schedulers should use
`POST /v1/workers/credit-settlements/run` rather than the admin settlement
surface. The worker route accepts the same settlement policy fields plus an
optional `limit` for the maximum source credit events to settle in one run. If
omitted, the worker route settles at most 100 source events; explicit limits
must be between 1 and 500. Responses include the applied `limit`, the total
eligible source-event count, and `pending_after_count` so schedulers can retry
bounded batches until no eligible settlement work remains.
For ranking-derived credit, `POST /v1/workers/credit-cycle/run` wraps this route
after model calibration, promotion, and prediction-credit issuance so a
scheduler can move one model/policy/target through the credit path without
gaining generic admin settlement access.

Before a non-dry-run batch is finalized, the server rereads finalized settlement
batches and rejects any source credit event that has already been finalized in a
different batch. This gives retries and overlapping workers a final duplicate
issuance guard in addition to the initial idempotent source selection.

Contributors can read only their own append-only central credit events:

```http
GET /v1/contributors/me/credit-events
Authorization: Bearer <tenant-token>
```

Users can inspect their local ledger with:

```bash
ironclaw traces credit
ironclaw traces list-submissions
```

## Research Hooks

The MVP envelope intentionally reserves fields for later processing without implementing the whole central pipeline:

- `trace_card` documents consent scope, allowed uses, source channel, tool categories, retention, and revocation.
- `value_card` documents the score version, full scorecard, limitations, and user-visible credit explanation.
- `embedding_analysis` stores canonical summary hashes, vector IDs, nearest traces, clusters, duplicate score, novelty score, and coverage tags once a private worker fills them.
- `hindsight` keeps failed traces useful by allowing later subgoal/recoverability labels.
- `training_dynamics` supports future dataset cartography labels such as easy, ambiguous, or hard.
- `canonical_summary_for_embedding` builds redacted-only summaries for embedding and duplicate detection.
