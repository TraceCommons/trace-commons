# TraceCommons Server

Hosted server-side control plane for Trace Commons / TraceCommons.

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

## Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run settlement
batches, and optional NEAR receipt calls are queued only after off-chain
settlement finalizes. The worker `POST /v1/workers/credit-cycle/run` route can
run the production credit path in bounded steps for a single model/version:
calibration, model promotion, prediction credit, settlement, then a NEAR outbox
dry-run, explicit submit, or explicit confirmation poll. Settlement retries repair missing NEAR outbox rows
from finalized batches, and revocation propagation can append deterministic
negative ledger rows plus `reverse_credit_receipt` NEAR outbox calls for settled
revoked sources. Reviewer/admin credit summaries report tenant-wide settled line
items while contributor summaries stay principal-scoped. With the DB mirror
configured, utility attestations, settlement batches, credit holds, and NEAR
receipt outbox rows are dual-written to PostgreSQL;
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
`settle_credit_receipt`, `reverse_credit_receipt`, and `freeze_credit_account`
calls, rejects malformed NEAR account ids, and rejects any other NEAR credit
method. Configure
`TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` to let the scoped submit worker hand
pending or failed-retry calls to an operator-owned NEAR relayer; the server then
records the public transaction hash or a hashed failure. Configure
`TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` to let the scoped confirm worker
poll submitted transactions through an operator-owned confirmer and move rows to
confirmed or failed status. Workers can still manually mark items submitted,
confirmed, or failed for fallback operations. The server ledger remains
authoritative; NEAR payloads contain batch ids, account hashes, source-list
hashes, policy versions, amounts, and issuer-signature hashes, never trace
bodies or raw contributor identity. Confirmation requests stay hash-only around
the original call payload.
`GET /v1/admin/config-status` exposes only safe NEAR readiness fields for this
path: whether a submitter is configured, the configured submit timeout, outbox
submit bounds, whether a confirmer is configured, the configured confirmation
timeout, outbox confirmation bounds, and the credit-cycle step count. It does
not expose the relayer/confirmer URL, bearer token, hosts, or contributor
identity.

Ranking evidence is stored separately from settlement. Admins can register
hash-only holdout calibration dataset manifests with source counts, label-source
counts, label-actor counts, and lifecycle status, while workers can register
feature hashes, model predictions, lab/reviewer/evaluator labels, and calibration
runs that record aggregate error, confidence, threshold policy, per-source
quality gates, joined-evidence hashes, reason codes, and a hash-only report
digest.
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
settles.
Registered calibration datasets marked `deprecated` or `archived` are retired
and cannot feed new calibration runs for that target use and policy; promotion,
dataset-readiness, and active-model risk surfaces also report retired registered
holdouts as blockers.
Production deployments can set
`TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY=true` so
calibration runs fail closed unless the requested model holdout hash has a
matching registered non-retired calibration dataset row for the same target use
and policy, and the same missing-registry blocker is enforced before promotion
or model-derived credit paths trust an active model.
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
counts, and the freshly recomputed current-evidence hash, report hash, counts,
thresholds, error metrics, low-confidence count, promotability flag, and reason
codes, giving operators a hash-only preflight record before activation.
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
queued export scheduler, and controlled failed-job retry for replayable worker
exports, stale export-job blockers with an admin-only stale export-job recovery
route, PostgreSQL RLS readiness aggregate counts, safe
promotion-gate counts in the read audit row, and structured warning logs for the
aggregate gate set plus each individual promotion gate whenever promotion gates
are blocked or warning. Admins can also scrape `/v1/admin/operational-metrics`
for a Prometheus-text snapshot of the same safe promotion, per-gate,
worker-skip, rollout-smoke, submission, review SLA, export, retention, vector,
benchmark, and delayed-credit gauges. The same summary response includes a
`rollout_smoke` preflight block with the required canary smoke-check names,
including `tenant_canary_isolation`, `db_reconciliation_clean`,
`rollback_flag_drill`, `key_rotation_drill`, PostgreSQL RLS readiness, and
audit-chain verification, along with promotion-gate readiness, recorded evidence
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

- `trace-commons-ingest`: hosted ingest/review/admin/worker API.
- `trace-commons-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/trace-commons-protocol`: shared TraceCommons protocol DTOs and redaction helpers.
- `crates/trace-commons-server`: Rust server binaries.
- `migrations`: TraceCommons server database schema, renumbered as this repo's
  first migration.
- `docs`: copied Trace Commons design, storage, and roadmap docs that now belong
  with the hosted server/control plane.

## Local Development

From this repository:

```bash
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo now builds without an Ironclaw path dependency. Ironclaw should depend
on the shared `trace-commons-protocol` crate when the client-side integration is
rewired.
