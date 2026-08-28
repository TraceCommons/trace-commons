# NEAR settlement mode and payout designation

Slice 3b makes on-chain credit settlement an explicitly-moded operation and
makes payout destination an explicit, contributor-designated choice. This
runbook covers the `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` knob, the per-request
`dry_run` preview flag, and payout designation / holds.

## Current posture: settlement is OFF, deliberately (as of 2026-08-27)

Read this before diagnosing "credit is stuck". Nothing is broken.

The pilot runs `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled`
(`deploy/pilot-gcp/ingest.env.template`), which makes the settlement worker a
no-op that leaves every outbox row `pending`. On top of that, none of the three
schedulers that would even drive the worker are set at all, so they take their
`false` defaults:

- `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED`
- `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_ENABLED`
- `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_ENABLED`

**Consequence, measured on the pilot DB 2026-08-27:** 307 credit events on
`tenant-zaki-pilot`, every one of them `pending`, and no other settlement state
present in the table at all. Three months of pilot traffic have produced zero
settled credit. That is the configuration working as designed, not a fault to
chase.

### This is not a config flip to undo

Turning settlement on for real needs three things that do not exist yet:

1. **A deployed NEAR credit contract.**
2. **A funded issuer key** to pay for the calls.
3. **An external signing adapter** behind
   `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` / `..._CONFIRMATION_URL`. There is
   no implementation of it in this repository; `http` mode is a seam, not a
   service.

Beyond those, `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE`
gates a further ~19-item config profile — see
`credit_settlement_central_issuer_profile_missing_config` in
`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`, which enumerates
exactly what is missing and is the authoritative list. Do not transcribe it
here; it will drift.

Until items 1-3 land, `disabled` is the only honest setting. `dry_run` proves
the state machine end-to-end but writes synthetic transaction hashes, so it
must not be used to make a contributor's credit *look* settled.

### What the contributor is told meanwhile

Because settlement being off is invisible from the outside — an accepted trace,
a pending figure, and a blank `FINAL` column look identical to "still
working on it" — the submission receipt states the deployment's posture in
words. `settlement_posture_explanation` in `trace-commons-ingest.rs` maps the
mode to one contributor-facing line, and an accepted receipt carries it:

| Mode | Line on the receipt |
|---|---|
| `disabled` | Credit is recorded but not settled: on-chain settlement is not enabled on this deployment, so this figure stays pending. |
| `dry_run` | Settlement is running in dry-run: the credit ledger advances with synthetic transaction hashes and no on-chain credit is issued. |
| `http` | Credit is queued for on-chain settlement. |

The line is derived from the live mode rather than hardcoded, so flipping the
mode changes what contributors are told in the same deploy. If you add a mode,
add its sentence there — a receipt that describes a posture the deployment is
not in is the exact defect this section exists to prevent (#445).

## Settlement mode: `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`

The NEAR credit outbox is the state machine that drives account credit toward
on-chain settlement. Its global behavior is selected by
`TRACE_COMMONS_NEAR_SETTLEMENT_MODE`. Three values:

| Mode | Submitter | Network / funds | Use |
|---|---|---|---|
| `disabled` *(default)* | none | none | Pre-pilot safe default. No submission at all; outbox rows stay pending. |
| `dry_run` | in-process synthetic | **none** | Advances the outbox state machine with **deterministic synthetic tx hashes**. No network call, no funds move. **Never in production.** |
| `http` | external signing adapter | real | The signing-adapter seam: posts to `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL`. |

Notes:

- **`disabled` is the default** and the correct setting for any deployment that
  is not yet settling on-chain (the current pilot posture — no DB has live
  settlement yet). In `disabled` mode the in-process submitter is withheld, so a
  scheduler configured for a *live* (non-dry-run) submit pass will fail closed
  at startup with a diagnostic that names the resolved mode — the fix in that
  case is to set the mode to `http`, not to set the submitter URL.
- **`dry_run` mode** advances the full outbox lifecycle using deterministic
  synthetic tx hashes so the state machine can be exercised end-to-end without a
  network or funds. It is idempotent (re-running does not double-advance). It
  must **never** be used in production: the synthetic hashes are not real
  on-chain transactions.
- **`http` mode** is the external signing-adapter seam. The server posts the
  settlement to the configured submitter URL (and confirmer URL) and treats the
  adapter as the signer. **Real in-process NEAR transaction signing is
  deferred** to a future slice (3b-2); `http` mode is the supported production
  path until then.

### Per-request `dry_run` flag is a separate control

Independently of the global mode, a settlement *request* can carry a per-request
`dry_run` flag. This is **preview-only**: under `dry_run` mode it produces a
preview and does not commit live side effects. Do not confuse it with the global
`TRACE_COMMONS_NEAR_SETTLEMENT_MODE=dry_run` setting — the per-request flag is a
caller-scoped preview switch, the env var is the deployment-wide mode.

### Failure handling

In `http` mode, if the submitter adapter errors, the affected outbox row is
marked **failed** with a hash-only `last_error_hash` (no adapter response body,
URL, or secret is stored). Recovery is idempotent.

## Payout designation and holds

Credit settles to a **NEAR identity designated as the account's payout
destination**. An account may have multiple enrolled NEAR identities but at most
**one active payout designation** (enforced by a partial-unique index; a revoked
identity is excluded so a fresh identity can take over).

Payout resolution at settlement time is **fail-closed**:

| Active NEAR identities on the account | Resolution |
|---|---|
| 0 enrolled | **Hold** (NoneEnrolled) — credit is not settled. |
| exactly 1 active | Settles to that sole active identity (SoleActive). |
| ≥ 2 active, none designated | **Hold** (AmbiguousNoDesignation) — credit is not settled. |
| ≥ 2 active, one designated | Settles to the designated identity. |

When credit is **held**, the settlement step emits **no outbox row** — the
credit simply does not reach the chain. This is intentional fail-closed
behavior: the system never guesses a payout destination.

### Hold recovery

Holds are recoverable. Once the contributor designates a payout identity (or the
ambiguity is otherwise resolved to a single active identity), the **repair path
emits the outbox row once** for the previously-held credit. Recovery is
idempotent — it will not emit duplicate outbox rows.

### Designating payout

The endpoint is:

```
PATCH /v1/account/near-identities/{public_key}/payout
```

It is **strong-auth-gated** (a weak/login-only session is rejected `403`).
Designating one identity clears any prior designation on the same account (the
single-active-designation invariant). Unknown / revoked / cross-account public
keys are rejected (`404` for cross-account — no existence oracle).

## Submit-worker concurrency (no double-submit)

The `http`-mode submit worker must never externally submit an outbox row twice.
It is serialized so two overlapping runs (a manual `POST`, a scheduler tick, and
the credit-cycle step all route through the same handler) cannot double-fire:

- A session-level Postgres advisory lock (`pg_try_advisory_lock`, keyed per
  tenant) wraps the submit pass; a contended run no-ops and leaves rows
  `pending`. Candidate rows are read **under** the held lock from the **committed
  DB** outbox state (DB-authoritative), so a serialized later run sees a prior
  run's `submitted` writes and skips them. The `pending -> submitted` write also
  carries a `status IN ('pending','failed')` guard.
- Because that correctness depends on DB-authoritative writes, the submit worker
  **fail-closes (503)** when a DB mirror is present but
  `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES` is not `true`. The pilot sets it `true`.

**Known limitation (non-production, tracked for 3b-2):** the advisory lock is a
Postgres lock, so a hypothetical **file-only** deployment (no DB mirror at all)
would not serialize concurrent live submits. This repo is **PostgreSQL-only**, so
file-only is a test shape that does not occur in production or the pilot — and the
pilot additionally runs settlement `disabled`. Hardening the file-only path (or
simply forbidding live `http` submit without a DB mirror) is folded into the
deferred **3b-2** work that wires real in-process NEAR transaction signing; until
then, never run `http`-mode settlement without a Postgres mirror.

## See also

- [`account-merge.md`](./account-merge.md) — merging devices; a merge clears the
  absorbed account's payout designation.
- [`env-reference.md`](./env-reference.md) — the full `TRACE_COMMONS_*` env
  surface including the submitter/confirmer URLs and bearer tokens.
- [`hash-only-logging.md`](./hash-only-logging.md) — interpreting the hash-only
  `last_error_hash` on a failed settlement.
