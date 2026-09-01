# Register-stats public-read role

The public register-stats endpoint (`GET /v1/public/register-stats`) serves
one aggregate row from `trace_register_stats` to an **unauthenticated**
caller. It *reads* the whole row — `singleton`, `traces_accepted`,
`contributors`, `points_issued`, `withheld`, `suppressed`, `as_of`,
`refreshed_at` — and *publishes* at most `traces_accepted`, `contributors`,
`points_issued`, plus `withheld`, `scope`, `as_of` and `posture`. The three
counts are withheld together; the rest of the row never reaches the wire.

An unauthenticated request has no tenant, so the ordinary RLS predicate
(`tenant_id = trace_current_tenant_id()`) matches nothing. This is handled
with a dedicated, least-privilege PostgreSQL role —
`trace_commons_public_read` — rather than any of the tempting broad fixes
(`BYPASSRLS`, a superuser pool, or dropping `FORCE ROW LEVEL SECURITY`).

## How this differs from the login-resolver role

The login-resolver role (`docs/operator/login-resolver-role.md`) needs its
**own connection pool** because it authenticates a bootstrap request before
any session exists. `trace_commons_public_read` does not: it is assumed with
`SET ROLE trace_commons_public_read` on the **existing runtime connection**,
for the duration of a single read, then the connection reverts. No second
pool, no second env var, no second credential to provision.

## Why provisioning is (usually) still required

The `V55__register_stats_public_read.sql` migration creates the role as:

```sql
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_commons_public_read') THEN
        CREATE ROLE trace_commons_public_read NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

GRANT SELECT (singleton, traces_accepted, contributors, points_issued, withheld, suppressed, as_of, refreshed_at)
    ON trace_register_stats TO trace_commons_public_read;

GRANT trace_commons_public_read TO CURRENT_USER;
```

Two properties are load-bearing:

- **`NOBYPASSRLS`** — the role MUST remain NOBYPASSRLS. Its read is
  authorized only by the role-scoped policy `trace_register_stats_public_read`
  (`FOR SELECT TO trace_commons_public_read USING (true)`). If the role
  bypassed RLS the policy would be irrelevant and it would see every row of
  every RLS-forced table it had a column grant on — it doesn't have one on
  anything else, but the property is what keeps that true even as the schema
  grows.
- **`GRANT trace_commons_public_read TO CURRENT_USER`** — the migration runs
  as whichever role applies it, which in every real deployment is the app's
  own runtime role (the same role the ingest binary connects as). This grant
  is what lets that runtime role later `SET ROLE trace_commons_public_read`
  at request time. **Without it, `SET ROLE` fails with "permission denied to
  set role"** and the endpoint 500s on every request — the DO block above
  creates a role nobody can ever assume.

**Requirement this leaves on the deployment: the role that actually serves
requests must be a member of `trace_commons_public_read`.** Because the
`GRANT ... TO CURRENT_USER` runs automatically as part of the migration,
**no separate provisioning step is required for the common case** where
migrations are applied by the same role the ingest binary connects as — the
applier and the server are the same role, so the grant already covers it.

This is *not* automatic when your deployment applies migrations as a
*different* role than the one serving requests (e.g. a separate
migration-runner credential on managed Postgres). In that shape, the
mismatch does not surface at migration time — it surfaces at the first
request, as `SET ROLE trace_commons_public_read` failing with "permission
denied to set role". Check which role your ingest binary actually connects
as, and if it differs from whoever ran the migration, grant membership to
it explicitly after migrating:

```sql
GRANT trace_commons_public_read TO <runtime_role>;
```

## Verifying it locally

`SET ROLE` succeeds trivially for a PostgreSQL superuser regardless of
membership, so testing against a local superuser-owned database does not
prove the grant is doing anything. To verify for real, connect as a
non-superuser role with no other membership and confirm both directions:

Run **the statement the server actually issues**, character for character —
a shortened projection is not a check. A recipe that dropped the real
query's filter once passed here while every live request was denied:

```sql
-- as the runtime role, after migrating:
SET ROLE trace_commons_public_read;  -- must succeed
SELECT traces_accepted, contributors, points_issued, withheld, suppressed, as_of, refreshed_at FROM trace_register_stats WHERE singleton = TRUE;  -- must succeed
UPDATE trace_register_stats SET traces_accepted = 0;  -- must be refused
                                                       -- ("permission denied
                                                       -- for table ...")
```

That SELECT is `REGISTER_STATS_SELECT_SQL`
(`crates/trace-commons-server/src/db/postgres.rs`), and a unit test fails if
this file and that constant ever disagree.

**Note the `WHERE singleton = TRUE`, and note that `singleton` is in the grant
above.** PostgreSQL column privileges cover every column a query *references*,
`WHERE` included — not just the ones it projects — so filtering on a column
that is not granted denies the **whole table** under this role, with an error
that names no column:

```
ERROR:  permission denied for table trace_register_stats
```

That was a real defect: the grant omitted `singleton` while the query filtered
on it, and every request 500'd. If you ever add a column to that statement, add
it to the grant in the same change. Do not "fix" such a denial by dropping the
filter — the filter is what keeps the read correct if the table's
`CHECK (singleton)` constraint is ever relaxed, since `query_one` demands
exactly one row.

## The contributor floor

`TRACE_COMMONS_REGISTER_STATS_CONTRIBUTOR_FLOOR` sets how many contributors
the configured communities must hold before the endpoint publishes
`contributors` and `points_issued` at all. It defaults to **25**, and an
unset, blank, malformed or negative value resolves to that default rather
than to a floor that suppresses nothing.

Below the floor **every figure is absent from the response**, not zero, and
`withheld` is `true`. That includes `traces_accepted`: it counts submissions
rather than people, but below the floor the people are few by construction and
`withheld: true` tells the caller so, which makes that field one person's trace
count and its delta between refreshes that person's submission rate. The
response then carries only `withheld`, `scope`, `as_of` and `posture`.

The endpoint also reports `scope: "configured_communities"`, because the
refresh aggregates the tenants this deployment configured as communities, not
every tenant the server holds.

Set it lower only against a real contributor count. With few contributors a
known cohort plus a published total is one person's earnings.

## Suppressing publication during an incident

`trace_register_stats.suppressed` is **the operator's off switch**, and the
only one. Set it and the endpoint publishes no figure until you clear it:

```sql
UPDATE trace_register_stats SET suppressed = TRUE WHERE singleton = TRUE;
-- and to resume:
UPDATE trace_register_stats SET suppressed = FALSE WHERE singleton = TRUE;
```

**The refresh never writes this column**, so a scheduled refresh running on a
timer will not lift your suppression. That is the whole reason it exists as a
separate column.

Do **not** use `withheld` for this. It is the computed/never-computed marker:
it starts `TRUE`, and *every* refresh clears it. Setting it by hand during an
incident would look like it worked and then be silently undone by the next
scheduled tick, with no error and no log entry.

## Nothing schedules the refresh yet

The refresh worker route (`POST /v1/workers/register-stats/refresh`,
`RegisterStatsWorker` token role) computes and writes the aggregate. Nothing
in this repo schedules it — wire it to a timer (cron, a scheduled Cloud Run
job, systemd timer, etc.) as part of your deployment. Until it has run at
least once, `refreshed_at` stays `NULL` and the public endpoint refuses to
publish any figure at all — not even `traces_accepted` — because a zero would
be a claim about the register that nobody made.

The row's own `withheld` column tracks exactly that state and has the same
effect while it is set. **It is not the operator control**, and setting it by
hand does not stop publication for long: the refresh owns that column and
clears it on every run. To stop publication deliberately, use `suppressed` —
see "Suppressing publication during an incident" above.
