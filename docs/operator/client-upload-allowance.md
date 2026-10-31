# Client daily upload allowance

The server advertises daily upload-count and byte limits for the authenticated account. Validated earned trust can raise both daily caps automatically through the existing admission policy or an independent client-upload policy. These client volume limits do not approve sessions or change consent or pause. Server admission still applies its separate processing-cost units and accounting period; a daily cap does not establish that an account is ready to submit or has admission capacity remaining.

## Base caps and earned increases

The optional ingest settings define the base daily caps before an earned increase:

```sh
TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY=200
TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY=2147483648
```

Set both variables and restart ingest. Both must be positive integers. The count must not exceed 1,000, and bytes must not exceed 5,368,709,120 (5 GiB). Partial or invalid configuration refuses startup with a fixed configuration label.

For a bounded account, the server reads the validated effective allowance under the configured admission policy or the independent client-upload policy described below. The independent policy performs an advisory trust read; it does not activate admission or reserve/debit processing budget. When `effective_allowance` exceeds the policy's `bounded_allowance`, it scales each base cap by their ratio:

```text
daily cap = min(hard ceiling, floor(base cap × effective_allowance / bounded_allowance))
```

Count and bytes scale independently, with integer division rounding down, and retain the hard ceilings of 1,000 uploads and 5 GiB. If both base settings are absent, earned growth uses the shared defaults of 50 uploads and 200 MiB (209,715,200 bytes) as its base. For example, a validated ratio of 2 advertises 100 uploads and 400 MiB without any operator base settings. With no configured base and no validated growth above 1, the response is `{"allowance":null}`.

Both existing external-trust and tiered-trust admission modes supply the account's applied effective allowance. Their existing validation, freshness, and generation safeguards remain authoritative; this endpoint does not introduce scoring, promotion thresholds, or an independent trust decision. An account whose trust falls back to its ordinary admission allowance loses the earned uplift on a successful refresh: the response contains the configured base, or null when no base was configured. A transient status-read failure returns an error rather than pretending that trust fell back.

Legacy and invited identities that use their independent compatibility admission path keep the configured base unchanged. They do not gain an earned multiplier from this endpoint; with no configured base, they receive null.

## Endpoint and client behavior

`GET /v1/account/upload-allowance` uses normal native bearer or account-cookie authentication and session rotation. Closed accounts are refused. Successful responses carry `Cache-Control: no-store` and either:

```json
{"allowance":{"schema_version":1,"max_uploads_per_day":200,"max_bytes_per_day":2147483648}}
```

or `{"allowance":null}` when neither a configured base nor an earned increase applies. No tenant/account identifiers, trust scores, or credentials appear in the response.

Supporting clients refresh at startup and every five minutes even when their local daily cap is spent. A server allowance can raise default limits; explicitly chosen local limits remain in force separately for count and bytes. Today's usage counters are preserved, so a higher cap can immediately release an already-approved queue. Defaults remain 50 uploads and 200 MiB when no allowance applies. Existing clients keep their prior behavior.

Clients retain the last valid allowance for the current signed-in identity after a transient refresh failure, until restart. A successful null response removes the previous uplift. Signing out or switching account, tenant, or server discards the old allowance. Allowances are runtime state, separate from saved local settings.

On upgrading legacy local settings, nondefault count/byte values are preserved as explicit choices. A legacy value equal to the default cannot be distinguished from an untouched setting and follows the server; future explicit writes retain that choice even when it equals the default.

Unsetting both operator variables and restarting removes the configured base, but accounts with validated earned growth still receive caps scaled from the shared defaults. Earned uplift follows the existing admission trust controls and falls back when admission no longer applies that growth. Supporting clients adopt the resulting base or null response at their next successful refresh; uploads already sent and daily usage counters are unchanged.


## Staging earned upload trust (opt-in)

Staging uses the independent policy so provisioned users keep their current
signup and upload path with both admission ledgers disabled. The checked-in
[policy template](../../deploy/staging-gcp/client-upload-trust-policy.json.template)
uses this progression when the optional base-cap settings above are unset:

| Qualified distinct traces in the last 84 days | Uploads/day | MiB/day |
| --- | --- | --- |
| 0–4 | 50 | 200 |
| 5–19 | 100 | 400 |
| 20–49 | 150 | 600 |
| 50 or more | 250 | 1000 |

There is no account-age or active-week waiting period. Each eligible
submission counts once, including existing eligible history. A trace must
be accepted, novel, first in its dedup cluster, and pass an allowlisted gate:
`require_gate_pass=true` applies to attested traces too. Duplicates,
reuploads, withdrawn/revoked/quarantined outcomes do not add eligible units.
Attestation has weight one and gives no bonus. The weekly cap is 10000;
the qualification window is 84 days and evaluation freshness is two days.
The scheduler records facts and evaluates every five minutes; clients
independently refresh their advertised allowance every five minutes.

Before enabling either mode:

1. Deploy the schema through V138 and all subsequent migrations required by
   the server revision. V137 removes the staging recorder's accepted-credit
   prerequisite while preserving its gate/novelty/outcome checks. Do not
   fabricate historical credits to make trust facts eligible. Confirm forced
   RLS and durable database-mirror writes.
   Provision a dedicated `tc_upload_trust_login` as LOGIN, NOSUPERUSER,
   NOBYPASSRLS, NOCREATEROLE, with no trust-table ownership. Apply the reviewed
   narrow role/grant SQL prepared with the readiness evidence: only the
   dedicated `trace_client_upload_trust_runtime` group, which inherits the
   narrow `trace_account_trust_worker` seams and exact status-read grants.
   Never grant `trace_account_admission_runtime`; it includes admission and
   trust writes unrelated to this feature. Do not grant
   membership in `app`, guard or evaluator groups, broad table privileges,
   or direct writes on trust facts/evaluations. Use the
   [earned trust readiness checks](account-trust.md#switching-growth-live)
   under this dedicated login, not the main app owner login.
   V138 replaces the per-row frontier discovery with indexed probes; retain
   V137 unchanged after deployment.
   Store its password in `tc-staging-db-upload-trust-password` (16 or more
   ASCII letters/digits). Enabled shadow and applied renders read it into
   `TRACE_COMMONS_CLIENT_UPLOAD_TRUST_DATABASE_URL` as the local
   `tc_upload_trust_login` connection. This dedicated pool serves trust
   workers and reads; the main `DATABASE_URL` remains unchanged. Startup
   verifies that both pools name the same endpoint and database, then
   checks the dedicated login's non-owner privileges and trust contracts.
2. Check frontier backfill coverage as a role that sees all tenants, not
   a tenant-scoped app connection. The independent applied policy requires
   external-growth invalidation/read readiness and tiered recorder/writer
   readiness. Startup fails closed if a control is missing. Enabling the
   invalidation system can invalidate old evaluations: record and evaluate
   fresh facts after it is enabled.
3. Read the actual staging gate and dedup decision versions. Supply those
   exact labels; do not copy pilot allowlists. Labels must be 1–64 ASCII
   letters/digits or `._+-`, without whitespace. Confirm recorder coverage
   and that qualified traces enter the expected tiers in shadow before
   applying the policy.
4. Mint an EdDSA-signed token accepted by staging's existing signed-token
   verifier with role `account_trust_worker`, nonempty tenant, `sub`, `jti`
   and a bounded `exp` within the verifier's configured maximum TTL. The
   scoped recorder/evaluator enumerates accounts through its existing
   server contract; an admin token is unnecessary. Store the token only in
   Secret Manager as `tc-staging-account-trust-worker-token` and grant the
   staging runtime account access. The renderer checks one-line JWT shape;
   server startup verifies authentication, role and expiry. With tenant-access
   enforcement on, provision a legitimate active `account_trust_worker` service
   grant bound to the token tenant, subject, issuer and audience before the
   first scheduler tick. Keep this service grant valid independently of the
   rotating token lifetime; do not reuse contributor/admin grants or disable
   enforcement. Never put the
   token in shell arguments, output, source, or a rollout report.

Render a shadow candidate first on the staging VM, using the existing
privileged rendering workflow and live decision labels:

```sh
TC_CLIENT_UPLOAD_TRUST_MODE=shadow
TC_UPLOAD_TRUST_GATE_POLICY_VERSION=<live-staging-gate-label>
TC_UPLOAD_TRUST_DEDUP_SIGNAL_VERSION=<live-staging-dedup-label>
```

Pass these settings with the other established renderer options. Mode is
unset by default; supplying labels alone refuses rather than enabling
anything. Each enabled mode reads both feature secrets and selects the dedicated
trust database pool. Disabled rendering reads neither secret and writes
no trust database URL. Shadow renders `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON`
and `_VERSION=staging-earned-upload-v1`, and the existing scoped scheduler
with interval 300 and limit 10000. Shadow evaluations do not raise caps.
Keep the candidate in shadow for at least a day, inspect evaluation/drill
results and recorder coverage, and qualify the 0/5/20/50 boundaries and
netted-out/duplicate cases before switching.

For the approved applied rollout, rerender with
`TC_CLIENT_UPLOAD_TRUST_MODE=applied` and the same qualified labels, then
restart ingest. It renders
`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON` and
`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_VERSION=staging-earned-upload-v1`.
These two settings must be present together, use `tiered-v1`, and cannot
coexist with account-admission configuration. The renderer keeps
`TRACE_COMMONS_ADMISSION_ENABLED=false` and
`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED=false`; it adds no witness
settings. Enabling this feature does not require a witness or change
signup, consent, review, credit, or payout.

Check scheduler liveness and fresh applied evaluations, then request
`GET /v1/account/upload-allowance` with each test account's own session.
Confirm the table above, no change to admission/signup, and that a capped
client resumes after refresh with its original daily counters. Retain
hash-only qualification evidence; local tests and a successful render do
not establish live behavior.

Rotate the worker token before `exp`: add a new Secret Manager version,
rerender with the same mode/labels, restart, and verify scheduler liveness
and fresh evaluations. Rotate the dedicated database password through its
Secret Manager version and database role using the reviewed credential
rotation procedure, then rerender/restart and recheck readiness; never
replace the main app login or broaden its grants to bypass this check.
Token validation is fail-closed at startup: an expired or revoked scheduler
token prevents **all ingest startup**, including signup and uploads. While
ingest stays running, expiry stops successful scheduler work; evaluations
then become stale and clients lose the earned uplift on a successful refresh.
For routine rotation, publish the new token version, rerender/restart, and
verify scheduler work **before revoking the old jti**. Never revoke the only
configured token and then restart. For a compromised token, revoke it
immediately and install a replacement before the next restart; if no valid
replacement is available, disable the optional upload-trust scheduler/policy
and restart to restore the existing staging upload/signup path. The scoped
[renewal timer](../../deploy/staging-gcp/upload-trust-rotation.md) rotates before
expiry and checks the restarted process; it does not relax authentication.

To roll back this independent staging feature, unset
`TC_CLIENT_UPLOAD_TRUST_MODE`, both `TC_UPLOAD_TRUST_*_VERSION` renderer
labels, and any optional base-cap overrides; rerender and restart ingest.
With admission still off and no independent policy, the endpoint must
return `{"allowance":null}`. Verify that response with an account session
and verify a supporting client successfully refreshes it before claiming
its default limits are restored. Transient failures retain the last valid
allowance. Already-sent uploads, counters, and explicit local limits remain
unchanged. Keep the external invalidation controls enabled while any
other instance still depends on them.
