# Dedicated staging upload-trust database login

`prepare-upload-trust-role.sql` prepares only `tc_upload_trust_login` in an
already migrated database. It does not deploy ingest, migrate, record facts,
evaluate accounts, enable invalidation, or change either admission switch.
The main `app` migration/runtime identity remains unchanged.

The independent upload-trust pool uses
`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL`, pointing to the **same** host,
port and database as ingest, with this separate login. Keep the login out of
`DATABASE_URL`, the login-resolver URL, and the gate-driver URL. The new pool
must not run migrations. An owner/guard login cannot satisfy the existing trust
readiness checks, even when its protected memberships have inheritance disabled.

## Preconditions

Verify the current staging VM/project, deployed revision, database identity,
backup and applied migration set before executing the SQL. PostgreSQL 16 and
migrations through V138 are required for activation. The bootstrap requires
at least V137; V138 supplies the indexed frontier repair. V137 must repair both fact candidates and the
fact writer to recognize the verified `trace_account_owned_principals` view
through `trace_account_trust_live_owned_principal`;
source/qualification regression tests must pass before rollout.

Run as the PostgreSQL administrator, with an explicit expected database name:

```sh
psql --no-psqlrc --dbname=<reviewed staging administrator connection> \
  --set=expected_database=tracecommons \
  --file=deploy/staging-gcp/prepare-upload-trust-role.sql
```

For a local isolated fixture, substitute its explicit `admission_test_*`
database name in both the connection and `expected_database`. Do not run this
against a production database while following the staging procedure.

The SQL is transactional and repeatable for an already safe login. It refuses
an existing identity with elevated attributes, unexpected memberships,
ADMIN/SET membership options, object ownership, schema-creation rights, or
forbidden source/trust-table writes. It does not revoke unexpected grants or
normalize another role to make a preflight pass. A refusal requires review of
that identity; select a clean, reviewed identity rather than weakening checks.

The login inherits only `trace_client_upload_trust_runtime`. That NOLOGIN
role inherits the existing `trace_account_trust_worker`, whose reviewed grants
are guarded recorder/evaluator EXECUTE seams and tenant-scoped evaluation
SELECT. Both membership edges use `INHERIT TRUE`, `SET FALSE` and `ADMIN FALSE`.
The login cannot inherit or SET the admission runtime, an app role, a guard,
or the external evaluator. An existing login with the old broad admission
membership is refused; this script does not revoke grants to make it pass.

The upload-specific group adds only growth-invalidation enable/read and input
generation EXECUTE, plus the exact account/device/trust/budget SELECT columns
used by `resolve_contribution_account` and advisory `account_admission_status`.
It has no direct table or column INSERT/UPDATE/REFERENCES rights, table
DELETE/TRUNCATE/TRIGGER rights, admission reservation/transition/legacy-resume
EXECUTE, reservation frontier locks, or dependency-pruning authority. SELECT
outside the explicit status columns and evaluation table is also refused.
Raw submission, gate, audit and credit rows stay behind the existing definer
seams. Verification checks both table privileges and column privileges;
`has_table_privilege` alone does not detect an isolated column grant.

## Password and configuration

A newly created login has no password. An existing safe login's password is
left untouched. Provision a unique password separately through the existing
Secret Manager workflow; proposed staging secret name:
`tc-staging-db-upload-trust-password`. No password belongs in this SQL, a
committed environment file, a shell argument, a report or a log.

Pass the password and complete worker connection string through the existing
secret renderer or protected stdin, and store the resulting
`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL` with the runtime environment's
existing restrictive permissions. Do not print the URL. Grant the staging
runtime service account access only to the new credential it requires.
Preparing a password or connection is not activation and must not enable
account admission.

## Read-only preflight

The SQL ends with `BEGIN READ ONLY; SET LOCAL ROLE tc_upload_trust_login` and
prints only role attributes and contract booleans. Required role attributes:
LOGIN/INHERIT true; SUPERUSER/BYPASSRLS/CREATEDB/CREATEROLE false.
Required upload-trust/worker membership, worker/status EXECUTE and evaluation
SELECT checks must be true. Admission-runtime membership, admission-transition
EXECUTE, and direct table or column writes must be false.

`external_invalidation_enabled` is allowed to be false at this preparation
stage. The bootstrap never enables it. A later reviewed application startup
activates the invalidation contract and must then pass
`client_upload_trust_runtime_ready` and `tiered_account_trust_runtime_ready`
before running the scheduler. Client-only readiness retains the external
evaluation, ownership, invalidation, generation and forced-RLS checks, while
refusing broad writes and admission mutation privileges. It does not require
the unused reservation-lock and dependency-pruning seams. Existing admission
runtime readiness and its required privileges are unchanged.
A password or successful SQL completion does not establish those final checks,
account evaluation freshness, or client-visible increased limits.

After the dedicated pool is configured, independently verify the pool's actual
`current_user`, database identity and startup readiness; then verify account
facts/evaluation generation and the authenticated client allowance response.
Keep `TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED` and
`TRACE_COMMONS_ADMISSION_ENABLED` false for this staging rollout.

## Observed source labels

Read-only staging evidence on 2026-10-25 found gate version
`staging-2026-10` and dedup signal
`events.v1+fnv1a-3shingle-set.v2`. Recheck them before preparing the policy.
The approved policy counts 5/20/50 novel accepted unique contributions for
2x/3x/5x daily caps, with no age wait. Do not replace these labels with guessed
versions, infer quality from upload count, synthesize old credit events, or
change historical trace/account attribution to make qualification succeed.
