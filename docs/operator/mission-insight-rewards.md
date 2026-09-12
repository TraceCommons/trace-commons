# Mission and insight rewards

The reward pilot records fixed, program-specific units for a published evidence task. It is a tenant-scoped, operator-managed PostgreSQL workflow. A participant hash is `operator_asserted`: it does not authenticate a Trace Commons account, prove a unique person, establish independence, or identify a wallet. Units have no redemption, exchange rate, settlement, cash value, clawback, or adjustment path.

An issuer reserves and submits evidence. An independent reviewer accepts or rejects it under the published rubric. Qualified negative and inconclusive results may qualify when the rubric permits them. Local mission drafts, private Insights work, and a digest by itself do not establish completion.

## Preconditions

- Publish the task definition, rubric, evaluator policy, required evidence, rights, challenge policy, sponsor identity, unit schedule, and closing time. Store the published artifacts in the approved external evidence location.
- Assign one issuer and one reviewer with different stable SHA-256 actor hashes. Check affiliations and likely Sybils manually; hash inequality does not prove independence.
- Use the PostgreSQL login assigned to the operator. The CLI has no `--actor` or `--reviewer` override, and session authentication determines authority.
- Build `trace-commons-reward-operator` from the release being operated. Successful output is JSON, including with `--json`; failures return a safe label and next action on stderr.

## DBA provisioning

Run migration `V69__mission_insight_rewards.sql` through normal server migration registration. It creates `trace_reward_guard` and `trace_reward_runtime` as `NOLOGIN NOBYPASSRLS`, forces RLS on every `trace_reward_` table, and revokes runtime table access. The runtime group receives `EXECUTE` only on the eight public reward functions. Do not grant table DML, access to `trace_reward_operators`, `BYPASSRLS`, superuser, role administration, database creation, or schema creation to an operator login.

Create separate direct-login roles. Run this as a DBA with table-write privileges and BYPASSRLS or superuser authority; never use either operator login for these statements.

```sql
CREATE ROLE reward_issuer LOGIN INHERIT NOSUPERUSER NOBYPASSRLS
  NOCREATEDB NOCREATEROLE NOREPLICATION;
CREATE ROLE reward_reviewer LOGIN INHERIT NOSUPERUSER NOBYPASSRLS
  NOCREATEDB NOCREATEROLE NOREPLICATION;
GRANT trace_reward_runtime TO reward_issuer, reward_reviewer;

INSERT INTO trace_reward_operators
  (tenant_id, login_role, actor_hash, operator_role)
VALUES
  ('pilot-tenant', 'reward_issuer',
   'sha256:1111111111111111111111111111111111111111111111111111111111111111',
   'issuer'),
  ('pilot-tenant', 'reward_reviewer',
   'sha256:2222222222222222222222222222222222222222222222222222222222222222',
   'reviewer');
```

`operator_role` is the required column name. Use real, distinct, stable actor hashes; the values above are examples only.

Each `(tenant_id, login_role)` and each `(tenant_id, actor_hash)` is unique. Revoke membership and remove the grant through the DBA change process when an operator leaves the pilot.

The security-definer functions reject superusers and BYPASSRLS sessions. A tenant setting alone grants nothing. Provision a real tenant before the grant, and confirm the login's `session_user` is the exact mapped `login_role`.

## Connection and terms

Inject the connection string through the calling login's environment, keeping passwords out of shell history, command arguments, terms files, and output capture. The binary reads no `DATABASE_URL` fallback.

```bash
export TRACE_COMMONS_REWARDS_DATABASE_URL='postgresql://reward_issuer@127.0.0.1:5432/rewards'
```

The existing PostgreSQL adapter uses `NoTls`. The CLI therefore accepts only loopback IP addresses or Unix sockets, including when `hostaddr` overrides a host. Remote operators must connect through a protected local tunnel. The `DATABASE_SSLMODE` field does not enable TLS in this adapter.

The terms file is JSON, must be 16 KiB or smaller, and rejects unknown fields. Use only positive integer units, with `award_units <= participant_cap_units <= capacity_units`; do not use floating point. This is a structurally valid example. Replace every digest, identifier, unit amount, and closing time with the actual published inputs before creating a program.

```json
{
  "schema_version": 1,
  "activity_kind": "mission_completion",
  "definition_hash": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "rubric_hash": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "evaluator_policy_hash": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "required_evidence_hash": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "rights_hash": "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
  "challenge_policy_hash": "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
  "sponsor_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  "award_units": 100,
  "capacity_units": 1000,
  "participant_cap_units": 200,
  "closes_at": "2026-12-31T23:59:59Z",
  "reservation_ttl_seconds": 86400
}
```

## Operating sequence

Place `--tenant` before the command. Substitute actual UUIDs and lowercase `sha256:` digests; examples below are command shapes, not publishable inputs.

```bash
# Issuer: create and inspect the immutable program.
trace-commons-reward-operator --tenant pilot-tenant program-create \
  --program 00000000-0000-0000-0000-000000000101 --terms published-terms.json
trace-commons-reward-operator --tenant pilot-tenant program-show \
  --program 00000000-0000-0000-0000-000000000101

# Issuer: reserve one complete work unit, then submit retained evidence by digest.
trace-commons-reward-operator --tenant pilot-tenant reserve \
  --program 00000000-0000-0000-0000-000000000101 \
  --reservation 00000000-0000-0000-0000-000000000201 \
  --participant-hash "sha256:<64-lowercase-hex>" \
  --work-hash "sha256:<64-lowercase-hex>" --consent-hash "sha256:<64-lowercase-hex>"
trace-commons-reward-operator --tenant pilot-tenant claim-submit \
  --reservation 00000000-0000-0000-0000-000000000201 \
  --evidence-hash "sha256:<64-lowercase-hex>" \
  --evaluation-hash "sha256:<64-lowercase-hex>"

# In the reviewer login environment: assess evidence and record a decision.
trace-commons-reward-operator --tenant pilot-tenant review \
  --reservation 00000000-0000-0000-0000-000000000201 \
  --decision 00000000-0000-0000-0000-000000000301 \
  --accept true --reason completion_verified

# Either authorized role: inspect bounded history or invalidate evidence.
trace-commons-reward-operator --tenant pilot-tenant history \
  --participant-hash "sha256:<64-lowercase-hex>" --limit 50
trace-commons-reward-operator --tenant pilot-tenant invalidate \
  --evidence-hash "sha256:<64-lowercase-hex>" \
  --decision 00000000-0000-0000-0000-000000000401
```

The review command requires `--accept true` or `--accept false` explicitly.

- Accepted: `completion_verified`, `qualified_negative`, `qualified_inconclusive`.
- Rejected: `evidence_incomplete`, `rubric_not_met`, `conflict_unresolved`,
  `consent_withdrawn`.

An authorized issuer may cancel an unsubmitted reservation:

```bash
trace-commons-reward-operator --tenant pilot-tenant cancel \
  --reservation 00000000-0000-0000-0000-000000000201 \
  --decision 00000000-0000-0000-0000-000000000302
```

Reservations expire at the earlier of the terms TTL and program close. A timely submission remains held during review. Rejection and cancellation release the hold. Invalidation blocks a later submission and releases a pending submitted hold; an existing award stays recorded and counted, with invalidation visible in history. No command changes terms, reopens a terminal decision, refunds an accepted award, or redeems units.

## Retries and incidents

Repeat only the exact request with the same identifier after an interruption. Changed payloads for an existing program, reservation, or decision return `reward_payload_conflict`. Work and evidence hashes are unique across programs within the tenant, including after rejection or invalidation. Preserve the CLI's safe `reward_*` label and the retained evidence record in the incident ticket; do not attach connection strings or raw evidence.

History returns the newest `--limit` entries and full awarded totals, including zero, for the programs represented in those entries. Reads share the tenant lock with mutations so each response describes one consistent ledger state.

`truncated` marks omitted older entries; `programs_scope` is `visible_entries`. Invalidating rejected evidence preserves the rejection and adds the invalidation flag.

## Isolated verification

Use Rust 1.96.1 and a fresh local PostgreSQL database named `reward_test_...`; CI uses PostgreSQL 16 and local validation uses 17. Set `TRACE_COMMONS_REWARDS_PG_TEST_URL` through the environment to that disposable database using an administrative test login. The ignored integration tests install V69 with a minimal tenant schema and create distinct operator logins; do not pre-apply migrations. Never use production or rely on `DATABASE_URL`.

The disposable test cluster needs loopback-only trust authentication for its synthetic logins because the fixture generates no passwords.

```bash
RUSTFLAGS="-D warnings" cargo +1.96.1 test -p trace-commons-server \
  --test mission_rewards_pg -- --ignored --test-threads=1
```

The test matrix also covers:

- issuer/reviewer swaps, no grant, wrong tenant, superuser, and direct runtime writes;
- malformed terms, replay conflicts, cross-program work and evidence duplicates, expiry, cancellation, participant caps, and invalidation before and after award;
- concurrent last-slot reserve and review-versus-invalidate cases.

A migration shape check or mock does not establish these controls.
