# Client daily upload allowance (retired)

The contributor clients no longer have a daily upload limit, so the server
no longer advertises one. Server admission is unchanged: the account's
weekly processing-cost budget, its earned-trust growth (see
[account trust](./account-trust.md)) and the per-principal submission rate
still apply. An account that spends its weekly budget sees the existing
`account_limit_reached` refusal.

## Endpoint

`GET /v1/account/upload-allowance` stays, with normal native bearer or
account-cookie authentication and session rotation. Unbound and closed
accounts are refused. Every other account gets, with
`Cache-Control: no-store`:

```json
{"allowance":null}
```

The field is always present and always `null`. Older clients treat a
missing field as malformed and an explicit `null` as "no server
allowance", so this answer leaves them on their own local defaults. It is
kept until no supported client reads it.

## Retired settings

`TRACE_COMMONS_CLIENT_MAX_UPLOADS_PER_DAY` and
`TRACE_COMMONS_CLIENT_MAX_BYTES_PER_DAY` are no longer read. A deployment
that still sets them starts normally and ignores them; remove them at the
next configuration change. See
[the environment reference](./env-reference.md#client-daily-upload-allowances-retired).

## Staging earned upload trust (opt-in)

The independent policy pair
(`TRACE_COMMONS_CLIENT_UPLOAD_TRUST_POLICY_JSON`/`_VERSION`) is still
accepted and still validated at startup, so a staging render that sets it
keeps starting, and its scoped scheduler still records trust facts and
writes applied evaluations. Nothing reads those evaluations any more: they
used to scale the advertised client caps, which are gone. The procedure
below is kept so an existing staging deployment can be operated and rolled
back; do not enable the applied mode anew. Its removal is a follow-up.

There is no account-age or active-week waiting period. Each eligible
submission counts once, including existing eligible history. A trace must
be accepted, novel, first in its dedup cluster, and pass an allowlisted gate:
`require_gate_pass=true` applies to attested traces too. Duplicates,
reuploads, withdrawn/revoked/quarantined outcomes do not add eligible units.
Attestation has weight one and gives no bonus. The weekly cap is 10000;
the qualification window is 84 days and evaluation freshness is two days.
The scheduler records facts and evaluates every five minutes.

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

Check scheduler liveness and fresh applied evaluations, and confirm no
change to admission/signup. Retain hash-only qualification evidence; local
tests and a successful render do not establish live behavior.

Rotate the worker token before `exp`: add a new Secret Manager version,
rerender with the same mode/labels, restart, and verify scheduler liveness
and fresh evaluations. Rotate the dedicated database password through its
Secret Manager version and database role using the reviewed credential
rotation procedure, then rerender/restart and recheck readiness; never
replace the main app login or broaden its grants to bypass this check.
Token validation is fail-closed at startup: an expired or revoked scheduler
token prevents **all ingest startup**, including signup and uploads. While
ingest stays running, expiry stops successful scheduler work and
evaluations become stale.
For routine rotation, publish the new token version, rerender/restart, and
verify scheduler work **before revoking the old jti**. Never revoke the only
configured token and then restart. For a compromised token, revoke it
immediately and install a replacement before the next restart; if no valid
replacement is available, disable the optional upload-trust scheduler/policy
and restart to restore the existing staging upload/signup path. The scoped
[renewal timer](../../deploy/staging-gcp/upload-trust-rotation.md) rotates before
expiry and checks the restarted process; it does not relax authentication.

To roll back this independent staging feature, unset
`TC_CLIENT_UPLOAD_TRUST_MODE` and both `TC_UPLOAD_TRUST_*_VERSION` renderer
labels; rerender and restart ingest. Keep the external invalidation
controls enabled while any other instance still depends on them.
