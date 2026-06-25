# NEAR settlement mode and payout designation

Slice 3b makes on-chain credit settlement an explicitly-moded operation and
makes payout destination an explicit, contributor-designated choice. This
runbook covers the `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` knob, the per-request
`dry_run` preview flag, and payout designation / holds.

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
