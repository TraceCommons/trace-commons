# Salting the NEAR account anchor

**Status:** design, not scheduled. A prerequisite of on-chain settlement, not
standalone work. See "When this becomes required".

## The observation

`trace_near_account_anchors.anchor_hash` is derived in
`crates/trace-commons-server/src/account_onboarding.rs:142-145`:

```rust
anchor_hash: framed_hash(
    b"trace_commons.near_account_anchor.v1\n",
    &[self.network.as_bytes(), self.account_id.as_bytes()],
),
```

`PendingNearProvisioning.account_id` is a `String` (`:50`) holding the NEAR
account name, and `network` is `mainnet` or `testnet`. Both are public, and the
domain-separation prefix is in open source. The hash takes no secret.

So **any wallet user's tenant id is computable offline**, because
`V58__near_account_provisioning.sql:25` binds them:

```sql
CHECK (tenant_id = 'near-' || substring(anchor_hash from 8))
```

Given `alice.near`, anyone can compute `sha256` of the framed input and know
that contributor's `tenant_id` without touching our systems.

Note the framing is doing its job — it prevents a hash from one protocol being
replayed as another. What is absent is a *secret*, which is a different
property. Do not read this as a mistake in the original design; the anchor was
built to be a self-verifying internal identifier, and as an internal identifier
it is fine.

## What this does and does not expose today

**Does not.** Knowing a `tenant_id` grants no access. Every Trace Commons table
has forced RLS with a `trace_current_tenant_id()` predicate, and the anchors
table is no exception (`V58:29-34`). Tenant scoping is auth-derived; a guessed
identifier authenticates nothing.

**Does.** It converts "is `alice.near` a Trace Commons contributor?" from a
question requiring access into a question requiring arithmetic — *if* a
`tenant_id` is ever observable. Today none is: it appears in RLS predicates,
internal rows, and hash-only audit surfaces.

**The exposure is entirely conditional on a `tenant_id` becoming observable.**
That is why this is written as a settlement prerequisite rather than a bug.

## The obvious fix is a trap

Peppering the existing derivation in place -- `framed_hash(domain, [pepper,
network, account_id])` -- looks like a one-line change. It is not, and it
should be rejected:

1. **The pepper could never be rotated.** `anchor_hash` is `UNIQUE`, half the
   `PRIMARY KEY`, and bound to `tenant_id` by a `CHECK`. `tenant_id` is a
   foreign key into `trace_tenants` and is referenced by
   `trace_near_provisioned_devices` and `trace_accounts`. Rotating the pepper
   re-derives every anchor and therefore every wallet tenant id -- a rewrite of
   the identity graph, not a migration.
2. **Losing the pepper is unrecoverable.** A returning wallet user is matched
   by recomputing their anchor. Without the pepper that computation is
   impossible, so every wallet-provisioned account is orphaned: the rows exist
   and nothing can ever be matched to them again.

That trades a conditional disclosure risk for a permanent, unrotatable secret
whose loss is catastrophic and silent until someone tries to log in. It is a
worse position than today.

## The design: decouple identity from derivation

Stop deriving the tenant id from the account, and keep a peppered hash only as
a lookup key.

- `tenant_id` becomes a random value, generated at provisioning. Nothing public
  derives it.
- `anchor_hash` becomes a **blind index**: `HMAC(pepper, network || account_id)`,
  used solely to find an existing anchor on a returning login.
- The `CHECK (tenant_id = 'near-' || ...)` constraint is dropped.

What this buys:

- **Enumeration is defeated.** Without the pepper there is no offline map from
  a NEAR account name to anything we store.
- **The pepper becomes rotatable.** Re-computing an index column is an ordinary
  migration: read each row's account name, recompute, write. Re-computing a
  primary key is not.
- **Pepper loss degrades rather than orphans.** The failure becomes "returning
  wallet users cannot be matched by wallet", recoverable by re-deriving from
  stored account names.

## What it costs, stated plainly

1. **A real invariant is lost.** The `CHECK` makes the anchor-to-tenant mapping
   self-verifying: a corrupted row cannot satisfy the constraint. Replacing it
   with an index lookup gives that up. Mitigation is a uniqueness constraint on
   the blind index plus a foreign key, which catches duplication but not
   substitution.
2. **A secret enters a path that had none.** Rotatable, but it must be present
   at every provisioning and every returning login. That is a new availability
   dependency on the same footing as the database.
3. **Rotation requires the account name.** To recompute the index we must hold
   the NEAR account name, which the current design deliberately avoids storing.
   This is the sharpest tradeoff in the document: **the design trades data
   minimisation for rotatability.** If we would rather not store account names,
   the pepper is unrotatable again and the trap above applies. Decide this
   explicitly; do not let it be settled by whoever writes the migration.
4. **A migration over existing anchors**, which must run before any wallet
   account exists in production, or it becomes an identity rewrite.

## When this becomes required

**Not now.** Tenant ids are internal, RLS is the control that matters, and this
is defence in depth against a disclosure that has no path today.

**Before any `tenant_id` could reach a public ledger.** On-chain settlement is
the concrete case. Note the settlement memo already recommends publishing only
a random account hash and never a tenant id -- if that holds, the exposure
never opens and this work stays optional. This design exists so that decision
is made deliberately rather than by omission.

**Do not build it speculatively.** The cost is a lost invariant, a new secret,
and a stored account name. Paying that for a risk with no path is the wrong
trade.

## Rejected alternatives

- **Pepper in place** -- section above. Unrotatable, catastrophic loss.
- **Encrypt the account name and index the ciphertext** -- deterministic
  encryption is a blind index with extra steps and the same key-rotation
  problem; non-deterministic encryption cannot be indexed.
- **Drop the anchor and store the account name in plaintext behind RLS** --
  simplest, defeats offline enumeration since nothing is derived, and needs no
  secret. Rejected on data minimisation: a database disclosure would then hand
  over wallet identities directly, where today it hands over hashes. Worth
  revisiting only if we conclude we must store account names anyway for
  rotation (cost 3), at which point this becomes the honest choice.

## Verification, if built

- A test that the tenant id is not a function of any public input: two
  provisionings of the same account under different peppers must produce
  different index values and unchanged tenant ids.
- A rotation test that re-indexes and leaves every tenant id byte-identical.
- A test that a missing pepper **refuses** provisioning with a named
  missing-control label rather than falling back to an unsalted hash. This is
  the fail-closed rule; an unsalted fallback would silently restore the
  defect.
- Each mutation-proven: remove the control, show the test fails.

## Provenance

Found while assessing on-chain settlement, 2026-09-08. The claim was verified
in source rather than taken from the report that raised it: an earlier summary
described `account_id` as a random UUID, which is true of a different struct in
the same file (`:129`). `PendingNearProvisioning.account_id` at `:50` is a
`String` carrying the NEAR account name, which is what the hash consumes.
