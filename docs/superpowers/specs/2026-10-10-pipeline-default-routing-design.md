# Pipeline default routing: design

Status: design, 2026-10-10. Implemented on branch `pipeline-default-routing`.

Implementation note: the single scope predicate below is two in the code.
`pipeline_tenant_in_activation_scope` (the receipts list, or the mode is
`all`) is the activation gate's scope check. `pipeline_tenant_served` (the
receipts list, or the routed-tenant cache) decides "served", replay, and the
worker's list, so a process serves only tenants whose bundles it checked. An
upload of a routed tenant the cache does not hold yet admits it on demand.
Re-qualification after a deploy covers only rows default routing wrote (owner
decision in the PR). The operator docs are authoritative for behaviour.

## Why

From the internal release on 2026-10-12, each person who signs up gets their
own tenant. Today a tenant reaches the versioned pipeline only when an operator
lists it on `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`, restarts, and posts
`qualifications` and `activate` with the 22 signed results for it. A new signup
therefore lands on the legacy path. This design adds an operator-armed mode
that routes every tenant with no routing row to the pipeline. The owner decided
the following on 2026-10-10:

- **Scope: every tenant.** When armed, every tenant with no
  `pipeline_tenant_routing` row is activated. This includes existing and
  pooled tenants.
- **Authority: operator-armed per revision.** The mode activates nothing
  until the host holds a signed check-result set for the running revision.
  That is the same material `s3-activate-api` posts: `signed-package.json` and
  `*.attestation.json`. Each deploy is re-armed by a promote cycle. The
  attestations do not depend on the tenant.

## Components

| Component | Where | What it does |
|---|---|---|
| `PipelineDefaultRouting` config | new `trace_commons_ingest_internal/pipeline_default_routing.rs` (AGPL header) | Parses the env vars and refuses a bad configuration at start. Holds the `Arming` state behind an `RwLock`. |
| `Arming` | same | Either `Disarmed { label }` or `Armed { signed_package, attestations, bundle_id, expires_at, set_digest }`. Rebuilt from the directory on every loop pass. |
| `pipeline_tenant_in_scope(state, tenant_id)` | `pipeline_activation.rs` | The single scope predicate: the env receipts list, or the mode is configured (`all`). Replaces the env-only checks listed below. |
| `RoutedTenantCache` | `pipeline_default_routing.rs` | Tenant ids whose row is `pipeline` or `contained`, and whose bundles passed `check_tenant_bundles` in this process. TTL 30 s. Refreshed by the loop, and a tenant is added at once when this process activates it. |
| `qualify_for_tenant` / `activate_for_tenant` | `pipeline_activation.rs`, extracted from `pipeline_qualify_handler` (:1133) and `pipeline_activate_handler` (:1211) | The admin routes keep authentication and `require_admin`, then call these. Default routing calls the same functions. There is no second path. |
| Default-routing loop | `pipeline_default_routing.rs`, spawned from `run_pipeline_app` (pipeline_runtime.rs:2053) beside the worker, stopped with the same `join_or_abort` | Re-arms, routes tenants that have no row, and re-qualifies routed tenants on a new revision. |
| V124 | `migrations/V124__pipeline_default_routing.sql` | Two cross-tenant enumeration functions that return tenant ids only. |

Every caller of the old scope check moves to `pipeline_tenant_in_scope` or to
the cache. Today each of them reads only `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`:

- `gate_inputs` scope check, pipeline_activation.rs:1033 (`pipeline_tenant_not_in_scope`).
- `pipeline_runtime_for_tenant`, trace-commons-ingest.rs:14490. This is "served",
  and it feeds `decide_new_receipt_route`.
- `pipeline_runtime_for_replay`, trace-commons-ingest.rs:14513. It inherits the change.
- `pipeline_worker_tenant_ids`, pipeline_runtime.rs:1934, and `spawn_pipeline_worker`
  (:1950). The worker reads the set once at start today; it will re-read it on every pass.
- `validate_pipeline_tenant_bundles` (:1909), `register_default_bundles_for_rollout_tenants`
  (:2010), and `warn_pipeline_tenants_not_qualified` (:616). These stay env-list only, and
  "Startup" below covers tenants in the database.

When the mode is absent, `pipeline_tenant_in_scope` reduces to the env check,
the cache is empty, no enumeration function is called, and no loop is spawned.
The behaviour is byte-for-byte today's.

## Environment

| Variable | Values | Default |
|---|---|---|
| `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING` | `off`, `all` | `off` (unset or empty means `off`) |
| `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_RESULTS_DIR` | directory with `signed-package.json` and `*.attestation.json` | required with `all` |
| `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_INTERVAL_SECONDS` | 5..=3600 | 60 |
| `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_BATCH` | 1..=1000 | 50 |

Ingest refuses to start with `all` in any of these cases. Each refusal is a
safe label only, like the existing pipeline configuration refusals.

| Condition | Label |
|---|---|
| a value other than `off` or `all` | `pipeline_default_routing_mode_invalid` |
| `TRACE_COMMONS_PIPELINE_RUNTIME` is not `production` | `pipeline_default_routing_requires_production_runtime` |
| either trust store is missing | `pipeline_trust_store_missing` (existing) |
| no build revision | `pipeline_code_revision_unset` (existing) |
| no routing or qualification store | `pipeline_routing_store_missing` (existing) |
| `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is set (unqualified routing) | `pipeline_default_routing_test_dependencies_conflict` |
| results-dir variable unset or empty | `pipeline_default_routing_results_dir_missing` |
| interval or batch out of range | `pipeline_default_routing_config_invalid` |
| runtime role lacks EXECUTE on the V124 functions (start preflight, as `external_account_trust_runtime_ready` does, admission_ledger.rs:676) | `pipeline_default_routing_enumeration_unavailable` |

## Arming (fail closed)

`load_arming(dir)` runs at start and again at the top of each loop pass. When
the result changes, it logs one line with the label and `set_digest`. It runs
the same verifications the admin routes run, in this order:

1. Read `signed-package.json` and every `*.attestation.json`, each at most
   `PIPELINE_ADMIN_BODY_MAX_BYTES`, and at most `PIPELINE_MAX_ATTESTATIONS` of them.
2. `package_trust.verify` and `check_trust.verify_all`.
3. `evaluate_promotion(now)`. The promotion must be ready, and its
   `code_revision_hash` must equal the running revision.
4. `ProductionDependencyProfile::for_bundle` and
   `service.check_runnable_package(&package, main_gate, true)`.

Any failure gives `Disarmed { label }`. Then no tenant is auto-activated,
uploads keep their current routing (legacy for a tenant with no row), and one
WARN is logged at each change of state.

| Case | Label in WARN and config-status |
|---|---|
| directory or `signed-package.json` absent or unreadable | `pipeline_default_routing_results_missing` |
| parse error, too large, too many files | `pipeline_default_routing_results_invalid` |
| package or attestation signature, or the signer is untrusted | the verifier's label (`bundle_package_signer_untrusted`, `check_attestation_signature_invalid`, ...) |
| promotion names another revision | `pipeline_default_routing_revision_mismatch` |
| any attestation past its maximum age | `pipeline_default_routing_results_stale` |
| promotion not ready for another reason | `bundle_qualification_promotion_not_ready` |
| package not runnable on this process | that check's label |

`expires_at` is the earliest `observed_at + maximum_age_seconds` across the
set. The signer chooses the age; it defaults to 24 h and is capped at 7 days
(versioned_pipeline_qualification.rs:76-81). **An armed set therefore has a
lifetime.** A set signed with the default age disarms the next day. Operators
sign default-routing sets with `--evidence-max-age-seconds 604800` and re-arm
at least weekly, and after every deploy. The 7-day ceiling is not changed here.

`GET /v1/admin/config-status` gains `pipeline_default_routing_mode` (`off` or
`all`), `pipeline_default_routing_armed` (bool),
`pipeline_default_routing_label` (refusal label or null),
`pipeline_default_routing_expires_in_seconds`,
`pipeline_default_routing_last_pass` (counts: activated, refused, skipped,
requalified; pass duration in ms), and `pipeline_default_routing_routed_tenant_count`.
It never shows a path, a tenant id, or a key id. From 24 h before expiry, each
pass logs `pipeline_default_routing_results_expiring`.

## Per-tenant activation (`default_route_tenant`)

The upload path and the loop both call this, and only when `Armed`:

1. Skip the tenant if it is in the per-tenant backoff map. A refusal backs it
   off for 10 minutes, and a later success clears the entry. Take an
   in-process per-tenant `tokio::Mutex`, so concurrent uploads on one process
   pay the work once. Then build a `TenantAuth` like `system_audit_tenant`
   (trace-commons-ingest.rs:67199) with the system actor below.
2. `qualify_for_tenant(signed_package, attestations)`: `qualify_bundle_attested`.
   This registers the package and its four policy rows (`register_bundle`,
   serialised by its advisory lock). It is idempotent:
   `ON CONFLICT (tenant_id, bundle_id, code_revision_hash) DO NOTHING`, and an
   identical metadata compare, because `evidence_hash` leaves out the
   evaluation time.
3. `activate_for_tenant` with `expected_record_id = ExpectedRecord::None` and
   `reason_code = pipeline_default_routing`. It runs `gate_inputs` (the scope
   check now passes, promotion evaluated now, package signer still trusted,
   `check_runnable_package`, dependency profile), then
   `ActivationReadiness::from_operational_summary`, then
   `PipelineActivationStore::activate_tenant`. `activate_qualified_bundle_in`
   upserts `pipeline_active_bundles` (versioned_pipeline_qualification.rs:1894-1898),
   so `register_default_bundle` is not needed.
4. `record_action` writes the audit row for qualify and activate under the
   system actor. If the audit append fails after the commit, the code logs
   `pipeline_change_committed_audit_failed` and continues. It does not retry,
   because the routing event is already in place.
5. On success, insert the tenant into `RoutedTenantCache`, after
   `check_tenant_bundles` passes.

**System actor.** `DEFAULT_ROUTING_ACTOR_REF` is
`principal_sha256:` + hex(sha256(`"trace_commons.system.pipeline_default_routing"`)).
It is derived the way `principal_storage_ref` derives refs
(trace_upload_claim_issuer.rs:3334), stored as a `&'static str` literal, and
pinned by a unit test that recomputes it. `validate_actor` accepts it
(versioned_pipeline.rs:974). Routing events show the actor and
`reason_code = pipeline_default_routing`, so an automatic activation can be
told apart from an operator's.

**Operator decisions are respected by the store.** `require_expected_record`
(versioned_pipeline_activation.rs:251) runs under the exclusive routing lock
(`lock_routing`, :1040). With `ExpectedRecord::None`, any existing row refuses
with `pipeline_routing_state_changed`: `legacy` after a deactivate,
`contained`, or `pipeline`. Default routing treats that refusal as "already
decided". It writes nothing, does not back off, and does not WARN. The loop
never calls `contain`, `deactivate`, `rollback`, or an activate with a
non-`None` expectation. Drain-list semantics are unchanged.

## Data flow: first upload

In `submit_trace_handler`, the routing is read
(`pipeline_owner_and_routing_for_new_upload` or `routing_for_new_receipt`).
If the mode is `Armed`, the upload is not a remediation, and `routing` is
`None`, then:

1. `default_route_tenant(tenant)`. Any refusal is logged once per backoff
   window, with `tenant_storage_ref` and the label, and never fails the upload.
2. If it activated the tenant, or got `pipeline_routing_state_changed`, read
   `routing_for_new_receipt` again.
3. `decide_upload_route` runs unchanged on the routing it now has. "Served" is
   `pipeline_tenant_in_scope`, so a `pipeline` row goes to the pipeline.

Concurrent first uploads:

- On one process, the per-tenant mutex lets one caller activate. The others
  find the row on their re-read.
- Across processes, `lock_routing` serialises the activations. The second sees
  the first's row and gets `pipeline_routing_state_changed`. Exactly one
  `activate` event is written.
- A legacy upload that passed `decide_upload_route` before the activation
  committed completes on the legacy path, with its legacy claim. Today's
  manual activation behaves the same way (`pipeline_receipt_ownership`, V110).

The activation adds latency to the first upload only: 22 signature checks, the
operational summary, and the gate. Lock waits are bounded at 5 s
(`PIPELINE_ADMIN_LOCK_TIMEOUT_SQL`). The future is never cancelled mid-transaction.

## Data flow: the loop

Each pass, every `INTERVAL` seconds:

1. `load_arming`. If the result is `Disarmed`, refresh the cache and stop the pass.
2. **Route.** Page `trace_pipeline_unrouted_tenants(after, BATCH)` from a
   cursor that wraps at the end of the list, and call `default_route_tenant`
   on each tenant. A pass handles at most `BATCH` tenants.
3. **Re-qualify after a deploy.** Page `trace_pipeline_routed_tenants(after, BATCH)`.
   For each tenant whose row is `pipeline` or `contained`, whose active bundle
   is the armed `bundle_id`, and which has no qualification on the running
   revision, call `qualify_for_tenant` only. This leaves the routing row as it
   is: a contained tenant stays contained. Without this step, every routed
   tenant would answer `503 pipeline_bundle_not_qualified` after each deploy
   until someone re-qualified it by hand. A tenant whose active bundle is
   another bundle (an operator rollback) is skipped with
   `pipeline_default_routing_requalify_bundle_differs`.
4. **Refresh the cache.** If the TTL has expired, read every page of
   `trace_pipeline_routed_tenants`. Run `check_tenant_bundles` on each tenant
   that is new to the cache. A failure keeps the tenant out of the cache and
   WARNs `pipeline_default_routing_tenant_bundle_check_failed`. Uploads for
   that tenant then get `503 pipeline_tenant_not_served`, the existing
   fail-closed answer.

The worker builds its tenant list on every pass: env receipts ∪ env drain ∪
`RoutedTenantCache`. Legacy rows are not in the cache, so a tenant deactivated
to legacy drains only through the env drain list, as today.

**Startup.** Tenants on the env lists keep today's refusing checks. Tenants
routed in the database get no boot-time check. The first loop pass, which runs
before the listener opens, fills the cache under the same 5 s deadline as
`warn_pipeline_tenants_not_qualified_within` (`PIPELINE_START_CHECK_TIMEOUT`).
Tenants it does not reach are added on later passes. One bad tenant never
blocks the start.

## Cross-tenant enumeration (V124)

V124 reuses the V85 enumeration guard pattern
(`migrations/V85__account_trust_fact_recorder.sql:219-253`) and its caller
shape (`crates/trace-commons-server/src/db/postgres_account_trust_growth.rs:21-38`):

- `trace_pipeline_routing_enumeration_guard`, created `NOLOGIN NOBYPASSRLS`.
  It gets `GRANT SELECT (tenant_id)` on `trace_tenants` and
  `GRANT SELECT (tenant_id, routing_state)` on `pipeline_tenant_routing`
  (V110). It also gets two policies, `FOR SELECT TO <guard> USING (TRUE)`,
  that name only this role.
- `trace_pipeline_unrouted_tenants(p_after TEXT, p_limit BIGINT) RETURNS TABLE(tenant_id TEXT)`.
  It selects tenants in `trace_tenants` that have no routing row, in
  `tenant_id COLLATE "C"` order after `p_after`, with
  `LIMIT LEAST(GREATEST(COALESCE(p_limit,0),0),1000)`.
- `trace_pipeline_routed_tenants(p_after TEXT, p_limit BIGINT) RETURNS TABLE(tenant_id TEXT, routing_state TEXT)`.
  It returns the same page shape for rows in `pipeline` or `contained`.
- Both are `LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog`.
  The guard takes ownership through a temporary `GRANT CREATE ON SCHEMA public`,
  then the migration runs `REVOKE ALL ... FROM PUBLIC` and
  `REVOKE <guard> FROM CURRENT_USER`. The migration is idempotent and can be
  applied by a non-superuser CREATEROLE migrator, as V85 and V101 are.
- `GRANT EXECUTE` goes directly to `trace_ingest_runtime`, following the V90
  convention. V85 used a separate worker role, and that role needed a manual
  `GRANT` on the pilot (pilot-cutover-2026-09.md:491). A new role here would
  bring back the same operator step before Monday.
- Signups have `trace_tenants` rows (`postgres_account_onboarding.rs`,
  `postgres_account_binding.rs`), so "tenants that exist but have not uploaded"
  is well defined.

The PR must say that V124 may need renumbering (V119 to V123 may be taken by
open PRs). V124 has no dependency beyond V110.

## Throughput (requirement 8: no redesign)

- Score takes a per-tenant advisory lock, and the worker drains at most 32 runs
  per tenant per pass (`PIPELINE_WORKER_MAX_RUNS_PER_TENANT`, pipeline_runtime.rs:749).
  A pooled tenant with many users progresses at most 32 runs per pass, with
  Score serial inside that tenant. Its queue grows while arrivals exceed that rate.
- `run_pipeline_worker_pass` drains tenants one after another (:991). Pass time
  grows linearly with routed tenants, because an idle tenant still costs its
  drain queries. With every tenant routed, the 200 ms poll interval becomes a
  floor, not the actual cadence.
- Queue depth and age per tenant already exist:
  `PipelineOperationalSummary.work[].count` and `.oldest_age_seconds`
  (versioned_pipeline_product.rs:331-342), on `GET /v1/admin/pipeline/operational-summary`.
  New: `GET /v1/pipeline/readiness` reports `worker_last_pass_tenant_count` and
  `worker_last_pass_duration_ms`.
- The implementation PR reports a measurement: idle per-tenant drain ms and
  pass ms with 1, 100, and 500 routed tenants against local PostgreSQL. The
  operator docs state that number.

## Tests (red first where they assert new behaviour)

All PostgreSQL tests go in a new `pipeline_default_routing_pg_tests.rs` beside
`pipeline_activation_pg_tests.rs`. They also run with the URL of a non-superuser
login that is a member of `trace_ingest_runtime`.

1. Unarmed (`off`): config-status shows `off`, a no-row tenant's upload stays
   legacy, no enumeration call is made, and no loop is spawned.
2. Armed, valid set: a new tenant's first upload returns a pipeline receipt,
   and its event shows `DEFAULT_ROUTING_ACTOR_REF` and `pipeline_default_routing`.
3. A tenant with a `legacy` row after a deactivate is never re-activated, by
   the upload or by the loop.
4. A contained tenant is untouched: still `503 pipeline_receipt_intake_contained`,
   and no event is added.
5. A set for another revision: nothing is activated, the label is
   `pipeline_default_routing_revision_mismatch`, and the upload is legacy.
   Missing, stale, and untrusted sets give their own labels.
6. Concurrent first uploads (N tasks, and two `AppState`s on one database):
   exactly one `activate` event.
7. A tenant routed after startup: the worker processes its run without a restart.
8. Both enumeration functions work as the non-superuser runtime role, return
   only tenant ids (and the state), and the runtime still cannot read
   `trace_tenants` across tenants directly.
9. Re-qualify: after a revision change, a routed tenant gains a qualification
   row and its routing row is unchanged.
10. Startup refusals: one unit test per label in the Environment table.
11. Unit tests: `DEFAULT_ROUTING_ACTOR_REF` is pinned, and the backoff map works.

## Rollout, re-arm, disarm

- **Arm:** run the promote cycle on the deployed revision and sign with
  `--evidence-max-age-seconds 604800`. Place the set in the results directory
  (mode 0700, owned by the service user). Set both variables and restart once.
  Then confirm `pipeline_default_routing_armed: true` and watch
  `last_pass.activated`.
- **Re-arm after a deploy:** the new binary starts `Disarmed` with
  `revision_mismatch`. New tenants then stay legacy, and routed tenants answer
  `503 pipeline_bundle_not_qualified` until re-armed. To re-arm, run a promote
  cycle on the new revision and replace the directory's contents atomically
  (write a sibling directory, then rename). No restart is needed. The next pass
  arms and re-qualifies. Re-arm before the set expires, as well.
- **Disarm:** remove the set and the next pass disarms. Or set the mode to
  `off` and restart. Tenants routed before stay on the pipeline. Disarming does
  not deactivate anyone.
- **Abort rule** (smoke-tests stage 4): `contain` or `deactivate` of one tenant
  still works and writes a row, and the loop never undoes it. To stop all
  auto-activation at once, disarm.
- **Docs:** `docs/operator/pipeline-activation.md` gets a new section, "Default
  routing". `docs/operator/pipeline-smoke-tests.md` stage 4 covers arm,
  re-arm, disarm, and the abort rule. `docs/operator/README.md` lists both.
- All replicas run the same build and the same mode ("Run one build and one
  configuration").
