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
  and `"growth_rule":"none"` (a flat allowance; see
  [earned growth](#earned-growth-live-tiered-v1) for `"tiered-v1"`)
- `TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS` to an explicit positive
  duration no greater than 86400

There are no default numeric allowances. The bound is a conservative unit of
server processing work. It must be calibrated against enforced request size
and work ceilings; it is not money, tokens, or NEAR credit. A fixed period has
an advisory retry delay to its next boundary. A lifetime period has no reset
time, so the API does not invent one. Growth requires either the explicit
external evaluation configuration or a `tiered-v1` policy, both described
below.

V77 stores deduplicated, typed source facts for accepted credit events and
gate evaluations. The database verifies the source row, account ownership,
and outcome before recording a fact. Fact recording alone never increases
admission: flat policies use the base allowance, and external policies consume
only a matching, fresh applied evaluation.

Grant the ingest login `trace_account_admission_runtime` after applying V77.
The role cannot mint or revoke invite codes and has no `BYPASSRLS` privilege.
Only an active server-validated invite grant removes cumulative volume caps.
The account still uses an idempotency row, processing lease, authenticated
device, request size bounds, and transient worker capacity controls. An
invited grant does not certify a witness or alter the privacy gate. Redeeming a
new valid grant atomically changes existing bounded authority to invited and
increments its trust version; repeating the same redemption consumes no extra
use or version. Revoking the grant still removes invited admission authority.

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
proof as a new authority. Historical bodies without `source_session` must be
replayed unchanged; adding the field changes their identity and is refused.
These legacy rows retain submission-level withdrawal protection only. They do
not acquire a guarantee against a resumed session using a new submission UUID.
Existing withdrawal tombstones still refuse replay and recovery. All new
account-ledger submissions require valid `source_session` metadata before any
budget reservation or content staging. A new upload with no `source_session`
that offers admission evidence is not an account-ledger submission: it takes
the V59 evidence ledger (see "Attested uploads" below). One with no
`source_session` and no evidence is refused `422 source_session_invalid`.

Before enabling, record read-only counts of legacy invite tenants, wallet
accounts, NEAR AI accounts, ambiguous links, and unlinked devices with
`scripts/operator/legacy-invite-counts.sql` ([how](./legacy-invite-counts.md)).
Those counts are **unknown** until measured on the target deployment. Verify control of
both identities before migrating a legacy link; never infer a merge from a
name or invite. The switch is global. Since V81 it governs only the
`near-`/`nearai-` namespaces: legacy `tenant-…` invite identities coexist,
linked or not, on the path they always used, and pooled tenants never link.
See [legacy invite migration](./legacy-invite-migration.md) for the link
endpoint, the pooled-tenant marker, and conflict resolution. Verify the client
holds/retries safe refusals without disarming
folders, the Z4 withdrawal/source-session guard and Z5 capacity pacing are
ready, and the R1–R7 witness and consent copy is approved. This code does not
prove production admission, scoring, or settlement.

Cutover readiness is checked at every process start. Ingest refuses to start
with `account_admission_permissions_or_linkage_not_ready` if its login lacks
an admission privilege, owns a protected table, is superuser/BYPASSRLS, can
assume a guard role, or the durable fleet inventory contains an ambiguous
identity. Since V81 that means: an active `near-`/`nearai-` device without
supported, live account linkage; an active legacy-namespace device that is not
invite-onboarded; an open legacy account whose tenant is neither pooled nor
backed by any invite-onboarded device; or an unresolved claim of a non-pooled
legacy tenant by a second account. Invite-onboarded legacy devices and their
tenants' accounts, linked or not, and pooled tenants coexist and do not
block. The
cross-tenant check is a boolean-only function owned by a NOLOGIN/NOBYPASSRLS
role with read-only RLS policies; the runtime cannot enumerate identities.
Static contributor credentials on that replica must also resolve to a live
account. A legacy namespace contribution after startup takes the legacy path, as it
did with admission off; the account status route still answers such a
session with the safe 403 label `account_identity_unlinked`. There is no
automatic migration. Closing or revoking old credentials without verified identity
migration is not a substitute for the linkage review.

Readiness covers durable accounts/devices and local static contributor tokens;
it does not attest every replica's external signed-token issuer, invite file,
or future onboarding configuration. Before activation, operators must inventory
those sources, prevent new legacy identities, and verify all replicas' settings.
The switch remains blocked until that inventory is complete; V81's
coexistence rules are the reviewed handling of existing invite identities.

Configure offered-evidence verification independently with
`TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_PROVIDER_SIGNERS`,
`TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_GATEWAY_SIGNERS`,
`TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_ACCEPTED_MODELS`, and
`TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_MIN_REQUEST_BYTES`. They use the same
signer/model/minimum-byte validation as legacy evidence policy; at least one
signer class is required. Missing or invalid policy refuses startup. If any variable in the account evidence namespace is present, only that
namespace is used and a partial configuration fails closed. If none is present,
the complete legacy `TRACE_COMMONS_ADMISSION_*` evidence policy is accepted for
rolling-deployment compatibility. Copy the reviewed evidence policy into the
independent namespace before removing the legacy evidence settings;
with that independent policy, removing `TRACE_COMMONS_ADMISSION_MIN_REQUEST_BYTES` cannot disable account-mode
evidence verification. Offered partial or invalid evidence is always refused.
Keep the legacy admission/challenge configuration on every replica throughout
the rolling deployment so clients can continue obtaining and sending evidence.
Since 2026-10-11 it is required, not only kept: with account admission on,
ingest refuses to start with `account_admission_requires_legacy_evidence_policy`
unless the V59 evidence policy (`TRACE_COMMONS_ADMISSION_*`, including its
limits) is configured, and that policy's own startup check
(`admission_runtime_permissions_not_ready`) then requires the ingest login to
keep EXECUTE on `trace_reserve_admission` and `trace_transition_admission`.

A contribution-status response describes only the responding process. Clients
must retain their R3 evidence check and send evidence until an operator attests
that every serving replica enforces account admission and the reviewed evidence
withdrawal is deployed. Seeing `bounded` or `invited` from one process is not
fleet attestation. Preserve `trace_account_admission_runtime` after cutover: it
includes the V59 transition grant needed to complete and release legacy resumes.

Invited accounts still obey the hourly submission quota, request limits, and
transient capacity controls. Only cumulative account allowance is exempted.

## Attested uploads (owner decision, 2026-10-11)

An upload that offers NEAR AI admission evidence which verifies is admitted
without debiting the bounded allowance. "Verifies" is the full first-use check
the evidence path has always made, unweakened: the witness signature under the
configured pin and redaction-policy allowlist, a provider-TEE or gateway
signer from the account evidence policy, an accepted model, the minimum
request bytes, an unexpired evidence window, and binding to this account's
anchor and this exact artifact. Everything else still applies to it: the
idempotency row, the processing lease, live account/device/principal checks,
the trust version and invite-grant consistency, the hourly submission quota,
request-size and capacity controls, `source_session` claiming and its
withdrawal guard, and submission tombstones. An upload without evidence is
debited exactly as before, and partial or invalid offered evidence is refused
`403 admission_refused` as before.

What is recorded, and what follows from it:

- The reservation row is written with `authority_kind = 'attested'` (V125
  widened that CHECK from `'account'` only) and `last_charged = FALSE`. No
  budget row is created or read for it. The kind is fixed by a submission's
  first reservation, like its account and body binding.
- Release before processing refunds nothing, because nothing was charged.
- A retry after a release or an expired lease re-reserves on the kind its row
  records, with or without headers: an attested row recovers with no debit,
  and an `'account'` row stays charged even if the retry now offers fresh
  evidence. A replay's offered evidence is checked by stored signature and
  binding only, which never establishes a new attested authority.
- The freeze rule (a budget row whose limit, bound or version differs from the
  configured policy refuses the reservation) guards the allowance, which an
  attested reservation does not touch, so it does not refuse one. It still
  refuses every unattested upload until the policy is corrected.
- Invited accounts are already exempt from the allowance; an attested upload
  from one is recorded as `'attested'`, with the same outcome.
- `GET /v1/account/contribution-status` is unchanged. `ready` describes the
  allowance an unattested upload would draw on: a fresh account reads
  `ready: true`, and an account that has spent its allowance reads
  `ready: false` with `account_limit_reached` even though an attested upload
  from it would still be admitted.

An upload with no `source_session` -- every 0.12.x client, and the witness
build of 2026-09-04, which strips the field -- cannot be claimed against a
source session, so it does not enter the account ledger. If it offers
evidence, it takes the V59 evidence ledger exactly as it does with account
admission off: the same verification against `TRACE_COMMONS_ADMISSION_*`, the
same challenge, and the same per-account and global V59 limits. That is why the
legacy evidence policy is required at startup. No account-ledger row is written
for it. With no evidence it is refused `422 source_session_invalid`, and with
partial or invalid evidence `403 admission_refused`. Note the asymmetry: an
attested upload with `source_session` is not bounded by any account allowance,
while one without it is still bounded by the V59 limits, exactly as today.

Ingest logs no line for an attested admission; `authority_kind` on the
reservation row is the audit record, and holds a label only.
Lifetime allowance compares the requested cost with the sum of spending across
all lifetime policy versions; changing the version, cost, or allowance cannot
erase that history. Each version retains its original budget row, allowing an
unprocessed reservation to refund its exact original debit. Fixed periods also
sum spending across versions sharing the same duration and bucket. A version bump
inside that bucket does not refresh the allowance. Changing period mode/duration
is a separate reviewed policy change, not an implicit lifetime reset. Lease liveness uses PostgreSQL time.

Trust facts are what the earned-trust rule reads. Admission reads facts only
through an `applied` evaluation (see [earned growth](#earned-growth-live-tiered-v1)).
`POST /v1/admin/record-account-trust-facts?limit=N&dry_run=true` (admin or
`account_trust_worker` bearer, fail-closed without a DB mirror) records
missing facts for open accounts in anchored tenants and returns label-only
counts; a re-run records nothing new. Since V126 the facts include
`attested_submission`: an accepted submission whose V76 witness certificate is
version 2 with a `provider_tee_final_call` and whose stored artifact matches
the current submitted envelope. Gateway receipts and v1 certificates never
count. The login running it needs the NOLOGIN role
`trace_account_trust_worker` (V85, widened by V126), which carries EXECUTE on
definer functions and no table privilege. See
`docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.

A candidate growth policy can also be evaluated in shadow, supplied
separately: `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON` (a base policy
with `"growth_rule": "tiered-v1"` and a `growth` object) and
`TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_VERSION`. Both absent is off. One
without the other, or a malformed policy, refuses startup. Admission never
reads a shadow row. The routes:

- `POST /v1/admin/evaluate-account-trust?limit=N&mode=shadow|applied&as_of=RFC3339`
  (admin or `account_trust_worker` bearer): stores an evaluation for each
  account with facts, and appends a hash-only `account_trust_tier_changed`
  row (outcome = the mode) to `trace_account_audit` when the tier moves.
  `shadow` is the default and runs the shadow candidate at `as_of`.
  `applied` (also accepted as `production`) runs the admission policy's own
  `tiered-v1` rule and refuses `as_of`. Run it on a schedule: decay takes
  effect only when it runs.
- `GET /v1/admin/account-trust/explain?account_ref=sha256:<hex>&mode=shadow|applied`
  (admin): returns the stored and recomputed evaluation side by side, with
  `reproduced`. The ref is
  `sha256("trace-account-trust-ref.v1\n" || tenant_id || "\n" || account_id)`.
- `POST /v1/admin/account-trust-drill` (`{"record_evidence": true, "mode": "applied"}`,
  admin): reproduces every stored evaluation in the mode (default `shadow`)
  and records `account_trust_explain` rollout-smoke evidence. It passes only if at least one evaluation was
  checked and every one reproduced. A later dedup rederive or a change to
  credit quality legitimately breaks reproduction until the next evaluation
  run. The check is not in the required rollout-smoke set while growth is
  shadow-only.

An account merge carries the absorbed account's trust facts to the survivor
(V87), each source once. Evaluations are not carried: the next evaluation run
re-evaluates the survivor over the union of facts. Each account reservation
row has nullable `earned_tier` and `trust_evaluation_digest` columns (V88).
They are NULL under `"growth_rule":"none"`; under `tiered-v1` or `external`
they record the tier and digest the reservation relied on (tier 0 and a NULL
digest when no fresh applied evaluation applied). When a
reservation is refused as `account_limit_reached` and the account's fresh
shadow evaluation would have fitted it under the candidate policy, ingest logs
the label-only line `account_trust_shadow_would_admit` with a process-local
running count. That is the number the switch-on decision turns on. It never
changes the refusal.

The runtime can insert trust rows and can update only authority, version, and
timestamp, including demotion after invite revocation. An inserted or updated
authority field alone cannot grant invited admission: reserve and
processing both require the independent live invite grant. Client copy for
`account_limit_reached` is “Sent, and this account has reached its contribution
allowance”; it is distinct from the legacy resetting-window message.

An exact account-ledger retry is recognized under the processing guard before
checking first-use evidence expiry. If evidence is offered on that retry, its
signature, authenticated account anchor, and exact artifact hash must still
match; partial or invalid signatures and foreign/altered bindings are refused.
A completed retry returns the stored receipt with no debit. Released or expired
leases re-reserve through the account ledger and its current live-identity and
budget checks, on the authority kind the row recorded (an attested row is not
debited). This recovery exception does not make expired evidence valid for
a submission UUID that has never been reserved. The shared allowance health
condition makes no promise that a lifetime budget will reset.

## Earned growth, live (tiered-v1)

Approved by Zaki on 2026-10-11 ("yes I approve live growth"). An admission
policy that names `"growth_rule":"tiered-v1"` carries its own `growth` object,
parsed by the same rule as a shadow candidate. Ingest then evaluates it
itself: the worker writes an `applied` evaluation per account, and admission
reads the newest one through V114's read. An account's allowance for the
period is `min(bounded_allowance * multiplier[tier], allowance_ceiling)`. A
missing evaluation, one older than `evaluation_max_age_seconds`, one from
another policy version, or one whose inputs changed since it was computed
(a new fact, a gate or dedup change, a principal or merge change) reads as
tier 0: the base allowance, never less. `"growth_rule":"none"` is unchanged.

Attested inference earns trust (decided the same day, reversing decision 5 of
the spec). An accepted, un-netted, first-in-cluster submission with an
`attested_submission` fact qualifies without passing the gate or the quality
threshold; it counts `attested_unit_weight` units (default 1) toward the
tier, under the same weekly cap; and a tier may require `attested_units`
(default 0). A provider-TEE attested upload the gate never assigned a dedup
cluster is not first in any cluster, so it earns nothing until the dedup pass
reaches it.

### The policy

For user testing, with a weekly period. Replace `<BASE>` (100 recommended)
and `<CEILING>` (`<BASE>` times 8: 800 for 100), and
`PILOT_GATE_POLICY_VERSION` with the gate policy version in effect (query D1
in the growth-tier draft: `SELECT gate_policy_version, count(*) FROM
trace_gate_decisions GROUP BY 1`). Every number is a first draft.

```json
{
  "version": "earned-trust-live-2026-10-11.1",
  "processing_cost_bound": 1,
  "bounded_allowance": <BASE>,
  "period": {"mode": "fixed", "seconds": 604800},
  "growth_rule": "tiered-v1",
  "growth": {
    "window_seconds": 7257600,
    "weekly_cap": 10,
    "attested_unit_weight": 2,
    "q_min_micros": null,
    "credit_quality_calibration_versions": [],
    "penalty_cooldown_seconds": 7776000,
    "evaluation_max_age_seconds": 172800,
    "allowance_ceiling": <CEILING>,
    "evaluator_versions": ["PILOT_GATE_POLICY_VERSION"],
    "dedup_signal_versions": ["events.v1+fnv1a-3shingle-set.v2"],
    "tiers": [
      {"units": 0,  "active_weeks": 0, "age_seconds": 0,       "multiplier": 1},
      {"units": 10, "active_weeks": 2, "age_seconds": 1209600, "multiplier": 2},
      {"units": 30, "active_weeks": 4, "age_seconds": 3628800, "multiplier": 4, "attested_units": 5},
      {"units": 60, "active_weeks": 8, "age_seconds": 7257600, "multiplier": 8, "attested_units": 10}
    ]
  }
}
```

Tier 1 needs no attestation, so contributors without NEAR AI inference can
still earn. `attested_unit_weight` 2 and the tier 2 and 3 `attested_units`
are proposals for review, not decided values. Setting the policy is a version
bump: `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION` must equal `version`.
Spend in the current fixed bucket carries across versions, so switching from
a lifetime period to a weekly one is a separate reviewed change and a weekly
bucket starts counting from its boundary.

### Switching growth live

1. Apply migrations through V126, and confirm V115's frontier backfill left
   no account without a frontier row (the query under the external contract
   below).
2. Grant the ingest login `trace_account_trust_worker` beside
   `trace_account_admission_runtime`. Check, as the ingest login:
   `SELECT pg_has_role(current_user,'trace_account_trust_worker','MEMBER')`.
3. Run the shadow candidate with the same growth object for at least a day and
   read `evaluate-account-trust?mode=shadow` and the drill. Fix any
   `evaluator_versions` or `dedup_signal_versions` that leave every account at
   tier 0.
4. Mint the scheduler token: an EdDSA-signed tenant token with
   `"role":"account_trust_worker"`, any non-empty `tenant_id` (the routes
   enumerate every anchored tenant), a `sub`, a `jti`, and an `exp` within
   `TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS` if that is set. It opens the
   record and evaluate routes and nothing else; every other admin and worker
   route returns 403 to it. Put it only in the ingest environment file.
   Revoke it by adding its `jti` to `TRACE_COMMONS_SIGNED_TOKEN_REVOKED_JTIS`.
   If tenant access grants are enforced, this role has no storable grant
   (V1's role CHECK predates it): use the signed-token path.
5. Set the policy and the scheduler, then restart ingest:

   ```
   TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION=earned-trust-live-2026-10-11.1
   TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON='<the policy, minified>'
   TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_ENABLED=true
   TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_TOKEN=<the token>
   TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_INTERVAL_SECONDS=3600
   ```

   At startup ingest turns external growth on (V114), checks V114's contract
   (`external_account_trust_contract_not_ready`) and V126's
   (`tiered_account_trust_contract_not_ready`: the login must hold the
   recorder's, the input reads', the generation read's and the V126 writer's
   EXECUTE, and no direct write on facts or evaluations), and authenticates
   the scheduler token. Any failure refuses startup with that label.
6. Each scheduler tick records missing facts (up to 10 passes of
   `TRACE_COMMONS_ACCOUNT_TRUST_SCHEDULER_LIMIT` sources), then evaluates
   `applied`, then `shadow` if a candidate is also set. The interval must be
   shorter than `evaluation_max_age_seconds` or startup refuses; six hours
   against a two-day maximum leaves room for several missed ticks. Ticks log
   label-only counts and register liveness as `account_trust_scheduler`.
   Between ticks, any change to an account's inputs (a new fact, a re-score or
   dedup update to a gate decision in one of its clusters, a near-duplicate
   joining one) sends it to the base allowance until the next tick. Start at
   `INTERVAL_SECONDS=3600`, watch how often `account_trust_tier_changed`
   audit rows flip, and lengthen the interval once that rate is known. A tick
   evaluates at most 10,000 accounts; beyond that, the rest stay at base.
7. Verify, right after a tick (a gate-field update between ticks legitimately
   breaks reproduction until the next one): `POST /v1/admin/account-trust-drill`
   with `{"mode":"applied","record_evidence":true}` passes;
   `/v1/account/contribution-status` for a tier 1 account reports `ready`
   past the base allowance; new reservation rows carry `earned_tier` and
   `trust_evaluation_digest`.

An operator can also run the two routes by hand with the scoped token or an
admin token:

```
H='Authorization: Bearer <account_trust_worker token>'
curl -fsS -X POST -H "$H" 'http://127.0.0.1:3907/v1/admin/record-account-trust-facts?limit=10000'
curl -fsS -X POST -H "$H" 'http://127.0.0.1:3907/v1/admin/evaluate-account-trust?limit=10000&mode=applied'
```

### Rollback

- **Stop growth:** bump the admission policy to a new version with
  `"growth_rule":"none"` and restart. Admission stops reading evaluations at
  once; spend already recorded stays, so nobody gets a fresh allowance from
  the switch. Unset the scheduler variables too, or startup refuses a
  scheduler with nothing to evaluate (unless a shadow candidate is set).
- **The scheduler dies or its token expires:** every applied evaluation goes
  stale within `evaluation_max_age_seconds` and every account falls to the
  base allowance. That is the fail-safe direction; liveness escalates the
  failing driver.
- **External growth stays on.** Enabling it turned on V114's gate-write
  invalidation, which takes dependency locks and scans gate facts on every
  gate write, and the runtime has no path to turn it off. A policy bump to
  `none` stops the reads, not the triggers. Turning the switch off is an
  owner action on `trace_account_trust_external_growth`, safe only when no
  ingest runs a `tiered-v1` or `external` policy.
- **Bad evaluations:** a `tiered-v1` policy with a new version invalidates
  every applied row of the old one immediately (admission reads only its own
  version), so a corrected policy is a version bump plus one scheduler tick.

## External evaluation contract

V114 adds an optional `"growth_rule":"external"` policy mode. Keep `"none"`
for a flat allowance. External mode additionally requires explicit
`growth_policy_version`, `allowance_ceiling` and
`evaluation_max_age_seconds`. The ceiling must be at least the base allowance;
all cost values and the maximum age must be positive and bounded. A `none`
policy must omit those external fields. This contract introduces no configured
allowance values or activation defaults.

Admission and contribution status use the same transaction-local decision:
select the newest applied evaluation of the authenticated tenant/account and
configured growth policy whose input generation still matches. Order by
`as_of DESC, recorded_at DESC, evaluation_id DESC`. Future or nonfinite
timestamps are excluded. An absent or stale evaluation yields the base
allowance, tier zero, and no digest. A usable allowance is clamped between the
base allowance and the ceiling. The stored budget limit remains the base
allowance, and previously charged period spend survives changes in evaluation.
Reservations record the actual tier and digest used. Invited authority retains
its existing bypass and records no external evaluation attribution.

The evaluator login needs only membership in `trace_account_trust_evaluator`,
with `NOSUPERUSER NOBYPASSRLS`. Do not grant membership in the worker, admission,
or guard roles. It can enumerate open accounts through
`trace_account_trust_worker_accounts(TEXT,UUID,BIGINT)` and read facts through
`trace_account_trust_evaluation_inputs(TEXT,UUID)`. Enumeration returns only
account keys; every account read and write requires a transaction-local
`trace_commons.trace_tenant_id` set to that account's tenant. It cannot record
facts or directly modify evaluations, accounts, admission rows, or budgets.

Inside a `REPEATABLE READ` (or `SERIALIZABLE`) transaction, read
`trace_account_trust_input_generation(TEXT tenant, UUID account)` and the
account's fact inputs from the same snapshot. Cluster selection and membership
exclude nonfinite decisions and decisions later than `transaction_timestamp()`.
This remains a current projection; it does not offer historical replay for an
arbitrary earlier timestamp. The generation is a `BIGINT`,
or `NULL` for missing/wrong-scope input. Obtain `as_of` from the database's
`transaction_timestamp()`. Then write through:

```sql
trace_record_external_account_trust_evaluation(
    TEXT tenant, UUID evaluation, UUID account,
    TEXT policy_version, TEXT mode, TIMESTAMPTZ as_of,
    INTEGER tier, BIGINT effective_allowance, TEXT facts_digest,
    BIGINT expected_generation
) RETURNS BOOLEAN
```

Mode is explicitly `shadow` or `applied`. The digest uses
`sha256:` followed by lowercase hexadecimal. Evaluation IDs are unique per
tenant. The returned boolean indicates a tier change within the same account,
policy and mode; unchanged tiers still create an evaluation. The writer refuses
closed accounts, future/nonfinite timestamps, wrong tenant scope, and changed
input generations. It also enforces the snapshot rule above: a call from a
`READ COMMITTED` transaction is refused with
`account_trust_evaluation_snapshot_required`, and any call while external
growth is switched off (below) with `account_trust_external_growth_disabled`. Roll back the entire batch on a database failure or changed
inputs; PostgreSQL serialization failures require a new snapshot. Do not reuse
an evaluation computed from an earlier generation when retrying.

The forced-RLS frontier table advances transactionally on fact mutations,
account/principal mutations, current gate projections and their shared cluster
dependencies, and completed merge hooks. This includes merges
with no newly copied facts. Admission holds a scoped definer frontier lock
until reservation commit; the login gains no frontier write privilege. The
writer locks account before frontier, matching admission's order. Legacy
feature fields are nullable for compact rows; the original shadow writer and
feature decoder continue to handle only complete legacy evaluations.

The gate-write invalidation is switched off until external growth is enabled,
so a `growth_rule:none` deployment pays nothing for it: a gate write takes no
dependency lock and scans no facts, the writer refuses, and admission and status
read no evaluation. An ingest started with an external policy turns it on
(`trace_account_trust_enable_external_growth()`, granted to
`trace_account_admission_runtime`) before its readiness check. The first enable
advances every frontier, so nothing recorded before it is consumed; later calls
change nothing. The runtime has no path back. Turning it off again is an owner
action, safe only once no ingest runs an external policy, and the next enable
invalidates every evaluation again.

Every account needs a frontier row before external growth is enabled: the
writer refuses an account without one, so that account can never record an
evaluation. The trigger creates a row only when an account is inserted, so
accounts that existed before V114 depend on V114's backfill. That backfill read
`trace_accounts` with no tenant context, and a migrator that is neither
superuser nor `BYPASSRLS` (the pilot's `app`, for one) seeded no row. V115
re-runs it as the evaluation guard and leaves existing rows alone. Do not
enable external growth on a database below V115. To confirm, as a role that
sees every tenant's rows:

```sql
SELECT count(*) FROM trace_accounts a
 WHERE NOT EXISTS (SELECT 1 FROM trace_account_trust_frontiers f
                    WHERE f.tenant_id = a.tenant_id AND f.account_id = a.account_id);
```

The count must be 0. Run as `app` with no tenant set, it is always 0, because
`app` sees no account; that result proves nothing.

Contribution status reads the generation through the non-locking
`trace_account_trust_input_generation`; only a reservation takes the frontier
lock.

External startup additionally checks the role, writer/read contract, grants and
forced RLS. Missing controls refuse startup with
`external_account_trust_contract_not_ready`; readiness errors use
`external_account_trust_readiness_unavailable`. These controls prove the
consumption boundary, not that all upstream outcomes have been recorded as
facts. Operators must separately qualify recorder coverage before activating
applied evaluations. This migration does not activate production admission.


Gate changes invalidate accounts whose recorded gate inputs name the decision or
submission, plus all recorded gate-input accounts in affected old/new clusters
across tenants. Internal dependency lock rows serialize gate and fact mutations
before dependency enumeration; a conflicting repeatable-read writer must retry
from a new snapshot. Only a NOLOGIN input guard reads dependency keys and
advances generations; it returns no cross-tenant inputs to either login. This
internal lock table uses guard-only forced RLS and is tracked separately from
ordinary tenant-readable tables. Dependency and account frontier locks are
acquired in deterministic key order within each trigger; the trigger acquires
no account row locks afterward. Multi-row transactions must still roll back and
retry on PostgreSQL serialization or deadlock failures.

The enumeration uses partial indexes on the gate facts by
`(tenant_id, submission_id)` and `(tenant_id, source_id)`, and an index on
`trace_gate_decisions(dedup_cluster_id)`.

Dependency lock rows carry `touched_at`. The retention worker
(`POST /v1/workers/retention-maintenance`, when account admission is
configured) calls `trace_account_trust_prune_dependency_locks(limit, dry_run)`,
which removes at most 1000 rows per run that nothing has touched for seven days
(a dry run counts them). A row only serializes writes that overlap in time, so
one idle that long guards no transaction still running. Deleting an account
deletes its frontier row; principal and fact changes only advance an existing
one.
