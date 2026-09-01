# Register-stats public-read role

The public register-stats endpoint (Task 4 of the credit-numbers API, not yet
shipped) serves one aggregate row — `traces_accepted`, `contributors`,
`points_issued`, `as_of`, `refreshed_at` — from `trace_register_stats` to an
**unauthenticated** caller. An unauthenticated request has no tenant, so the
ordinary RLS predicate (`tenant_id = trace_current_tenant_id()`) matches
nothing. This is handled with a dedicated, least-privilege PostgreSQL role —
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

GRANT SELECT (traces_accepted, contributors, points_issued, withheld, as_of, refreshed_at)
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

```sql
-- as the runtime role, after migrating:
SET ROLE trace_commons_public_read;  -- must succeed
SELECT traces_accepted, contributors FROM trace_register_stats;  -- must succeed
UPDATE trace_register_stats SET traces_accepted = 0;  -- must be refused
                                                       -- ("permission denied
                                                       -- for table ...")
```

## Nothing schedules the refresh yet

The refresh worker route (`POST /v1/workers/register-stats/refresh`,
`RegisterStatsWorker` token role) computes and writes the aggregate. Nothing
in this repo schedules it — wire it to a timer (cron, a scheduled Cloud Run
job, systemd timer, etc.) as part of your deployment. Until it has run at
least once, `refreshed_at` stays `NULL` and the public endpoint (Task 4)
refuses to publish, because a zero would be a claim about the register that
nobody made.
