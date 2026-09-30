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
budget reservation or content staging.

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

A contribution-status response describes only the responding process. Clients
must retain their R3 evidence check and send evidence until an operator attests
that every serving replica enforces account admission and the reviewed evidence
withdrawal is deployed. Seeing `bounded` or `invited` from one process is not
fleet attestation. Preserve `trace_account_admission_runtime` after cutover: it
includes the V59 transition grant needed to complete and release legacy resumes.

Invited accounts still obey the hourly submission quota, request limits, and
transient capacity controls. Only cumulative account allowance is exempted.
Lifetime allowance compares the requested cost with the sum of spending across
all lifetime policy versions; changing the version, cost, or allowance cannot
erase that history. Each version retains its original budget row, allowing an
unprocessed reservation to refund its exact original debit. Fixed periods also
sum spending across versions sharing the same duration and bucket. A version bump
inside that bucket does not refresh the allowance. Changing period mode/duration
is a separate reviewed policy change, not an implicit lifetime reset. Lease liveness uses PostgreSQL time.

Trust facts are recorded for the earned-trust shadow and are still read by no
admission path. `POST /v1/admin/record-account-trust-facts?limit=N&dry_run=true`
(admin bearer, fail-closed without a DB mirror) records missing facts for open
accounts in anchored tenants and returns label-only counts; a re-run records
nothing new. The login running it needs the NOLOGIN role
`trace_account_trust_worker` (V85), which carries EXECUTE on three definer
functions and no table privilege; do not grant it to
`trace_account_admission_runtime`. See
`docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.

Earned-trust evaluation runs in shadow only. Admission never reads it, the
production policy keeps `"growth_rule": "none"`, and V86's write function
refuses every mode but `shadow`. A candidate growth policy is supplied
separately: `TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON` (a base policy
with `"growth_rule": "tiered-v1"` and a `growth` object) and
`TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_VERSION`. Both absent is off. One
without the other, or a malformed policy, refuses startup. With it set, each
of these takes the admin bearer:

- `POST /v1/admin/evaluate-account-trust?limit=N&as_of=RFC3339`: stores a
  `shadow` evaluation for each account with facts, and appends a hash-only
  `account_trust_tier_changed` row (outcome `shadow`) to `trace_account_audit`
  when the tier moves. Run it on a schedule: decay takes effect only when it
  runs.
- `GET /v1/admin/account-trust/explain?account_ref=sha256:<hex>`: returns the
  stored and recomputed evaluation side by side, with `reproduced`. The ref is
  `sha256("trace-account-trust-ref.v1\n" || tenant_id || "\n" || account_id)`.
- `POST /v1/admin/account-trust-drill` (`{"record_evidence": true}`): reproduces
  every stored shadow evaluation and records `account_trust_explain`
  rollout-smoke evidence. It passes only if at least one evaluation was
  checked and every one reproduced. A later dedup rederive or a change to
  credit quality legitimately breaks reproduction until the next evaluation
  run. The check is not in the required rollout-smoke set while growth is
  shadow-only.

An account merge carries the absorbed account's trust facts to the survivor
(V87), each source once. Evaluations are not carried: the next evaluation run
re-evaluates the survivor over the union of facts. Each account reservation
row has nullable `earned_tier` and `trust_evaluation_digest` columns (V88).
They are NULL on every reservation while growth is shadow-only. When a
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
budget checks. This recovery exception does not make expired evidence valid for
a submission UUID that has never been reserved. The shared allowance health
condition makes no promise that a lifetime budget will reset.
