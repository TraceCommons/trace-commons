# Trace Commons Production Storage Plan

This document tracks the migration path from the file-backed Trace Commons ingest MVP to production storage. It assumes the existing `ironclaw.trace_contribution.v1` envelope remains the upload contract, while production ingest moves trust, authorization, retention, review, export, and credit accounting into durable services.

## Current State

The hosted TraceCommons server still supports tenant-scoped JSON pilot state under `TRACE_COMMONS_DATA_DIR` for local development and controlled pilots, but the production storage boundary now lives in this repository rather than in Ironclaw.

This repository owns the production-storage surface:

- `migrations/V1__trace_commons_schema.sql`, consolidated as this repo's first landing schema for the full Trace Commons relational control plane.
- `migrations/V2__trace_credit_settlement.sql`, adding hash-only utility attestations, settlement batches, credit holds, and the NEAR non-transferable receipt outbox.
- `migrations/V3__trace_ranking_evidence.sql`, adding ranking model versions, feature hashes, prediction records, labels, and calibration-report source tables.
- `migrations/V4__trace_ranking_calibration_runs.sql`, adding persisted aggregate calibration runs for model promotion gates.
- `migrations/V5__trace_credit_settlement_ranking_gate.sql`, adding ranking calibration-gate metadata to finalized credit settlement batches.
- `migrations/V6__trace_force_rls.sql`, forcing RLS on every Trace Commons tenant-scoped table.
- `migrations/V7__trace_ranking_calibration_label_source_gate.sql`, adding label-source diversity evidence to persisted calibration runs.
- `migrations/V8__trace_ranking_calibration_source_error_gate.sql`, adding per-label-source calibration error evidence to persisted calibration runs.
- `migrations/V9__trace_ranking_calibration_joined_evidence_hash.sql`, adding deterministic joined prediction/label evidence hashes to persisted calibration runs.
- `migrations/V10__trace_credit_settlement_joined_evidence_hash.sql`, binding ranking credit settlements to calibration joined-evidence hashes.
- `migrations/V11__trace_ranking_worker_runs.sql`, adding a hash-only ranking automation run ledger for prediction-credit and model-promotion workers.
- `migrations/V12__trace_ranking_worker_run_lifecycle.sql`, adding running/completed/failed lifecycle fields to ranking worker-run rows.
- `migrations/V13__trace_credit_settlement_exclusion_reasons.sql`, preserving aggregate ranking-credit exclusion reason counts on settlement batches.
- `migrations/V14__trace_ranking_preference_labels.sql`, adding hash-only pairwise preference labels for ranking training evidence.
- `migrations/V15__trace_benchmark_registry_outbox.sql`, adding durable hash-only external benchmark registry outbox rows.
- `migrations/V16__trace_ranking_calibration_datasets.sql`, adding the hash-only ranking calibration/holdout dataset registry.
- `migrations/V17__trace_ranking_calibration_dataset_manifest_immutability.sql`, preventing calibration dataset manifest rewrites after creation.
- `migrations/V18__trace_central_rls_tenant_predicate.sql`, centralizing the tenant predicate used by every Trace Commons RLS policy.
- `migrations/V19__trace_ranking_calibration_label_actor_count.sql`, adding distinct joined label-actor diversity evidence to persisted calibration runs.
- `crates/trace-commons-server/src/trace_corpus_storage.rs` and the PostgreSQL `TraceCorpusStore` implementation.
- `crates/trace-commons-server/src/trace_artifact_store.rs` and the encrypted local service object-store provider.
- Optional ingest-service DB dual-write behind `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- Optional DB-backed tenant policy reads behind `TRACE_COMMONS_DB_TENANT_POLICY_READS=true`.
- Tenant-policy export ABAC for replay, benchmark, and ranker exports using existing allowed consent scopes and allowed uses.
- Optional fail-closed benchmark/ranker source object-ref validation behind `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true`.
- Admin-token tenant policy management through `/v1/admin/tenant-policy`, with hash-chained file audit events and safe DB audit metadata for policy version, allow-list counts, and the policy projection hash.
- Admin-token config inspection through `/v1/admin/config-status`, returning only safe schema, DB cutover, object-primary, guardrail, max export item cap, analytics broad-release noise readiness/max delta plus epsilon accounting caps, submission quota, legal hold, ranking calibration freshness, holdout registry and active-holdout requirements, label-source diversity, worker-run stale-threshold, NEAR submitter/confirmer readiness and timeout, NEAR outbox submit/confirm bounds, credit-cycle step count, credit-cycle scheduler bounds/preflight support, export-job scheduler enablement and bounded limits without bearer tokens or raw retry notes, vector-index worker bounds plus scheduler enablement/interval/limit/dry-run, private embedder readiness/timeout/required status, and private searcher readiness/timeout/required status without bearer tokens, URLs, or raw purpose text, object-store provider alias, object-store I/O enabled, object-primary object-store eligibility, and PostgreSQL RLS readiness fields with a read audit event.
- Admin-token operational inspection through `/v1/admin/operational-summary` and `ironclaw traces operational-summary`, returning only safe tenant-scoped aggregate counts for submission status/risk, review SLA pressure, DB export manifests/jobs including stale running export-job blockers plus export-scheduler readiness, retention jobs, analytics broad-release min-cell/noise readiness, vector coverage plus private embedder/searcher/scheduler readiness booleans and nearest-neighbor policy-gap blockers for stale or cross-profile vector duplicate/novelty evidence, artifact object-store readiness booleans, benchmark lifecycle readiness plus external evaluator, registry submitter/confirmer, failed registry outbox rows, and external registry adapter/invalidation blockers, ranking model/credit readiness including process-evaluator readiness, backtest pass/fail counts, and reason totals, label-adjudication issue counts and reason totals, calibration-registry manifest-conflict blockers, ranking worker-run lifecycle health plus skip totals/reason aggregates, PostgreSQL RLS readiness blockers with aggregate table counts only, object-primary object-store readiness blockers, promotion-gate warnings for actionable ranking worker risk/ineligible skips, delayed credit totals with explicit revocation-reversal event counts and reversed points, credit-settlement account-cap readiness for positive delayed credit, and a `rollout_smoke` preflight block that names required canary checks, including `tenant_canary_isolation`, `db_reconciliation_clean`, `rollback_flag_drill`, `key_rotation_drill`, retention dry-run, vector indexing, analytics release, benchmark pipeline readiness, ranking-model readiness, credit settlement, object-store migration, PostgreSQL RLS readiness, and audit-chain verification, while reporting recorded, passed, failed, stale, and missing rehearsal evidence. Admin-only `GET /v1/admin/rollout-smoke/preflight` returns the same readiness block plus latest hash-only evidence per required check, writes a safe read audit row, and gives operators a single promotion-readiness read after recording drills. Admin-only `GET /v1/admin/vector-entries` lists bounded tenant-scoped vector metadata with hash/model/score/lifecycle fields but no raw trace bodies or embedding values. Reviewer/admin list-style reads default to 100 items and reject explicit limits outside 1..=500; outbox submit/confirm workers and the revocation-propagation worker reject explicit batch limits outside their documented 1..=500 range as well. Admin-only `GET /v1/admin/rollout-smoke/evidence` lists hash-only smoke evidence, with `latest_only=true` for current per-check state, and `POST /v1/admin/rollout-smoke/evidence` appends a hash-only evidence event for one required check to the tenant audit chain; optional external references are persisted only as hashes, only latest fresh `passed` evidence clears a required-check gap, and latest passed/failed/stale checks remain visible as separate signals. Admin-only `POST /v1/admin/db-reconciliation-drill` computes `db_reconciliation_clean` evidence from the existing file-vs-DB reconciliation engine without maintenance expiration, backfill, vector-index, or audit-chain side effects, returning only safe aggregate counts and compact blocker codes before optionally appending the hash-only smoke evidence row. Admin-only `POST /v1/admin/rollback-drill` computes the `rollback_flag_drill` evidence from file/DB submission, audit, and tombstone parity without deleting or rewriting rows, and can append the hash-only smoke evidence row directly. Admin-only `POST /v1/admin/key-rotation-drill` computes `key_rotation_drill` evidence from safe managed-EdDSA keyset counts, issuer/audience/JTI/TTL policy, and guarded refresh freshness without exposing key ids, PEMs, hosts, URLs, or fetch credentials. Admin-only `POST /v1/admin/postgres-rls-drill` computes `postgres_rls_readiness` evidence from safe PostgreSQL table-count, policy-count, FORCE RLS, and role-bypass diagnostics without exposing row data. Admin-only `POST /v1/admin/retention-dry-run-drill` computes `retention_dry_run` evidence through the real maintenance dry-run selector with backfill, vector indexing, reconciliation, and audit-chain verification disabled, returning only aggregate candidate counts, dry-run deletion guards, and compact blocker codes before optionally appending hash-only evidence. Admin-only `POST /v1/admin/vector-index-drill` computes `vector_index` evidence through the real vector worker selector in dry-run mode, returning candidate/index coverage counts, private vector readiness booleans, nearest-neighbor policy-gap counts, and compact blockers without writing vector entries or exposing scheduler tokens/purposes. Admin-only `POST /v1/admin/analytics-release-drill` computes `analytics_release` evidence through the broad analytics privacy-budget preflight, returning only safe min-cell/noise/readiness fields, compact blocker codes, and a hash while never exposing the count-noise key or raw aggregate maps. Admin-only `POST /v1/admin/benchmark-readiness-drill` computes `benchmark_pipeline` evidence from benchmark artifacts, evaluator readiness, registry outbox status, and adapter gaps without exposing raw benchmark artifacts. Admin-only `POST /v1/admin/ranking/readiness-drill` computes `ranking_model_readiness` evidence from the active-model risk, dataset-readiness, and ranking-credit readiness reports while returning only safe counts, reason-code aggregates, blocker codes, and an evidence hash. Admin-only `POST /v1/admin/credit-settlement-drill` computes `credit_settlement` evidence through the credit-risk summary and dry-run settlement selector, validates the optional NEAR contract id without writing settlement batches or outbox rows, requires a configured per-account settlement cap by default unless `require_account_cap` is disabled for a pilot rehearsal, and returns only safe account hashes, aggregate risk counts, exclusion reason counts, blocker codes, and an evidence hash. Admin-only `POST /v1/admin/object-store-migration-drill` computes `object_store_migration` evidence by writing, reading, and deleting a short encrypted probe artifact through the configured service-owned object-store abstraction while returning only safe aliases, booleans, blocker codes, and hashes. Admin-only `POST /v1/admin/audit-chain-drill` computes `audit_chain_verification` evidence from file and optional DB audit-chain verifier counts while returning only blocker codes and failure hashes. The read audit event records only safe item, readiness, blocking, warning, and actionable-skip counts. Non-clean promotion gates also emit an aggregate structured warning log plus one per-gate structured event with the safe tenant storage ref, severity, gate name, and count. `/v1/admin/operational-metrics` returns a Prometheus-text snapshot of the same safe promotion, per-gate object-primary object-store blockers, PostgreSQL RLS subcheck/gap, artifact object-store readiness, worker-skip, rollout-smoke recorded/passed/failed/stale/missing evidence, submission, review SLA, export job and export-infrastructure, retention, analytics release readiness, vector coverage, vector nearest-neighbor policy gaps, and vector-infrastructure readiness, benchmark including failed registry outbox rows, ranking evaluator readiness, delayed-credit, delayed-credit reversal, and credit-settlement account-cap gauges and writes a read audit row for the scrape.
- Admin-only `POST /v1/admin/analytics-release-drill` computes `analytics_release` evidence through the broad analytics privacy-budget preflight, returning only safe min-cell/noise/epsilon-ledger readiness fields, compact blocker codes, and a hash while never exposing the count-noise key or raw aggregate maps.
- `/v1/admin/operational-metrics` also emits safe ranking model, backtest, adjudication, ranking-credit readiness, calibration manifest-conflict, and reason-code gauges so dashboards can track credit-issuance blockers without reading raw trace or lab evidence.
- `/v1/admin/operational-summary` and `/v1/admin/operational-metrics` expose safe NEAR non-transferable credit outbox readiness: aggregate `pending`, `submitted`, `confirmed`, `failed`, `pending_without_submitter`, and `submitted_without_confirmer` counts plus submitter/confirmer configured flags. Those projections make pending rows without a submitter, submitted rows without a confirmer, and failed outbox rows promotion blockers without exposing transaction hashes, account hashes, contract ids, adapter URLs, or raw failure text.
- Admin-only `POST /v1/admin/canary-read-drill` turns an existing canary submission into hash-only evidence for the `rollout_smoke` canary/read readiness checks: `submit_status`, `tenant_canary_isolation`, `contributor_credit`, `reviewer_metadata`, `replay_export_selection`, and `audit_reads`, returning aggregate booleans, counts, and blocker codes without exposing contributor tokens or raw trace content.
- Admin-only `POST /v1/admin/object-primary-read-drill` records `object_primary_reads` smoke evidence for an existing canary submission by checking service-owned submitted-envelope object refs, tenant/hash readability, review and replay object-ref reads, plaintext-body absence, and a fallback tenant that stays off object-primary rollout.
- Admin-only `POST /v1/admin/object-store-migration-drill` records `object_store_migration` smoke evidence for the configured service-owned object-store backend by proving hash-only encrypted probe write/read/delete behavior without returning object keys, paths, credentials, or probe payloads.
- Admin-only `POST /v1/admin/ranking/readiness-drill` records passed or failed `ranking_model_readiness` smoke evidence by checking that active ranking models have no current risk codes, model-target readiness is unblocked, calibration dataset manifests have no conflicts, and pending model-derived ranking credits are ready when required.
- Admin-only `POST /v1/admin/revocation-propagation-drill` computes `revocation_propagation` smoke evidence through the real worker dry-run selector, returning only aggregate due/completed/failed/skipped/pending counts and dry-run blocker codes without claiming propagation rows.
- Admin-only `POST /v1/admin/revocation-effects-drill` records `delayed_credit_reversal` and `object_deletion_refs` smoke evidence after the live revocation worker has run, using only aggregate counts for reversed credit events, NEAR reversal outbox rows, deleted service-owned object refs, and physical-delete receipts.
- Admin-triggered Trace Credits settlement through `/v1/admin/credit-settlements`, plus utility-worker automation through `/v1/workers/credit-settlements/run`, the model-scoped `/v1/workers/credit-cycle/run` coordinator, and `/v1/workers/credit-cycle/scheduler/run` for bounded next-eligible model selection that skips live credit-cycle claims and preflight-skips models without promotable current ranking evidence or with uncleared pairwise policy risk before claiming work, with side-effect-free `preflight_only` eligibility decisions, dry-run support, bounded source-event limits for worker schedulers, idempotent source-event selection, a late finalized-source conflict guard before batch persistence, held-account exclusion, contributor pending/settled/held projections, tenant-wide reviewer/admin settled-credit projections, ranking-utility exclusion unless the request names an `active` model version with a latest promotable calibration run for the same policy and target use, no uncleared active-model risk codes, and each ranking credit event references a matching `ranking_prediction:<uuid>` external ref, aggregate ranking-credit exclusion reason counts in dry-run/live responses, optional NEAR non-transferable receipt outbox rows, retry repair for finalized batches missing their NEAR outbox rows, relayer-backed submit worker automation through `/v1/workers/near-credit-outbox/submit`, confirmation polling through `/v1/workers/near-credit-outbox/confirm`, worker status updates through `/v1/workers/near-credit-outbox/mark-status`, and revocation-triggered `reverse_credit_receipt` outbox rows for settled sources whose original batch had a NEAR contract. Utility attestations, settlement batches, credit holds, and NEAR outbox rows now dual-write into the PostgreSQL mirror when configured; `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` makes those control-plane mirror failures fail closed, and `TRACE_COMMONS_DB_REVIEWER_READS=true` serves the admin credit settlement/hold/attestation/outbox lists from the DB mirror.
- Admin credit risk inspection through `/v1/admin/credit-risk-summary`, returning only tenant-scoped aggregate pending/held/over-cap counts and bounded per-account hashes before issuance so operators can review caps or holds without raw account refs, source event ids, external refs, or hold details. The optional `limit` defaults to 100 accounts and is capped at 500; totals remain pre-truncation. `POST /v1/admin/credit-settlement-drill` wraps that same risk projection with a side-effect-free settlement dry run, requires a production-shaped NEAR contract and configured per-account settlement cap by default, blocks on held accounts, over-cap accounts, truncated risk summaries, settlement exclusions, missing account-cap policy, and missing pending credit unless explicitly relaxed, and can record `credit_settlement` rollout-smoke evidence.
- Admin credit-hold release through `/v1/admin/credit-holds/{hold_id}/release`, requiring an admin token and non-empty operator reason before projecting a held account back into current released state. Hold placement and release append hash-only credit-mutation audit rows; the release path keeps API responses hash-safe, stores only the existing hold fields plus `released_at`, and lets settlement resume after the risk summary or settlement drill shows no remaining active hold, cap, or ranking-policy blocker.
- Utility-worker evidence ingestion through `/v1/workers/utility-attestations`, storing policy version, source ids, evidence hash, and external-ref hash without raw lab notes or trace bodies.
- Ranking evidence ingestion through `/v1/workers/ranking/features`, `/v1/workers/ranking/features/run`, `/v1/workers/ranking/predictions`, `/v1/workers/ranking/prediction-credit`, `/v1/workers/ranking/prediction-credit/run`, `/v1/workers/ranking/labels`, `/v1/workers/ranking/preference-labels`, `/v1/workers/ranking/calibration-runs`, and `/v1/workers/ranking/calibration-runs/run`, plus admin inspection through `/v1/admin/ranking/model-versions`, `/v1/admin/ranking/calibration-datasets`, `/v1/admin/ranking/calibration-dataset-conflicts`, `/v1/admin/ranking/calibration-dataset-conflicts/quarantine`, `/v1/admin/ranking/model-promotions`, `/v1/admin/ranking/features`, `/v1/admin/ranking/predictions`, `/v1/admin/ranking/labels`, `/v1/admin/ranking/preference-labels`, `/v1/admin/ranking/adjudication-report`, `/v1/admin/ranking/labeler-reliability-report`, `/v1/admin/ranking/calibration-report`, `/v1/admin/ranking/pairwise-evaluation-report`, `/v1/admin/ranking/model-backtest-report`, `/v1/admin/ranking/dataset-readiness-report`, `/v1/admin/ranking/worker-runs`, and `/v1/admin/ranking/calibration-runs`. Calibration dataset registrations are hash-only holdout manifests: they store the calibration dataset hash, target use, policy version, source-list manifest hash, source count, label-source count, label-actor count, lifecycle status, and hashed actor principal, never raw trace bodies or lab references; status-only lifecycle updates for the same `(calibration_dataset_hash, target_use, policy_version)` holdout key must keep source manifest/count metadata unchanged, file-backed writes and PostgreSQL mirror writes reject manifest/count rewrites, file-backed readers fail closed on legacy manifest conflicts, and manifest or count changes require a new dataset hash or policy version; dataset-readiness, credit-readiness, and operational-summary routes project legacy conflicts into safe `calibration_dataset_manifest_conflict_count` or promotion-gate blocker counts instead of hiding the blocker behind a generic read failure, while the dedicated conflict report lists exact hash-only conflict keys, latest projected registry metadata, and remediation hints. Conflict quarantine appends an `archived` status row using the latest projected manifest metadata for the conflicted key, so operators can retire bad legacy history without rewriting prior rows; retired conflicts no longer count as active manifest-conflict blockers, while the archived holdout still blocks new calibration, promotion, and credit through the ordinary `calibration_dataset_retired` gate. Under strict DB mirror writes, quarantine mirrors a status-only archive update when Postgres already has older immutable manifest metadata for the key, preserving the DB manifest fields instead of accepting a rewrite. `deprecated` and `archived` rows retire that target/policy holdout from new calibration runs, model promotion, dataset-readiness, and active-model risk clearance. Prediction writes must bind to a registered active/candidate model version, that model's policy and feature schema, and an existing feature vector hash for the same source; the feature-run worker can derive server-provenanced features from accepted redacted-summary derived metadata, can require active vector metadata to use vector-backed duplicate/novelty inputs, reserves `feature_provenance:server_derived` plus `feature_input:*` tags so manual feature writes cannot claim those inputs, and `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE=true` makes prediction-credit, readiness, and settlement require that server-owned feature evidence. Pairwise preference labels bind preferred/rejected accepted submissions for the same target use and store only hashes plus a positive preference strength, so reward/preference-model training evidence does not bypass absolute-utility calibration. Ranking label-source authority is role-bound before writes: utility workers may write `frontier_lab`, reviewer tokens may write `reviewer`, benchmark workers may write `benchmark`, process-evaluation workers may write `system`, and admins are the explicit override. Ranker training-pair exports select eligible pairwise preference labels before falling back to score-order heuristics and serialize only safe label metadata, while pairwise-evaluation reports score latest candidate/active model settlement ordering against those labels with joined/correct/reversed/tied counts, accuracy micros, and margin aggregates; both surfaces still require only hash-only evidence and no raw trace bodies or external refs. Model-backtest reports apply the current calibration and pairwise checks to candidate and active model targets, returning pass/fail counts, latest calibration hashes, joined-evidence/error metrics, pairwise accuracy, and the same reason-code vocabulary before promotion or credit issuance. Active model-risk reports include the same pairwise counts and configured pairwise evidence/accuracy thresholds, flag `pairwise_evidence_below_threshold` when joined pairwise support is below the deployment floor, flag `pairwise_accuracy_below_threshold` when joined pairwise evidence exists and correct ordering falls below the deployment threshold, and flag `calibration_dataset_not_registered` or `calibration_dataset_retired` when holdout registry policy is not satisfied, so prediction-credit, readiness, operational summaries, and settlement gates can block at-risk active models. Calibration runs must use the registered model's calibration dataset hash and, when `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY=true`, a matching non-retired calibration dataset registry row for the target use and policy; they ignore mismatched legacy prediction policy/schema rows, de-duplicate repeated labels from the same source on the same submission and target use to the latest non-disputed row before counting samples or computing joined-evidence/error metrics, treat a latest `disputed` label as a challenge that removes that source from joined calibration evidence until superseded, persist a deterministic hash of the joined prediction/label evidence set, can require multiple distinct joined label sources through `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT`, require matching distinct label-writing actor principals for that source-diversity floor, and reject promotion when any joined label-source cohort exceeds the configured average absolute error threshold even if the global average passes. Registered model manifests are immutable for a `model_version`: changing schema, policy, training dataset, calibration dataset, or artifact hash requires a new model version while status changes reuse the same manifest. Admins can register candidate models before calibration, and the promotion route changes a model version to `active` only when the requested model/policy/target-use/calibration-dataset evidence is promotable, the latest model/policy/calibration-dataset evidence is still promotable, the holdout registry gate is satisfied, and the current joined evidence hash still matches that calibration run, so active model status is evidence-backed even if legacy mutable metadata exists. The dataset-readiness report groups latest model manifests by registered calibration dataset hash and derives target-use readiness from the same model, prediction, label, calibration, and holdout-registry ledgers, exposing only hashes, counts, thresholds, errors, and reason codes. The calibration-run worker scans active/candidate models in bounded dry-run or live batches and records the same hash-only worker-run lifecycle rows as other ranking automation. The prediction-credit worker converts a positive active-model prediction settlement score into one idempotent `ranking_utility` credit event bound to `ranking_prediction:<uuid>` only when the active model/target has no current model-risk report codes, and the run route lets schedulers scan stored predictions in bounded batches while skipping existing credit without consuming the limit and, by default, skipping uncredited predictions whose active model/target has uncleared model-risk report codes; readiness and settlement re-check those active model-risk codes so manual ranking utility rows cannot bypass drift review. The credit-cycle worker sequences calibration, model promotion, prediction credit, settlement, and NEAR outbox dry-run/live submit/confirm checks for one model/policy/target without granting generic admin settlement access, while recording its own `credit_cycle` lifecycle row and rejecting overlapping live non-stale cycles for the same model/policy/target. Ranking calibration, prediction-credit, model-promotion, and credit-cycle worker runs persist hash-only run rows with lifecycle status, limits, counts, result refs, skip reason aggregates, and safe fatal-error hashes. Ranking model versions, calibration dataset registry rows, features, predictions, labels, preference labels, calibration runs, worker runs, and the resulting credit/audit rows now dual-write into the PostgreSQL mirror when configured; `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` makes those ranking evidence mirror failures fail closed, and `TRACE_COMMONS_DB_REVIEWER_READS=true` serves the admin ranking evidence, worker-run, and calibration surfaces from the DB mirror.
- Ranking dataset isolation treats a model's registered calibration dataset as holdout evidence. New model manifests whose training and calibration dataset hashes overlap are rejected, and legacy overlapping manifests are blocked by model-promotion, prediction-credit, settlement, and active model-risk gates using `training_calibration_dataset_overlap`.
- Admin ranking adjudication inspection through `/v1/admin/ranking/adjudication-report`, which groups latest absolute labels and pairwise preferences into safe issue counts for unresolved `disputed` labels, cross-source absolute-label outcome conflicts, and reversed pairwise preferences without exposing raw external refs. Calibration runs, backtest reports, and active-model risk reports emit `ranking_adjudication_issues_present` for target uses with unresolved adjudication issues, so model promotion, prediction credit, settlement, and operational summary all see the same blocker.
- Admin ranking labeler-reliability inspection through `/v1/admin/ranking/labeler-reliability-report`, which aggregates absolute-label, preference-label, dispute, absolute-conflict, pairwise-conflict, and total issue participation counts by label source and `sha256:` actor-principal hash without exposing raw actor refs or external refs.
- Ranking worker-run rows now include lifecycle status, completion timestamps, and optional hashed error details while keeping operator reason text hashed. Calibration worker runs count non-promotable persisted calibration run reasons, including label-adjudication blockers, in their reason aggregates so operators can distinguish a completed artifact write from credit-ready evidence. Operational metrics export safe worker-run lifecycle, checked/succeeded/skipped totals, pending-after backlog, and reason-code counts for ranking automation dashboards. Live non-dry-run ranking calibration, prediction-credit, model-promotion, and credit-cycle schedulers reject overlapping active non-stale rows for the same tenant and broad/narrow matching filters before appending a new `running` row, while admins can append-finalize stale running rows as `failed` through `/v1/admin/ranking/worker-runs/{ranking_worker_run_id}/recover-stale` and append a `ranking_worker_run_recovery` audit row without storing raw recovery notes.
- Ranking model promotion automation through `/v1/workers/ranking/model-promotions/run`, letting utility workers run bounded dry-run or live scans of latest candidate models for a target use, reuse the admin calibration/freshness/diversity promotion gate plus overlapping live-run guard, skip ineligible candidates with reason counts, and promote calibrated candidates without generic admin promotion scope.
- Ranking production policy includes `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY`, `TRACE_COMMONS_RANKING_REQUIRE_ACTIVE_CALIBRATION_DATASET`, `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE`, `TRACE_COMMONS_RANKING_MIN_LABEL_COUNT`, `TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD`, `TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS`, `TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT`, and `TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS`, server-owned quality gates applied after worker request parsing and model-risk recomputation so calibration and prediction-credit workers cannot bypass active holdout stewardship, holdout registration, server-derived feature provenance, or the deployment's minimum sample-size, ordering-accuracy, or quality requirements.
- Model promotion and active-model risk recomputation re-apply the current server-owned calibration floors to stored calibration evidence, so raising production thresholds blocks old calibration runs from activating models, minting prediction credit, or settling ranking utility until fresh evidence clears the new policy.
- Direct admin registration of an `active` ranking model is rejected because activation is target-use scoped. Admins register `candidate` model manifests and use `/v1/admin/ranking/model-promotions`, or utility workers use `/v1/workers/ranking/model-promotions/run`, to append the evidence-backed `active` status row.
- Model-promotion dry-runs now act as the candidate holdout-readiness preflight: responses include the registered calibration dataset hash, stored calibration report/joined-evidence hashes and counts, distinct joined label-actor counts, freshly recomputed current evidence hash/report hash/counts, effective server-owned thresholds, aggregate/per-source error metrics, low-confidence prediction count, promotability flag, and reason codes without exposing trace bodies or raw lab references. Promotion also requires the current model backtest to pass, so pairwise evidence, ordering failures, or unresolved label-adjudication issues block activation before a candidate can mint or settle ranking credit.
- Admin ranking model-backtest inspection through `/v1/admin/ranking/model-backtest-report`, which applies current calibration, pairwise preference, and label-adjudication checks to latest candidate and active model/target-use pairs, returns pass/fail counts, latest calibration hashes, joined-evidence/error metrics, pairwise accuracy, and the same machine-readable reason codes before promotion or credit issuance.
- Admin ranking model-risk inspection through `/v1/admin/ranking/model-risk-report`, which recomputes current joined-evidence hashes for active model/target-use pairs and reports current joined-label source diversity, calibration thresholds, aggregate/per-source error metrics, pairwise preference ordering accuracy, post-calibration evidence drift, low-confidence fresh predictions, stale/non-promotable calibration status, training/calibration dataset overlap, aggregate risk-code counts, and per-model risk codes without exposing trace bodies or raw lab references.
- Admin ranking credit readiness inspection through `/v1/admin/ranking/credit-readiness-report`, which explains whether pending positive ranking utility credit events are ready to settle or blocked by prediction refs, model status, calibration freshness/promotability/diversity, held accounts, score mismatches, confidence gates, calibration-registry manifest conflicts, or uncleared active-model risk codes.
- Ranking prediction-credit issuance and ranking-utility readiness/settlement all enforce the latest calibration confidence threshold and active-model risk report for the referenced active-model prediction, so low-confidence or currently at-risk predictions can be monitored but cannot mint or settle credit.
- Ranker training candidate and pair exports collapse exact canonical-summary hash duplicates before the export source-list hash and idempotent delayed utility credit are produced, preferring the representative with the lower duplicate score and then higher novelty/ranker score.
- Optional encrypted local artifact storage behind `TRACE_COMMONS_ARTIFACT_KEY_HEX`, with `TRACE_COMMONS_OBJECT_STORE=local_service` selecting the service-owned local encrypted backend used for production-shaped object refs. `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true` can fail startup unless the configured service-owned object store reports both object versioning and restore-after-delete support; local-service and default filesystem-remote report false, while filesystem-remote with `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING=true` archives encrypted records on delete and supports restore-after-delete rehearsal.
- Optional object-primary submit/review mode behind `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true`, which requires the DB/object-ref cutover guards and skips plaintext submitted/reviewed envelope body files while retaining compatibility metadata, derived records, and file audit rows. Object-primary envelope writes use unique encrypted artifact object ids per logical snapshot so review/process-evaluation writes do not overwrite ciphertext behind older submitted-envelope object refs.
- Optional object-primary replay export mode behind `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true`, which requires DB replay selection, required replay object refs, required DB mirror writes, and a service-owned encrypted object store (`local_service` or enabled `remote_service`).
- Optional object-primary benchmark/ranker export mode behind `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true`, which requires DB reviewer reads, required source object refs, export guardrails, required DB mirror writes, and a service-owned encrypted object store (`local_service` or enabled `remote_service`) before skipping plaintext benchmark artifact and ranker provenance files.
- `TRACE_COMMONS_OBJECT_STORE=remote_service` now parses provider, bucket/root, KMS key, and service credential references. `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=file_system` enables a filesystem-backed service-owned remote adapter for production-like object I/O rehearsals, using `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET` as an absolute root path plus `TRACE_COMMONS_ARTIFACT_KEY_HEX` for envelope encryption. `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING=true` makes filesystem-remote deletes archive encrypted object records under the service-owned root so rollback drills can restore by object ref across process restarts. AWS/GCS/Azure provider selections still advertise the disabled `trace_commons_service_owned_remote_disabled` alias, refuse artifact I/O and plaintext fallback, and do not satisfy object-primary or required-versioning startup guards until those adapters are compiled.
- Optional legal-hold retention policy IDs behind `TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES`, preventing maintenance from newly expiring or purging matching server-derived policy classes; review, maintenance, replay export, benchmark conversion, and ranker source selection fail closed if file-backed metadata claims a mismatched server retention policy id or extends `expires_at` beyond the retention window derived from the stored allowed uses.
- Optional DB-backed review leases behind `TRACE_COMMONS_DB_REVIEWER_READS=true`, scoped by tenant and reviewer/admin principal for concurrent privacy review coordination. The reviewer/admin `POST /v1/review/leases/claim-next` and `POST /v1/review/leases/claim-batch` routes plus `ironclaw traces review-lease-claim-next` and `ironclaw traces review-lease-claim-batch` helpers select only available quarantined traces in tenant scope using review escalation/SLA ordering before persisting lease state and typed safe claim audit rows. Reviewer/admin `GET /v1/review/routing-summary` returns aggregate queue pressure and hash-only assignee load counts without trace bodies or raw reviewer principals. Reviewer/admin `POST /v1/review/batch-decisions` applies one bounded common decision to explicit submission ids while reusing the same per-item eligibility, lease, ABAC, body-read, mirror, and audit path as single review decisions.
- Durable DB revocation-propagation item rows track tenant-scoped downstream invalidation or retry work for object refs, export manifests/items, vectors, derived artifacts, benchmark/ranker artifacts, credit settlement reversals, and physical delete receipts.
- Replay dataset, benchmark conversion, and ranker export paths persist durable access-grant and export-job lifecycle rows; already-started jobs are terminalized as `failed` if DB metadata reads, retention metadata validation, source collection, source object-ref revalidation, source-read audit mirroring, or required object-ref body reads fail before export artifacts or manifests can be published. `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_TOKEN` enables an optional in-process scheduler that authenticates with an export-worker bearer token, retries due failed replayable jobs with bounded exponential backoff, then drains queued jobs through the same worker handlers used by external schedulers. `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` enables an optional in-process scheduler that authenticates with a vector-worker bearer token, requires the DB mirror at startup, and runs the same bounded `/v1/workers/vector-index` route on an interval without retention-maintenance side effects.
- Durable tenant access grant rows can store issuer-authorized principal, role, consent-scope, allowed-use, issuer/audience/subject, expiry, revocation, and safe metadata for hosted-agent multitenant permissioning. Admin routes and CLI helpers can create, list, and revoke the current tenant's grant rows while writing safe grant-update audit metadata. `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true` makes trace submission, contributor credit/status readback, reviewer/audit reads, review mutations, dataset/export paths, non-revocation worker mutations, maintenance, and admin ledger/observability reads fail closed unless the authenticated tenant/principal has an active exact-role grant. Signed EdDSA/Ed25519 claims must additionally match any configured grant issuer, audience, and JWT `sub` subject binding; static-token bridge grants keep exact-principal matching and ignore those signed-claim-only fields. Grant scope/use allow-lists are intersected with static or EdDSA claim allow-lists before the existing submission policy checks run; revocation/self-delete, revocation propagation, config-status, tenant-policy admin, and grant-management routes remain available for deprovisioning and recovery.
- Optional fail-closed maintenance promotion gate behind `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true`, which requires DB dual-write, rejects maintenance requests that omit `reconcile_db_mirror: true`, exposes compact `blocking_gaps`, turns DB/file reconciliation gaps into `409 Conflict` maintenance failures, treats DB audit hash-chain drift and canonical audit-payload projection drift as named blocking gaps, and only allows global or tenant-allowlisted DB reader promotion when `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` also makes mirror writes fail closed.
- Caller-level tests for tenant-scoped writes, DB-backed tenant policy enforcement, review/revocation state, delayed credit events, encrypted artifact receipts, DB object-ref replay reads through the service-owned local object-store backend, fail-closed embedded-tenant validation for file-backed metadata, stored envelope bodies with conflicting server tenant refs, derived, ledger, audit, tombstone, replay manifest, export provenance, and benchmark artifact reads, service-local object-ref key-ref and envelope server-tenant-scope verification, encrypted benchmark artifact body verification, vector/benchmark/ranker payload deletion verification, privileged-action ABAC for review decisions, destructive purge, and tombstones, reasoned privileged revocation persistence across file/DB tombstones and audit rows, and the admin operational summary aggregate surface.

Production still needs stronger guarantees before broad tenant rollout:

- Relational metadata for broader workflow state, credit, audit, retention, and export manifests.
- Encrypted object storage for redacted envelope bodies and large derived artifacts. The current service-owned local backend remains the lowest-risk migration step; enabled `remote_service` with the filesystem provider records a remote service-owned object-store alias in `trace_object_refs`, stores ciphertext under tenant/submission-hashed paths, verifies ciphertext hashes on read, and exercises the same object-primary guardrails. Cloud remote providers are still disabled until they can write/read ciphertext without exposing bucket credentials to contributors or reviewers.
- A vector store for approved redacted summaries and allowed redacted trace fields.
- Tenant isolation derived from authenticated request identity, never from envelope fields.
- Idempotent revocation and retention propagation across metadata, objects, vectors, worker queues, exports, and credit.

The production migration should not replace the local queue/capture semantics. Clients should keep producing locally redacted envelopes and the service should keep re-scrubbing before acceptance.

## Storage Boundaries

Use the relational database for metadata, authorization decisions, workflow state, hashes, object references, indexes, and append-only ledgers. Do not store full trace bodies, large benchmark payloads, vector embeddings, or export blobs directly in relational rows.

Use encrypted object storage for:

- Submitted redacted envelopes.
- Server re-scrubbed envelope versions.
- Review snapshots when reviewers need a frozen view.
- Benchmark/replay artifacts.
- Export result files and manifest payloads when they exceed comfortable row size.
- Worker intermediate artifacts that must survive restarts.

Use the vector database for:

- Embeddings generated only from approved redacted summaries or explicitly allowed redacted fields.
- Tenant-scoped vector ids linked back to submission ids and derived record ids.
- Duplicate, novelty, nearest-neighbor, and cluster metadata. Persist the final worker output in relational metadata so the vector index can be rebuilt.

Do not put bearer tokens, raw local paths, raw sidecar spans, unredacted trace text, or raw tool payloads into any production store.

## Schema Sketch

## Concrete DB Migration Slice

This first production-storage slice is now owned by the TraceCommons server repo. It creates the relational control plane only: envelope payloads belong in encrypted artifact storage, and vector payloads can stay in a vector store or backend-specific index. `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` can mirror metadata into the DB when `TRACE_COMMONS_DB_DUAL_WRITE=true`, including submission redaction counts, derived summary/tool/coverage metadata, vector-entry metadata, replay export manifest metadata, replay export source item rows, benchmark/ranker export provenance metadata, utility attestations, credit settlement batches, credit holds, NEAR credit outbox rows/status, ranking evidence plus calibration-run and worker-run summaries, tombstones, and read/export/credit audit events. Vector indexing computes deterministic redacted-summary similarity for nearest-neighbor metadata, keeping exact canonical-summary hash matches as the strongest duplicate signal; when `TRACE_COMMONS_VECTOR_EMBEDDER_URL` is configured, the worker sends only redacted canonical-summary embedding inputs plus hash/tool/category/coverage metadata to a trusted private embedder and stores the returned model/version/dimension/vector-store metadata. When encrypted artifact storage is configured, vector indexing also writes a redacted canonical-summary vector payload as a `worker_intermediate` object ref, and later vector-index passes can use readable active compatible payloads for model-backed nearest-neighbor duplicate/novelty scoring with `embedding:` cluster ids. When `TRACE_COMMONS_VECTOR_SEARCH_URL` is configured, vector indexing can query a trusted private vector-search adapter with the target embedding vector and hash-only metadata, then accepts only response vector-entry ids that still match active tenant DB vector metadata and current derived trace ids before using them for duplicate/novelty scoring; empty validated search results fall back to compatible payload scans and then summary similarity. Deployments can require the private embedder with `TRACE_COMMONS_VECTOR_EMBEDDER_REQUIRE_EXTERNAL=true` and the private vector-search adapter with `TRACE_COMMONS_VECTOR_SEARCH_REQUIRE_EXTERNAL=true`; the latter blocks startup and vector-index worker runs unless `TRACE_COMMONS_VECTOR_SEARCH_URL` is configured. Export audit rows now carry deterministic source-list hashes in `decision_inputs_hash` for replay datasets, benchmark conversion artifacts, and ranker training exports; aggregate read audit rows mirror typed safe metadata with only a code-owned surface and item count; trace-content-read audit rows carry safe typed metadata with a code-owned surface plus optional purpose hash instead of raw worker/export purpose strings; revocation audit rows carry only a reason hash in typed metadata while keeping the existing reason field for reviewer/operator context. The DB mirror derives aggregate, trace-content, and revocation metadata records from the safe reason fields when callers pass `Empty`, requires submitted audit rows to carry typed submission metadata with a status that matches the canonical submitted event, and rejects mismatched typed metadata before write. File-backed audit rows also carry optional `previous_event_hash`/`event_hash` fields so pilot logs can be checked for simple append-order tampering while legacy rows remain readable, and DB audit rows mirror those chain fields when they are present on the file-backed event. Maintenance can return a verifier report with `verify_audit_chain: true`. Replay dataset exports also mirror durable tenant-scoped manifest rows with source ids, source-list hashes, per-source status/hash snapshots, and the active submitted-envelope object ref used at export time. Benchmark and ranker exports write file-backed provenance manifests by default and opportunistically mirror them into the same export manifest/item tables with source derived artifact refs plus active canonical-summary vector entry refs when vector metadata has already been indexed, while the replay manifest listing endpoint filters DB rows back to replay dataset manifests. Tenant policies now gate replay, benchmark, and ranker export requests and source selection using the same allowed-scope/allowed-use columns as ingest. `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` requires DB dual-write and makes benchmark/ranker exports fail closed before artifact, provenance, or utility-credit publication when any selected source lacks an active submitted-envelope object ref that can be tenant/hash verified. `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` requires DB dual-write, required DB mirror writes, DB reviewer reads, reviewer object-ref reads, and an enabled service-owned encrypted object store, then writes submitted and reviewed envelope bodies only to the object store while leaving compatibility metadata/derived/audit files in place. `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT=true` requires DB dual-write, required DB mirror writes, DB replay export reads, replay object-ref-required reads, and an enabled service-owned encrypted object store, then keeps replay export body reads on active DB object refs without file fallback. `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` requires DB dual-write, required DB mirror writes, DB reviewer reads, required derived source object refs, export guardrails, and an enabled service-owned encrypted object store, then skips plaintext benchmark artifact/provenance and ranker provenance files; DB manifest/items remain the purpose-filter and lifecycle-invalidation index. The maintenance endpoint can expire past-due pilot records, mirror expiration status plus artifact invalidation into the DB, invalidate benchmark/ranker provenance manifests, backfill pilot file records plus credit, audit, replay-manifest, ranking model, ranking evidence, ranking calibration, and ranking worker-run control-plane rows into the DB, index accepted canonical summaries into deterministic or private-adapter vector metadata rows with `index_vectors: true`, and return a file-vs-DB reconciliation report with `reconcile_db_mirror: true`. Backfill-only maintenance isolates malformed submission metadata files, malformed derived metadata files, per-submission envelope/derived gaps, credit-event failures, audit-event failures, replay-manifest failures, and ranking row failures, returns `db_mirror_backfill_failed` plus bounded failure details, and keeps valid records moving while DB setup/listing failures still fail fast. Audit backfill preserves typed safe metadata for tenant policy updates, tenant access-grant updates, exports, benchmark conversions, review leases, aggregate reads, trace-content reads, revocations, and ranking worker-run recovery rows when the file-backed audit reason carries those fields. Reconciliation now includes submission, derived, object-ref, vector, credit-ledger, audit-event, replay/export-manifest, export-item, revocation/tombstone counts, DB canonical audit-payload projection failures, ranking model/evidence/calibration/worker-run counts and drift diagnostics, active derived/export rows that still point at invalid sources, reader-projection parity for contributor credit, reviewer metadata, analytics, audit, and replay/export manifest surfaces, plus compact `blocking_gaps`; audit projection drift is reported as `db_audit_canonical_projection_failures`, trace-content-read metadata drift is compared back to the canonical reason, revocation metadata drift is compared back to the canonical reason hash, and audit-reader parity includes only a safe DB error hash. `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` requires DB dual-write, rejects maintenance requests that omit `reconcile_db_mirror`, and turns those promotion-blocking gaps into `409 Conflict` maintenance failures after the normal maintenance audit event is appended. Reconciliation without a configured DB mirror returns `503 Service Unavailable`. File-backed APIs remain the default source of pilot responses. `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true` can switch contributor credit, credit-event, and submission-status reads to the DB mirror after dual-write or backfill is in place. `TRACE_COMMONS_DB_REVIEWER_READS=true` can switch reviewer/admin metadata reads for analytics, trace listing, quarantine queue, active-learning queue, benchmark candidate conversion, ranker exports, credit settlement/hold/attestation/outbox lists, and ranking evidence/calibration runs to the DB mirror; review decisions also prefer active DB object refs for submitted-envelope body reads, mirror a content-read audit row, append a fresh reviewed-envelope object ref after approval or rejection, and can fail closed with `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` when no active object ref exists. `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true` can select replay export records from DB metadata and resolve submitted envelope bodies through active DB object refs for file or encrypted local artifact stores, with tenant/object-ref/hash verification and content-read audit mirroring that records `object_ref_id` for DB object-ref reads. Compatibility mode falls back to the file-backed envelope body if no active DB object ref exists; `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` makes that surface fail closed. `TRACE_COMMONS_DB_AUDIT_READS=true` can serve reviewer audit reads from the DB mirror. `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` is the write-side production cutover switch: it requires DB dual-write and makes critical mirror misses on submissions, revocations, reviews, credit events, credit settlement control-plane rows, exports/provenance, ranking evidence/calibration/worker-run rows, and audit/content-read rows fail closed.

The dedicated `POST /v1/workers/vector-index` route now runs the same vector metadata writer without entering the broader retention-maintenance path. It requires the DB mirror up front, applies a bounded `limit` with a default of 100 and a maximum of 500, rejects invalid explicit limits with a client error before DB mirror checks, returns checked/indexed/skipped/pending counts, and leaves expiration, purge, retention-ledger, and reconciliation work to the admin or retention worker routes. The optional `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` loop calls this route with the configured interval, limit, dry-run flag, and purpose, after startup validation proves the token has vector-worker authority and the DB mirror exists.

Submitted audit-event backfill enriches typed `Submission` metadata from the matching file submission record when available, preserving the stored status and privacy risk instead of falling back to `unknown`. Reconciliation also reports `db_audit_submission_metadata_mismatches` when submitted-audit metadata carries a stale privacy-risk projection for the DB submission row, and treats it as a promotion blocker.

Export-job control rows preserve `trace_export_job_request.v1` metadata across
start, completion, and failure. The metadata records safe replayability inputs
for requested/effective limits, status/privacy/consent filters, and hashes of
worker external refs without storing raw external ticket or lab references.
Export workers can atomically claim the oldest unexpired queued job through the
PostgreSQL control plane, which moves the row to `running` with safe claim
metadata and leaves expired queued rows untouched for recovery/inspection. The
executor path uses that claimed row for replay, benchmark-conversion,
ranker-candidate, and ranker-pair exports: it reconstructs filters from the
safe metadata snapshot, publishes the export through the existing
manifest/artifact/provenance/audit paths, and terminalizes the same row as
`complete` or `failed` instead of creating a second job. A bounded scheduler
route can drain up to `max_jobs` queued rows across all supported dataset kinds
(default 10, max 50; explicit values outside 1..=50 are rejected), or one
requested kind, and keeps making progress after per-job failures by
marking those claimed rows `failed` with hash-only failure metadata before
claiming the next row. Admin retry can requeue only failed, unexpired jobs that
still carry the replayable request snapshot; retry clears terminal execution
fields, increments a safe retry counter, and preserves only reason hashes plus
hashed/admin principal refs before workers claim the row again. The worker retry
pass applies the same replayability guard with bounded max jobs, retry-count
caps, and exponential-delay backoff before returning due failed rows to
`queued`; explicit retry values outside `max_jobs` 1..=50, `max_retry_count`
1..=25, or delay windows 0..=86400 seconds are rejected, and
`max_delay_seconds` must be at least `base_delay_seconds`.

Reconciliation also reports DB audit hash-chain drift as `db_audit_hash_chain_failures` when mirrored audit rows have invalid hash format, a genesis or predecessor mismatch, or a canonical-payload hash mismatch. Those failures are promotion blockers just like canonical projection drift.

Reconciliation also samples the latest bounded audit-reader page from files and the DB mirror and compares their public reader projections. Drift is reported as `audit_reader_sample_parity=failed` with hashed diagnostics in `audit_reader_sample_failures`, so legacy DB rows with matching ids and counts but stale action/reason projection still block reader promotion without exposing raw audit reasons.

Reviewer audit-event reads remain bounded even after DB audit reads are enabled. The API applies the `limit` at the storage boundary, parsing only the latest file-backed audit tail in compatibility mode or querying PostgreSQL by tenant with `audit_sequence DESC LIMIT` in DB-backed mode. Both paths recompute the returned rows' hash-chained payload hashes when chain fields are present, and reject incomplete or mismatched hash fields instead of serving a tampered bounded page.

Credit-settlement promotion uses the same maintenance channel. Backfill now counts and mirrors file-backed utility attestations, credit settlement batches, credit holds, and NEAR credit outbox rows/status with isolated per-row failure reporting; reconciliation reports file/DB counts plus missing-id gaps for each of those control-plane families, status drift for settlement batches and NEAR outbox rows, hold-release drift, and those gaps feed `blocking_gaps`.

### Safe Migration Naming

TraceCommons server migrations start from this repository's own history:

- PostgreSQL storage lands as `migrations/V1__trace_commons_schema.sql`.
- Ironclaw no longer lands Trace Commons relational migrations; its retained
  `migrations/V25__wasm_fuel_limit_bump.sql` belongs to the client/runtime repo.
- Future server migrations should use the next server-local version number.

### PostgreSQL DDL Sketch

```sql
CREATE TABLE trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'suspended', 'retention_only', 'deleted')),
    data_residency_region TEXT,
    default_retention_policy_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    updated_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_access_grants (
    grant_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('contributor', 'reviewer', 'admin', 'export_worker', 'retention_worker', 'revocation_worker', 'vector_worker', 'benchmark_worker', 'process_eval_worker')),
    allowed_scopes TEXT[] NOT NULL DEFAULT '{}',
    allowed_uses TEXT[] NOT NULL DEFAULT '{}',
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by TEXT,
    revoked_by TEXT,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, principal_ref, role)
);
CREATE INDEX idx_trace_access_grants_principal ON trace_access_grants(tenant_id, principal_ref);

CREATE TABLE trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes TEXT[] NOT NULL,
    allowed_uses TEXT[] NOT NULL,
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('received', 'accepted', 'quarantined', 'rejected', 'revoked', 'expired', 'purged')),
    privacy_risk TEXT NOT NULL CHECK (privacy_risk IN ('low', 'medium', 'high')),
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending NUMERIC(18, 6),
    credit_points_final NUMERIC(18, 6),
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ,
    review_assigned_to_principal_ref TEXT,
    review_assigned_at TIMESTAMPTZ,
    review_lease_expires_at TIMESTAMPTZ,
    review_due_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    purged_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id)
);
CREATE INDEX idx_trace_submissions_status_received ON trace_submissions(tenant_id, status, received_at DESC);
CREATE INDEX idx_trace_submissions_summary_hash ON trace_submissions(tenant_id, canonical_summary_hash);
CREATE INDEX idx_trace_submissions_expires ON trace_submissions(tenant_id, expires_at);
CREATE INDEX idx_trace_submissions_contributor ON trace_submissions(tenant_id, contributor_pseudonym);
CREATE INDEX idx_trace_submissions_review_lease ON trace_submissions(tenant_id, status, review_lease_expires_at, received_at DESC);

CREATE TABLE trace_object_refs (
    object_ref_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL CHECK (artifact_kind IN ('submitted_envelope', 'rescrubbed_envelope', 'review_snapshot', 'benchmark_artifact', 'export_artifact', 'worker_intermediate')),
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    compression TEXT,
    created_by_job_id UUID,
    valid_from TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, object_store, object_key)
);
CREATE INDEX idx_trace_object_refs_submission ON trace_object_refs(tenant_id, submission_id, artifact_kind);

CREATE TABLE trace_derived_records (
    derived_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('current', 'invalidated', 'superseded', 'revoked', 'expired')),
    worker_kind TEXT NOT NULL CHECK (worker_kind IN ('server_rescrub', 'summary', 'duplicate_precheck', 'embedding', 'ranking', 'benchmark_conversion', 'process_evaluation')),
    worker_version TEXT NOT NULL,
    input_object_ref_id UUID,
    input_hash TEXT NOT NULL,
    output_object_ref_id UUID,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    task_success TEXT,
    privacy_risk TEXT CHECK (privacy_risk IS NULL OR privacy_risk IN ('low', 'medium', 'high')),
    event_count INTEGER,
    tool_sequence TEXT[] NOT NULL DEFAULT '{}',
    tool_categories TEXT[] NOT NULL DEFAULT '{}',
    coverage_tags TEXT[] NOT NULL DEFAULT '{}',
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    UNIQUE (tenant_id, submission_id, derived_id)
);
CREATE INDEX idx_trace_derived_current ON trace_derived_records(tenant_id, submission_id, status, worker_kind);
CREATE INDEX idx_trace_derived_summary_hash ON trace_derived_records(tenant_id, canonical_summary_hash);

CREATE TABLE trace_vector_entries (
    vector_entry_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID NOT NULL,
    vector_store TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension > 0),
    embedding_version TEXT NOT NULL,
    source_projection TEXT NOT NULL CHECK (source_projection IN ('canonical_summary', 'redacted_messages', 'redacted_tool_sequence')),
    source_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'invalidated', 'deleted')),
    nearest_trace_ids TEXT[] NOT NULL DEFAULT '{}',
    cluster_id TEXT,
    duplicate_score REAL,
    novelty_score REAL,
    indexed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id) ON DELETE CASCADE,
    UNIQUE (tenant_id, submission_id, vector_entry_id)
);
CREATE INDEX idx_trace_vector_entries_source ON trace_vector_entries(tenant_id, submission_id, status);

CREATE TABLE trace_audit_events (
    audit_event_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    job_id UUID,
    submission_id UUID,
    object_ref_id UUID,
    export_manifest_id UUID,
    action TEXT NOT NULL CHECK (action IN ('submit', 'read', 'review', 'credit_mutate', 'revoke', 'export', 'retain', 'purge', 'vector_index', 'benchmark_convert', 'process_evaluate')),
    reason TEXT,
    request_id TEXT,
    decision_inputs_hash TEXT,
    metadata_kind TEXT NOT NULL DEFAULT 'empty' CHECK (metadata_kind IN ('empty', 'submission', 'review_decision', 'export', 'maintenance')),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    previous_event_hash TEXT,
    event_hash TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id)
);
CREATE INDEX idx_trace_audit_events_target ON trace_audit_events(tenant_id, submission_id, created_at DESC);
CREATE INDEX idx_trace_audit_events_action ON trace_audit_events(tenant_id, action, created_at DESC);

CREATE TABLE trace_credit_ledger (
    credit_event_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN ('accepted', 'privacy_rejection', 'duplicate_rejection', 'benchmark_conversion', 'regression_catch', 'training_utility', 'reviewer_bonus', 'abuse_penalty')),
    points_delta NUMERIC(18, 6) NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL CHECK (settlement_state IN ('pending', 'final', 'reversed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE
);
CREATE INDEX idx_trace_credit_ledger_account ON trace_credit_ledger(tenant_id, credit_account_ref, created_at DESC);
CREATE INDEX idx_trace_credit_ledger_submission ON trace_credit_ledger(tenant_id, submission_id);

CREATE TABLE trace_export_manifests (
    export_manifest_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    requested_by_principal_ref TEXT NOT NULL,
    purpose TEXT NOT NULL CHECK (purpose IN ('replay_dataset', 'benchmark_eval', 'ranking_training', 'model_training', 'analytics')),
    consent_scope_filter TEXT[] NOT NULL DEFAULT '{}',
    allowed_use_filter TEXT[] NOT NULL DEFAULT '{}',
    review_state_filter TEXT[] NOT NULL DEFAULT '{}',
    privacy_risk_filter TEXT[] NOT NULL DEFAULT '{}',
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'complete', 'failed', 'revoked_invalid', 'expired_invalid')),
    item_count INTEGER NOT NULL DEFAULT 0,
    manifest_object_ref_id UUID,
    result_object_ref_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    FOREIGN KEY (tenant_id, manifest_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    FOREIGN KEY (tenant_id, result_object_ref_id) REFERENCES trace_object_refs(tenant_id, object_ref_id),
    UNIQUE (tenant_id, export_manifest_id)
);

CREATE TABLE trace_export_manifest_items (
    export_manifest_id UUID NOT NULL REFERENCES trace_export_manifests(export_manifest_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID,
    object_ref_id UUID,
    vector_entry_id UUID,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    revoked_after_export_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (export_manifest_id, tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, vector_entry_id) REFERENCES trace_vector_entries(tenant_id, submission_id, vector_entry_id)
);

CREATE TABLE trace_benchmark_artifacts (
    benchmark_artifact_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID,
    benchmark_kind TEXT NOT NULL CHECK (benchmark_kind IN ('replay', 'process_eval', 'regression_case', 'ranking_pair')),
    artifact_version TEXT NOT NULL,
    object_ref_id UUID NOT NULL,
    requirements_hash TEXT,
    status TEXT NOT NULL CHECK (status IN ('candidate', 'approved', 'published', 'invalidated', 'deleted')),
    created_by_job_id UUID,
    published_export_manifest_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, derived_id) REFERENCES trace_derived_records(tenant_id, submission_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id, object_ref_id) REFERENCES trace_object_refs(tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, published_export_manifest_id) REFERENCES trace_export_manifests(tenant_id, export_manifest_id)
);

CREATE TABLE trace_tombstones (
    tombstone_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_at TIMESTAMPTZ NOT NULL,
    retain_until TIMESTAMPTZ,
    created_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (tenant_id, submission_id)
);
CREATE INDEX idx_trace_tombstones_hashes ON trace_tombstones(tenant_id, redaction_hash, canonical_summary_hash);

CREATE TABLE trace_retention_jobs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    retention_job_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'dry_run', 'running', 'complete', 'failed', 'paused')),
    requested_by_principal_ref TEXT NOT NULL,
    requested_by_role TEXT NOT NULL,
    purge_expired_before TIMESTAMPTZ,
    prune_export_cache BOOLEAN NOT NULL DEFAULT TRUE,
    max_export_age_hours BIGINT,
    audit_event_id UUID,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    selected_revoked_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_revoked_count >= 0),
    selected_expired_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_expired_count >= 0),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id)
);
CREATE INDEX idx_trace_retention_jobs_created ON trace_retention_jobs(tenant_id, created_at DESC);
CREATE INDEX idx_trace_retention_jobs_status ON trace_retention_jobs(tenant_id, status, updated_at DESC);

CREATE TABLE trace_retention_job_items (
    tenant_id TEXT NOT NULL,
    retention_job_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('revoke', 'expire', 'purge')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'skipped')),
    reason TEXT NOT NULL,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id, submission_id, action),
    FOREIGN KEY (tenant_id, retention_job_id) REFERENCES trace_retention_jobs(tenant_id, retention_job_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE
);
CREATE INDEX idx_trace_retention_job_items_submission ON trace_retention_job_items(tenant_id, submission_id, created_at DESC);
```

The server-owned `V1__trace_commons_schema.sql` migration includes the first PostgreSQL RLS policy layer for the tenant-scoped Trace Commons metadata tables. After `V18__trace_central_rls_tenant_predicate.sql`, the canonical policy shape is:

```sql
ALTER TABLE trace_submissions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_submissions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
```

The later server-owned `V6__trace_force_rls.sql` migration applies `FORCE ROW LEVEL SECURITY` to every Trace Commons RLS table, so table-owner roles no longer bypass policies for normal runtime access. `V18__trace_central_rls_tenant_predicate.sql` centralizes the policy predicate behind `trace_current_tenant_id()`, then drops and recreates the tenant isolation policy on every Trace Commons RLS table. That gives readiness diagnostics one canonical policy shape to validate and prevents future migrations from drifting into subtly different tenant-setting predicates. The PostgreSQL readiness diagnostics probe `SELECT set_config('trace-commons.trace_tenant_id', $1, true)` inside a transaction and verify the tenant context clears after commit; the fail-closed startup gate and `POST /v1/admin/postgres-rls-drill` require that transaction-local behavior along with policy/RLS/FORCE-RLS coverage and a runtime role that neither bypasses RLS nor owns Trace Commons tables. Runtime/worker roles should be non-superuser roles without `BYPASSRLS` and should be separate from migration/table-owner roles; any explicit worker policy variants should stay narrow rather than becoming blanket bypass.

### Rust Store Contract Shape

The Rust contract now lives in `crates/trace-commons-server/src/trace_corpus_storage.rs`. `TraceCorpusStore` is part of the server crate's `Database` facade and is backed by PostgreSQL in this repository. `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` still serves file-backed pilot responses by default, but it can mirror submit/review/credit/revoke mutations into the configured DB for dark-launch verification, including reasoned privileged revocation tombstones and revoke audit rows, and uses DB-backed reviewer reads for durable review leases. Claim-next and claim-batch stay reviewer queue ergonomics slices: they do not broaden review decision authority, and they only claim available quarantined tenant rows before lease/audit persistence.

The first implementation-facing shape should stay close to:

```rust
#[async_trait]
pub trait TraceCorpusStore: Send + Sync {
    async fn upsert_trace_submission(
        &self,
        submission: TraceSubmissionWrite,
    ) -> Result<TraceSubmissionRecord, DatabaseError>;

    async fn get_trace_submission(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<TraceSubmissionRecord>, DatabaseError>;

    async fn upsert_trace_tenant_policy(
        &self,
        policy: TraceTenantPolicyWrite,
    ) -> Result<TraceTenantPolicyRecord, DatabaseError>;

    async fn get_trace_tenant_policy(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TraceTenantPolicyRecord>, DatabaseError>;

    async fn update_trace_submission_status(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        status: TraceCorpusStatus,
        actor_principal_ref: &str,
        reason: Option<&str>,
    ) -> Result<(), DatabaseError>;

    async fn append_trace_object_ref(&self, object_ref: TraceObjectRefWrite) -> Result<(), DatabaseError>;
    async fn append_trace_derived_record(&self, derived_record: TraceDerivedRecordWrite) -> Result<(), DatabaseError>;
    async fn append_trace_audit_event(&self, audit_event: TraceAuditEventWrite) -> Result<(), DatabaseError>;
    async fn append_trace_credit_event(&self, credit_event: TraceCreditEventWrite) -> Result<(), DatabaseError>;
    async fn write_trace_tombstone(&self, tombstone: TraceTombstoneWrite) -> Result<(), DatabaseError>;
}
```

The concrete backend methods must take `tenant_id` as an explicit argument or as part of every write struct. Avoid generic `get_by_id` helpers for tenant-scoped trace rows.

The names below are intentionally close to the MVP concepts, but are not proposed migrations yet. All primary records carry `tenant_id`, `created_at`, and `updated_at` unless noted. All tenant-scoped reads must filter by `tenant_id` through row policy or an equivalent query guard.

### Tenants and Access Grants

`trace_tenants`

| Column | Purpose |
|--------|---------|
| `tenant_id` | Auth-derived tenant id. |
| `display_name` | Operator-facing label. |
| `status` | `active`, `suspended`, `retention_only`, `deleted`. |
| `data_residency_region` | Region pin for DB/object/vector placement. |
| `default_retention_policy_id` | Fallback central retention policy. |

`trace_tenant_policies`

| Column | Purpose |
|--------|---------|
| `tenant_id` | Tenant boundary and primary key. |
| `policy_version` | Operator-defined policy version applied during ingest. |
| `allowed_consent_scopes` | Consent scopes accepted for new submissions. |
| `allowed_uses` | Trace-card uses accepted for new submissions. |
| `updated_by_principal_ref` | Admin or job principal that last changed policy. |

`trace_access_grants`

| Column | Purpose |
|--------|---------|
| `grant_id` | Stable grant id. |
| `tenant_id` | Tenant boundary. |
| `principal_ref` | Hash or external subject id from AuthN. |
| `role` | `contributor`, `reviewer`, `admin`, `export_worker`, `retention_worker`, `revocation_worker`, `vector_worker`, `benchmark_worker`, `process_eval_worker`. |
| `allowed_scopes` | Consent scopes or ABAC scope list. |
| `allowed_uses` | Debugging, evaluation, benchmark, ranking, training, analytics. |
| `expires_at` | Optional expiry for short-lived grants. |
| `revoked_at` | Revocation timestamp. |
| `created_by`, `revoked_by`, `reason` | Audit-friendly provenance. |

Access grants authorize service operations. Envelope contributor fields remain attribution only.
In the current ingest service, `export_worker` is limited to replay/ranker export surfaces,
`benchmark_worker` is limited to benchmark conversion, benchmark evaluation batches, and benchmark registry publication batches, `retention_worker` is limited to
retention/cache cleanup maintenance, `revocation_worker` is limited to the
revocation-propagation worker route, and `vector_worker` is limited to vector-index
maintenance. `process_eval_worker` is limited to writing bounded process-evaluation
metadata for accepted submissions, running configured process-evaluator batches over
derived summaries and hashes, when supplied with an external reference appending
idempotent `training_utility` delayed credit for the evaluated accepted submission, and
attaching idempotent hash-only system ranking labels for ranking-allowed traces. These
worker roles are intentionally not treated as reviewers for generic trace listing, audit
reads, policy administration, review decisions, or unrestricted credit mutation.

### Submissions

`trace_submissions`

| Column | Purpose |
|--------|---------|
| `submission_id` | Client-provided UUID, unique within tenant. |
| `tenant_id` | Auth-derived tenant. |
| `trace_id` | Envelope trace id. |
| `auth_principal_ref` | Authenticated principal that submitted. |
| `contributor_pseudonym` | Envelope pseudonymous contributor id, nullable. |
| `submitted_tenant_scope_ref` | Envelope tenant scope ref for analytics only. |
| `schema_version` | Envelope schema version. |
| `consent_policy_version` | Submitted consent policy version. |
| `consent_scopes` | Normalized list or join table. |
| `allowed_uses` | Centralized allowed-use projection. |
| `retention_policy_id` | Central retention policy selected at ingest. |
| `status` | `received`, `accepted`, `quarantined`, `rejected`, `revoked`, `expired`, `purged`. |
| `privacy_risk` | Server-computed residual risk. |
| `redaction_pipeline_version` | Server accepted/re-scrubbed pipeline. |
| `redaction_counts` | Safe aggregate redaction label counts; never raw Privacy Filter spans. |
| `redaction_hash` | Hash over redacted content projection. |
| `canonical_summary_hash` | Duplicate precheck key. |
| `submission_score` | Current scoring result. |
| `credit_points_pending` | Mutable pending credit estimate; do not count it as settled credit. |
| `credit_points_final` | Explicit final credit snapshot when settled. Missing values are treated as `0` for aggregate settled totals. |
| `review_assigned_to_principal_ref`, `review_assigned_at`, `review_lease_expires_at`, `review_due_at` | Optional DB-backed review lease state, scoped to the authenticated reviewer/admin principal and cleared when the trace leaves quarantine. |
| `received_at`, `reviewed_at`, `revoked_at`, `expires_at`, `purged_at` | Lifecycle timestamps. |

Indexes: `(tenant_id, submission_id) unique`, `(tenant_id, status, received_at)`, `(tenant_id, canonical_summary_hash)`, `(tenant_id, expires_at)`, `(tenant_id, contributor_pseudonym)`, `(tenant_id, status, review_lease_expires_at, received_at)`.

### Envelopes and Object References

`trace_object_refs`

| Column | Purpose |
|--------|---------|
| `object_ref_id` | Stable object reference id. |
| `tenant_id` | Tenant boundary. |
| `submission_id` | Source submission. |
| `artifact_kind` | `submitted_envelope`, `rescrubbed_envelope`, `review_snapshot`, `benchmark_artifact`, `export_artifact`, `worker_intermediate`. |
| `object_store` | Bucket/provider alias. |
| `object_key` | Opaque key, preferably content-addressed under tenant partition. |
| `content_sha256` | Integrity hash over ciphertext or canonical plaintext projection. |
| `encryption_key_ref` | KMS key or envelope-encryption key reference. |
| `size_bytes` | Object size. |
| `compression` | Optional compression. |
| `created_by_job_id` | Producing worker/job. |
| `valid_from`, `invalidated_at`, `deleted_at` | Artifact lifecycle. |

Object keys must not reveal raw user ids, local paths, prompt content, or secrets. The service, not reviewer/user tokens, owns object-store credentials. Any backend write that stores an object reference must prove the referenced object belongs to the same `(tenant_id, submission_id)` as the row being written; bare UUID lookups are not sufficient in a multitenant corpus.

### Derived Records

`trace_derived_records`

| Column | Purpose |
|--------|---------|
| `derived_id` | Stable derived record id. |
| `tenant_id`, `submission_id`, `trace_id` | Source linkage. |
| `status` | Mirrors source eligibility: `current`, `invalidated`, `superseded`, `revoked`, `expired`. |
| `worker_kind` | `server_rescrub`, `summary`, `duplicate_precheck`, `embedding`, `ranking`, `benchmark_conversion`, `process_evaluation`. |
| `worker_version` | Version of producing code/model/policy. |
| `input_object_ref_id` | Exact input artifact, resolved only through `(tenant_id, submission_id, object_ref_id)`. |
| `input_hash` | Hash of input projection. |
| `output_object_ref_id` | Optional large output, resolved only through `(tenant_id, submission_id, object_ref_id)`. |
| `canonical_summary` | Redacted short summary when safe for DB. |
| `canonical_summary_hash` | Hash for duplicate checks. |
| `summary_model` | Producing summarizer/model or deterministic summary policy id. |
| `task_success`, `privacy_risk`, `event_count` | Queryable attributes. |
| `tool_sequence`, `tool_categories`, `coverage_tags` | Queryable arrays or join tables. |
| `duplicate_score`, `novelty_score`, `cluster_id` | Utility metadata. |

Process-evaluation derived rows should use `worker_kind = process_evaluation`, store the
evaluator version in `worker_version`, keep label and rubric output in bounded metadata or
an optional `worker_intermediate` object ref, and expose only safe aggregate values such as
tool-selection, argument-quality, ordering, verification, side-effect-safety ratings, and
overall score. DB derived ids are versioned by evaluator version rather than overwritten, so
rerunning a new rubric version preserves the previous process-evaluation row. Process-evaluation
requests may also include a bounded utility credit delta plus external reference; the
service appends `training_utility` delayed credit idempotently and reports appended/skipped
counts without making the worker a generic credit mutator. Requests may also include a
ranking-label projection; the service validates the target use before reading the trace body
and stores only process-evaluation evidence hashes plus external-ref hashes in ranking label
rows. Batch process-evaluation workers can call a configured evaluator adapter with derived
summaries, summary hashes, hashed submission/trace ids, evaluator refs, and purpose hashes,
then persist the adapter result through the same process-evaluation and ranking-label path.
Process-evaluation audit rows use
typed safe metadata with evaluator-version and external-reference hashes, label counts,
rating counts, score band, and optional credit delta, never raw evaluator payloads or raw
external refs. Derived coverage tags use the wire-format enum values, for example
`process_label:proper_verification`, `process_verification:pass`, and `process_eval:high`,
so analytics can aggregate process quality without reading trace bodies. DB-backed reviewer
metadata reads normalize these records into one primary duplicate-precheck row per
submission and merge process-evaluation coverage tags into that row, so
list/export/analytics paths do not double-count a trace just because a process evaluator
appended a second derived record. Consumers should require `status = current` and a
non-revoked source submission.

### Vector Index Metadata

`trace_vector_entries`

| Column | Purpose |
|--------|---------|
| `vector_entry_id` | Stable id shared with vector DB. |
| `tenant_id`, `submission_id`, `derived_id` | Source linkage. |
| `vector_store` | Backend alias: pgvector, PostgreSQL vector index, Qdrant, etc. |
| `embedding_model`, `embedding_dimension`, `embedding_version` | Rebuild metadata. |
| `source_projection` | `canonical_summary`, `redacted_messages`, `redacted_tool_sequence`. |
| `source_hash` | Hash of embedded redacted projection. |
| `status` | `active`, `invalidated`, `deleted`. |
| `nearest_trace_ids`, `cluster_id`, `duplicate_score`, `novelty_score` | Latest analysis snapshot. |
| `indexed_at`, `invalidated_at`, `deleted_at` | Lifecycle timestamps. |

Production may keep the vector payload in an external vector DB. Relational metadata remains the source of truth for revocation and rebuild.

### Audit Events

`trace_audit_events`

| Column | Purpose |
|--------|---------|
| `audit_event_id` | Append-only event id. |
| `tenant_id` | Tenant boundary. |
| `actor_principal_ref` | Human, token, or worker principal. |
| `actor_role` | Role at time of action. |
| `job_id` | Optional worker job. |
| `submission_id`, `object_ref_id`, `export_manifest_id` | Optional targets. |
| `action` | Submit, read, review, credit mutate, revoke, export, retain, purge, vector index, benchmark convert, policy update. |
| `reason` | Required for privileged actions. |
| `request_id` | API request or worker trace id. |
| `decision_inputs_hash` | Hash of reviewed policy/input projection. |
| `metadata_kind`, `metadata` | Typed `TraceAuditSafeMetadata` projection only. Backends must reject arbitrary request bodies, tool payloads, raw paths, token values, and unallowlisted JSON keys. |
| `created_at` | Append timestamp. |
| `previous_event_hash`, `event_hash` | Optional tamper-evident chain per tenant. |

Audit rows should be append-only. Corrections are new events.

### Credit Ledger

`trace_credit_ledger`

| Column | Purpose |
|--------|---------|
| `credit_event_id` | Append-only event id. |
| `tenant_id`, `submission_id`, `trace_id` | Source linkage. |
| `credit_account_ref` | Pseudonymous credit account. |
| `event_type` | Accepted, privacy rejection, duplicate rejection, benchmark conversion, regression catch, training utility, reviewer bonus, abuse penalty. |
| `points_delta` | Signed decimal. |
| `reason` | Human-readable explanation. |
| `external_ref` | Review decision, benchmark artifact, training run, or export manifest. |
| `actor_principal_ref`, `actor_role` | Mutating actor. |
| `settlement_state` | `pending`, `final`, `reversed`. |
| `created_at` | Append timestamp. |

Do not mutate historical ledger rows. Materialized credit totals can be cached separately and rebuilt.

### Export Manifests

`trace_export_manifests`

| Column | Purpose |
|--------|---------|
| `export_manifest_id` | Export job id. |
| `tenant_id` | Tenant boundary. |
| `requested_by_principal_ref` | Requesting actor. |
| `purpose` | `replay_dataset`, `benchmark_eval`, `ranking_training`, `model_training`, `analytics`. |
| `consent_scope_filter`, `allowed_use_filter`, `review_state_filter`, `privacy_risk_filter` | Export policy inputs. |
| `status` | `planned`, `running`, `complete`, `failed`, `revoked_invalid`, `expired_invalid`. |
| `item_count` | Source trace count. |
| `manifest_object_ref_id` | Object ref for full manifest if large. |
| `result_object_ref_id` | Object ref for export payload. |
| `created_at`, `completed_at`, `invalidated_at` | Lifecycle timestamps. |

`trace_export_manifest_items`

| Column | Purpose |
|--------|---------|
| `export_manifest_id`, `tenant_id`, `submission_id` | Source trace membership. |
| `derived_id`, `object_ref_id`, `vector_entry_id` | Exact artifact versions used. |
| `source_status_at_export`, `source_hash_at_export` | Verification snapshot. |
| `source_invalidated_at`, `source_invalidation_reason` | Set when revocation, expiration, or purge invalidates a prior export item. |

Every export item needs an audit event or an audit batch event with a cryptographic item list hash. The pilot replay, benchmark, and ranker export paths already write a deterministic source-list hash into both the exported artifact/manifest and the mirrored audit `decision_inputs_hash`; benchmark conversion and ranker training candidate/pair exports also collapse exact canonical-summary hash duplicates before source-list hashing and delayed utility credit, benchmark and ranker exports persist file-backed provenance manifests, and replay dataset exports promote that hash plus item-level source snapshots into durable DB manifests.

The consolidated Trace Commons migration implements the compact `trace_export_manifests` control row for replay dataset exports in PostgreSQL. It stores tenant id, export manifest id, artifact kind, purpose, audit event id, source submission ids, source-list hash, item count, generation time, and invalidation/deletion timestamps. It also includes `trace_export_manifest_items` rows for each replay export source, durable `trace_retention_jobs` and `trace_retention_job_items` rows, durable review lease fields on `trace_submissions`, revocation-propagation rows, tenant-access grants, and export access grant/job rows. Replay, benchmark conversion, ranker-candidate, and ranker-pair export call sites now mirror one-shot grants and running/complete job state into these tables, with required DB mirror mode failing closed if the durable job row cannot be started or completed. `POST /v1/admin/export/jobs/{export_job_id}/recover-stale` gives admins a hash-only recovery path for expired-grant jobs stuck in `running`: the PostgreSQL update is conditional on the row still being `running` and expired, records only the reason hash and safe recovery markers, and writes a typed audit event without reading trace bodies.

### Benchmark Artifacts

`trace_benchmark_artifacts`

| Column | Purpose |
|--------|---------|
| `benchmark_artifact_id` | Stable artifact id. |
| `tenant_id`, `submission_id`, `derived_id` | Source linkage. |
| `benchmark_kind` | `replay`, `process_eval`, `regression_case`, `ranking_pair`. |
| `artifact_version` | Conversion schema version. |
| `object_ref_id` | Encrypted object payload. |
| `requirements_hash` | Required tools/assertions/environment. |
| `status` | `candidate`, `approved`, `published`, `invalidated`, `deleted`. |
| `created_by_job_id` | Conversion worker. |
| `published_export_manifest_id` | Optional export linkage. |
| `created_at`, `invalidated_at`, `deleted_at` | Lifecycle timestamps. |

Benchmark conversion must fail closed if the source is revoked, expired, not approved for the target use, missing replay metadata, or out of policy. Benchmark lifecycle publication must also fail closed until the artifact has passed evaluator metadata plus registry/evaluator refs, timestamps, and score.

### Tombstones and Retention Jobs

`trace_tombstones`

| Column | Purpose |
|--------|---------|
| `tombstone_id` | Stable id. |
| `tenant_id`, `submission_id` | Revoked/purged submission. |
| `trace_id` | Trace id if retained by policy. |
| `redaction_hash`, `canonical_summary_hash` | Re-ingest/export prevention keys. |
| `reason` | Contributor revocation, policy expiry, admin purge, abuse. |
| `first_seen_at`, `effective_at` | Idempotency timestamps. |
| `retain_until` | Tombstone retention window. |
| `created_by_principal_ref` | Actor or worker. |

Tombstones should outlive content deletion long enough to prevent re-ingest or re-export of the same material.

`trace_retention_jobs`

| Column | Purpose |
|--------|---------|
| `tenant_id`, `retention_job_id` | Tenant-scoped job id. |
| `purpose` | Maintenance run purpose such as `retention_maintenance`. |
| `dry_run` | Whether the run only planned actions. |
| `status` | `planned`, `dry_run`, `running`, `complete`, `failed`, `paused`. |
| `requested_by_principal_ref`, `requested_by_role` | Actor or worker that requested the run. |
| `purge_expired_before`, `prune_export_cache`, `max_export_age_hours` | Selection knobs used by the maintenance run. |
| `audit_event_id` | Linked append-only maintenance audit event. |
| `action_counts` | Aggregate lifecycle/export/object/vector counts. |
| `selected_revoked_count`, `selected_expired_count` | Source selection counters. |
| `started_at`, `completed_at` | Lifecycle timestamps. |

`trace_retention_job_items`

| Column | Purpose |
|--------|---------|
| `retention_job_id`, `tenant_id`, `submission_id` | Selected source. |
| `action` | `revoke`, `expire`, or `purge`. |
| `status` | `pending`, `done`, `failed`, `skipped`. |
| `reason` | Lifecycle reason, for example `retention_expired`. |
| `action_counts` | Per-submission object/vector/export invalidation counts. |
| `verified_at` | Post-action verification time. |

Retention jobs must be resumable and must verify that tenant, policy, consent, and revocation state still match immediately before destructive actions.

### Export Grants and Jobs

`trace_export_access_grants`

| Column | Purpose |
|--------|---------|
| `tenant_id`, `grant_id` | Tenant-scoped short-lived grant id. |
| `export_job_id` | Export job slice the grant authorizes. |
| `caller_principal_ref` | Caller or service principal receiving the grant. |
| `requested_dataset_kind` | Dataset class such as replay, benchmark, or ranker. |
| `purpose` | Bounded export purpose. |
| `max_item_cap` | Optional per-grant item cap. |
| `status` | `active`, `consumed`, `revoked`, or `expired`. |
| `requested_at`, `expires_at` | Grant validity window. |
| `metadata_json` | Safe request metadata only. |

`trace_export_jobs`

| Column | Purpose |
|--------|---------|
| `tenant_id`, `export_job_id` | Tenant-scoped export job id. |
| `grant_id` | Durable grant consumed by the job. |
| `caller_principal_ref`, `requested_dataset_kind`, `purpose` | Request projection copied from the grant for auditability. |
| `max_item_cap` | Optional bounded item cap used by the job. |
| `status` | `queued`, `running`, `complete`, `failed`, `cancelled`, or `expired`. |
| `requested_at`, `started_at`, `finished_at`, `expires_at` | Lifecycle and expiry timestamps. |
| `result_manifest_id` | Optional manifest produced by the job. |
| `item_count`, `last_error` | Result count or bounded failure reason. |
| `metadata_json` | Safe job metadata only. |

Export grants and jobs must stay tenant-scoped and idempotent. PostgreSQL RLS is the production tenant-isolation boundary for these tables.

## PostgreSQL Operational Notes

PostgreSQL is the TraceCommons server control plane for multi-tenant service deployments. Use native UUIDs, enums or checked text, JSONB for small metadata projections, GIN indexes for tags/scopes, row-level security for tenant isolation, and transactional migration support. If vectors stay in Postgres, use pgvector for approved redacted embeddings; otherwise keep vector metadata in Postgres and use an external vector store for payload/search.

PostgreSQL rules:

- Keep table names, logical columns, status values, and state transitions stable across migrations.
- Store timestamps as timestamptz.
- Use numeric/decimal for credit to avoid float drift in ledgers.
- Treat arrays as join tables when query correctness matters. JSON arrays are acceptable only for non-authoritative display metadata.
- Put redacted envelope bodies and large artifacts in encrypted object storage.
- Keep object refs and vector metadata in DB; keep object payloads in encrypted object storage; keep vector payloads in vector DB or a dedicated PostgreSQL vector index.
- Implement DB trait operations at the server crate facade first, then extend `PgBackend`.

## Rollout Plan

1. Define storage contracts without changing ingest behavior.
   - Freeze status enums, object ref kinds, credit event types, audit actions, retention actions, and export purposes.
   - Add serialization fixtures that map current file-backed records to the proposed logical rows.

2. Add relational metadata behind a dark-launch flag.
   - Keep file storage as the served path.
   - Dual-write submission metadata, derived metadata, credit events, tombstones, and audit events to DB.
   - Store envelope payloads in the existing file store and write DB object refs pointing at those files during the bridge phase.

3. Add encrypted object storage.
   - On new ingest, write server re-scrubbed envelopes to object storage and record `trace_object_refs`.
   - Keep file-backed reads compatible by falling back from DB object ref to existing path layout; require active object refs with `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS=true` for production review decisions, `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true` for production replay export reads, and `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS=true` for production benchmark/ranker source validation. Use `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW=true` to skip plaintext submitted/reviewed envelope body files once DB/object-ref review guards and an enabled service-owned encrypted object store are configured. Use `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS=true` to skip plaintext benchmark/ranker derived export files once DB reviewer reads, export guardrails, required source refs, and service-owned object storage are configured.
   - Verify object integrity by hash before review/export reads.

4. Add vector worker as a derived-artifact consumer.
   - Index only accepted, unrevoked, unexpired, approved redacted projections.
   - Write `trace_vector_entries` and `trace_derived_records` after checking revocation immediately before publish.
   - Keep novelty/duplicate scores advisory until reconciliation jobs are green.

5. Switch reads to DB-first. This remains the next major storage cutover.
   - Contributor status, credit, review queues, analytics, and exports read from DB metadata.
   - Envelope body reads resolve through `trace_object_refs`.
   - File-backed records remain a compatibility fallback for pilot data.

6. Backfill existing file-backed data.
   - Scan each tenant directory, validate JSON, recompute hashes, create object refs, insert metadata, and append audit import events.
   - Include utility attestations, credit settlement batches, credit holds, and NEAR credit outbox rows/status in the tenant backfill so credit settlement can promote without losing pilot control-plane rows. Configure the NEAR submit and confirmation workers only after this parity is clean so relayer submissions and confirmations update both the file and DB outbox projections.
   - Quarantine records with validation mismatches rather than accepting silently.
   - Produce a per-tenant migration manifest with source file hashes and resulting row/object ids.

7. Enable production retention and revocation propagation.
   - Revocation writes tombstones first, then invalidates submissions, derived rows, vectors, benchmarks, and exports.
   - Retention jobs run in dry-run mode first and require verification reports before destructive deletes.
   - Destructive object/vector deletion is delayed behind a grace period.

8. Disable file-backed writes for production tenants.
   - Leave read compatibility for one release window.
   - Keep rollback by continuing DB/object dual-write until verification succeeds for all active tenants.

## Migration Verification

Each migration batch should verify:

- Every accepted/quarantined/rejected/revoked file record has exactly one `trace_submissions` row.
- Every submission has at least one envelope object ref or an explicit tombstone/purged state.
- DB `redaction_hash`, `canonical_summary_hash`, consent scopes, status, credit snapshot, and privacy risk match recomputed values.
- Credit ledger totals match local credit responses for each contributor principal.
- Sampled contributor credit/status/events, reviewer queues/lists, analytics summaries, audit event counts, and replay/export manifest listings match file-backed reader projections before a DB read flag is promoted.
- Object hashes match stored refs and decrypt under the expected tenant/key policy.
- Vector entries do not exist for revoked, rejected, quarantined, expired, or out-of-scope submissions.
- Audit import events cover every migrated submission and object ref.

## Operator Promotion Runbook Draft

This is a finish-line checklist for canary tenants. It documents the current branch shape; it does not mean Trace Commons is broadly production-ready.

Common preflight:

- Start with `GET /v1/admin/config-status` and confirm DB dual-write, required DB mirror writes, object-store mode, object-store I/O enabled, object-primary object-store eligibility, tenant rollout gates, RLS readiness, and issuer/keyset health without relying on raw tenant ids, key ids, PEMs, hosts, or credentials in logs. Then read `GET /v1/admin/operational-summary` or run `ironclaw traces operational-summary` for safe aggregate submission, review SLA, export, stale running export-job blockers, retention, vector, benchmark lifecycle readiness, external benchmark registry adapter gaps, ranking model/credit readiness, PostgreSQL RLS promotion blockers, calibration-registry manifest-conflict blockers, ranking worker-run skip warnings, stale or failed ranking worker-run blockers, delayed-credit rollout signals, and the `rollout_smoke` required-check/missing-evidence block; the read audit row also records safe promotion-gate counts for later rollout evidence. After recording rehearsal drills, use `GET /v1/admin/rollout-smoke/preflight` as the single promotion-readiness read for the current rollout-smoke gate state and latest hash-only evidence per check.
- Require active tenant access grants for the exact tenant/principal/role combinations being tested. With `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS=true`, submission, contributor status, reviewer/audit reads, review mutations, dataset/export paths, non-revocation worker mutations, maintenance, and admin ledger/observability reads fail closed without matching grants.
- Run `POST /v1/admin/db-reconciliation-drill` with `record_evidence: true` to refresh `db_reconciliation_clean` evidence without maintenance expiration, backfill, vector-index, or audit-chain side effects. A green response has `ready: true`, no `blocking_gaps`, matching file/DB aggregate counts for the mirrored control-plane families, and a recorded hash-only evidence row. If `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` is enabled, requests to the broader maintenance route that omit `reconcile_db_mirror` are expected to fail closed, and any global or tenant-allowlisted DB reader promotion requires `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` at startup.
- Run `POST /v1/admin/maintenance` with `dry_run: true`, `reconcile_db_mirror: true`, and `verify_audit_chain: true` when the promotion rehearsal also needs retention/cache selection, backfill/vector previews, or the combined maintenance report.
- Treat any `blocking_gaps`, object-ref readability/hash failures, key-ref mismatches, projection drift, audit-chain mismatches, RLS readiness failures, signed-claim failures, or unexpected worker skips as promotion blockers.

Per-tenant rollout:

- Promote one tenant and one surface family at a time. Prefer the tenant allowlist flags for DB contributor, reviewer, replay export, audit, tenant-policy, object-ref-required, and object-primary gates before enabling global flags.
- Read cutover order should be DB reader flags first, then object-ref-required modes, then object-primary submit/review, replay export, and derived export modes. Keep file-backed reads available for the rollback window.
- Each promoted tenant needs a smoke pass for submit/status, contributor credit/events, reviewer queue/list/review mutation with required reasons, replay export selection, audit reads, PostgreSQL RLS readiness, tenant access grant enforcement, and one negative cross-tenant same-id read.
- Keep a fallback tenant on file-backed behavior during the canary so operator smoke checks prove rollout gates are actually scoped.

Rollback:

- Roll back with flags and allowlists first: disable the tenant's DB read, object-ref-required, object-primary, vector, export, or retention-delete gates and leave DB/object dual-write evidence intact.
- Before and after changing flags, run `POST /v1/admin/rollback-drill` with `record_evidence: true`. A green response has `ready: true`, no `blocking_gaps`, matching file/DB submission, audit, and tombstone counts, and a recorded `rollback_flag_drill` evidence row. A service without a DB mirror returns a `TRACE_COMMONS_DB_DUAL_WRITE` operator error instead of pretending rollback evidence exists.
- Do not delete DB rows, audit events, revocation tombstones, retention job/items, export manifests, or physical-delete receipts as part of rollback. They are evidence and replay inputs.
- If object-primary reads fail, pause the affected surface, inspect object-ref readability/hash/key-ref diagnostics, and fall back to file-backed reads where compatibility still exists.
- If retention deletion misfires, stop retention/revocation workers, preserve tombstones/audit rows, and restore from object versioning where available rather than rewriting history.

Key rotation:

- Rotate upload-claim issuer keys through managed EdDSA/Ed25519 keysets with `kid` selection. Publish the new key, wait for guarded refresh to report healthy safe counts, then begin issuing claims with the new `kid`.
- Run `POST /v1/admin/key-rotation-drill` with `record_evidence: true` before switching claim issuance and again before removing the old key. A green response has `ready: true`, at least two active managed EdDSA keys, managed EdDSA enforcement enabled, issuer/audience/JTI/TTL policy configured, a fresh guarded refresh window, no `blocking_gaps`, and a recorded `key_rotation_drill` evidence row.
- Keep the previous key active through the maximum accepted claim lifetime plus the issuer-keyset refresh and max-stale window. Remove it only after old-`kid` claims have aged out.
- Production-gated paths should use managed EdDSA/Ed25519 claims. Static tokens and HS256 claims remain bridge credentials and should not be part of the production rotation drill.
- A missing, inactive, unmanaged, or stale `kid` should fail closed; use that as a smoke check before broadening a rotation.

PostgreSQL RLS readiness:

- Run `POST /v1/admin/postgres-rls-drill` with `record_evidence: true` before enabling DB reader promotion. A green response has `ready: true`, complete policy/RLS/FORCE RLS coverage, transaction-local tenant context, a non-bypassing non-owner runtime role, no `blocking_gaps`, and a recorded `postgres_rls_readiness` evidence row. A service without a DB mirror returns a `TRACE_COMMONS_DB_DUAL_WRITE` operator error instead of pretending RLS evidence exists.
- Treat missing policies, disabled RLS, disabled FORCE RLS, policy-expression drift, sticky tenant context, a bypassing runtime role, or a runtime role that owns Trace Commons tables as blockers, even if ordinary DB reads appear to work during a canary.

Object-store migration:

- The implemented service-owned object backend is `TRACE_COMMONS_OBJECT_STORE=local_service`. It is suitable for object-primary canaries and verifies tenant storage refs, encryption key refs, decryptability, and object hashes.
- `TRACE_COMMONS_OBJECT_STORE=remote_service` with `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=file_system` enables the service-owned filesystem-backed remote adapter. Use an absolute `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET` path as the remote object root, configure KMS and credential references for operator intent, and provide `TRACE_COMMONS_ARTIFACT_KEY_HEX` for ciphertext envelope encryption. Set `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING=true` to retain encrypted deleted versions for restore rehearsal. AWS/GCS/Azure selections still fail closed behind `trace_commons_service_owned_remote_disabled`.
- Set `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true` only for a cutover candidate that must be rollback-complete at startup. Config status, operational summary, promotion gates, and operational metrics expose the versioning-required, object-versioning-supported, and restore-after-delete-supported booleans without object keys, bucket paths, or credentials. Local-service and default filesystem-remote intentionally fail this guard; versioned filesystem-remote can satisfy it for rehearsal, while cloud providers still need production adapters.
- Run `POST /v1/admin/object-store-migration-drill` with `record_evidence: true` after configuring a service-owned object store. A green response has `ready: true`, no `blocking_gaps`, successful encrypted probe write/read/delete booleans, a sha256-prefixed probe object-ref hash, a sha256-prefixed migration manifest hash, and a recorded `object_store_migration` evidence row whose evidence ref points to that manifest hash without returning object keys, filesystem paths, credentials, or the probe payload. Set `require_versioning: true` for cutover rehearsal; the drill then also restores the deleted probe, verifies the restored ciphertext decrypts to the expected payload, reports `restore_after_delete_succeeded`, and deletes the restored live object again. Local-service and default filesystem-remote return `object_versioning_unsupported` plus `restore_after_delete_unsupported`; versioned filesystem-remote should pass, and disabled cloud providers should continue to fail closed until compiled.
- Do not promote cloud remote object storage until the production provider, object versioning/restore story, payload delete receipts, and rollback drill exist. Existing migration tooling is still a local-service bridge; filesystem-backed remote rehearsals should prove object-primary behavior before any cloud cutover.
- For local-service and enabled filesystem-remote object-primary canaries, smoke submit/review envelope reads, replay export body reads, benchmark/ranker source object-ref validation, retention deletion for expired submitted-envelope artifacts, and revocation deletion for submitted/review, vector, benchmark, and ranker payload refs against exact tenant/object/hash/key-ref matches.
- Refresh rollout-smoke evidence within 24 hours of promotion, then read `GET /v1/admin/rollout-smoke/preflight` for the final gate check. Operational summary treats latest per-check evidence older than that window as stale, reports the stale check names separately from missing or failed checks, and keeps rollout-smoke readiness blocked until fresh evidence is recorded.

Retention purge dry runs:

- Run `POST /v1/admin/retention-dry-run-drill` with `record_evidence: true` before any destructive retention pass. A green response has `ready: true`, `dry_run: true`, no `blocking_gaps`, aggregate candidate counts for expired/revoked/purge/cache/provenance work, zero object-deletion counts, and a recorded `retention_dry_run` evidence row. Use `/v1/admin/maintenance` or `/v1/workers/retention-maintenance` for the broader maintenance dry-run report when operators also need reconciliation, audit-chain verification, backfill, or vector previews.
- Run `POST /v1/admin/vector-index-drill` with `record_evidence: true` after the vector-index source canary is accepted and mirrored. A green response has `ready: true`, `dry_run: true`, no `blocking_gaps`, nonzero candidate coverage unless `require_candidates` is explicitly disabled, no pending candidates beyond the requested limit, zero nearest-neighbor policy gaps, and a recorded `vector_index` evidence row while leaving vector metadata unchanged.
- Inspect retention job and item rows through `/v1/admin/retention/jobs` and `/v1/admin/retention/jobs/{retention_job_id}/items` before destructive purge. Dry runs should not mark records purged or delete object files.
- Destructive purge should remain tenant-scoped, resumable, and reversible where object versioning exists. It should delete service-owned local and filesystem-remote submitted-envelope artifacts only during live purge runs with an explicit purpose, while dry runs preserve payloads. It should not start while reconciliation or audit-chain checks are red.

Audit-chain verification:

- Use `POST /v1/admin/audit-chain-drill` with `record_evidence: true` for the tenant before enabling DB audit reads or publishing derived/export artifacts from that tenant. The drill runs the same file-backed and optional DB audit-chain verifier as maintenance but returns only safe counts, last hashes, blocker codes, and failure hashes.
- Require append-order verification for hash-chained file-backed audit rows and DB mirror hash fields where present. Projection drift or stale predecessor rejection is a blocker, not a warning.
- Export smoke checks should verify source-list hashes and that revoked, expired, quarantined, rejected, or out-of-scope sources cannot enter new manifests.

Promotion-gate smoke checks:

- Final preflight: after the drill rows below are recorded, read `GET /v1/admin/rollout-smoke/preflight` and require `rollout_smoke.ready: true`, no blocker reasons, and fresh latest hash-only evidence for every required check before promotion.
- Canary read surfaces: after submitting an accepted canary trace, run `POST /v1/admin/canary-read-drill` with the canary `submission_id`, a fallback `isolation_tenant_id`, and `record_evidence: true`. A green response records fresh evidence for `submit_status`, `tenant_canary_isolation`, `contributor_credit`, `reviewer_metadata`, `replay_export_selection`, and `audit_reads` without exposing raw trace content, contributor tokens, or the fallback tenant id.
- DB reconciliation: `/v1/admin/db-reconciliation-drill` returns `ready: true`, records fresh `db_reconciliation_clean` evidence, and has empty `blocking_gaps`, including current retention job/item gaps, credit-ledger gaps, credit settlement control-plane gaps, audit-event gaps, ranking calibration dataset manifest-conflict keys, reader-projection parity gaps, and object-ref readability/hash/key-ref gaps.
- PostgreSQL RLS: `/v1/admin/postgres-rls-drill` returns `ready: true`, records fresh `postgres_rls_readiness` evidence, and has empty `blocking_gaps` for tenant policy installation, RLS, FORCE RLS, policy expression, runtime role-bypass, and runtime table-owner checks.
- Tenant access: active grants allow intended operations and revoked/expired/wrong-role grants fail closed without blocking revocation/self-delete, revocation propagation, config-status, tenant-policy admin, or grant-management recovery paths.
- Analytics release: run `POST /v1/admin/analytics-release-drill` with `record_evidence: true` after setting the production minimum cell count and `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_KEY`; optionally set `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_MAX_DELTA` to the approved count-noise bound and `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_EPSILON_MICROS` plus `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_MAX_EPSILON_MICROS` for tenant-level publication accounting. A green response has `ready: true`, empty `blocking_gaps`, `noise_applied: true`, the configured min-cell/noise/accounting readiness fields, a sha256-prefixed evidence hash, and a recorded `analytics_release` evidence row without exposing the noise key or raw aggregate maps. Verify missing noise config, suppressed cells, disabled privacy budgets, or exhausted epsilon caps fail closed with safe reason codes before any operator republishes aggregates.
- Ranking model readiness: after promoting a canary active model and creating at least one prediction-bound pending `ranking_utility` credit, run `POST /v1/admin/ranking/readiness-drill` with `record_evidence: true`. A green response has `ready: true`, no active-model risk codes, no blocked model-target readiness rows, no calibration-dataset manifest conflicts, at least one ready ranking credit when `require_ready_credit` is true, and a recorded `ranking_model_readiness` evidence row without exposing prediction ids, model artifacts, raw lab notes, or external refs. For failure rehearsal, verify legacy holdout manifest conflicts return `ready: false`, a `calibration_dataset_manifest_conflicts=N` blocker, and failed hash-only evidence before quarantining or re-registering the holdout.
- Credit settlement: set `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` for production issuers, then run `POST /v1/admin/credit-settlement-drill` with `record_evidence: true`, the production policy version, and the non-transferable NEAR contract id. A green response has `ready: true`, `settlement_account_cap_configured: true`, `risk_summary.truncated: false`, no held or over-cap accounts, no settlement/ranking exclusion reason counts, nonzero pending dry-run settlement work unless `require_pending` is explicitly disabled, and a recorded `credit_settlement` evidence row without writing settlement batches or NEAR outbox rows. For failure rehearsal, verify a missing cap produces `settlement_account_cap_missing` and held or over-cap accounts produce blocker codes such as `held_credit_accounts_present`, `over_cap_credit_accounts_present`, and `account_settlement_amount_exceeds_cap`, then release the test hold through `/v1/admin/credit-holds/{hold_id}/release` and rerun the drill to prove settlement resumes only after the risk projection is clean.
- Object-store migration: run `POST /v1/admin/object-store-migration-drill` with `record_evidence: true` after selecting `local_service` or enabled filesystem `remote_service` object storage. A green response has `ready: true`, the service-owned object-store alias, successful write/read/delete probe booleans, no raw object key or probe payload, a hash-only migration manifest, and a recorded `object_store_migration` evidence row that references the manifest hash. For restore rehearsals, set `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING=true`, repeat with `require_versioning: true`, and require `restore_after_delete_succeeded: true`; then enable `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true` in the same canary environment and confirm config status, operational summary, promotion gates, and metrics all show required and supported restore semantics. Disabled cloud `remote_service` providers should fail this drill with safe blocker codes until a production provider is compiled.
- Ranking credit: set `TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS`, `TRACE_COMMONS_RANKING_MIN_LABEL_COUNT`, `TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD`, `TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS`, `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT`, `TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT`, and `TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS` for production issuers, then verify stale calibration, insufficient joined-label sample size, duplicate same-source labels that should not count as independent evidence, multiple label-source enums written by the same actor principal, joined-evidence drift after calibration, low-confidence predictions, excessive aggregate/per-source error, insufficient label-source diversity, insufficient pairwise evidence, weak pairwise ordering accuracy, training/calibration dataset overlap, calibration-registry manifest conflicts, and a bad per-source calibration cohort block model promotion, prediction-credit issuance, and ranking-utility settlement before NEAR outbox submission or confirmation is enabled.
- Revocation propagation: run `POST /v1/admin/revocation-propagation-drill` with `record_evidence: true` after creating a canary revocation. A green response has `ready: true`, `dry_run: true`, no `blocking_gaps`, pending due-item counts, and a recorded `revocation_propagation` evidence row without claiming items. After the live worker runs, run `POST /v1/admin/revocation-effects-drill` with the canary `submission_id` and `record_evidence: true`; a green response records `delayed_credit_reversal` and `object_deletion_refs` evidence by proving exact reversed credit events, `reverse_credit_receipt` NEAR outbox rows when the original settlement used a NEAR contract, deleted service-owned object refs, physical-delete receipts, and filesystem-remote artifact removal without exposing raw trace bodies or object keys.
- Object-primary: after submitting an accepted canary trace with a service-owned object store, run `POST /v1/admin/object-primary-read-drill` with the canary `submission_id`, a fallback `fallback_tenant_id`, and `record_evidence: true`. A green response records fresh `object_primary_reads` evidence, verifies tenant key refs and hashes for submitted/review/replay reads, proves the submitted plaintext body file is absent, and confirms the fallback tenant remains outside object-primary rollout. Disabled cloud `remote_service` providers and unsupported artifact kinds remain skipped or future work rather than silently succeeding.
- Retention: dry-run counts match the intended tenant slice, legal holds are honored, and destructive purge is not enabled until the dry-run, reconciliation, audit-chain, and rollback evidence has been reviewed.

## Migration and Test Checklist

Implementation checklist for the first real storage migration:

- Add one PostgreSQL Trace Commons DDL migration in this server repo. Completed as `migrations/V1__trace_commons_schema.sql`.
- Keep `TraceCorpusStore` inside the server crate's `Database` facade. Completed with the `PgBackend` implementation in this repository.
- Keep DB writes behind a dark-launch or dual-write flag until parity checks pass. Completed with `TRACE_COMMONS_DB_DUAL_WRITE=true`.
- After parity checks pass, promote critical writes with `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` so DB mirror failures fail closed instead of creating file-only accepted submissions, credit events, credit settlement control-plane rows, export provenance, or audit/content-read rows.
- Keep DB reads behind surface-specific rollout flags until parity checks pass. Contributor credit/status reads are gated by `TRACE_COMMONS_DB_CONTRIBUTOR_READS=true`, reviewer/admin metadata plus credit settlement control-plane and ranking evidence/calibration reads by `TRACE_COMMONS_DB_REVIEWER_READS=true`, replay export selection by `TRACE_COMMONS_DB_REPLAY_EXPORT_READS=true`, and audit event reads by `TRACE_COMMONS_DB_AUDIT_READS=true`. Once `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` is enabled, those global flags or their tenant allowlists require `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true` so a clean reconciliation cannot drift immediately through best-effort mirror writes.
- Keep object payloads in encrypted artifact/object storage; write only object refs and hashes into DB. Completed for the local encrypted artifact sidecar, DB object-ref-backed replay envelope reads, schema-versioned benchmark conversion artifacts with audited registry/evaluation lifecycle updates, source object-ref gating for benchmark/ranker derived exports, object-primary submit/review envelope body storage, object-primary replay export body reads, filesystem-remote object-store migration smoke coverage, and versioned filesystem-remote restore rehearsal; cloud remote service-owned object storage and broader object-primary read surfaces remain future work.
- Propagate revocation and retention expiration to DB metadata before DB-first reads. Completed for submission status, tombstones, reasoned privileged revocation audit rows, object-ref invalidation, derived-record invalidation, vector-entry invalidation, replay export manifest/item invalidation, file-backed benchmark/ranker provenance invalidation, exact delayed-credit settlement reversal with optional NEAR reversal outbox, contributor credit/status reads, reviewer metadata reads, maintenance repair of already file-marked revoked submissions, retention-expired submission/object/derived/export invalidation, audit events for invalidation counts, and durable retention job/item ledger rows for maintenance runs.
- Add a backfill tool that reads the file-backed tenant directories, validates envelopes, recomputes redaction and summary hashes, writes metadata, and emits audit import events. Initial maintenance-triggered DB mirror backfill exists for already-derived file-backed submissions plus credit events, audit events, replay/export manifests, utility attestations, credit settlement batches, credit holds, NEAR credit outbox rows/status, ranking model versions, ranking feature/prediction/label evidence, ranking calibration runs, and ranking worker-run rows, and now isolates malformed submission/derived metadata plus per-item failures with bounded reporting; full recompute/import manifests remain future work.
- Add a reconciliation command that compares file-backed responses with DB-backed metadata for status, review queues, credit, analytics, replay export, object refs, and tombstones. Maintenance reconciliation now covers metadata counts, credit settlement control-plane counts/gaps/status drift, ranking model/evidence/calibration/worker-run counts and drift diagnostics, invalid-source derived/export diagnostics, active object-ref readability/hash/key-ref mismatch diagnostics, reader-projection parity for contributor, reviewer metadata, analytics, audit, and replay/export manifest surfaces, and compact `blocking_gaps`; `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=true` can require reconciliation and make remaining gaps fail closed during production promotion. Remaining work is PostgreSQL breadth, remote object storage, and broader object-primary reads.

Test checklist for the same branch:

- PostgreSQL migration test applies all migrations on an empty database and on a pre-Trace-Commons database.
- PostgreSQL integration tests insert logical submissions, object refs, audit events, credit events, derived records, vector entries, export manifests, export manifest items, ranking evidence, calibration runs, policies, grants, jobs, and tombstones through the server store trait when an available test database is configured.
- Tenant-isolation tests seed duplicate `submission_id`, `trace_id`, `canonical_summary_hash`, and contributor pseudonym under two tenants and prove all public store methods filter by tenant. Implemented for the PostgreSQL store facade and ingest DB mirror path, including rejection coverage for export manifest items and derived records that try to link cross-tenant object, derived, or vector refs.
- Handler-level tests drive the future ingest/review/revoke/export callers, not only helper predicates, and assert each mocked DB/object/vector call receives `tenant_id`, `actor_principal_ref`, and `submission_id`. Implemented for submit/review/credit/revoke dual-write, DB-backed export selection, maintenance-triggered vector metadata indexing, the bounded dedicated vector-index worker route, the admin vector-entry metadata list route, and the credit-cycle worker sequence from ranking evidence through settlement plus NEAR outbox creation, submission, and confirmation.
- Revocation and retention propagation tests prove tombstone-first ordering and invalidation of submissions, derived rows, vectors, benchmark artifacts, exports, and credit settlement. Current coverage verifies DB tombstone/status plus object-ref, derived-row, vector-entry, replay export manifest/item invalidation, exact delayed-credit settlement reversal for newly discovered and already file-marked revocations, benchmark-conversion credit settlement reversal with NEAR `reverse_credit_receipt` enqueueing, contributor balance netting for reversed settled credit, service-owned submitted/review/vector/benchmark/ranker object-payload deletion, and retention-expired DB submission/object/derived/export invalidation; broader multi-benchmark and remote-registry settlement drills remain future work.
- Retention tests run dry-run, policy-change, legal-hold, retry, and resumed-job paths before any destructive object/vector deletion path is enabled.
- Export tests prove revoked, quarantined, rejected, expired, and out-of-scope submissions cannot enter new manifests, and existing manifests are invalidated after source revocation.
- Security tests verify PostgreSQL RLS with `trace-commons.trace_tenant_id` across tenants.
- Migration rollback tests prove DB-first reads can be disabled without deleting rows and that audit/tombstone rows remain append-only. Current caller-level coverage drives `POST /v1/admin/rollback-drill`, records hash-only smoke evidence, and re-reads DB submissions/tombstones after the drill.

## Rollback

Rollback should be operational, not destructive:

- Keep file-backed serving path available until DB-first reads have passed verification.
- During dual-write, mark DB rows with `write_source = file_bridge` or equivalent migration metadata so they can be ignored by rollback readers.
- If object storage fails, continue accepting to file-backed storage for pilot tenants and pause DB object refs with a visible audit event.
- If DB writes fail during dual-write, accept only when the tenant is still configured for MVP mode; production tenants should fail closed after the cutover gate.
- If vector indexing misbehaves, delete or invalidate vector entries and recompute novelty from relational/hash prechecks. Do not roll back submissions.
- If retention deletion misfires, stop workers, restore from object versioning where available, and keep tombstones/audit events. Never delete audit events to hide rollback.

## Retention Safety

Retention must be centrally controlled. The envelope `trace_card.retention_policy` is an input to policy selection, not an executable instruction.

Before deleting content, a retention worker must:

- Re-read the submission through tenant policy.
- Confirm consent scope, allowed use, status, legal hold, and export membership.
- Write or confirm a tombstone.
- Invalidate derived rows, vectors, benchmark artifacts, and exports.
- Emit audit events for planned and completed actions.
- Verify object/vector deletion or mark the item failed for retry.

Use dry runs, sampled manual review, object versioning, delayed hard deletes, and per-tenant kill switches. Retention jobs should be idempotent and resumable.

## Tenant Row Policy

The trusted tenant id comes from authentication. Production request handling should bind a `TenantCtx` or ingest equivalent before any store call.

PostgreSQL policy model:

- Enable row-level security on all `trace_*` tables except global policy dictionaries.
- Set a transaction-local tenant setting such as `trace-commons.trace_tenant_id` after authentication.
- Keep all tenant policies on the shared `USING (tenant_id = trace_current_tenant_id())` and matching `WITH CHECK` predicate.
- Give service-worker roles narrow policies for only their job type.
- Keep admin cross-tenant access behind explicit system-scope methods that always emit audit events.

Repository guardrails:

- RLS is the primary tenant boundary, but every repository method still takes `tenant_id` and includes it in predicates.
- Avoid generic "get by id" helpers for tenant-scoped tables.
- Integration tests must seed same UUIDs across two tenants and prove cross-tenant reads, writes, review decisions, revocations, exports, and credit queries cannot cross the tenant boundary.

## Test Plan

Tenant isolation tests:

- Contributor token for tenant A cannot list, status-check, revoke, review, export, or credit-sync tenant B submissions, even when it knows `submission_id`.
- Reviewer/admin token for tenant A cannot access tenant B quarantine, analytics, audit, object refs, vectors, exports, or credit ledger rows.
- Same `submission_id`, `trace_id`, `canonical_summary_hash`, and contributor pseudonym can exist in two tenants without collisions.
- DB-backed queries include tenant predicates at the caller level, not just in low-level helpers.
- PostgreSQL RLS tests run with `trace-commons.trace_tenant_id` set to tenant A and confirm tenant B rows are invisible.
- PostgreSQL integration tests use a shared database with two tenants and assert public repository methods scope by tenant and RLS context.

Revocation propagation tests:

- Revocation is idempotent and preserves the first revocation timestamp/reason while appending later audit context.
- Revocation writes a tombstone before content invalidation.
- After revocation, status sync reports revoked, review approval fails, credit finalizes or reverses according to policy, and dataset export excludes the source.
- Vector worker checks revocation before read and before publish; a revoked source cannot create or keep an active vector entry.
- Benchmark conversion and export jobs fail closed when revocation occurs between selection and publish.
- Benchmark lifecycle publication rejects registry `published` updates that lack passed evaluator metadata and concrete registry/evaluator evidence fields.
- Existing replay export manifests and their item rows are marked invalid when a source is revoked after export.
- Retention jobs skip or alter actions when revocation or legal hold state changes after dry-run selection.
- Reconciliation finds active derived artifacts, vectors, benchmark artifacts, or exports whose source is revoked and invalidates them.

Caller-level regression tests should drive the actual handlers or store facades that perform side effects, not only helper predicates. Mocks of DB/object/vector APIs must capture tenant id, actor principal, object ref id, and submission id for every call so missing propagation is visible.

## Implementation Notes

When this plan becomes code:

- Add server DB facade methods before backend-specific implementations.
- Implement PostgreSQL migrations in the server repo.
- Keep all new writes behind feature flags until dual-write verification exists.
- Update `FEATURE_PARITY.md` only if user-visible Trace Commons status changes.
- Update `docs/internal/trace-commons.md` if endpoint behavior, threat model, or MVP caveats change.
- Run targeted tests for changed storage, web handlers, migration tooling, and the Rust storage contract.
