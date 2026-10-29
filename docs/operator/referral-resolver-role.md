# Referral-resolver role grant

Referral rewards (`docs/superpowers/specs/2026-10-29-referral-rewards-design.md`)
put a referrer and each person they refer in different tenants. Three things
read across tenants, and all of them go through one least-privilege role,
`trace_referral_resolver`:

- applying a code, which resolves the code index to the tenant and account
  that own it;
- the bonus pass, which lists the tenants that hold codes and counts recent
  suspensions;
- the operator routes `GET /v1/admin/referrals/suspended` and
  `POST /v1/admin/referrals/dry-run-report`, which enumerate suspended codes.

Unlike `trace_login_resolver` (`login-resolver-role.md`) it has no pool of its
own. The ingest service assumes it inside a single transaction with
`SET LOCAL ROLE trace_referral_resolver`, from the login it already connects
as, and drops it when that transaction ends.

## What V142 creates

```sql
CREATE ROLE trace_referral_resolver NOLOGIN NOBYPASSRLS;
GRANT SELECT (tenant_id, account_id, code_index, disabled_at, suspended_at, voided_at)
    ON trace_referral_codes TO trace_referral_resolver;
-- plus the role-scoped permissive policy trace_referral_resolver_cross_tenant_read
--   (FOR SELECT TO trace_referral_resolver USING (true))
```

The role reads six columns of one table and nothing else: not the sealed code,
not attributions, bonus rows, audit rows or network marks. It must stay
`NOLOGIN` and `NOBYPASSRLS`: the cross-tenant read is authorized only by the
role-scoped policy, and with `BYPASSRLS` the policy would no longer be what
confines it.

V142 deliberately grants the role to no one, as V132 does for the payout roles.
The operator grants it.

The role carries no settings. `SET LOCAL ROLE` never loads a role's
`ALTER ROLE ... SET` values, so the 2 second statement timeout is set by each
transaction that assumes the role. Do not add one to the role: it would never
apply.

## Grant it to the ingest login

Grant the role to the login the ingest service connects as, with no
inheritance, so the login's own sessions never hold the role's privilege and
only an explicit `SET ROLE` does:

```sql
GRANT trace_referral_resolver TO <ingest login> WITH INHERIT FALSE, SET TRUE;
```

On staging the ingest login is `app`, which created the role by migration and
holds the admin option `GRANT` needs:

```bash
PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password \
    --project tracecommons-pilot-2026)" \
  psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1 <<'SQL'
GRANT trace_referral_resolver TO app WITH INHERIT FALSE, SET TRUE;
SQL
```

Make the grant before either of these is set:

- `TRACE_COMMONS_REFERRAL_PROGRAM_START` (the program accepts codes), or
- `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE` other than `disabled` (the bonus pass
  runs).

## Boot check (fail closed)

While either setting above is on and the service has a database, ingest
checks at startup that its login can assume the role, by opening a
transaction, assuming it, and rolling back. It refuses to start with:

- `referrals refused to start: referral_resolver_unprovisioned` -- the
  `SET ROLE` was refused: the grant is missing, or was made with `SET FALSE`.
- `referrals refused to start: referral_resolver_check_failed` -- any other
  database error during the check (connection lost, role missing because V142
  has not been applied).

With referrals off, nothing is checked and the role is never assumed.

## Verification

```sql
-- NOLOGIN and NOBYPASSRLS: both false.
SELECT rolcanlogin, rolbypassrls, rolconfig FROM pg_roles
 WHERE rolname = 'trace_referral_resolver';
-- rolconfig must be NULL.

-- The grant: inherit false, set true.
SELECT r.rolname AS role, m.rolname AS member, a.inherit_option, a.set_option
  FROM pg_auth_members a
  JOIN pg_roles r ON r.oid = a.roleid
  JOIN pg_roles m ON m.oid = a.member
 WHERE r.rolname = 'trace_referral_resolver';

-- The role cannot write, nor read another referral table.
SELECT has_table_privilege('trace_referral_resolver', 'trace_referral_codes', 'INSERT');            -- f
SELECT has_table_privilege('trace_referral_resolver', 'trace_referral_bonus_accruals', 'SELECT');   -- f
```

As the ingest login, assuming the role must succeed and report the timeout:

```sql
BEGIN;
SET LOCAL ROLE trace_referral_resolver;
SET LOCAL statement_timeout = '2s';
SELECT current_user, current_setting('statement_timeout');  -- trace_referral_resolver | 2s
ROLLBACK;
```

`crates/trace-commons-server/tests/referral_rewards_pg.rs` exercises the same
invariants against a test database.
