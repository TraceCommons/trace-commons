# Trace withdrawal — server side, implementation report

Date: 2026-08-08
Branch: `withdrawal-server` (worktree `.worktrees/withdrawal-server`, base `b7dbcd8`)
Spec: `docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md`
Scope: server only. No daemon method, no CLI action, no History UI, and no
change to `crates/trace-commons-contributor`.

## PostgreSQL baseline, measured before any implementation

The repo's DB-backed ingest suite is documented as requiring
`--test-threads=1` (see the note in `postgres_backend_for_ingest_test`): the
account tests all share the `tenant-a` rows and clean them per-test, so under
default parallelism they delete each other's fixtures. Both numbers were
measured, because only the serial one is meaningful.

| Run | Base `b7dbcd8` | With this work |
|---|---|---|
| Whole crate, `DATABASE_URL` set, `--test-threads=1` | 1366 passed / **104 failed** | 1370 passed / **104 failed** |
| Whole crate, `DATABASE_URL` set, default parallelism | 1314 passed / 156 failed | 1319 passed / 155 failed |
| Whole crate, no `DATABASE_URL` | 1470 passed / 0 failed | 1474 passed / 0 failed |

The serial comparison is the honest one: **104 failures before, 104 after, and
exactly the four new tests added to the passing count.** No regression.

The parallel numbers are noise in both directions — a diff of the failing sets
between two base-commit runs already moves tests in and out
(`account_ctx_bearer_resolves_linked_account_and_principal_set`,
`login_resolver_reads_tenant_across_rls_under_set_role`,
`evaluate_and_record_gate_scores_and_writes_decision_row`). They are recorded
here only so nobody reads a green-looking parallel run as evidence.

**The 104 pre-existing failures are not addressed by this work and were not
investigated.** CI never runs the PG suite, so a no-env-var run stays green and
proves nothing about them.

## What was built

### 1. `POST /v1/account/traces/{submission_id}/withdraw`

Registered on `authenticated_account_routes`, so it is guarded by the account
session middleware — the same auth as
`GET /v1/account/traces/{submission_id}/content`, not the device key.
Withdrawal is an account-level act and survives losing a device.

- Ownership is checked against the auth-derived `AccountCtx.principal_set`
  before anything is touched. Not-found and not-owned both return the
  byte-identical `404` (`"trace not found"`) the detail and content handlers
  use. Never `403`: existence is not disclosed.
- Nothing is read from a request body. There is no request body.
- Idempotent. The tombstone is first-writer-wins, so a second call returns the
  same `withdrawn_at`, the same `prior_status`, and the same tier.

Response:

```json
{
  "submission_id": "...",
  "withdrawn_at": "2026-08-08T...Z",
  "prior_status": "quarantined",
  "distribution_reach": "not_distributed",
  "already_distributed": false,
  "credit_retained": true
}
```

`distribution_reach` is the tier, so the client can tell the contributor the
truth rather than a generic success:

| Label | Meaning |
|---|---|
| `not_distributed` | Prior status was not `accepted`. Content deleted, nothing was distributed. |
| `commons_not_distributed` | Accepted, never published. Content deleted, excluded from future exports and training sets. |
| `commons_distributed` | Accepted and already in a published export or benchmark. Content deleted and excluded going forward; copies already distributed cannot be recalled. |

`credit_retained` is always `true`. Credit already awarded is not clawed back
and the credit columns are never written by this path.

### 2. Content and artifact deletion

`delete_withdrawn_trace_objects` sweeps three sources, because no single one is
complete:

1. the **file-side submission record**, which is the only place the encrypted
   artifact receipt survives — the DB projection
   (`trace_commons_record_from_storage_submission`) hard-codes
   `artifact_receipt: None`;
2. the **`trace_object_refs` rows**, so a submission with no file-side record
   still gets its encrypted artifact deleted, and each ref is marked deleted;
3. **every status-derived envelope path**, because the object key encodes the
   corpus status and a status transition can leave bytes at an earlier path.

Any failure propagates to a generic label-only `500`. Withdrawal is never
reported complete while content may survive.

### 3. Derived-surface eviction (the part the spec flagged as most likely missed)

`evict_withdrawn_trace_from_derived_surfaces` handles the forms in which the
content survives after the envelope is deleted:

- **Vector index, DB side.** `invalidate_trace_vector_entries_for_submission`.
- **Vector index, in-memory side.** New
  `list_trace_vector_entry_ids_for_submission` unions
  `trace_vector_entries`, the legacy per-decision `trace_gate_decisions.vector_entry_id`
  (V24), and the per-chunk `trace_gate_chunk_vector_entries` (V37), then calls
  `gate_service.invalidate_vector_entry` for each, with the canonical
  `tenant_storage_ref` the gate worker used at insertion time — a different
  form would route the delete to the wrong shard and leave the embedding live.
- **Dedup cluster.** New `clear_trace_dedup_cluster_for_submission` NULLs the
  V40 columns on this submission's decisions, so it leaves its cluster. Peer
  rows keep their own assignment; their `dedup_cluster_size` snapshot is
  refreshed by the existing recluster pass, not here.
- **Exports and benchmarks going forward.** Submission artifacts invalidated,
  export manifests and manifest items invalidated with reason `revoked`.

### 4. Migration

`migrations/V43__trace_withdrawal.sql`.

**V43, not V42.** `V42__onboarding_invite_grants` is held by the unmerged
`db-authoritative-invites` branch (PR #213) — confirmed by enumerating
`migrations/` across every remote branch, and independently by
`_trace_commons_migrations` on the shared test DB, which already carries
version 42 from that branch. Taking V42 would have collided.

`run_migrations` is hand-rolled and discovers nothing: the new file is wired in
explicitly with its own version gate. The table is also added to
`TRACE_COMMONS_RLS_TABLES` and to both migration lists in
`trace_commons_rls_registry_matches_migration_policy_coverage`, or the RLS
registry guard would have failed.

The table is five columns and no more — `tenant_id`, `submission_id`,
`withdrawn_at`, `prior_status`, `distribution_reach` — with a CHECK constraint
pinning the tier label set, forced RLS, and the standard
`trace_current_tenant_id()` policy. There is deliberately **no** FK to
`trace_submissions`: the tombstone must outlive any future hard-delete of the
submission row.

`trace_submissions.withdrawn_at` is added alongside. The status itself moves to
`revoked`, which every existing consumer/export/training predicate already
excludes; `withdrawn_at` is what distinguishes a contributor withdrawal from an
operator or policy revocation. `purged_at` is set too, because the content is
genuinely gone.

## Tests

`crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`,
four tests, all passing serially against local PostgreSQL:

| Test | Covers |
|---|---|
| `account_trace_withdraw_quarantined_deletes_content_and_is_idempotent` | Own trace succeeds; staged content file is genuinely gone afterwards; twice returns the same tier and timestamp; the tombstone row carries no contributor identity, no path, no content, and has no columns that could. |
| `account_trace_withdraw_unowned_and_missing_are_uniform_404` | Another principal's submission returns `404` and not `403`, byte-identical to a nonexistent id; no tombstone is written for the rejected attempt. |
| `account_trace_withdraw_reports_each_distribution_tier` | All three tier labels; the exported trace's manifest item is invalidated going forward. |
| `account_trace_withdraw_evicts_vector_entry_and_dedup_cluster` | Vector entry status becomes `invalidated`; the gate decision's dedup cluster becomes NULL. |

Written before the implementation and run against it — the first run failed to
compile on the missing handler, which is the intended failing state.

## Deviations from the brief, and things worth knowing

1. **"submitted" is not a status in this codebase.** The storage enum is
   `Received | Accepted | Quarantined | AwaitingPiiBackstop | Rejected |
   Revoked | Expired | Purged`. The tier-1 set is therefore "anything not
   `Accepted`", and the tombstone records the real label.

2. **The account read surfaces cannot see `received` traces at all.**
   `trace_commons_record_from_storage_submission` maps
   `StorageTraceCorpusStatus::Received` to `None`, so the detail and content
   handlers 404 them. The withdraw handler deliberately does **not** go through
   that projection — it works from the storage record — or `received` traces
   would be permanently un-withdrawable. The tier test uses a `received` row
   specifically to pin this.

3. **Much of the withdrawal machinery already existed at the device-key layer.**
   `DELETE /v1/traces/{submission_id}` (`revoke_submission`) already writes a
   `TraceCommonsRevocation` tombstone, invalidates export provenance, and
   propagates benchmark invalidation. It differs from withdrawal in two ways
   that mattered: it **does not delete the stored content or the encrypted
   artifact**, and it **sets `credit_points_final = Some(0.0)`**, which is
   exactly the clawback the spec forbids. The withdrawal path is separate for
   those reasons, not for lack of looking.

4. **New store methods default to a fail-closed error, not a no-op.** Several
   existing `TraceCorpusStore` methods default to a warn-and-continue no-op.
   That shape is wrong here: a backend that silently "succeeds" at deleting
   nothing, or that reports `commons_not_distributed` because it cannot count
   memberships, is worse than a refusal. All five new methods default to
   `DatabaseError::Query("TraceWithdrawalBackendMissing")`. Defaults (rather
   than required methods) are necessary because
   `crates/trace-commons-contributor` has an out-of-crate test double that this
   slice must not modify.

5. **The distribution tier counts invalidated export memberships too.** An
   export that was published and later invalidated still put copies in other
   hands. Under-reporting here would make the API lie in the one place the UI
   quotes it verbatim.

6. **Two unrelated clippy fixes are included.** `partialeq_to_none` on two
   lines in `tests.rs` from PR #235 (landed 2026-08-07) fail the CI
   `-D warnings` clippy gate. They are converted to `.is_none()` in the test
   commit and called out there; without that this branch's clippy job is red
   for a reason that has nothing to do with withdrawal.

## What I could not verify

- **The 104 pre-existing PG failures.** Untouched and uninvestigated. The
  claim made here is only that the count did not change.
- **The gate service's in-memory ANN eviction, against a real index.** The test
  state uses the deterministic gate service, whose `invalidate_vector_entry` is
  a no-op. The call is made with the canonical tenant ref and its failure is
  fail-closed, but only the DB-side invalidation is asserted by a test. A real
  enclave/usearch index is not exercised anywhere in this suite.
- **Encrypted-artifact deletion against a configured artifact store.** The
  tests run with no artifact store configured, so only the file-side deletion
  path is asserted. The `trace_object_refs` and receipt-reconstruction branches
  are compiled and follow the shape of `cleanup_export_artifact_publication`,
  but are not covered by a test here.
- **`cargo check --features local-gpu-models` and `--features near-ai-scorer`.**
  CI checks both; only the default-feature check and the workspace clippy run
  were done locally. This change touches no feature-gated code.
- **The 48 quarantined pilot traces.** This endpoint is the exit for them, but
  nothing here contacts the pilot or withdraws anything on their behalf.

## Verification run

```
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins      # clean
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run    # clean
cargo clippy -p trace-commons-server --all-targets -- -D warnings \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching                                 # clean
cargo fmt --all -- --check                                             # clean
DATABASE_URL=... cargo test -p trace-commons-server --no-fail-fast -- --test-threads=1
  # 1370 passed / 104 failed (base: 1366 / 104)
```
