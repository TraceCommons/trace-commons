# TraceDAO Server

Hosted server-side control plane for Trace Commons / TraceDAO.

This repository is the extraction point for the pieces that should not live
inside Ironclaw long term: public/private ingest, review queues, tenant access
grants, audit/retention/revocation workers, export jobs, upload-claim issuing,
object storage, and the production database schema.

## Current Extraction Status

This is a bridge extraction from Ironclaw's `gecko-pass` worktree. The hosted
server now owns its database, object-storage, and shared protocol surface
locally: the `TraceCorpusStore` trait, PostgreSQL backend, server migrations,
RLS diagnostics, encrypted artifact-store provider code, contribution envelope
DTOs, status DTOs, and deterministic redaction helpers live in this repo.
It also owns the first server-side Trace Credits settlement surface: hash-only
utility attestations, admin-triggered dry-run/final settlement batches, credit
holds, contributor pending/settled/held projections, and a NEAR non-transferable
credit receipt outbox. Utility workers can also run a bounded credit-cycle
coordinator that sequences calibration, model promotion, prediction credit,
settlement, and NEAR outbox submit/confirm checks for one model/policy/target. The
`POST /v1/workers/credit-cycle/scheduler/run` route lets utility-worker cron
jobs select the next eligible candidate or active model for a target/policy and
run at most a bounded number of credit cycles while skipping live claims and
models whose current ranking evidence is not yet promotable. Scheduler
`preflight_only` requests return eligible/skipped candidate decisions without
creating worker rows, credit events, settlement batches, or NEAR outbox rows. The
next ranking substrate is also server-owned: model
version records, hash-only feature records, prediction records, frontier/reviewer
labels, calibration reports, persisted model-promotion calibration runs, and
PostgreSQL-backed ranking evidence reads/writes behind the same DB mirror
cutover gates.
Benchmark publication now has the same server-side control-plane shape:
passed benchmark artifacts enqueue hash-only registry publish outbox rows,
source invalidation of published artifacts enqueues matching hash-only revoke
rows, workers can submit and confirm those rows through a configured external
registry adapter, mark external submit/confirm/fail status with receipt refs and hashed errors,
operators can inspect the outbox through the admin API, and maintenance
backfill/reconciliation covers registry outbox drift before external registry
adapter readiness is promoted.
Export job automation can now also run in-process when
`TRACE_COMMONS_EXPORT_JOB_SCHEDULER_TOKEN` is configured with an export-worker
bearer token. The scheduler retries due failed export jobs with bounded
backoff, then drains queued replay, benchmark-conversion, and ranker export jobs
through the same authenticated worker handlers used by external cron jobs.

## Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run settlement
batches, and optional NEAR receipt calls are queued only after off-chain
settlement finalizes. Operators record a hash-only central issuer approval
through `GET|POST /v1/admin/credit-settlement-approvals` after a dry-run
produces the exact settlement source-list hash. For partial issuance batches,
operators run the drill/admin route with `source_event_limit` or the worker and
scheduler routes with `limit`, record approval for the returned bounded
source-list hash, and reuse the same bound and approval hash when finalizing.
Deployments that set
`TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL=true` reject live
settlement unless the supplied `issuer_approval_evidence_hash` matches a recorded
approval for the tenant, policy version, canonical lowercase `sha256:<64 hex>`
source-list hash, and canonical approval evidence hash while still allowing
dry-runs. Settlement and issuer-approval `policy_version` values are bounded
safe identifiers, and when
`TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` is configured the
approval endpoint rejects unlisted policy versions before writing audit rows, so
they can be stored, audited, and mirrored into NEAR receipt args without
carrying free-form operator text. Set
`TRACE_COMMONS_CREDIT_SETTLEMENT_ISSUER_APPROVAL_MAX_AGE_HOURS` to make those
central approvals expire for live issuance and settlement drills; startup
rejects that max-age knob unless required issuer approval is enabled.
Set `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE=true` for
the early production mode where TraceDAO centrally runs issuance instead of
trusting a broader actor reputation system. That profile fails startup unless
DB mirror writes and PostgreSQL RLS readiness are fail-closed, the expected
PostgreSQL serving role is pinned with `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256`,
per-account caps are configured, managed EdDSA signed-token enforcement plus tenant access-grant
enforcement are enabled, exact central issuer principal refs are pinned with
`TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS`, fresh central source-list
approvals are required, a single NEAR credit contract is pinned and required,
NEAR submit/confirm adapters are configured, and adapter bearer auth is required
for both directions. The profile also requires
`TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY=true`, which makes
live settlement require a fresh green rollout-smoke preflight while keeping
dry-runs and drills available. When that principal allowlist is configured,
unlisted admins can still inspect dry-runs but cannot record issuer approvals,
finalize live settlement, manually mark NEAR credit outbox rows, or create NEAR
account freeze/unfreeze transitions from credit holds, and unlisted
reviewer/admin/worker principals cannot create positive credit through manual
credit mutation, utility credit/attestation, prediction-credit, process-evaluation
utility credit, or credit-bearing export generation. Those paths fail before
source reads, derived artifact writes, or queued benchmark/ranker export-job
claims. Unlisted utility-worker principals cannot start live settlement,
credit-cycle, or NEAR credit outbox schedulers or run live NEAR submit/confirm
workers. Admin
config status returns safe missing-control names for that central issuer profile
so operators can see which gate still needs configuration without exposing
adapter URLs, bearer tokens, approval evidence, or account refs. The
credit-settlement drill returns the same safe missing-control names, and
operational summary/metrics expose the missing-control count plus managed EdDSA
and tenant-grant enforcement booleans for dashboards. Live settlement also
re-checks the complete profile before writing settlement batches or NEAR outbox
rows, then checks rollout-smoke readiness before any live settlement repair or
write path; dry-runs remain available for diagnosis.
The worker `POST /v1/workers/credit-cycle/run` route can run the production
credit path in bounded steps for a single model/version:
calibration, model promotion, prediction credit, settlement, then a NEAR outbox
dry-run, explicit submit, or explicit confirmation poll. Live credit-cycle runs
also fail before claiming work when the central issuer profile is required but
incomplete or rollout-smoke readiness is required but not green; scheduler
preflight reports the same blockers as safe skip reasons.
Settlement retries repair missing NEAR outbox rows
from finalized batches, and revocation propagation can append deterministic
negative ledger rows plus `reverse_credit_receipt` NEAR outbox calls for settled
revoked sources. Reviewer/admin credit summaries report tenant-wide settled line
items while contributor summaries stay principal-scoped. Admins can place and
release credit holds around fraud/review investigations for existing
tenant-local credit ledger accounts; active holds block settlement, and released
holds project current state so later settlement resumes without exposing raw
hold/release reason text. Hold placement and release also
append hash-only credit-mutation audit rows. When a central NEAR contract is
configured, those hold transitions enqueue account freeze/unfreeze outbox rows
for the non-transferable contract; if the central issuer principal allowlist is
configured, only listed admins can create those NEAR account transitions. With the DB mirror configured, utility
attestations, settlement batches, credit holds, and NEAR receipt/account outbox
rows are dual-written to PostgreSQL;
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves the admin credit control-plane
lists from the tenant-scoped DB mirror.

Benchmark registry status is treated as a credit-readiness input, not as a raw
artifact transport. Published benchmark artifacts enqueue durable outbox rows
containing ids, registry refs, artifact/source hashes, evaluator refs, scores,
and lifecycle status only. The operational summary counts pending, submitted,
confirmed, and failed registry outbox work, and a published artifact remains in
the external-registry-adapter gap until a confirmed outbox receipt is recorded;
a revoked artifact with a registry ref remains in the external-registry
invalidation gap until a confirmed revoke receipt is recorded.
Configure `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` to let benchmark evaluation
workers call an operator-owned evaluator adapter over bounded derived benchmark
candidate summaries, artifact hashes, source-list hashes, and evaluator refs
instead of only using the in-process structural evaluator. Worker requests can
set `require_external_evaluator=true` to fail closed when that adapter is not
configured; optional bearer auth and timeout are controlled by
`TRACE_COMMONS_BENCHMARK_EVALUATOR_BEARER_TOKEN` and
`TRACE_COMMONS_BENCHMARK_EVALUATOR_TIMEOUT_MS`.
Configure `TRACE_COMMONS_PROCESS_EVALUATOR_URL` to let process-evaluation
workers run bounded evaluator batches over accepted trace derived summaries and
hashes, then write normal process-evaluation metadata plus optional hash-only
system ranking labels. Worker requests can set `require_external_evaluator=true`
to fail closed when the adapter is absent; optional bearer auth and timeout are
controlled by `TRACE_COMMONS_PROCESS_EVALUATOR_BEARER_TOKEN` and
`TRACE_COMMONS_PROCESS_EVALUATOR_TIMEOUT_MS`.
Configure `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_URL` to let benchmark
workers submit pending or failed rows to an operator-owned registry adapter;
optional bearer auth and timeout are controlled by
`TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_BEARER_TOKEN` and
`TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_TIMEOUT_MS`. Configure
`TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_URL` to let benchmark workers
poll submitted rows to confirmation; optional bearer auth and timeout are
controlled by `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_BEARER_TOKEN` and
`TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_TIMEOUT_MS`.

The NEAR path is intentionally an outbox of deterministic method-call payloads
for a non-transferable receipt contract. The payload builder only emits
`settle_credit_receipt`, `reverse_credit_receipt`, `freeze_credit_account`, and
`unfreeze_credit_account` calls, rejects malformed NEAR account ids, validates
the method-specific hash-only argument shape, recomputes the idempotency key,
and rejects any other NEAR credit method. Configured credit holds enqueue
account-freeze rows when an account transitions from zero to one active holds
and account-unfreeze rows when the last active hold is released. Stored outbox
calls are revalidated before submit or confirm workers hand them to a relayer,
so tampered local/DB rows fail closed as retryable outbox failures instead of
leaving the server boundary. Configure
`TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` to let the scoped submit worker hand
pending or failed-retry calls to an operator-owned NEAR relayer; the server then
records the public 43-44 character base58 NEAR transaction hash or a hashed
failure. Configure
`TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` to let the scoped confirm worker
poll submitted transactions through an operator-owned confirmer and move rows to
confirmed or failed status. Workers can still manually mark items submitted,
confirmed, or failed for fallback operations; when adapter auth is required,
manual submitted/confirmed marks also require the matching authenticated adapter
configuration so missing bearer-token readiness cannot be bypassed. Manual
confirmation follows the same lifecycle as the confirmer worker: an item must
already be submitted with a transaction hash, and confirmed items cannot be
downgraded through the fallback route. Set
`TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH=true` for production credit
issuance so configured submitter and confirmation adapters must also provide
their bearer-token settings before startup, live worker submission, or live
confirmation can proceed; pilot deployments can leave the gate disabled while
using private adapter endpoints. The server ledger remains
authoritative; NEAR payloads contain batch ids, account hashes, source-list
hashes, policy versions, optional issuer-approval evidence hashes folded into
attestation/signature hashes, amounts, and issuer-signature hashes, never trace
bodies or raw contributor identity. Confirmation requests stay hash-only around
the original call payload.
`GET /v1/admin/config-status` exposes only safe NEAR readiness fields for this
path: whether a submitter is configured, the configured submit timeout, outbox
submit bounds, whether a confirmer is configured, the configured confirmation
timeout, outbox confirmation bounds, and the credit-cycle step count. It does
not expose the relayer/confirmer URL, bearer token, hosts, or contributor
identity. It also exposes the configured issuer-approval max age as hours, never
the approval reason or external evidence reference.

Ranking evidence is stored separately from settlement. Admins can register
hash-only holdout calibration dataset manifests with source counts, label-source
counts, label-actor counts, and lifecycle status, while workers can register
feature hashes, model predictions, lab/reviewer/evaluator labels, and calibration
runs that record aggregate error, confidence, threshold policy, per-source
quality gates, joined label-source and label-actor diversity, joined-evidence
hashes, reason codes, and a hash-only report digest.
Client-supplied ranking evidence hashes must be canonical lowercase `sha256:<64 hex>`
digests across model manifests, calibration dataset manifests, feature evidence,
label/preference evidence, and calibration run evaluation datasets; symbolic
`sha256:` placeholders are rejected before persistence.
For an existing `(calibration_dataset_hash, target_use, policy_version)` holdout
key, lifecycle status updates must keep the source manifest hash and count
metadata unchanged; changing those manifest fields requires a new dataset hash
or policy version. File-backed writes and the PostgreSQL mirror enforce the
same invariant before the row can become ranking-credit evidence, and
file-backed readers fail closed if legacy JSONL history contains conflicting
manifest rows for the same holdout key.
Prediction writes must name a registered active or candidate model, match its
policy and feature schema, and reference an existing feature vector hash for the
same source. Utility workers can derive server-provenanced ranking features
through `/v1/workers/ranking/features/run`, which projects accepted
redacted-summary metadata into deterministic feature hashes, can require active
vector metadata for vector-backed duplicate/novelty signals, and reserves the
`feature_provenance:server_derived` and `feature_input:*` coverage tags so
manually posted feature rows cannot claim that provenance. Production
deployments can set
`TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE=true` to make
prediction-credit, readiness, and settlement require that server-owned feature
evidence before ranking utility credit can mint or settle. Calibration treats
repeated labels from the same source on the same submission and target use as
corrections, so only the latest
`(submission_id, target_use, label_source)` non-disputed label contributes to
sample counts, joined evidence, and error metrics. A latest `disputed` label
suppresses that source's calibration evidence until a newer non-disputed label
arrives. Label-source authority is enforced at write time: utility workers write
`frontier_lab`, reviewers write `reviewer`, benchmark workers write `benchmark`,
process-evaluation workers write `system`, and admins are the explicit override.
Admins can inspect label-adjudication readiness through
`/v1/admin/ranking/adjudication-report`, which groups latest absolute labels and
pairwise preferences by source, reports unresolved disputes and cross-source
direction/outcome conflicts, and exposes only safe counts, source/actor totals,
submission IDs, and reason codes. `/v1/admin/ranking/labeler-reliability-report`
rolls the same evidence up by label source and hashed actor principal so
operators can spot noisy sources or reviewers without exposing raw external
refs. Calibration runs also record `ranking_adjudication_issues_present` and
remain non-promotable while unresolved adjudication issues exist for the target
use, so stored calibration evidence cannot look credit-ready before review
settles. The admin ranking readiness drill includes the same clean-adjudication
gate by default and records `ranking_adjudication_issues=<count>` when disputed
or conflicting labels still need review.
Registered calibration datasets marked `deprecated` or `archived` are retired
and cannot feed new calibration runs for that target use and policy; promotion,
dataset-readiness, and active-model risk surfaces also report retired registered
holdouts as blockers. Registered holdout metadata must also meet the configured
source-count floor plus label-source and label-actor diversity floor before it
can feed a calibration run, so a thin manifest cannot become credit-bearing
evidence merely because the current label rows pass local error thresholds.
Production deployments can set
`TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY=true` so
calibration runs fail closed unless the requested model holdout hash has a
matching registered non-retired calibration dataset row for the same target use
and policy, and the same missing-registry blocker is enforced before promotion
or model-derived credit paths trust an active model. Production deployments can
also set `TRACE_COMMONS_RANKING_REQUIRE_ACTIVE_CALIBRATION_DATASET=true` to
require that registered holdout row to be `active`, turning candidate holdouts
into visible stewardship blockers until an admin promotes the curated manifest;
startup rejects the active-holdout gate unless the registry gate is also
enabled.
Startup also rejects an explicitly configured nonzero pairwise ordering-accuracy
floor unless `TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT` is greater than
zero, so operators cannot think pairwise accuracy is protecting credit while the
pairwise evidence floor is disabled.
When deployments require multiple joined label sources, calibration also
requires that many distinct label-writing actor principals so one worker cannot
satisfy source diversity by switching label-source enums. Model manifests must
use disjoint training and calibration dataset hashes, so the registered
calibration dataset acts as holdout evidence instead of a reused training split.
Admins can
inspect calibration reports, persisted calibration runs, dataset-readiness
reports, active-model risk reports, ranking-credit readiness reports, and
operational summary blocker counts before deciding whether model-derived credit
can settle. Dataset-readiness and ranking-credit readiness reports also surface
legacy calibration-registry manifest conflicts as safe aggregate blocker counts,
and operational summaries promote the same signal into
`ranking_calibration_dataset_manifest_conflicts` promotion-gate blockers.
Admins can inspect exact hash-only conflict keys through
`GET /v1/admin/ranking/calibration-dataset-conflicts`, which reports the latest
projected registry metadata and a remediation hint without exposing trace bodies
or raw lab references. If a key was imported with conflicting manifest history,
admins can retire it through
`POST /v1/admin/ranking/calibration-dataset-conflicts/quarantine`, which appends
an `archived` status row using the latest projected manifest metadata and
removes that key from active manifest-conflict blockers without rewriting
history. Strict DB mirror deployments preserve any older immutable manifest
metadata already stored in Postgres and mirror only the archival status update.
The quarantine action also appends a hash-only audit event with the conflict key
hash and operator-reason hash.
Dataset-readiness reports group candidate and active models by
registered holdout calibration dataset hash and show target-use readiness,
current evidence hashes, effective thresholds, error metrics, and blocker reason
counts without raw trace or lab evidence. Model promotion rechecks the current
joined prediction/label
evidence hash and current server-owned calibration floors against the latest
promotable calibration run before writing an active model status, so a candidate
cannot activate after labels, predictions, or production thresholds drift under
its calibration. The promotion dry-run response exposes the registered holdout
calibration dataset hash, stored calibration joined-evidence/report hashes and
counts including distinct joined label actors, and the freshly recomputed
current-evidence hash, report hash, counts including distinct joined label
actors, thresholds, error metrics, low-confidence count, promotability flag, and
reason codes, giving operators a hash-only preflight record before activation.
Settlement excludes `ranking_utility` credit events unless the request names an
active model version with a fresh promotable calibration run for the same policy,
target use, and registered calibration dataset, and every credit event is bound
to a matching `ranking_prediction:<uuid>` reference with a settlement score that
matches the stored prediction. Prediction-credit workers also require the active
model/target pair to have no uncleared model-risk report codes by default so new
credits wait for calibration/drift review before settlement. Readiness reports
and settlement re-check the same active-model risk codes, so manually appended
prediction-bound ranking credits cannot settle while current evidence is still
at risk. Admins can inspect candidate and active model/target backtests through
`/v1/admin/ranking/model-backtest-report`, which combines current calibration
metrics, pairwise preference ordering checks, label-adjudication blockers,
latest calibration hashes, and the same machine-readable reason codes before a
model is allowed to influence credit; model promotion now rejects candidates
whose current backtest has any blocking reason code. Process-evaluation workers
can also attach an idempotent
hash-only ranking label for ranking-allowed traces, letting trusted rubric
evaluators feed calibration without storing raw evaluator notes in ranking
metadata. Settlement
responses report aggregate ranking-credit exclusion reason counts for dry-runs
and live runs. A scoped promotion
run lets utility workers promote calibrated candidate models through the same
server-owned gate without generic admin access, and a scoped calibration run
lets utility workers schedule bounded calibration passes across active or
candidate models. Calibration, prediction-credit, promotion, and full
credit-cycle automation runs now persist a hash-only worker-run ledger with
running/completed/failed lifecycle status, limits, counts, result refs, skipped
reason aggregates, and safe hashed fatal-error refs for admin review. Live
non-dry-run ranking schedulers reject overlapping active non-stale runs before
appending a new running row; stale running rows surface as operational-summary
blockers until an admin append-finalizes them through the stale recovery API,
which also writes a hash-only audit event for the recovery action. Operational
summary ranking readiness now also includes candidate/active backtest pass/fail
counts, backtest reason-code totals, label-adjudication issue blockers,
ranking worker skip totals/reason aggregates, promotion-gate warnings for
actionable worker skips, export-job request metadata, atomic queued-job
claiming, claimed replay/benchmark/ranker-training job execution, a bounded
queued export scheduler, controlled failed-job retry, and bounded retry/backoff
worker automation for replayable worker exports, stale export-job blockers with
an admin-only stale export-job recovery route, PostgreSQL RLS readiness aggregate
counts, runtime-role hash match gauges, safe
promotion-gate counts in the read audit row, and structured warning logs for the
aggregate gate set plus each individual promotion gate whenever promotion gates
are blocked or warning. NEAR credit and benchmark registry outbox workers also
emit safe completion/failure logs with tenant refs, purpose hashes, row ids, and
counts only, avoiding adapter URLs, raw receipts, transaction hashes, account
hashes, or raw error text. Core DB dual-write, export scheduler, evaluator
adapter, vector payload, backfill, and generic internal-error logs likewise emit
stable `error_hash`/`reason_hash` fields instead of raw error strings. Admins
can also scrape `/v1/admin/operational-metrics` for a Prometheus-text snapshot
of the same safe promotion, per-gate,
worker-skip, rollout-smoke, submission, review SLA, export, retention, vector,
benchmark, and delayed-credit gauges. The same summary response includes a
`rollout_smoke` preflight block with the required canary smoke-check names,
including `tenant_canary_isolation`, `db_reconciliation_clean`,
`rollback_flag_drill`, `key_rotation_drill`, vector indexing, analytics release,
ranking-model readiness, credit settlement, object-primary reads, object-store
migration, PostgreSQL RLS readiness, and audit-chain verification, plus revocation-effect checks for delayed credit
reversal and object deletion receipts, along with promotion-gate readiness, recorded evidence
counts, passed evidence counts, failed evidence counts, stale evidence counts,
and explicit missing rehearsal-evidence counts so operators do not mistake a
clean gate snapshot for an up-to-date rehearsed rollout. Latest per-check smoke
evidence older than 24 hours is reported as stale and blocks rollout-smoke
readiness until a fresh pass or fail is recorded. Admins can list or append
hash-only smoke rehearsal evidence through `GET` and `POST`
`/v1/admin/rollout-smoke/evidence`; `GET`
accepts `latest_only=true` when operators only need current per-check state. The
server stores the evidence hash and a hash of any external reference in the
tenant audit chain, then clears the matching required-check gap only after a
latest fresh `passed` evidence event exists and reports latest passed checks,
failed checks, stale checks, and never-recorded checks separately.
With the DB mirror configured, ranking dataset registry rows, evidence,
calibration runs, and ranking worker runs are dual-written to PostgreSQL.
Maintenance backfill mirrors file-backed ranking model versions, calibration
dataset registry rows, feature/prediction/label evidence, calibration runs, and
worker-run rows into the DB. Maintenance reconciliation compares ranking model
versions, calibration dataset registry rows, feature/prediction/label evidence,
calibration report hashes, and worker-run lifecycle rows across file and DB
storage, and it reports legacy calibration-registry manifest conflicts as
`ranking_calibration_dataset_manifest_conflict_keys` blockers before DB reviewer
reads or credit-bearing ranking paths are promoted; the admin readiness and
operational-summary surfaces expose only counts for the same conflict class.
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves admin ranking lists, calibration
reports, model-risk reports, credit-readiness reports, calibration-run history,
and worker-run history from the tenant-scoped DB mirror.

## Binaries

- `tracedao-ingest`: hosted ingest/review/admin/worker API.
- `tracedao-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/tracedao-protocol`: shared TraceDAO protocol DTOs and redaction helpers.
- `crates/tracedao-server`: Rust server binaries.
- `migrations`: TraceDAO server database schema, renumbered as this repo's
  first migration.
- `docs`: copied Trace Commons design, storage, and roadmap docs that now belong
  with the hosted server/control plane.

## Local Development

From this repository:

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo now builds without an Ironclaw path dependency. Ironclaw should depend
on the shared `tracedao-protocol` crate when the client-side integration is
rewired.
