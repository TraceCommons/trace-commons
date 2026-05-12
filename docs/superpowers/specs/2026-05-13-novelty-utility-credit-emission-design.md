# `novelty_utility` Credit Event Emission — Design (Phase A5)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets + Credit lanes
Predecessors:
- `2026-05-11-private-vector-system-design.md` (rephased; defines the `novelty_utility` event kind)
- `2026-05-12-perplexity-scorer-design.md` (A2)
- Migration V23 (schema additions for `gate_version_hash`, `gate_policy_version`, `trace_gate_decisions`)

## Goal

When the gate worker route (`POST /v1/workers/gate/evaluate`, shipped in PR
#11) returns a passing `GateDecision`, mint a `novelty_utility` credit
event into the existing `trace_credit_ledger` so the contributor's pending
credit ledger reflects the novel trace. The credit goes through the same
central-issuer ABAC + audit-row hashing + idempotency pipeline that
`utility_credit_handler` uses for the other utility-credit kinds — no
new credit machinery, just a new event kind routed through the existing
pipeline.

This is the missing piece that turns the gate-decision plumbing (PRs
#9-#12) into actual contributor credit. Without it, gate passes get
audit rows but no credit; with it, the gate-pass workflow is complete
end-to-end.

## Why this needs design before implementation

The previous implementer who wired the gate worker route correctly
deferred this slice with the note that "credit-ledger insertion for
`novelty_utility` events is not a simple helper call — it routes
through tenant-policy ABAC, allowed-uses enforcement, central-issuer
principal gating, and audit-row hashing." Each of those is a real
control, and naive emission would silently bypass them. This spec
walks each control and decides how `novelty_utility` flows through.

## Non-goals

- A new credit-event kind beyond `novelty_utility`. The schema and the
  Rust enum gain exactly this one variant.
- A new admin/operator API surface. Emission is internal to the gate
  worker route.
- Batching multiple submissions into one credit-emit call. The gate
  evaluates submissions one at a time; credit is emitted one at a time.
- A new idempotency mechanism. Reuse the existing
  `append_automatic_utility_credit_events_once_with_counts` helper.
- Settlement-side changes. `novelty_utility` flows through the existing
  settlement pipeline by virtue of being a recognized `event_type`.
  Settlement gates (central-issuer-approval allowlist, etc.) apply
  unchanged.
- Custom credit-points-delta. Use a configured per-deployment value;
  see §"Credit amount" below.

## Existing pipeline (what we plug into)

`utility_credit_handler` at `bin/tracedao-ingest.rs:13411` runs through
these steps for the existing utility-credit kinds (`RegressionCatch`,
`TrainingUtility`, `RankingUtility`):

1. **Auth + tenant grant.** `authenticate_with_tenant_access_grant`.
2. **Role gate.** `require_utility_operator` — only utility-worker
   bearer tokens can mint utility credit.
3. **Event-type whitelist.** `event_type.is_utility_job_type()` —
   today the set is `{RegressionCatch, TrainingUtility, RankingUtility}`.
4. **Finite + bounded credit-delta.** `credit_points_delta.is_finite()`
   and `<= MAX_DELAYED_CREDIT_POINTS_DELTA` (currently 100.0).
5. **Central-issuer ABAC.**
   `require_positive_credit_issuance_principal_if_configured` —
   when the central-issuer profile is required, only allowlisted
   principals can mint positive credit.
6. **Non-empty `reason` and `external_ref`.**
7. **Non-empty `submission_ids`.**
8. **Tenant policy fetch.** `tenant_utility_credit_policy_for_request`
   with the event-type's required allowed-uses.
9. **Per-submission record load + ABAC.** For each submission:
   - Load `SubmissionRecord` via `read_utility_submission_record`.
   - Reject if status != `Accepted`.
   - `record_matches_utility_credit_policy_abac` against tenant policy
     + signed claim.
10. **Single-pass emit.**
    `append_automatic_utility_credit_events_once_with_counts` writes
    the events idempotently keyed by `external_ref`.

This helper handles the audit-row hashing internally. We do not need
to reach below it.

## How `novelty_utility` plugs in

The gate worker route already runs steps 1, 2, 9 (read record), and
the gate-decision logic. It does **not** yet emit credit. After this
spec, it adds the equivalent of steps 4, 5, 8, 10 — re-using the
helper functions wherever they exist.

### Step-by-step in the new flow

The `gate_evaluate_worker_handler` adds these steps after a successful
gate decision with `perplexity_passed && novelty_passed`:

```rust
// after the gate decision is written:
if decision.perplexity_passed && decision.novelty_passed {
    let credit_delta = state.gate_service_credit_points_delta();
    let credit_event_type = TraceCreditLedgerEventType::NoveltyUtility;
    require_positive_credit_issuance_principal_if_configured(
        state.as_ref(),
        &tenant,
        credit_delta,
    )?;
    let tenant_policy = tenant_utility_credit_policy_for_request(
        state.as_ref(),
        &tenant,
        credit_event_type.required_allowed_uses(),
    ).await?;
    if !record_matches_utility_credit_policy_abac(
        &submission_record,
        &tenant,
        tenant_policy.as_ref(),
        credit_event_type.required_allowed_uses(),
    ) {
        // log + write a gate-decision row that records the credit was withheld
        // for policy reasons but DO NOT fail the gate evaluation as a whole.
        return finish_without_credit(decision, "policy");
    }
    let external_ref = format!(
        "novelty_utility:{}:{}",
        decision.gate_version_hash,
        decision_id,
    );
    let reason = format!(
        "novelty_utility:{}",
        decision.gate_policy_version,
    );
    let counts = append_automatic_utility_credit_events_once_with_counts(
        state.as_ref(),
        &tenant,
        AutomaticUtilityCreditConfig {
            idempotency_label: credit_event_type.utility_idempotency_label(),
            idempotency_ref: Some(external_ref.clone()),
            event_type: credit_event_type,
            credit_points_delta: credit_delta,
            reason,
            external_ref,
        },
        vec![AutomaticUtilityCreditSource {
            submission_id: submission_record.submission_id,
            trace_id: submission_record.trace_id,
            auth_principal_ref: submission_record.auth_principal_ref.clone(),
        }],
    ).await?;
    decision_response.credit_emitted = counts.appended > 0;
    decision_response.credit_event_kind = Some(credit_event_type);
}
```

### Step changes vs `utility_credit_handler`

| Step | utility_credit_handler | gate_evaluate_worker_handler (new) |
|------|------------------------|-------------------------------------|
| 1. Auth | tenant access grant | unchanged |
| 2. Role | `require_utility_operator` | **`require_vector_operator`** (gate worker already uses this; no role change) |
| 3. Event-type whitelist | `is_utility_job_type()` | **Not used** — the gate handler always emits `NoveltyUtility` |
| 4. Finite delta | yes | yes (constant per deployment, so check at startup) |
| 5. Central-issuer ABAC | yes | **yes** |
| 6. Non-empty reason/external_ref | required at API surface | **synthesized internally** from `gate_version_hash` + `decision_id` |
| 7. Non-empty submission_ids | yes | always exactly one submission |
| 8. Tenant policy | yes | **yes** |
| 9. Per-submission ABAC | yes | **yes** |
| 10. Idempotent emit | yes | **yes**, same helper |

The key changes from the existing handler:

- **Single submission**, not a batch.
- **Internal authentication context** is the gate worker, not a utility
  operator. The vector-operator role gates `POST /v1/workers/gate/evaluate`
  already (PR #11). Central-issuer ABAC still applies because the gate
  worker is itself emitting positive credit — `require_positive_credit_issuance_principal_if_configured`
  treats this the same as any other positive-credit mint.
- **`reason` and `external_ref` are deterministic**, not operator-supplied.
  This means there's no place for an operator to inject reason text;
  the audit-row reason becomes a stable label tied to the gate version.
  This is intentional — the gate decision is the reason.
- **ABAC failure does not fail the gate evaluation.** The decision row
  still lands; the credit just isn't emitted. The decision row gains a
  `credit_withheld_reason` column (see §"Schema changes" below) so
  the operator can audit why credit didn't mint despite a gate pass.

## Schema changes

### `trace_credit_ledger.event_kind` — new variant

V23 already added `'novelty_utility'` to the `event_kind` check constraint
(per the dstack-enclave foundation PR #9). The Rust-side enum
`TraceCreditLedgerEventType` in `bin/tracedao-ingest.rs` does **not** yet
have the variant — A5's PR adds it:

```rust
enum TraceCreditLedgerEventType {
    BenchmarkConversion,
    RegressionCatch,
    TrainingUtility,
    RankingUtility,
    ReviewerBonus,
    AbusePenalty,
    NoveltyUtility,  // <-- new in A5
}

impl TraceCreditLedgerEventType {
    fn requires_external_ref(self) -> bool { matches!(self, ... | Self::NoveltyUtility) }
    fn is_utility_job_type(self) -> bool {
        // NoveltyUtility is NOT a job type — it's gate-emitted.
        // The is_utility_job_type predicate gates utility_credit_handler's API.
        // Stays unchanged.
        matches!(self, Self::RegressionCatch | Self::TrainingUtility | Self::RankingUtility)
    }
    fn utility_idempotency_label(self) -> &'static str {
        match self {
            // ... existing arms ...
            Self::NoveltyUtility => "utility-novelty-credit",
        }
    }
    fn required_allowed_uses(self) -> &'static [TraceAllowedUse] {
        match self {
            // ... existing arms ...
            Self::NoveltyUtility => &[TraceAllowedUse::TrainingDataset],
        }
    }
}
```

The `required_allowed_uses` decision: a trace that's novel for the
corpus is most directly useful as training data. Operators who want
novelty credit to require a different use (e.g., `BenchmarkSource`)
can extend this in a future tenant-policy refinement.

### `trace_gate_decisions.credit_withheld_reason` — new column (migration V25)

When ABAC blocks credit emission, the decision row records why so the
operator can audit. Nullable; only populated when the gate passed but
credit was withheld.

```sql
ALTER TABLE trace_gate_decisions
  ADD COLUMN IF NOT EXISTS credit_withheld_reason TEXT;
```

Values are stable enum-like strings: `"policy_mismatch"`,
`"central_issuer_denied"`, `"submission_not_accepted"`. Operator-facing
metric. The actual reason is hash-only; no raw tenant policy state.

## What we do NOT change

- **The settlement pipeline.** `novelty_utility` events flow through
  the existing `central-issuer-approval` allowlist mechanism unchanged.
  The settlement allowlist already keys on `policy_version`; for
  `novelty_utility`, the `policy_version` IS the gate's
  `gate_policy_version` (already stamped in V23). Operators approve
  source-lists per gate version. This was the elegant property of the
  rephased private-vector spec — gate-version stamping doubles as
  settlement-policy versioning.
- **Revocation / clawback semantics.** Pre-settlement events under a
  rolled-back gate version reverse via the existing
  revocation-propagation path (already wired in PR #12 + the
  novelty-utility delete flow). Post-settlement credit grandfathers.
  Nothing in A5 changes this.
- **Audit-row shape.** `append_automatic_utility_credit_events_once_with_counts`
  already emits typed, tenant-scoped, hash-only audit rows. The new
  event-kind variant rides on the same path.

## Credit amount

Per-gate-pass credit delta is a configured per-deployment value, not
operator-supplied per call:

| Env | Default | Notes |
|-----|---------|-------|
| `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | `1.0` | Per-pass credit. Must be finite and `<= MAX_DELAYED_CREDIT_POINTS_DELTA`. |

`1.0` is a deliberate guess. Operators tune based on the gate's pass
rate and the desired total budget. For a deployment where the gate
passes 10% of submissions and the operator wants ~10 points per
typical contributor per month, `1.0` × 100 submissions × 10% pass = 10
points/month — plausible starting point. Operators see this through
the existing operational-summary credit aggregates.

A future refinement could make the delta depend on the gate decision's
numeric values (higher novelty → more credit). Out of scope for v1;
the simple constant is the right shape to start.

## Idempotency

`external_ref` is `format!("novelty_utility:{gate_version_hash}:{decision_id}")`:

- `gate_version_hash` ensures a gate-version rollback doesn't accidentally
  collide with credit emitted under the old version.
- `decision_id` is the `trace_gate_decisions.decision_id` UUID, unique per
  evaluate call.

The existing helper rejects duplicate `external_ref` values, so
re-calling the gate worker route with the same submission idempotently
returns the existing credit event without duplicating it. Important
for retry-safe workers.

## Failure modes

| Situation | Behavior |
|-----------|----------|
| Both gates pass + ABAC pass + policy pass | Credit emitted. `credit_emitted: true` in the response. Audit row + ledger row land. |
| Both gates pass + ABAC fails (central-issuer principal not allowlisted) | `credit_withheld_reason = "central_issuer_denied"` on the decision row. Gate decision row still written. `credit_emitted: false`. Gate worker returns 200. |
| Both gates pass + policy fails (submission's allowed-uses don't include `TrainingDataset`) | `credit_withheld_reason = "policy_mismatch"`. Decision row written. `credit_emitted: false`. |
| Gate fails (perplexity or novelty floor) | No credit attempt. `credit_emitted: false`. Decision row carries the gate-fail reason in its existing columns. |
| `external_ref` collision (retry of the same `decision_id`) | The existing helper returns `skipped_existing: 1, appended: 0`. Response says `credit_emitted: false` (technically true that no NEW credit was emitted), but the audit trail shows the prior emit. |
| Ledger write fails (DB error) | `anyhow::bail!("NoveltyUtilityCreditFailed: <hash>")`. The gate decision row was already written (it lands before the credit attempt). Operator sees a hash-only failure log + the decision row without a paired ledger event. Retry path: re-call the gate worker with the same submission; idempotency catches duplicate gate-decision rows (the orchestrator can be made to refuse re-evaluation if a decision already exists for the submission + gate_version_hash) and the credit attempt re-runs cleanly. |

The orderdependency is: **gate decision row first, ledger event second**.
This is deliberate — the decision row is the source of truth for
"what the gate decided"; the credit event is the downstream
consequence. If credit emission fails, the decision row alone is the
audit trail.

## Configuration

Phase A5 introduces one new env:

| Env | Default | Notes |
|-----|---------|-------|
| `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | `1.0` | Per-gate-pass credit |

`AppState` holds the parsed value; the gate worker reads it on each
invocation.

Operators that want to disable novelty credit while keeping the gate
decision rows can set the delta to `0.0`. The helper accepts `0.0`
and emits a zero-delta event row, which is useful for audit but
adds no credit. Documented in the operator runbook.

## Reason field stability

The `reason` field on the audit-row trail is `format!("novelty_utility:{gate_policy_version}")`.
Operators reading audit rows see a stable label. The actual gate
inputs are recovered via `decision_id` lookup into `trace_gate_decisions`
(which carries the perplexity / novelty / tail metrics).

This satisfies the "reason must be non-empty" check in the helper
without leaking trace content into the reason field.

## Testing

### Unit tests

Inside `bin/tracedao-ingest.rs`:

1. `TraceCreditLedgerEventType::NoveltyUtility::utility_idempotency_label()` returns `"utility-novelty-credit"`.
2. `required_allowed_uses(NoveltyUtility)` returns `[TrainingDataset]`.
3. `is_utility_job_type(NoveltyUtility)` returns `false` (it's gate-emitted, not job-API-emitted).

### Caller tests

In `bin/tracedao-ingest.rs` worker-route tests:

1. **Happy path:** seed an accepted submission with `TrainingDataset`
   in its allowed uses, configure `InMemoryGateService` (always
   passes), `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=2.0`,
   hit the gate worker route. Assert:
   - Gate decision row written
   - Credit ledger row with `event_kind = 'novelty_utility'`, `credit_points_delta = 2.0`, `external_ref` containing `gate_version_hash:decision_id`
   - Response has `credit_emitted: true`

2. **Policy ABAC denied:** seed a submission whose `allowed_uses` does
   NOT include `TrainingDataset`. Gate passes; ABAC fails. Assert:
   - Gate decision row has `credit_withheld_reason = "policy_mismatch"`
   - No ledger row
   - Response has `credit_emitted: false`

3. **Central-issuer denied:** configure central-issuer principal
   allowlist, exclude the gate worker's principal. Assert:
   - Gate decision row has `credit_withheld_reason = "central_issuer_denied"`
   - No ledger row

4. **Gate fail:** configure a `MockPerplexityScorer` that returns
   below-floor perplexity. Assert:
   - Decision row written with `perplexity_passed = false`
   - No ABAC attempt
   - No ledger row

5. **Idempotent retry:** call the route twice with the same submission
   under the same gate version. Assert:
   - Two decision rows? or one decision row, depending on the gate
     orchestrator's de-dup logic. **Open question** — see §"Open
     questions" below.
   - Exactly one ledger row.

6. **Tenant isolation:** seed identical submission ids under two
   tenants. Emit credit under tenant A. Assert tenant B's ledger does
   not see the event.

### PG integration test

Extend `tests/trace_corpus_pg_store.rs`:

1. Insert a `trace_gate_decisions` row with `credit_withheld_reason`
   set. Read back, assert the column round-trips.

## Migration

V25 = `ALTER TABLE trace_gate_decisions ADD COLUMN IF NOT EXISTS credit_withheld_reason TEXT;`

Existing rows have `NULL` in the new column — semantically "credit not
applicable" (gate didn't pass) or "pre-A5 row, status unknown."
Backward compatible.

## Open questions

1. **Should re-evaluation of the same submission under the same gate
   version create a new `trace_gate_decisions` row?** Two options:
   - **Allow duplicates.** Each call is a fresh decision; the ledger's
     idempotency handles the credit side. Simpler implementation; more
     decision rows.
   - **Refuse duplicates.** Check `WHERE submission_id = ? AND
     gate_version_hash = ?` before evaluating; if a row exists, return
     it. Cleaner audit story; slightly more complex.

   Recommendation: **Allow duplicates.** The orchestrator's index
   already has the embedding inserted from the first call; re-running
   the gate is bounded-cost. The ledger idempotency means no double-
   credit. Document that duplicate decision rows under the same gate
   version are expected on retry.

2. **Should the credit-points delta vary with novelty score?**
   Higher novelty → more credit is intuitively appealing but invites
   gaming. Recommendation: **constant in v1**, revisit after
   calibration data accumulates.

3. **Should the gate worker route refuse to emit credit if
   `gate_service.safe_status().is_production_trust_boundary == false`?**
   I.e., refuse to credit when the gate is running under
   `InMemoryGateService` or a mock orchestrator. This is the same
   shape as the existing KEK trust-boundary gate. Recommendation:
   **yes for production deployments**, controlled by a new env
   `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true`.
   Dev defaults to off.

4. **Should `required_allowed_uses` for `NoveltyUtility` be
   configurable?** Different deployments may want different uses
   (e.g., a deployment focused on benchmark generation might want
   `BenchmarkSource` instead of `TrainingDataset`). Recommendation:
   **single hardcoded value in v1** (`TrainingDataset`); make
   configurable in v2 when there's a deployment with a different
   need.

## Out-of-scope items

- New admin/operator API for inspecting gate-credit emission stats —
  reuse the existing `operational-summary` aggregates.
- Custom credit-points-delta per-tenant — single deployment-wide value.
- Gate-pass credit caps per-contributor per-period — operator-level
  policy concern, not v1 emission concern.
- Refunds when a gate version is retroactively deemed too generous —
  the existing revocation-propagation reversal path covers
  pre-settlement events; post-settlement credit is locked in per the
  grandfather-settled policy.

## Cost estimate

| Item | Estimate |
|------|----------|
| Schema migration V25 + Rust enum variant | <1 day |
| Gate-worker-route emission logic + helper integration | 2-3 days |
| Caller tests (6 cases above) | 1-2 days |
| PG integration test extension | <1 day |
| Documentation | <1 day |
| **Total** | **~5-7 days of focused work** |

Comparable to A4. Smaller than A2 (no model loading).

## What this spec commits to

- One new credit-event variant: `NoveltyUtility`
- One new schema column: `trace_gate_decisions.credit_withheld_reason` (V25)
- One new env: `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`
- Optional new env: `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE`
  (per open question #3)
- Reuse of every existing utility-credit helper
- No new admin API
- No changes to settlement, revocation, or audit-row plumbing

## What this spec does not commit to

- Specific credit-points-delta default (operators tune)
- Specific `required_allowed_uses` set (hardcoded; configurable in v2)
- Specific behavior on duplicate evaluation of same submission +
  gate_version_hash (open question; lean toward "allow duplicate
  decision rows")
