# Atomic first-authenticator enrollment

The enrollment finish handler still performs its early gate and verifies the
single-use NEP-413 ceremony and FullAccess binding. After verification it calls
`Database::enroll_near_identity`, passing session strength from `AccountCtx`.
The method's default implementation fails closed; it never composes the old
count and insert operations. A database refusal maps to the existing generic
403 and actor-only denial audit.

PostgreSQL explicitly uses READ COMMITTED and locks the live account row with
`FOR UPDATE`. A separate statement after acquiring that lock counts active
wallet identities plus the live NEAR AI anchor. The identity insert and commit
occur while retaining the lock. Two weak finishes cannot both add a first
authenticator. A strong session can still enroll additional keys; uniqueness,
tenant isolation, and the existing proof checks remain enforced. Removing all
strong authenticators returns the account to the existing bootstrap policy.

## Existing writers and lock order

The lock must remain `FOR UPDATE`, not `FOR NO KEY UPDATE`. V33's wallet identity
and V58's anchor tables have immediate, non-deferrable composite
`(tenant_id, account_id)` foreign keys to `trace_accounts`. PostgreSQL checks
new references with `FOR KEY SHARE`, held until transaction completion; that
conflicts with this gate's `FOR UPDATE`.

- Trusted wallet insertion delegates to the same method with strong authority.
- Wallet provisioning inserts acquire the account FK lock. A new NEAR AI
  anchor is created with its new account in one transaction. Its FK lock also
  participates; an uncommitted account cannot admit a competing enrollment.
- Merge destination moves of identities and anchors acquire that destination's
  account FK lock. Moving an identity out cannot increase the source count;
  the merge already locks the absorbed account before moving wallet identities
  and closing it. A closed account refuses enrollment.
- NEAR AI rebind already holds the account `FOR UPDATE` throughout retirement,
  anchor insertion or revival, and wallet revocation. In-place revival is the
  case that cannot rely on a new-reference FK check.
- Wallet revocation explicitly takes the same account lock before revoking and
  computing the remaining-key count. Anchor retirement during merge only
  decreases the source count, and is committed with the serialized destination
  move. Pure label, last-used and payout-designation updates do not affect the
  authenticator count.

Enrollment/revocation take no anchor advisory locks. Provisioning, merge and
rebind retain their existing anchor/advisory order; the fix adds no new reverse
anchor acquisition. A writer that has its FK lock first completes before the
gate reads its fresh count; a writer arriving later cannot finish its new
reference until enrollment commits. Normal transaction conflict errors remain
fail-closed. No SQL migration, runtime dependency or privilege is added.

## Regression evidence

`bootstrap_atomicity_tests` exercises two valid weak enrollment finishes with
a barrier inside their mocked FullAccess RPC: both pass the old early gate,
but only one may commit. Serial weak/strong, duplicate key, tenant mismatch,
post-revocation bootstrap and fail-closed trait-default controls accompany it.
The actual database API is also raced against uncommitted wallet/anchor inserts
and account moves. Those tests observe PostgreSQL's real blocking relationship
before committing the writer, then require the weak request to see and reject
the committed strong authenticator. Run these with the disposable migrated
PostgreSQL suite; a missing database is a skip, never evidence of a DB pass.

PostgreSQL 16 documents the [row-lock conflicts](https://www.postgresql.org/docs/16/explicit-locking.html#LOCKING-ROWS)
and [READ COMMITTED statement snapshots](https://www.postgresql.org/docs/16/transaction-iso.html#XACT-READ-COMMITTED)
used by this protocol. The concurrent tests additionally exercise the actual
schema's foreign-key locks instead of assuming every writer calls this API.
