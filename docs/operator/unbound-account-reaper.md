# Unbound passkey-account reaper

Z2 slice S5 (`docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md`).
Cancel leaves an unbound passkey account inert, and a bind that finds an
existing near.ai account closes the passkey account (S3's refuse branch). The
reaper deletes both kinds once their window has passed. Bound and legacy
accounts (no binding row) are never touched.

## The two windows

| Account | Deleted after (default) | Clock | Variable |
|---|---|---|---|
| `unbound` | 7 days | binding `created_at` | `TRACE_COMMONS_UNBOUND_REAPER_UNBOUND_TTL_DAYS` |
| `closed` | 30 days | `trace_accounts.closed_at` | `TRACE_COMMONS_UNBOUND_REAPER_CLOSED_TTL_DAYS` |

Decided 2026-09-29. An unbound account is reaped on its bound status, not on
use: signing in again does not extend its window. An earlier draft kept an
account that signed in again for 30 days of idleness, which let one sign-in
hold an unbound account, and a slot under the unbound ceiling, for a month.

S3 sets `closed_at` in the same transaction that moves the binding to
`closed`, so `closed_at` is the close time. Closed accounts are not counted
against the unbound ceiling, so without the second window they would pile up
without limit.

Either kind is kept while it holds a **live session**, one that is unrevoked
and unexpired. A session is never reaped out from under the person holding it.
S3 revokes every session when it closes an account, so this matters only for
unbound accounts.

## What is deleted, and what is refused

A reaped account takes its tenant with it. That covers the tenant row, the
account, its binding, credential and sessions, and the tenant's
`trace_account_audit` rows. `trace_audit_events` rows are kept: they are
hash-chained and have no foreign key to the tenant.

The tenant delete cascades into about fifty tenant-keyed tables. The function
therefore first checks that the tenant holds only what a passkey tenant may
hold: the rows listed above, for this one account. If the tenant holds
another account, or any row in any other table keyed to the tenant or the
account, the candidate is **refused whole**. Nothing is deleted, and it
counts as `skipped`.

That set of tables comes from the catalog: every foreign key into
`trace_tenants` or `trace_accounts` through a `tenant_id` column. The reaper
role cannot bypass row security, so a table it cannot see into would look
empty. Rather than risk cascading such a table away unseen, the function
refuses the whole call with:

```
unbound_reaper_scope_incomplete: <table>, ...
```

The tick logs that error and deletes nothing. V99 grants the guard what it
needs on every such table that exists when V99 runs. **A later migration that
adds a tenant- or account-keyed table must do the same:**

```sql
GRANT SELECT (tenant_id) ON <table> TO trace_unbound_account_reaper_guard;
CREATE POLICY trace_unbound_reaper_scope ON <table>
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
```

Until it does, `unbound_account_reaper_pg` fails in CI. The error names
tables only, never a tenant or an account.

## Enable

Off unless `TRACE_COMMONS_UNBOUND_REAPER_ENABLED=true`. When it is on, boot
fails closed if `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL` is missing. Boot
also takes one connection from the reaper pool, so a login that the server
rejects, or a host that cannot be reached within 10 seconds, fails boot rather
than the first tick. That error never contains the URL.

| Variable | Default | Bounds |
|---|---|---|
| `TRACE_COMMONS_UNBOUND_REAPER_ENABLED` | off | |
| `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL` | none; required when enabled | never the runtime URL |
| `TRACE_COMMONS_UNBOUND_REAPER_UNBOUND_TTL_DAYS` | 7 | 1 to 3650 |
| `TRACE_COMMONS_UNBOUND_REAPER_CLOSED_TTL_DAYS` | 30 | 1 to 3650 |
| `TRACE_COMMONS_UNBOUND_REAPER_INTERVAL_SECONDS` | 3600 | 60 to 86400 |
| `TRACE_COMMONS_UNBOUND_REAPER_BATCH_SIZE` | 100 | 1 to 1000 |

An earlier, never-released draft read `TRACE_COMMONS_UNBOUND_REAPER_TTL_DAYS`
and `TRACE_COMMONS_UNBOUND_REAPER_NEVER_USED_TTL_DAYS`. They are not aliases:
the window they set no longer exists. If either is set, boot refuses and names
its replacement.

## Login

V99 creates two NOLOGIN roles:

- `trace_unbound_account_reaper_guard` owns the `SECURITY DEFINER` function
  `trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER)`. It holds the
  column-scoped grants and permissive policies the function needs.
- `trace_unbound_account_reaper` holds EXECUTE on that function and nothing
  else.

Create a LOGIN role that is unprivileged and inherits only the second role,
then point the URL at it:

```sql
CREATE ROLE tc_unbound_reaper_login LOGIN NOSUPERUSER NOBYPASSRLS PASSWORD '...';
GRANT trace_unbound_account_reaper TO tc_unbound_reaper_login;
```

The reaper does not need a superuser or BYPASSRLS login, and using one defeats
the point.

## Evidence

Each tick logs the counts `reaped_unbound`, `reaped_closed` and `skipped`, and
both TTLs. Nothing else leaves the database. The loop reports to the driver
liveness registry as `unbound_account_reaper`.

`skipped` counts candidates that were left alone for any of these reasons:

- locked by a concurrent bind, sign-in or tenant write;
- holding a live session since the scan;
- refused by the tenant check;
- refused by a foreign key;
- lost a lock wait (a 3-second `lock_timeout`);
- aborted by PostgreSQL's deadlock detector (40P01).

The last two resolve on a later tick. A `skipped` count that never falls to
zero means some account's tenant holds rows a passkey tenant should not hold,
and needs a look.

A skipped candidate does not use up the batch. Each call deletes up to `limit`
accounts and examines at most `10 * limit` candidates, oldest first, stepping
past skipped ones. A few refused accounts therefore do not starve the rest,
but a refused backlog larger than that scan cap would. That is one more reason
to look into a persistent `skipped`.
