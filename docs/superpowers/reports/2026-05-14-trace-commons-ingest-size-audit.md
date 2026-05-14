# trace-commons-ingest.rs size audit

Date: 2026-05-14
File: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
Repo commit at audit time: `033e623` (origin/main fast-forward base)

Investigation only. No code changes proposed in this commit.

## 1. Summary

| Metric | Value |
|---|---|
| Total LOC | 121,922 |
| Production code (lines 1–61,449) | 61,449 L (50.4%) |
| In-file `mod tests` (lines 61,450–121,923) | 60,474 L (49.6%) |
| `#[cfg(test)]` modules | 12 |
| `#[test]` items | 136 |
| `#[tokio::test]` items | 549 |
| Top-level / nested function defs | ~1,398 fn signatures detected |
| Routed `*_handler` functions | 135 |
| `impl` blocks | 109 |
| `struct` defs | 417 |
| `enum` defs | 27 |
| Route registrations in `fn app` | 121 |
| `_drill` functions | 62 |
| `mirror_*` functions (DB mirror writers) | 34 |
| `run_*` job/worker entrypoints | 50 |

### Top 5 largest items by LOC

| Lines | LOC | Kind | Name |
|---|---|---|---|
| 61,450–63,806 (+ continues) | 60,474 | `mod` | `tests` (entire in-file tests module — but the 2,357 figure below is the first banner-section; the full mod runs to EOF) |
| 35,478–36,748 | 1,271 | fn | `trace_operational_metrics_body` |
| 53,133–54,284 | 1,152 | fn | `reconcile_db_mirror` |
| 2,384–3,085 | 702 | impl | `impl AppState` |
| 51,794–52,405 | 612 | fn | `backfill_db_mirror_from_files` |

(Next tier: `impl TraceCommonsAuditEvent` 611L, `impl TraceOperationalPromotionGateSummary` 537L, `fn trace_commons_config_status_response` 515L, `fn app` 438L, `fn run_credit_settlement` 392L.)

### Top 5 most-referenced local helper functions

| Calls | Helper |
|---|---|
| 786 | `sha256_prefixed` |
| 426 | `api_error` |
| 389 | `tenant_storage_ref` |
| 171 | `principal_storage_ref` |
| 159 | `app` (test harness rebuilds the router) |

Secondary tier: `push_gap_count` 114, `read_submission_record` 106, `optional_trimmed_env` 105, `read_all_audit_events` 101.

These five helpers are the load-bearing primitives. Any split must keep them callable from every new module — either via `pub(crate)` re-export or by moving them into a shared helper module.

## 2. Logical sections

The file has **no maintained banner-comment structure**. A scan for `// ====` / `// ----` / ALL-CAPS-header banners produced 9 hits total, all clustered near the in-file tests module (L82,756–121,672). The production half of the file has zero banner comments — sections are implicit and only inferable from naming conventions.

The two structural macro-sections that do exist:

| Range | LOC | Content |
|---|---|---|
| 1–61,449 | 61,449 | Production: imports, state, config, types, handlers, jobs, drills, DB mirror, audit machinery |
| 61,450–121,923 | 60,474 | `mod tests` (12 `#[cfg(test)]` submodules, 685 test fns) |

Within the production half, naming-pattern clusters (not delimited by banners) include:

- L 1,483–1,749  `impl ConfiguredTraceArtifactStore` — artifact-store wiring
- L 2,384–3,085  `impl AppState` — state surface (702 L, central)
- L 5,497–5,934  `fn app` — Router with 121 `.route(...)` registrations
- L 8,737–9,647  health/config-status handlers + config-status response builder
- L 13,801–15,988 credit / utility-credit / credit-cycle worker handlers and runners
- L 17,967–18,216 benchmark-registry-outbox confirmation worker
- L 20,433–23,457 ranking model promotion / feature / prediction / calibration worker stack
- L 27,427–28,022 process-evaluation worker
- L 31,492–32,076 object-store-migration drill (largest single routed handler, 584 L)
- L 35,478–36,748 `trace_operational_metrics_body` (1,271 L — Prometheus surface)
- L 38,159–43,029 benchmark conversion / ranker training export / audit event projection
- L 44,106–50,305 revocation mirroring + audit normalization
- L 50,905–54,284 maintenance / DB mirror reconcile + backfill (1,152 L reconcile_db_mirror alone)
- L 56,997–60,463 `impl TraceDbReconciliationReport` and `impl TraceOperationalPromotionGateSummary` + related projections

## 3. Candidate split units

Of the 121 routes, only **6,625 L** sits inside routed `*_handler` functions. The other ~55k of production code is helpers, state, drills, jobs, audit/mirror machinery, and config builders. Splits keyed to auth gates therefore move handlers but **must** keep helpers reachable.

Auth-gate detection by handler body was inconclusive (gate calls live deep in shared helpers, not at the top of handler bodies). The URL-prefix grouping is the cleanest gate signal we have. Aggregating routed handler LOC by URL prefix:

| Group | Handlers | Handler LOC | Notes |
|---|---|---|---|
| `/v1/admin/*` (60 routes) | 60 | 2,318 | Mostly drills + read endpoints; biggest single drill is `object_store_migration_drill_handler` (584 L) |
| `/v1/workers/ranking/*` | 10 | 1,081 | Ranking promotion/feature/prediction/calibration |
| `/v1/review/*` | 9 | 643 | Public review surface |
| `/v1/workers/credit-cycle/*` | 2 | 502 | `credit_cycle_scheduler_run_handler` (276 L), `credit_cycle_worker_run_handler` (226 L) |
| `/v1/workers/export/*` | 4 | 327 | Export worker |
| `/v1/workers/utility-attestations` + `/v1/workers/utility-credit` | 2 | 284 | Utility credit/attestation |
| `/v1/workers/benchmark-registry-outbox/*` + `/v1/workers/benchmark-evaluations` + `/v1/workers/benchmark-convert` + `/v1/workers/benchmark-registry-publications` | 6 | ~186 | Benchmark worker family |
| `/v1/workers/near-credit-outbox/*` | 3 | 142 | NEAR credit outbox |
| `/v1/workers/gate` | 1 | 159 | Rollout gate evaluator |
| `/v1/workers/retention-maintenance` | 1 | 60 | Retention |
| `/v1/workers/vector-index` | 1 | 38 | Vector index |
| `/v1/workers/revocation-propagation` | 1 | 23 | Revocation propagation |
| `/v1/workers/process-evaluation(s)` | 2 | 22 | Process eval worker |
| `/v1/traces`, `/v1/contributors`, `/v1/analytics`, `/v1/datasets`, `/v1/ranker`, `/v1/benchmarks`, `/v1/audit` | 12 | 331 | Public contributor / dataset / audit surface |
| `/health` | 1 | 395 | Health (large because it embeds the readiness probe matrix) |

**Proposed module candidates** (handlers + their tightly-coupled local helpers; LOC estimates include adjacent helper functions called only from that group, not the shared core):

1. **`handlers::admin_drills`** — all `*_drill_handler` admin routes + their drill bodies. Est. moved LOC: 8,000–10,000 (drill helpers like `run_revocation_effects_drill` 256 L, plus 62 `_drill`-named fns scattered across L 30k–35k). Highest payoff single unit.
2. **`handlers::ranking`** — `/v1/workers/ranking/*` + `/v1/admin/ranking/*` (21 admin routes + 10 worker routes). Est. moved LOC: 4,000–6,000 including `run_ranker_training_pairs_export_job`, calibration, promotion.
3. **`handlers::credit`** — credit-cycle, utility-credit, utility-attestations, NEAR credit outbox, credit-settlement admin + worker routes. Est. moved LOC: 3,500–4,500 including `run_credit_settlement` (392 L).
4. **`handlers::benchmark`** — benchmark-convert, benchmark-evaluations, benchmark-registry-outbox + admin readiness drill. Est. moved LOC: 1,500–2,000.
5. **`handlers::review_and_public`** — `/v1/review/*`, `/v1/traces/*`, `/v1/contributors/*`, `/v1/analytics`, `/v1/audit`, `/v1/datasets`, `/v1/benchmarks` GETs. Est. moved LOC: 1,500–2,000.
6. **`audit_projection`** — `trace_commons_audit_event_from_storage` (299 L), `normalize_audit_event_metadata` (246 L), `impl TraceCommonsAuditEvent` (611 L), revocation/mirror audit helpers. Est. moved LOC: 3,000–4,000.
7. **`db_mirror`** — `reconcile_db_mirror` (1,152 L), `backfill_db_mirror_from_files` (612 L), `index_vector_metadata_from_db` (320 L), `mirror_revocation_to_db` (234 L) and the 34 `mirror_*` fns. Est. moved LOC: 4,500–6,000.
8. **`operational_metrics`** — `trace_operational_metrics_body` (1,271 L), `impl TraceOperationalPromotionGateSummary` (537 L), `trace_commons_config_status_response` (515 L). Est. moved LOC: 2,500–3,000.

The remaining production core (`AppState`, `app()` router, shared error/audit helpers like `sha256_prefixed`/`api_error`/`tenant_storage_ref`, signed-token verification stack, config parsing) stays in `trace-commons-ingest.rs` or a sibling `mod state`/`mod auth` — call it ~15–18k of shared core.

Pulling the in-file `mod tests` out into `crates/trace-commons-server/tests/` or a `#[path]`-included sibling is a separate ~60k-line move that is independently valuable and can be done first; it requires no behavioural understanding of the handlers.

## 4. Risks

- **Helper reachability.** The five most-called helpers (`sha256_prefixed` 786×, `api_error` 426×, `tenant_storage_ref` 389×, `principal_storage_ref` 171×, `read_submission_record` 106×) are referenced from every cluster. Any split must promote them to a `pub(crate)` shared module before the move, or builds fail with thousands of unresolved-name errors.
- **`AppState` coupling.** The 702-line `impl AppState` block exposes many `pub(crate)` fields/methods. Modules pulled into siblings will need explicit `pub(crate)` widening on every accessor they touch. Risk of accidentally widening visibility too far.
- **Route registration ordering.** `fn app` is a single 438-L chain of 121 `.route(...)` calls. Splitting it requires composing sub-routers (`Router::new().route(...).route(...)` returned per-module and `.merge(...)`ed). Axum middleware ordering and `with_state` propagation must be preserved; getting the merge order wrong silently changes auth behaviour (a worker handler accidentally exposed without its bearer gate is a fail-open).
- **Shared mutable state via channels.** `AppState` carries outbox senders, schedulers, and worker join handles. Modules invoking these must continue to access the *same* `Arc<AppState>` instance; splitting must not introduce per-module state caches.
- **In-file `mod tests` reaches into private items.** 685 tests in the same module enjoy private-item visibility. Moving handlers to siblings makes those private items unreachable from `mod tests` unless either (a) the tests move out simultaneously, or (b) each split exposes a `pub(crate) mod tests_support` surface. Path (a) is the right one and is the precondition for any handler split — otherwise the test compile breaks first.
- **Drill→smoke evidence wiring.** CLAUDE.md notes drills feed `rollout-smoke` required checks. Drills currently share state-mutation helpers with non-drill handlers; an aggressive `admin_drills` extraction risks duplicating helpers and producing slightly different hash-only evidence shapes.
- **Audit-projection invariants.** `trace_commons_audit_event_from_storage` and `normalize_audit_event_metadata` are referenced widely; splitting them risks two callers normalizing differently. Move as a unit only.
- **Signed-token verifier (`shared_signed_token_verifier`, 13 call sites; 10–11 `validate_*_eddsa_*` helpers).** The auth stack is centralized but its callers are not. A handler split that misses one re-export will silently fall back to a `require_admin` path or no-auth path — fail-open. Mitigation: split handlers last, after a refactor that funnels all auth checks through one helper per gate.

## 5. Recommended next step

**Defer the production-handler split. Land an isolated tests-module extraction first.**

Effort estimates:

- **Quick (≤ 0.5 day)**: extract the in-file `mod tests` (lines 61,450–121,923, 60k L) into `crates/trace-commons-server/tests/trace_commons_ingest_*.rs` integration tests or `#[path]`-included sibling files. This alone halves the file's apparent size, restores `cargo check -p trace-commons-server --bins` snappiness, and is reversible if it breaks visibility. Caveat: 685 in-module tests rely on private items; extraction requires either widening selected items to `pub(crate)` or keeping the tests in a `#[path = "tests/..."] mod tests;` include. Most of the visibility widening is mechanical.

- **Medium (3–5 days)**: after the tests extraction, peel off `db_mirror` (~5k L) and `operational_metrics` (~3k L). These two have the cleanest interfaces — they take `&AppState` and return owned values — and removing them shaves ~8k of the heaviest non-handler code.

- **Large (1.5–2 weeks)**: a full handler-by-gate split (candidates 1–5 in §3). Recommend bundling this with the next slice that already touches the routing layer (e.g. a new admin surface or a per-tenant router refactor) rather than as a standalone refactor, because the route-composition risk is real and the payoff is purely ergonomic.

Verdict: **split is annoying but not blocking.** The 121,922 LOC headline is misleading — half is tests, and the production half is dominated by a handful of giant functions (`trace_operational_metrics_body`, `reconcile_db_mirror`, `backfill_db_mirror_from_files`) that are individually splittable without touching routing. Take the Quick win on tests extraction the next time a slice touches this file; defer the route-level reshuffle until a slice already needs to touch `fn app`.
