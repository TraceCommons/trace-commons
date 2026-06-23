# Login-resolver role provisioning

The contributor-account redeem path (`/v1/account/login/redeem`) maps a
globally-unique login-link `code_hash` to its owning `tenant_id` while running
with **no tenant context** (the request is unauthenticated until the link is
resolved). To do this safely under forced row-level security it uses a
dedicated, least-privilege PostgreSQL role — `trace_login_resolver` — on a
**separate connection pool** from the runtime pool. The resolver pool is
configured by:

```sh
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL="postgres://<role>@/trace-commons?host=/cloudsql/.../trace-commons"
```

## Why provisioning is required

The `V30__trace_accounts.sql` migration creates the role as:

```sql
CREATE ROLE trace_login_resolver NOLOGIN NOBYPASSRLS;
GRANT SELECT (tenant_id, code_hash) ON trace_login_links TO trace_login_resolver;
-- plus the role-scoped permissive policy trace_login_resolver_cross_tenant_read
```

Two properties are load-bearing:

- **`NOLOGIN`** — the role is created without login so it cannot be connected to
  directly as shipped. The migration intentionally does **not** bake a password
  into the schema. An operator must make the resolver pool able to connect.
- **`NOBYPASSRLS`** — the role MUST remain NOBYPASSRLS. The cross-tenant read is
  authorized **only** by the role-scoped permissive policy
  `trace_login_resolver_cross_tenant_read` (added in V30, `FOR SELECT TO
  trace_login_resolver USING (true)`). If the role bypassed RLS, the policy
  would be irrelevant and the role would see every row of every RLS-forced table
  it had a grant on. Keeping it NOBYPASSRLS is what confines it to the
  one-table, two-column, policy-gated read.

Because the base role is `NOLOGIN`, leaving it as-is makes redeem **fail closed
and non-functional**: the resolver pool cannot establish a connection, so no
tenant ever resolves and every redeem 400s. You MUST provision one of the two
paths below before first contributor traffic.

## Recommended: dedicated LOGIN role with role membership

Create a separate LOGIN role and grant it membership in `trace_login_resolver`.
The login role inherits the column GRANT and the role-scoped SELECT policy, and
is itself NOBYPASSRLS.

```sql
CREATE ROLE tc_login_resolver_login LOGIN PASSWORD '<secret>' NOBYPASSRLS;
GRANT trace_login_resolver TO tc_login_resolver_login;
```

Then point the resolver pool at the login role:

```sh
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL="postgres://tc_login_resolver_login@/trace-commons?host=/cloudsql/.../trace-commons"
```

This keeps the privilege-bearing role (`trace_login_resolver`) separate from the
credential that connects, so the password can be rotated by recreating only the
login role, and the policy/grant surface is defined once on the base role.

> Membership inheritance requires the login role to have `INHERIT` (the
> default). Do not grant the login role `BYPASSRLS`.

## Alternative: make the base role directly connectable

If you prefer a single role, grant LOGIN to the base role directly:

```sql
ALTER ROLE trace_login_resolver LOGIN PASSWORD '<secret>';
```

```sh
export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL="postgres://trace_login_resolver@/trace-commons?host=/cloudsql/.../trace-commons"
```

This is simpler but couples the credential to the privilege-bearing role. Do
**not** add `BYPASSRLS` when doing this — the role must stay NOBYPASSRLS for the
permissive policy to be the sole authorization for the cross-tenant read.

## Passkey login also uses this same resolver role (Slice 2)

The discoverable passkey login path (`POST /account/passkey/login/finish`) has
the **same** unauthenticated credential-to-tenant bootstrap problem as redeem: it
must map a globally-unique `credential_id` to its owning `tenant_id` while running
with no tenant context, before any session exists. It uses the **same**
`trace_login_resolver` role on the **same** separate resolver pool — there is **no
extra login role to provision**. If you have already provisioned the resolver pool
for redeem (above), passkey login works with no further role setup.

The `V32__trace_webauthn_credentials.sql` migration extends the role:

```sql
GRANT SELECT (tenant_id, credential_id) ON trace_webauthn_credentials TO trace_login_resolver;
-- plus the role-scoped permissive policy trace_login_resolver_credential_read
--   (FOR SELECT TO trace_login_resolver USING (true))
```

This mirrors the `trace_login_links` grant exactly: a two-column SELECT plus a
role-scoped permissive policy, with the role still `NOBYPASSRLS`. The column grant
deliberately excludes every other column (e.g. `account_id`, `passkey`), so even
under the permissive policy the resolver can read only `(tenant_id,
credential_id)`. The owning-tenant's full credential row is loaded afterward on the
runtime pool, under that tenant's normal RLS.

**Fail-closed behavior:** if the resolver pool is unconfigured (or its connection
fails), the passkey LOGIN path fails closed exactly like redeem — every assertion
collapses to the uniform deny and no session is minted. Enrollment and management
of passkeys run on the authenticated runtime pool and are unaffected; only the
unauthenticated login bootstrap depends on the resolver pool.

## Verification

After provisioning, confirm the invariants:

```sql
-- Must be NOBYPASSRLS (false).
SELECT rolname, rolbypassrls FROM pg_roles
 WHERE rolname IN ('trace_login_resolver', 'tc_login_resolver_login');

-- Resolver must NOT be able to write or read other tables.
SELECT has_table_privilege('trace_login_resolver', 'trace_login_links', 'INSERT'); -- f
```

A failed/missing resolver connection surfaces as redeem returning 400 with the
session never establishing; check that the URL points at a role that (a) can
log in and (b) is or inherits `trace_login_resolver`.
