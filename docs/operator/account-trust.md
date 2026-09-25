# Account contribution admission (prepared; disabled by default)

The authenticated account path is controlled by
`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`. An absent value, `false`, or `0`
keeps the existing evidence admission behavior. This migration does not set
the flag, arm folders, change consent, or activate a service.

Activation requires a reviewed, explicit policy and a durable PostgreSQL
mirror with enforced tenant RLS. Set:

- `TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED=true`
- `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION` to the reviewed version
- `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON` to a JSON policy with the same
  version, positive `processing_cost_bound` and `bounded_allowance`, explicit
  `period` (`{"mode":"lifetime"}` or `{"mode":"fixed","seconds":...}`),
  and `"growth_rule":"none"`
- `TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS` to an explicit positive
  duration no greater than 86400

There are no default numeric allowances. The bound is a conservative unit of
server processing work. It must be calibrated against enforced request size
and work ceilings; it is not money, tokens, or NEAR credit. A fixed period has
an advisory retry delay to its next boundary. A lifetime period has no reset
time, so the API does not invent one. No quality-based growth is enabled.

V77 stores deduplicated, typed source facts for accepted credit events and
gate evaluations. The database verifies the source row, account ownership,
and outcome before recording a fact. No current acceptance/evaluator worker
calls this seam, and admission never reads it. It is historical storage for a
later reviewed growth policy, not earned allowance in this release.

Grant the ingest login `trace_account_admission_runtime` after applying V77.
The role cannot mint or revoke invite codes and has no `BYPASSRLS` privilege.
Only an active server-validated invite grant removes cumulative volume caps.
The account still uses an idempotency row, processing lease, authenticated
device, request size bounds, and transient worker capacity controls. An
invited grant does not certify a witness or alter the privacy gate.

`GET /v1/account/contribution-status` is an authenticated, `no-store` advisory
read. Its safe fields are `authority`, `policy_version`, `ready`, optional
`refusal_label`, and optional `retry_after_seconds`. A request still reserves
atomically at submit time; a status read does not guarantee a slot. Account
allowance exhaustion returns HTTP 429 with `account_limit_reached`; a live
device or account revocation returns 403, and in-progress and body-conflict
retries remain distinct 409 responses.

Submission UUIDs already present in the V59 evidence ledger stay on that
ledger after cutover. Exact completed retries read the old receipt without a
new debit. A released or expired lease for the same authenticated account,
anchor, and exact body resumes under V59's stored receipt/challenge, limits,
and cost bound; a live lease returns 409. An expired processing attempt keeps
its prior charge when it reserves another attempt. No account-ledger row is
created for that UUID. A recovery may omit expired first-use evidence; if it
offers admission evidence, its signature and stored binding must match.
Partial or altered offered evidence is refused. Recovery never treats a new
proof as a new authority.

Before enabling, record read-only counts of legacy invite tenants, wallet
accounts, NEAR AI accounts, ambiguous links, and unlinked devices. Those counts
are **unknown** until measured on the target deployment. Verify control of
both identities before migrating a legacy link; never infer a merge from a
name or invite. The switch is global and has no supported per-tenant fallback.
Enable it only after every affected legacy identity has verified linkage or a
separate reviewed coexistence design is deployed. Verify the client
holds/retries safe refusals without disarming
folders, the Z4 withdrawal/source-session guard and Z5 capacity pacing are
ready, and the R1–R7 witness and consent copy is approved. This code does not
prove production admission, scoring, or settlement.
