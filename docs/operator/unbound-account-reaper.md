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
use: signing in again does not extend its window, and neither does holding a
live session (see below). An earlier draft kept an account that signed in
again for 30 days of idleness, which let one sign-in hold an unbound account,
and a slot under the unbound ceiling, for a month.

S3 sets `closed_at` in the same transaction that moves the binding to
`closed`, so `closed_at` is the close time. Closed accounts are not counted
against the unbound ceiling, so without the second window they would pile up
without limit.

**A live session does not put the reap off** (decided 2026-09-30). An earlier
draft kept either kind while it held an unrevoked, unexpired session. Native
sessions last 12 hours, so signing in about twice a day kept an unbound
account, and its slot under the unbound ceiling, indefinitely. Now each kind
is reaped once its window has passed, whatever sessions it holds, and those
sessions are deleted in the same transaction as the account, by the
`ON DELETE CASCADE` from `trace_accounts`. A request already in flight on one
of them may finish; the next request with that session is refused, because
the session row is gone. S3 revokes every session when it closes an account,
so in practice this changes only unbound accounts.

## What is deleted, and what is refused

The reaper deletes the **account**, never the tenant (decided 2026-09-29). It
deletes the `trace_accounts` row and nothing else directly; everything that
goes, goes by `ON DELETE CASCADE` from that row. For a passkey account that is
its binding, credentials, sessions and login links.

The tenant row stays, possibly empty, and so does every row keyed to the
tenant rather than the account. That includes `trace_account_audit` and the
hash-chained `trace_audit_events`. What else the tenant holds does not matter
and does not refuse the candidate.

**Accepted: empty tenants accumulate** (decided 2026-09-30). Every passkey
creation makes its own tenant, so each reap leaves one empty tenant row and
its audit rows behind. Code that enumerates every tenant (the leaderboard,
corpus-analytics and register-stats totals in `db/postgres.rs`) iterates
those too. A later sweep, or those enumerations skipping tenants with no
account, is tracked in
[#1153](https://github.com/TraceCommons/trace-commons/issues/1153).

The same cascade from `trace_accounts` also reaches
`trace_account_principals`, `trace_near_identities`,
`trace_account_merge_proposals`, `trace_public_runs`, `trace_source_sessions`
and `trace_account_inference_connections`. The unbound gate refuses an unbound
or closed account's session on every route that writes those, so a candidate
is not expected to hold any. If one does, the reap deletes them with the
account. `unbound_account_reaper_pg` pins the exact list of tables that cascade
from `trace_accounts`, so a migration that changes it has to change that list
too, where review sees it. That is the only place it shows up: a migration owes
the reaper no grant, policy or other change.

An account-keyed row whose foreign key does **not** cascade (`ON DELETE
RESTRICT` or `NO ACTION`: `trace_reward_principal_accounts`, the account trust
and admission tables, the legacy invite link tables,
`trace_near_account_anchors`, and any added later) makes the account delete
fail with a foreign-key violation (23503). The candidate is then **refused
whole**: its sub-transaction rolls back, nothing of it is deleted, and it
counts as `skipped`.

A second account in the same tenant does not refuse the candidate and is not
touched. The delete is keyed to the candidate's `(tenant_id, account_id)` and
cascades only from that row. A tenant-wide refusal existed only to protect a
tenant delete, which the reaper no longer does.

## Enable

Off unless `TRACE_COMMONS_UNBOUND_REAPER_ENABLED=true`. When it is on, boot
fails closed if `TRACE_COMMONS_UNBOUND_REAPER_DATABASE_URL` is missing. Boot
also takes one connection from the reaper pool and checks
`has_function_privilege` for EXECUTE on
`trace_reap_unbound_accounts(bigint, bigint, integer)`. A login that the
server rejects, a host that cannot be reached within 10 seconds, a login that
connects without `trace_unbound_account_reaper`, or a database where the
function does not exist fails boot rather than the first tick. The first two
report a connection failure and the last two `unbound_reaper_execute_denied`.
Neither error contains the URL.

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

V101 creates two NOLOGIN roles:

- `trace_unbound_account_reaper_guard` owns the `SECURITY DEFINER` function
  `trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER)`. It holds
  column-scoped grants and permissive policies on `trace_account_bindings`
  and `trace_accounts`, and on nothing else. It reads no session: sessions
  no longer gate the reap, and the cascade that deletes them runs as the
  table owner.
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

- locked by a concurrent bind, sign-in or other write to the account (a
  sign-in that commits does not save the account on a later tick);
- refused by a non-cascading foreign key into the account (23503);
- lost a lock wait (a 3-second `lock_timeout`);
- aborted by PostgreSQL's deadlock detector (40P01).

The last two resolve on a later tick. A `skipped` count that never falls to
zero means some unbound or closed account is held by a non-cascading row (a
reward principal, trust or admission row, or a legacy invite link) that such
an account should not have, and needs a look.

A skipped candidate does not use up the batch. Each call deletes up to `limit`
accounts and examines at most `10 * limit` candidates, oldest first, stepping
past skipped ones. A few refused accounts therefore do not starve the rest,
but a refused backlog larger than that scan cap would. That is one more reason
to look into a persistent `skipped`.
