# Counting legacy invite identities

`scripts/operator/legacy-invite-counts.sql` produces the read-only inventory
that [account trust](./account-trust.md) asks for before account admission is
enabled. It measures under the coexistence rules of
[legacy invite migration](./legacy-invite-migration.md). It prints counts
only: no tenant, account, device or invite identifier and no label reaches
its output.

## Run it

```bash
PGOPTIONS='-c default_transaction_read_only=on' \
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -f scripts/operator/legacy-invite-counts.sql
```

- Connect as a role with `BYPASSRLS`, or as the superuser. Every table is
  `FORCE ROW LEVEL SECURITY`, so an RLS-bound role would see no rows and
  report zero of everything. The script refuses such a role with
  `LegacyInviteCountsRoleSubjectToRls` instead.
- It runs in one `READ ONLY`, `REPEATABLE READ` transaction and ends in
  `ROLLBACK`, so all figures come from one snapshot.
- It works on a schema before V81, so you can take the "before" figures prior
  to deploying the link migration. Without V81, every figure that needs the
  pooled marker, the links or the claims is **empty (NULL), not 0**, and
  every legacy invite tenant is reported as `unclassified`.

## What each row means

The output is `section|bucket|value`.

| Section | Bucket | Meaning |
|---|---|---|
| `source` | `v81_legacy_invite_link` | `1` if V81's tables are present, else `0`. |
| `legacy_invite_tenants` | `total` | Tenants outside `near-`/`nearai-` that redeemed an invite through `/v1/onboard` (they have an `onboarding_invites` row). |
| | `pooled` / `individual` | Split by the operator's pooled marker. Empty before V81. |
| | `unclassified` | Every tenant before V81, and 0 after it. |
| | `review_before_linking` | Tenants you have **not** marked pooled whose invite allows more than 3 uses (the individual invite shape), or that more than one invite routes to. Decide each one before enabling linking. This is a prompt, not a classification. |
| | `instance_enrolled_not_invite` | Legacy-namespace tenants with `invite`-origin devices but no redeemed invite. These are instance enrollments, which cannot link. |
| `devices` | `<class>:active` / `<class>:revoked` | Device keys in legacy invite tenants, by class. |
| `devices_per_tenant` | `<class>:<n>` | How many tenants of the class have n active devices, in the buckets 0, 1, 2, 3, 4-10, 11-100 and 101+. |
| `accounts` | `wallet:open` / `wallet:anchored` | Open accounts in `near-`, and how many of them carry a NEAR anchor. |
| | `near_ai:open` / `near_ai:anchored` | The same for `nearai-`. |
| | `legacy:open` | Open accounts in legacy-namespace tenants. |
| `links` | `individual:linked` / `individual:unlinked` | Individual tenants with and without a live link. Empty before V81. |
| | `pooled:linked` | Should be 0: pooled tenants cannot link. Nonzero means a tenant was marked pooled after it had been linked. |
| | `accounts_holding_a_link` | Distinct NEAR accounts holding at least one live link. |
| `ambiguous` | `tenants_claimed_by_more_than_one_account` | Non-pooled tenants with an unresolved second claim. |
| | `open_second_claims` | The unresolved claim rows behind that figure. |
| `readiness_blockers` | `1_…` to `4_…`, `total` | What would make `trace_account_admission_linkage_ready()` false, rule by rule. See "Readiness under coexistence" in the migration runbook. `total` is empty before V81, because the function actually in force then is V77's, which refuses every legacy identity. |

A `total` of 0 on a V81 schema means readiness passes on the durable
inventory. It does not mean that account admission may be switched on: the
other activation conditions in [account trust](./account-trust.md) still
apply, including static tokens on each replica.

## Keeping it honest

`crates/trace-commons-server/tests/legacy_invite_counts.rs` does two things:

- it fails when the script's copy of the account namespaces drifts from
  `ANCHOR_NAMESPACES`, and when the script stops being read-only;
- it measures seeded fleets through the real `psql`, on a current schema and
  on one stopped at V80. On the current schema it checks
  `readiness_blockers|total` against `trace_account_admission_linkage_ready()`
  itself, in both directions: blocked by a claim on an individual tenant, and
  not blocked by a claim on a pooled one.

CI runs it in `database suites against a real PostgreSQL` and fails that job
if the PostgreSQL half skipped.
