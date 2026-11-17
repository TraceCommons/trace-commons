# RLS readiness composition

The catalog diagnostic now checks the effective policy set, not only the
presence of `trace_corpus_tenant_isolation`. That named policy must be
permissive and retain its exact tenant expressions. It must apply to the
current role except on the twelve V69/V71 reward tables: each of those is
bound to the exact single operator or participant guard role named by its
migration. Their ordinary-runtime default deny is intentional; controlled
SECURITY DEFINER entry points hold those guard privileges. Changing a guard
policy to PUBLIC or another role fails readiness.

Every other applicable permissive policy must use the
recognized tenant expression on each side relevant to its command. An omitted
UPDATE/ALL `WITH CHECK` follows PostgreSQL's USING fallback. Restrictive
policies are permitted because they intersect the allowed rows; policies for
unrelated roles do not affect this decision. This is a conservative structural
check, not a SQL expression equivalence prover.

V56's community withdrawal drain is the one additional recognized policy
shape. Acceptance binds the exact table, policy name, PUBLIC role set,
SELECT/UPDATE command, scope predicate and nullable-drained-state transition
on both USING and WITH CHECK. The existing drain intentionally crosses tenants
only while its transaction-local `trace_commons.community_drain` scope is on;
the application controls that GUC as it controls the tenant GUC. This does not
make the runtime safe against arbitrary SQL execution. No other policy is
accepted merely for mentioning the drain function or using a familiar name.

Policy applicability uses PostgreSQL 16 `pg_has_role(..., 'USAGE')`, including
transitive inherited membership. `SET` reachability from the session user is
checked separately for superuser/BYPASSRLS roles and for table ownership,
including ownership inherited after an explicit role transition. The current
role is included too. FORCE RLS does not make an owner safe because owners can
alter the policies. BYPASSRLS attributes are not inherited merely through a
USAGE-only membership. A privileged session cannot hide behind SET ROLE.

The diagnostic keeps its existing result shape: unsafe policy composition is
reported in `policy_expression_mismatch_tables`; reachable ownership/bypass
sets the existing booleans. Both `tables_isolated()` and production readiness
therefore fail without a new caller-specific gate.

The referral startup probe first requires safe runtime isolation on
`trace_referral_codes`, then explicitly assumes the resolver and verifies its
effective grants: exactly the six V142 read columns, no other table or column
reads in the current schema, no writes, and NOLOGIN/NOBYPASSRLS/NOSUPERUSER.
The intended non-inherited SET LOCAL ROLE transition remains accepted.

Regressions in `trace_corpus_pg_rls.rs` call the actual catalog diagnostic
against all migrated tables with transaction-scoped role/policy mutations.
They cover direct and nested inheritance, unrelated and restrictive policies,
command-specific widening, named-policy applicability, inherited/assumable
ownership, nested assumable bypass, and the privileged-session case.
They also mutate each recognized guard/drain policy's role, expression, check,
name, command and table binding; unchanged full migrations remain accepted.
`referral_rewards_pg.rs` exercises the real startup probe and full readiness,
including narrow-column grant drift. These tests require disposable local
PostgreSQL fixtures; execution belongs to the integrated branch validation.

The check assumes trusted migration/role administrators and authentic
`trace_current_tenant_id()` implementation. It does not prove arbitrary
SECURITY DEFINER functions safe, validate HTTP tenant derivation, or prevent
later administrator changes to catalogs. Existing opt-in deployment readiness
configuration is unchanged.

Sources: [PostgreSQL 16 policy composition](https://www.postgresql.org/docs/16/ddl-rowsecurity.html),
[role privilege inquiries](https://www.postgresql.org/docs/16/functions-info.html#FUNCTIONS-INFO-ACCESS-TABLE),
and [session authorization](https://www.postgresql.org/docs/16/sql-set-session-authorization.html).
