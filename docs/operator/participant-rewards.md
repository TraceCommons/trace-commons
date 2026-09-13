# Participant reward offers

R1 lets a signed-in account inspect an offer, reserve its fixed allocation, and read its own reward history. Product copy calls the denomination **Cloud credits**. Reserved capacity, recorded awards and a provider-confirmed available balance remain separate states. This implementation exposes the first two; the existing Cloud balance connector remains authoritative for available credits.

The implementation extends the [operator reward ledger](mission-insight-rewards.md). Existing programs retain `operator_asserted` provenance. New offers use `account_bound` provenance and require a readable manifest. The migration does not attach historical pilot hashes to accounts.

## Deployment and privileges

Apply the normal migration chain through `V71__reward_participant_access.sql` before starting this release. The reward operator CLI does not run migrations. This migration is one atomic security boundary: schema, forced RLS, function ownership and runtime grants become visible together.

Provision the ingest server's existing account runtime login through the DBA process:

```sql
GRANT trace_reward_participant_runtime TO account_runtime;
INSERT INTO trace_reward_participant_logins (tenant_id, login_role)
VALUES ('your-tenant', 'account_runtime');
```

Replace both example identifiers with the deployment's existing tenant and direct login. Keep the existing account table privileges and separate login-resolver pool. The participant group adds function execution only. It grants no issuer or reviewer authority, table writes, role administration, or RLS bypass. The direct database session must have both participant membership and an explicit tenant mapping; superuser and BYPASSRLS sessions are refused for participant operations.

The account runtime supplies the tenant and account derived by the existing authentication middleware. Browser cookies and native `tcn1` account sessions can reserve their own capacity. Device upload claims cannot authorize these account routes. A native reservation acknowledges the specific offer version without changing authenticators, payout preferences or evidence permissions.

After participation begins, keep the participant grant on the account runtime that executes account merges. A merge involving reward identities fails closed when its grant is missing. The reward hook shares the account merge transaction: it requires the proposal consumed by that transaction, preserves every immutable participant hash, and moves ownership through aliases. A failed hook rolls back proposal consumption and the account merge. Accounts without reward mappings retain the existing merge behavior.

## Publish a readable offer

Use an issuer login provisioned by the operator runbook. Supply its protected local PostgreSQL URL through `TRACE_COMMONS_REWARDS_DATABASE_URL`; the existing CLI transport restriction requires loopback or a Unix socket.

`offer-publish` takes a terms file and a manifest file:

```sh
trace-commons-reward-operator --tenant your-tenant offer-publish \
  --program <program-uuid> --terms terms.json --manifest manifest.json
trace-commons-reward-operator --tenant your-tenant offer-suspend \
  --program <program-uuid> --suspended true
```

The manifest has exactly these fields: `schema_version: 1`, `definition`, `rubric`, `required_evidence`, `rights`, `challenge_policy`, `evaluator_policy`, and `reservation_terms`. Each text field must be nonempty and at most 4,096 UTF-8 bytes; the file is limited to 32 KiB. The existing terms file limit is 16 KiB. Unknown fields are rejected. The corresponding six terms hashes must equal SHA-256 of the manifest field's exact UTF-8 text. The sponsor hash remains an issuer-supplied identifier.

Publish both the eligibility rules and the fact that this version permits one reservation per account identity for the definition. Definition text supplies the current work namespace across offers. Expired or terminal reservations keep their duplicate marker; operators cannot promise renewal, repeat attempts or an appeal award in R1. Repeatable task families require the later source/work contract.

The server binds the complete manifest and immutable terms into `offer_version_hash`. Repeating publication with identical inputs returns the same offer. Changed terms or text require a new program; modifying a definition solely to evade duplicate protection violates the published work policy. Suspension is the only mutable offer control and affects new reservations. Exact retries and existing history remain readable while suspended.

Both commands return the public offer projection. There is no list or search endpoint in this slice; callers open a known published program UUID. Public capacity is advisory until reservation succeeds.

## HTTP contract

| Method and path | Authentication | Result |
| --- | --- | --- |
| `GET /v1/reward-offers/{program_id}` | Public | Readable manifest, version, units, capacity, deadline and suspension |
| `POST /v1/account/reward-offers/{program_id}/reservations` | Account | Reserve or replay this account's request |
| `GET /v1/account/reward-reservations/{reservation_id}` | Account | Own reservation, state and decisions |
| `GET /v1/account/rewards?limit=20&before={cursor}` | Account | Own history and awarded total |

Reservation JSON contains only `reservation_id` and `offer_version_hash`. Generate a non-nil UUID once and retain it for retries. Account IDs, participant hashes, role fields and evidence are rejected. Cross-site browser writes are refused. The body limit is 1 KiB. All successful responses and handled refusals carry `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.

History accepts limits from 1 through 100 and an optional owned UUID cursor, returning `entries`, `truncated`, `next_cursor` and the full account's `awarded_units` total. Foreign or unknown cursors return `reward_not_found`, including for an account with no history. Empty reads create no enrollment. Each page reflects a consistent transaction; later pages may observe later decisions.

Unit amounts are integer decimal strings in HTTP JSON, including totals, to preserve values beyond JavaScript's exact integer range. Clients should use decimal/integer arithmetic and format them with the Cloud credits label; timestamps use the typed API's UTC date-time representation.

Participant responses omit internal account, actor, work, consent, evidence and evaluation hashes.

States are `reserved`, `expired`, `submitted`, `awarded`, `awarded_invalidated`, `rejected`, `cancelled`, and `invalidated`.

Invalidation preserves an existing award and its capacity consumption. An expired unsubmitted reservation releases capacity but does not renew on replay. The existing operator workflow controls submission and review until the evidence-admission slice adds participant submission.

Errors use existing safe `reward_*` labels: invalid input is 400, missing account authentication is 401, foreign/unknown objects are 404, unavailable storage is 503, and state/capacity/version conflicts are 409. Cross-site writes return 403 and exhausted request limits return 429. Render actionable copy from the label; keep server/internal identifiers out of user-facing error prose.

Each ingest process admits at most two concurrent reward database queries across public and account routes, with at most one account reward operation per tenant. Contending requests return 429 before waiting for a database connection.

This leaves capacity in the default five-connection pool for other account operations; smaller custom pools offer less isolation. These limits are per process. PostgreSQL locks continue to enforce ledger consistency across processes and operator calls.

Capacity and awarded totals are calculated from the ledger on each request. Their cost grows with program participation and account history, including merged aliases. Profile these queries against the intended deployment's data volume before raising request limits or expanding the pilot; the functional tests do not establish a production load envelope.

## Verification

Use separate disposable loopback databases named `reward_test_*`. The selected integration tests fail when their required URL is missing; CI explicitly runs the ignored tests and checks that reservation rows were created.

```sh
cargo test -p trace-commons-server --test reward_participant_pg -- --ignored --test-threads=1
cargo test -p trace-commons-server --bin trace-commons-ingest \
  reward_participant_http_contract -- --ignored --test-threads=1
```

Set `TRACE_COMMONS_REWARDS_PG_TEST_URL` separately for each invocation. Tests provision distinct administrative, issuer, reviewer, participant and login-resolver roles. They exercise the complete migration chain, real TCP HTTP, accepted and rejected credentials, immutable publication, capacity races, exact replay, cross-account isolation, merged history, review and invalidation states, and expiry.

The next implementation boundary is source-specific attempt and evidence admission. It must reuse Private Chat and Insights storage, distinguish private evidence from corpus contribution, and bind accepted evidence to the reserved version before enabling participant submission. Reward delivery requires its own funded, idempotent provider receipt workflow.
