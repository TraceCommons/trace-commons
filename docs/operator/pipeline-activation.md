# Pipeline activation and migration

> **Status: design stage.** This runbook describes tooling that is not in this
> repository yet: the activation routes and the `versioned_pipeline_pg` suite.
> It arrives with the pipeline runtime and qualification changes, and its
> commands can change before then. Do not follow it on a deployment.

Pipeline activation routes the qualified pipeline for new submissions. Retained
legacy records stay readable. The switch assigns each receipt to one
implementation.

The pipeline now has these properties:

- A receipt ownership record selects the legacy executor or the pipeline.
- A retry of a pre-switch receipt returns the original operation.
- It does not start a new pipeline run.
- Unique ledger source keys prevent both paths from awarding the same receipt.
- Tenant routing is an explicit record. Timestamps do not select the bundle.
- Activation and expansion require current drills, readiness, corpus
  evidence, credit reconciliation, index checks, and invalidation checks.
- Rollback selects an earlier qualified bundle for later runs only.
- An unsafe bound policy must be suspended. The run keeps its package.
- First-rollout containment stops new pipeline receipts. Workers and
  packages remain available.
- Legacy writers disable only after their pending owned work completes.
- Destructive schema cleanup is not part of this phase.

## Pipeline receipt routing before activation

`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` lists the tenants whose new
receipts go to the pipeline. It has an effect only when the ingest build
injects a pipeline runtime. The repository binary injects none.

- Unset or empty (the default): every receipt takes the legacy path.
- Tenants listed with a runtime: those tenants' receipts go to the pipeline.
- Tenants listed without a runtime: ingest refuses to start with
  `pipeline_receipts_configured_without_runtime`.

`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` lists tenants whose pipeline work
the worker still processes while none of their receipts is routed to the
pipeline: runs in flight, withdrawal follow-ups, index invalidations, NEAR
payouts and confirmations, and the staged-receipt sweep. The worker drains
the union of the two lists. The drain list also needs a runtime: without
one, ingest refuses to start with
`pipeline_drain_tenants_configured_without_runtime`. The worker drains a
drain tenant's runs, credit and payouts through the runtime's own
dependencies, so a runtime that drains any tenant must be
production-qualified, as one that routes a tenant must: otherwise ingest
refuses to start with `pipeline_runtime_dependencies_not_production_qualified`
(unless `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is set).
`GET /v1/pipeline/readiness` reports the drain list's size as
`drain_tenant_count` (a count, no tenant ids). A retried upload from a drain
tenant that completed admission on the pipeline path replays its pipeline
receipt, as it did while the tenant was routed; a new upload takes the
legacy path.

To roll a tenant back from the pipeline, move it from
`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` to
`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` and restart ingest. Its new
receipts take the legacy path at once, and its existing pipeline work is
finished. Keep it on the drain list until the operational summary shows no
pending runs, invalidations, or payouts for it, and for as long as its
contributors need their pipeline submissions' statuses: the status route
(`POST /v1/contributors/me/submission-status`) reads the pipeline's view
only for a tenant on either list. A tenant on neither list is not processed
at all: its in-flight runs stop, a later withdrawal still queues an
invalidation (the withdrawal needs only a runtime), and queued invalidations
and payouts wait, unprocessed, until the tenant is listed again. Its status
answers are `main`'s alone: a submission only the pipeline recorded is not
described, and a document `main` holds carries no pipeline block.

Activation replaces this list with qualified routing.

The pipeline's own tables (V92 to V95, V105 and V106) grant the ingest runtime group,
`trace_ingest_runtime`, exactly what the pipeline reads and writes there. The
pipeline also reads and writes tables from V62 and earlier -- submissions,
object refs, derived records, tombstones, withdrawals, credit holds, the
Trace Credit ledger and settlement batches, export manifests, the NEAR
outbox, and, for Settle and the payout, the account links
(`trace_account_principals`) and NEAR identities (`trace_near_identities`)
-- which no pipeline migration grants anything on; an ingest login in
the group needs the pilot's V62-era table grants for those too, or the
pipeline fails closed with `permission denied`. To withdraw a submission
that belongs to a source session, both withdrawal routes also need the
ingest login to be a member of `trace_account_admission_runtime`, as
`main`'s withdrawal already does ([deployment.md](deployment.md), "V92 to
V95: the pipeline tables" and "V105 and V106: review, invalidation, and export
tables").

## Fail-closed dependency qualification

`assemble_ingest_pipeline_runtime` refuses to start an injected pipeline
runtime whose scorer, embedder, index, or any registered settlement adapter
is not production-qualified (`pipeline_runtime_is_production_qualified`),
with the safe label `pipeline_runtime_dependencies_not_production_qualified`,
whenever either is true:

- `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` lists at least one tenant, or
- `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED` is set.

Tenants routed to the pipeline is, on its own, enough to trigger the
refusal -- an assembly that lists tenants without also setting
`TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED` no longer runs real receipts
through a non-production-qualified dependency (the Reference scorer, the
in-memory `IsolatedPipelineIndex`, `RecordingSettlementAdapter`, or the
like) just because that flag was left unset.

`TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is the only way past this
refusal. **It is for tests and local development only. Production must
never set it.** Setting it:

- Lets an injected runtime with a non-production-qualified dependency start
  even when tenants are routed or the runtime is required, and logs one
  label-only warning (`pipeline_runtime_test_dependencies_allowed`) at
  startup when it does.
- Never combines with `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED`: both set
  refuses startup at once with
  `pipeline_test_dependencies_not_allowed_when_required`, regardless of
  whether the injected dependency is actually qualified.

## Per-phase claim lease

Each pipeline phase claims its run under its own lease length, sized for how
long that phase can actually run rather than one fixed lease every phase
shared. Score, in particular, runs the injected scorer and embedder inside
the lease -- a chunked NEAR AI perplexity scorer or a CPU-bound embedder can
exceed a short lease on the pilot.

- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_REVIEW` -- whole seconds, default 300
  (5 minutes).
- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SCORE` -- whole seconds, default 1800
  (30 minutes).
- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SETTLE` -- whole seconds, default 300
  (5 minutes).

Each variable is optional; an unset variable keeps its phase's default.
Each configured value must be between 1 second and 2 hours (7200 seconds)
inclusive. A value that does not parse as a non-negative integer, or that
falls outside that range, refuses ingest startup with the safe label
`pipeline_lease_config_invalid`.

`lease_expired` records exactly one situation: the worker that is still
holding the run's lease comes back and writes again -- the phase's own
commit, or a follow-up retry write -- after its own lease has already
expired (the scorer or embedder ran longer than the configured lease). That
write is recorded with `last_error_label = lease_expired` and returned to
`retry` without charging the attempt the claim took. A phase that always
overruns its lease this way shows `lease_expired` every time and never
reaches `failed`/`attempts_exhausted` on that account alone.

This does **not** cover a worker that crashes mid-phase. A crashed worker
never comes back to write anything, so nothing is recorded for it -- the run
simply stays `leased` until `lease_expires_at` passes on its own, at which
point the next claim (by any worker) reclaims it as an ordinary charged
attempt with `last_error_label` cleared. Repeated crashes still exhaust
`max_attempts` and end in `failed`/`attempts_exhausted`, with no record of
why. The same gap applies whenever more than one worker races the same run:
if a second worker reclaims an expired lease before the first worker's own
stale write runs, that write's lease token no longer matches anything (the
token-only fence in `record_lease_expired`), so it changes nothing -- that
attempt is silently lost, not recorded as `lease_expired` and not otherwise
un-charged.

Lease renewal (extending a lease a phase still holds, mid-phase) is PR 4
work and not implemented yet. It is what will close both gaps above -- a
live worker renewing its lease before it expires, rather than a phase
finding out only after the fact (or a crash never finding out at all).

## Authority and privacy at the receipt

Every pipeline receipt needs two controls from the injected runtime. The
receipt looks up the tenant's authority and checks that a privacy boundary
exists before any database work, and it runs the boundary's re-scrub before
it stages anything. A refused receipt leaves no run and no staged object.

- **Authority.** The runtime's authority provider must give the tenant a
  submission authority: its consent-scope and allowed-use allowlists. A
  tenant with no authority is refused with `authority_control_missing`. An
  authority that does not allow the envelope's consent scopes or allowed
  uses does not refuse the receipt: Admission records a `reject` outcome
  with the reason `grant_invalid`.
- **Privacy.** The runtime's privacy boundary re-scrubs the envelope after
  the legacy handler's own re-scrub. A runtime with no boundary refuses the
  receipt with `privacy_control_missing`. A boundary that fails (for
  example, its classifier is down) refuses it with
  `privacy_classification_failed`.

The HTTP response for all three refusals is the generic `500` label
`trace commons operation failed`, with no trace text; it does not name the
refusal. The log line (`Trace Commons ingestion operation failed`) does not
name it either: it carries `error_hash`, the SHA-256 of the refusal label.
Match the hash to its label:

| Refusal label | `error_hash` in the log line |
|---|---|
| `authority_control_missing` | `sha256:ace28b6e3470e2f5351a7b47cd67562829124903550a18f71c7a614d6c5cb898` |
| `privacy_control_missing` | `sha256:ee1bbda14beb581a856f01377bc4f99ae20a67027768b44afc0fec0d16ab720f` |
| `privacy_classification_failed` | `sha256:eb9a2cfa8cab96c6cee0eab15377b489ba143ae27a68b54cc284dbb053225b1f` |

To check a hash, compute it from the label:
`printf %s authority_control_missing | shasum -a 256`.

The stored source is the content after the boundary's re-scrub, and the
boundary's findings feed Admission's privacy risk. The replay identity does
not change: `request_content_hash` is the hash of the raw request, so a
retry of the same bytes replays the same run and does not call the boundary
again. `approved_content_hash` is the hash of the stored, transformed
content. Score and exports read that content only.

The authority provider and the privacy boundary are dependencies like the
scorer and the index. An unqualified one refuses startup with
`pipeline_runtime_dependencies_not_production_qualified` whenever tenants
are routed (the section above).

## Quarantined runs and human review

A run Review quarantines with no human assessment yet is parked in the
`awaiting_review` state, with `last_error_label = review_assessment_required`.
No claim query selects that state, so a parked run is not claimed and does
not retry hourly forever, and it is not charged: parking gives the claim's
attempt back the same way a transient retry does.

A reviewer moves a parked run on through three routes. Each route needs the
review credential (a `reviewer` or `admin` token) of the run's tenant, and
answers `404` when no pipeline runtime is injected:

- `GET /v1/review/pipeline/quarantine?limit=N` lists the tenant's
  quarantined runs that wait for an assessment, oldest first, with each
  run's Admission reason (default 50 runs).
- `POST /v1/review/pipeline/runs/{run_id}/claim` claims a run for the
  reviewer for 30 minutes and returns a `lease_token`. `404` means the run is
  not waiting for review (it is not at Review, not quarantined, already
  assessed, or its submission is no longer operable, for example because it
  was withdrawn). `409` means another reviewer holds a live claim.
- `POST /v1/review/pipeline/runs/{run_id}/assessment` records the
  reviewer's `approve` or `reject` for the claim's `lease_token`, with a
  reason label. An approval must list every Admission reason it resolves in
  `resolved_quarantine_reasons`, or it is refused with `422`
  (`quarantine reason is unresolved`). A stale claim is `409`.

An assessment moves the run back to `pending`, due at once, in the same
transaction. The worker then runs Review with the assessment: an approval
continues to Score, a rejection ends the run.

Two other events release a parked run to `pending`:

- A claim or an assessment on a run whose submission is no longer operable
  (withdrawn, revoked, purged, or expired). The route refuses the request,
  and the queue does not list such a run. The worker then ends the run with
  `submission_inoperable`.
- A withdrawal of the submission (below).

## Settle failures and settlement legs

A run waits in `retry` without a charge to its attempts when the failure is
not the trace's fault:

- `settlement_cap_missing`: no per-instrument cap is configured for an
  award's instrument. The leg's adapter is not called. Configure the cap;
  the next attempt settles the leg.
- `settlement_adapter_missing`: the service holds no adapter for the
  instrument.
- `database_unavailable` (any phase): the database could not serve a
  statement -- no pool connection, a lost connection, a serialization
  failure, a deadlock, a shutdown, or too many connections. If a leg's
  adapter call returned before the failure, the next attempt calls the
  adapter again with the same operation reference. The adapter must answer
  that call from the first one.

An amount above a configured cap is different: the cap refuses the payment,
the leg fails as `credit_cap_exceeded`, and the attempt is charged.

Settle reads whether the submission is still operable once, before its legs.
Only the Trace Credit leg checks it again, under the submission's row lock,
in the transaction that writes its ledger row. Any other leg's adapter call
is not re-checked under the lock, so a withdrawal that lands after that one
read does not stop it. Before a second adapter besides Trace Credit is
registered, its leg must re-check the guard under the lock, as the Trace
Credit leg does.

The adapter's answer decides what happens to a dispatched leg:

- A receipt that answers the request completes the leg. The leg records the
  receipt's result reference and, for an effect with an external record,
  its `external_receipt_hash`. One external receipt answers one leg of a
  tenant.
- `settlement_adapter_unavailable`: the adapter could not complete the
  effect now, or cannot say whether it did. The leg waits in `retry`, the
  run retries without a charge, and the next attempt sends the same
  request.
- `settlement_request_conflict` or `settlement_request_rejected`: the
  adapter says no effect happened and the operation must not be sent
  again. The leg is `failed` with that label.
- `settlement_result_mismatch`: the receipt does not answer the request (a
  different result reference, or an external receipt another leg already
  recorded). The effect is unknown. The leg is `failed` with that label.

A leg `failed` with one of the last three labels is never dispatched again.
While one exists, every Settle attempt is charged, so the other legs still
settle and the run's attempts run out. A request that cannot be formed
(`settlement_request_invalid`) is refused before any adapter call, and the
attempt is charged.

Before each adapter call, the leg moves to `leased` under the run's lease,
and `dispatched_at` records that the leg was sent. When a Settle run fails
(`attempts_exhausted`, or a terminal label such as `bundle_package_invalid`
or `index_key_conflict`), no leg stays open:

- A leg that was never dispatched is `forfeited` with `run_failed`. So is a
  Trace Credit leg that is not `complete`, one `failed` with
  `settlement_result_mismatch` included: it pays only through the ledger
  row that commits with its completion, so it paid nothing.
- A leg `failed` with `settlement_request_conflict` or
  `settlement_request_rejected` is `forfeited` and keeps its label: no
  effect happened. A leg of another instrument `failed` with
  `settlement_result_mismatch` stays `failed` with its label: its effect is
  unknown. Neither gets another adapter call.
- Any other dispatched leg of another instrument gets one more adapter call
  from the worker that fails the run. A result equal to the selected result
  reference makes the leg `complete`. Any other outcome makes it `failed`
  with `settlement_unreconciled`. The worker does not make the call when
  the submission is no longer operable, the adapter or cap is missing, the
  amount is above the cap, or the persisted selection has no result for the
  leg -- because none was ever seeded, or because the persisted selection
  does not decode.
- When the next claim fails a crashed worker's run (`attempts_exhausted`
  after its lease expired), no worker can call the adapter. The first two
  rules still apply; every other dispatched leg of another instrument is
  then `settlement_unreconciled`.

`settlement_unreconciled`, and `settlement_result_mismatch` on a leg of
another instrument, mean the external payment may have happened. Find the
leg's `operation_ref_hash` in the adapter's records and reconcile it by
hand. Nothing retries it once the run has failed. On a run that is still
live, Settle dispatches a `settlement_unreconciled` leg again on its next
attempt -- the label only means the *last* reconciling call did not confirm
a match, not that the leg is done being tried. A leg a withdrawal forfeits
(below) can carry this same label, for the same reason.

A withdrawal (or another way a submission stops being operable -- revoked,
purged, or expired) forfeits every leg that is not `complete` without an
adapter call. A dispatched leg of an external instrument -- `dispatched_at`
is set, and the instrument is not `trace_credit` -- is forfeited as
`settlement_unreconciled` instead of `submission_inoperable`: its adapter
was called at least once, so it may have taken effect, whatever its state or
label. That includes a leg already `settlement_unreconciled` on a live run
and a leg dispatched earlier and later refused by a lowered cap. The leg
keeps its `dispatched_at`; find its `operation_ref_hash` in the adapter's
own records and reconcile it by hand, the same as a `settlement_unreconciled`
leg a failed run leaves behind (above) -- nothing in this release reconciles
it automatically. The exception is a leg whose own label already says no
effect happened (`settlement_request_conflict` or
`settlement_request_rejected`): it stays `submission_inoperable`. An
undispatched leg had no effect, and a Trace Credit leg that is not
`complete` paid nothing -- it pays only through the ledger row that commits
with its completion -- so both of those stay `submission_inoperable` too.

## Withdrawal follow-ups and index invalidation

A contributor withdraws a pipeline submission through either route below.
Both need an account session, never a device key, and both answer the same
`404` for a submission that does not exist and one the account does not own.

- `POST /v1/contributors/me/pipeline-submissions/{submission_id}/withdraw`
  answers `404` when no pipeline runtime is injected.
- `POST /v1/account/traces/{submission_id}/withdraw`, `main`'s route, uses
  the pipeline withdrawal when a pipeline runtime is injected and the
  requested submission, or another submission of its source session, has a
  pipeline run. Otherwise it takes `main`'s path, unchanged.

An upload whose source session is withdrawn while the pipeline receipt is
still in progress is not recorded. The receipt's final transaction locks the
session row before it writes anything. A withdrawal that committed first
makes the receipt answer `409` `source_session_withdrawn`, as `main`'s
receipt does, and the receipt's staged object is deleted. A withdrawal that
comes later waits for the receipt, and then withdraws the new submission
with the rest of the session.

The pipeline withdrawal makes the writes `main`'s session withdrawal makes,
in one transaction: it withdraws the submission's source session and every
submission of that session. For each of those submissions that has a run,
the same transaction also:

- forfeits every settlement leg that is not `complete` (the section above);
- releases a run parked in `awaiting_review` to `pending`, so the worker
  ends it;
- queues an index invalidation when the run's index write may have written
  entries: a write that is `complete`, `pending` (cancelled now, and possibly
  partly written), `failed`, or `cancelled`;
- invalidates every export snapshot that carries the submission;
- invalidates every object ref of the submission and queues the deletion of
  each payload: the receipt's source envelope, Review's approved revision,
  and the two objects Score stores, the index command (embeddings and
  content hashes) and the neighbour set.

`main`'s revocation-propagation worker (`POST /v1/workers/revocation-propagation`)
deletes the queued payloads from the service-owned object store, with its
own retries; an object that is already gone counts as deleted. A run
withdrawn after Score and before Settle still completes Settle when its
index command is already deleted: it is excluded from the index and its
legs are forfeited. A Review or Score attempt whose commit is refused, for
any reason (the submission stopped being operable, its lease expired, a
settlement adapter is missing), deletes the objects it wrote.

`main`'s revocation routes (`DELETE /v1/traces/{id}`,
`POST /v1/traces/{id}/revoke`, `DELETE /v1/traces`) mark the submission
revoked as before, and, when a pipeline runtime is injected and the
submission has a pipeline run, then make the pipeline's follow-up in one
transaction: the export snapshot invalidations and payload deletions above,
the run's index invalidation (reason `revoked`), and the release of a run
parked in `awaiting_review`. Settle reads the revoked status and forfeits
every leg it has not completed.

The response is `main`'s withdrawal response plus two follow-up states,
`index_invalidation` and `revocation_propagation`. Each is `not_required`,
`pending`, `complete`, or `failed`. `credit_retained` is false when the
withdrawal forfeits a Trace Credit leg that was not `complete`, or when
`main`'s rule finds settlement-eligible credit that has not settled. A
completed leg is never clawed back, and its NEAR payout is still made (see
"NEAR payout").

The worker processes the invalidations. After a tenant's runs, at most every
10 seconds, it claims up to 32 of the tenant's due invalidations and removes
every index entry of the withdrawn revision. A withdrawal, a cancelled index
write, or a requeue on this ingest process runs that step on the worker's
next pass instead; one queued on another replica waits up to 10 seconds. A
step that claimed 32 runs again on the next pass. An index outage (the index answers `Failed`
or `Uncertain`) does not charge an attempt: the invalidation stays `pending`
and is retried after a delay that grows from 1 second to at most 1 hour,
measured from when it was queued. There is no retry limit, so an
invalidation retries through an outage of any length and completes when the
index comes back. During an outage, the operational summary shows the
pending invalidations. Only a failure that waiting cannot fix is charged: the
run's Score evidence names no index, or the index answers with a content
conflict. When those attempts run out, the invalidation is `failed` with
`index_invalidation_failed`, and the revision's entries can still be in the
index. A `failed` invalidation is not final. Once the fault is fixed,
`POST /v1/admin/pipeline/index-invalidations/requeue-failed` (an admin
credential; the tenant is the credential's) moves every `failed`
invalidation of the tenant back to `pending`, with no attempt charged and
due at once, and answers `{"requeued": <count>}`; the worker's next pass
on the same ingest process tries each again. Each call appends a `vector_index` audit row with the
count (`pipeline_index_invalidations_requeued`) and nothing else. Queuing the same revision's invalidation again (a repeated
withdrawal, for example) resets it in the same way.

## NEAR payout

Payout is a separate step after Settle. It never changes a Settle outcome,
and it is **disabled by default**: the injected runtime turns it on
(`PipelinePayoutConfig.enabled`). With payout disabled, nothing is submitted
to NEAR.

- Payout takes only a complete run's `trace_credit` legs that have the
  `near` payout rail and a settlement batch. A compatibility bundle's
  `NoveltyUtility` leg has no batch, so it is never paid, as on `main`.
  Other instruments never use the NEAR outbox.
- Score records a leg's payout state when it adds the leg. A run scored
  while payout was disabled has payout `disabled` and is never paid, even
  after payout is turned on.
- The NEAR call goes to the contract `main` is configured with,
  `TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID`. Ingest refuses to start
  a runtime whose payout is enabled with no contract or another contract.
  A retry sends the stored call again, never one built from the current
  configuration. When the configured contract changed since the call was
  stored, the call is not sent again (it could pay twice, on two
  contracts): the payout is `failed` with `near_contract_changed`. A call
  already submitted is still confirmed through its stored key.
- The payout applies every control `main`'s live credit settlement reads
  from its configuration, from `main`'s own values. These are process
  settings, and every pipeline batch has the same policy version and the
  same issuer, so each control is decided once, when ingest starts: a
  control that would refuse a pipeline payout, or that the payout cannot
  apply to a pipeline batch, refuses to start a runtime whose payout is
  enabled (ingest does not start). Fix the configuration and restart.

  | `main`'s control | Case | What the pipeline does |
  |---|---|---|
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` | refuses an enabled payout when it leaves the pipeline out | Every pipeline batch has policy version `pipeline-internal-v1`. A non-empty list without it: `credit_settlement_policy_version_not_allowed`. An empty list allows any version, as on `main`. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` | refuses an enabled payout when it leaves the pipeline out | The pipeline settles as `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`. A non-empty list with that issuer missing or not listed: `central_issuer_denied`. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL` | refuses an enabled payout | `issuer_approval_evidence_hash_missing`. `main`'s approval is evidence an operator records for one batch's source list and names in the settlement request; Settle has no request, and `main`'s own automated settlement does not run live under this flag either. A pipeline batch records no issuer approval evidence. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_ISSUER_APPROVAL_MAX_AGE_HOURS` | refuses an enabled payout | It needs `..._REQUIRE_ISSUER_APPROVAL`, so the row above applies. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY` | refuses an enabled payout | `credit_settlement_rollout_smoke_not_ready`. `main` checks recorded rollout-smoke evidence at each settlement run; Settle has no run to check it at. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` | refuses an enabled payout | `credit_settlement_account_cap_unsupported`. `main` keeps an account's line under the cap by leaving events for a later run; a pipeline leg settles its own event in one batch. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE` | refuses an enabled payout | Ingest does not start while the profile is incomplete (`credit_settlement_central_issuer_profile_incomplete` in the drill). A complete profile sets `..._REQUIRE_ISSUER_APPROVAL`, `..._MAX_POINTS_PER_ACCOUNT` and `..._REQUIRE_ROLLOUT_SMOKE_READY`, so the rows above refuse. |
  | `TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID`, `..._REQUIRE_NEAR_CONTRACT` | applied at startup | An enabled payout must name `main`'s contract (`pipeline_runtime_near_contract_mismatch`, `payout_near_contract_missing`). |
  | `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` | applied at every payout | Ingest hands the mode to the runtime and refuses one that holds another (`pipeline_runtime_near_payout_controls_mismatch`). `disabled` (the default): no outbox row is written and nothing is submitted or confirmed; each leg stays `pending`, as `main`'s rows do. `dry_run`: the full outbox state machine runs in process, with synthetic transaction hashes from each call's idempotency key, no network and no funds, and the injected adapter is not called. `http`: the injected adapter pays. |
  | `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH` | refuses an enabled payout on an adapter without a credential | As `main` refuses to start its NEAR adapters without their bearer tokens, whatever the mode: `near_payout_adapter_auth_missing`. The runtime must hold the same flag (`pipeline_runtime_near_payout_controls_mismatch`). |
  | Credit holds (`credit_holds`) | applied at Settle, to settled legs only | A held principal's settlement-eligible (`accepted`) leg is `held` and is not settled, as `main` leaves held accounts out of its batches and payouts. A compatibility run's `NoveltyUtility` leg ignores holds and writes its ledger row, as `main` writes `NoveltyUtility` credit regardless of holds; that event never settles or pays. |
  | Ranking calibration gates (`TRACE_COMMONS_RANKING_*`) | not applicable | They apply only to `RankingUtility` events; a pipeline leg writes an `accepted` event. |

- A contributor is paid as `main` pays them. A principal linked to an
  account settles under the account (`account:{account_id}`), so its batch
  line has the same credit-account hash as the account's legacy credit, and
  its payout goes to the account's designated NEAR account, or to its only
  active one. When the account has no active NEAR account (`none_enrolled`)
  or several with none designated (`ambiguous_no_designation`), the line is
  held as `main` holds it: the batch line records the label, no outbox row
  is written, and the payout stays `pending` under the label. A payout pass
  resolves the account again once per confirmation interval
  (`TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS`), so the line
  is paid within one interval after the contributor enrols or designates a
  NEAR account. A principal with no account is paid
  with no NEAR account, as on `main`. Holds (`credit_holds`) still apply per
  principal to settled legs, as on `main`; they never stop a `NoveltyUtility`
  leg (see "Compatibility credit").
- A withdrawal does not stop a payout. A leg is `complete` only when Settle
  completed it while the submission was operable, and a withdrawal forfeits
  only the legs Settle has not completed. A completed leg keeps its credit,
  and its payout line is submitted and confirmed after the withdrawal, as
  `main`'s NEAR submitter pays finalized credit.
- The worker runs a tenant's payout pass once per confirmation interval
  (`TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS`), and on
  its next pass after Settle on the same ingest process completes a Trace
  Credit leg of the tenant. A pass that processed 32 runs goes again on the
  next worker pass. A leg completed on another replica is paid within one
  interval.
- The payout uses `main`'s per-tenant NEAR submit lock, so a payout pass,
  a second ingest replica, and `main`'s NEAR submitter never submit for one
  tenant at once. When the lock is held, the pass skips the tenant's
  submits until its next run, and a direct `process_payout` is refused
  with `payout_lock_held`. `main`'s NEAR worker never submits or confirms a
  pipeline outbox row.
- `main`'s admin outbox routes do not reach a pipeline outbox row (one with
  an `instrument_id`): `GET /v1/admin/near-credit-outbox` leaves it out, and
  `POST /v1/workers/near-credit-outbox/mark-status` answers `404` (`NEAR
  credit outbox item not found`) and leaves it unchanged. The pipeline
  confirms its rows only with its adapter's evidence. The operational
  summary's NEAR outbox counts still include them.
- Confirmation of a submitted payout is polled without the lock, at most
  once per `main`'s NEAR scheduler interval,
  `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS` (60 seconds
  by default).
- An error on one run's payout marks that leg's payout `failed` with a
  label, and the pass goes on to the next run. Only a database error ends
  the pass. The worker does not retry a `failed` payout, including one that
  another ingest replica failed after this pass listed it. Only a direct
  `process_payout` for the run takes it up again.
- This release has no operator route or tool that retries a `failed`
  payout: nothing an operator can reach calls `process_payout`. An operator
  sees the leg's payout as `failed` with its label (for example
  `near_submit_failed`) in the run's forensic trace
  (`GET /v1/admin/pipeline/runs/{run_id}/forensic`) and in the contributor
  status, the outbox line as `failed` in the operational summary's NEAR
  outbox counts, and the settled credit itself unchanged. The payout stays
  `failed`: nothing in this release takes it up again. A later release adds
  an operator retry route. A failed submit may still have reached NEAR, so
  until then check a `failed` payout's outbox line against NEAR by hand.

## Pipeline exports

`POST /v1/pipeline/exports` takes a snapshot of the tenant's approved
revisions, and `POST /v1/pipeline/exports/{snapshot_id}/complete` delivers
it. Both need the export credential (an `export_worker` or `admin` token)
and answer `404` when no pipeline runtime is injected. The create route
needs an `idempotency-key` header: the same key with the same use, purpose,
consent scope, and limit returns the first snapshot, and with any of them
different is refused with `409` (`export_idempotency_conflict`).

The routes apply `main`'s export rules:

- With `TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS` set, a request needs an
  explicit purpose and an explicit consent scope, and the snapshot holds
  only submissions with `low` privacy risk. A quarantined submission a
  reviewer approved is left out too.
- The item limit follows `main`'s: 100 by default, never above
  `max_export_items_per_request`, and never above 500.
- The scoped credential's and the tenant policy's consent-scope allowlists
  narrow the selection, as they do for `main`'s exports.
- Only accepted submissions whose run is complete are exported, and each
  item is the approved revision's content hash, never the raw request.

A delivery records an export manifest in `main`'s tables, with the purpose
code `pipeline_export:<allowed use>`. `main`'s replay dataset list and its
replay manifest count leave that purpose family out, so a replay worker
never takes a pipeline export. `main`'s replay export refuses a purpose in
the reserved `pipeline_export:` family with `400` (`export_purpose_reserved`).

A snapshot that can no longer be delivered is refused with `409` and a label
that tells the caller to create a new snapshot:
`export_snapshot_invalidated_create_new_snapshot` after a withdrawal, and
`export_snapshot_stale_create_new_snapshot` when one of its submissions
expired or was revoked outside the pipeline. Each create and each delivery
appends one hash-only `export` audit event; a refused request appends none.

## Compatibility credit

The compatibility bundle reproduces `main`'s gate-path credit. Its
configuration is validated as `main` validates its gate at startup: a
production-compatible configuration with every floor zero is refused
(`compatibility_zero_floor`), and a zero tail-fraction floor with the other
floors positive, `main`'s pilot value, is accepted. A runtime that routes or
drains a tenant must bind a qualifiable configuration: the local reference
configuration (all floors zero) fails the qualification gate
(`pipeline_runtime_dependencies_not_production_qualified`) unless
`TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is set. Score inserts a
chunk into the index under `main`'s own threshold,
`TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS` (50000 unless configured),
never the novelty floor: ingest hands it to the runtime and refuses one whose
compatibility configuration holds another
(`pipeline_runtime_embed_insert_novelty_mismatch`). When both gate
floors pass, Score awards the `NoveltyUtility` delta to `trace_credit`, and
Settle records it as one `NoveltyUtility` ledger event, written as `main`
writes that event: settlement state `final`, actor role `vector_worker`, the
actor the pipeline's issuer (`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`,
where `main` records its issuing gate worker), and the reason
`novelty_utility:compatibility_quality_novelty_v1`. A runtime that routes or
drains a tenant through the compatibility bundle refuses to start without
that issuer (`pipeline_credit_issuer_principal_missing`). That event type
does not settle on `main`, so the pipeline never batches or pays it, and the
contributor status reports the leg as `not_settlement_eligible`.

A credit hold on the contributor does not stop this event, as it does not on
`main`: holds gate settlement batches and payouts only.

Before Settle writes the ledger event, it applies `main`'s `NoveltyUtility`
credit checks, in `main`'s order. A check that refuses the credit withholds
the leg: the leg completes with no ledger event, its `last_error_label` is
`main`'s reason, and the contributor status reports `withheld` with that
reason. The Score decision does not change, and a withheld leg is not a
charged Settle error.

| Check | Withheld as |
|---|---|
| With `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` set, the runtime's scorer and embedder must be production-qualified (where `main` requires a production gate service). | `non_production_gate` |
| With `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` set, the pipeline's issuer, `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`, must be on that list (where `main` checks its calling gate worker). With the list set and no pipeline issuer, every award is withheld. With the list unset, every award passes this check, as on `main`. | `central_issuer_denied` |
| The tenant's authority must exist, and so must its policy when the authority requires one. The policy comes from the runtime's authority provider, the source the receipt uses. | `credit_check_error` |
| The submission's allowed uses must include `model_training`, and its consent scopes and `model_training` must be inside the tenant policy's allowlists. The default consent scope (debugging and evaluation) does not allow model training, so its credit is withheld, as on `main`. | `policy_mismatch` |

`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF` is a canonical hashed
principal ref (`principal_sha256:` and 64 hexadecimal characters), the same
form the central-issuer list takes; ingest refuses to start with any other
value. `main` also checks the submission is `accepted`: Settle's operability
re-check already requires that, and a submission that fails it forfeits the
leg as `submission_inoperable`. `main` also applies its calling token's scoped
allowlists; Settle has no calling token, and the tenant authority's
allowlists were applied to the submission at the receipt. Ingest hands the
issuer list, the pipeline issuer, and the production-gate flag to the
injected runtime, and refuses to start one that does not hold the same values
(`pipeline_runtime_novelty_utility_checks_mismatch`).

A compatibility run's contributor status is `main`'s document for the same
credit, with contributor reads from the database and from files alike: the
status reads `accepted`, the `NoveltyUtility` event counts as ledger credit
(`credit_points_ledger` and `credit_points_total`), and where `main` shows its
gate's credit-quality figure and "Credit reflects the gate's scoring" line,
the document shows the Score evidence's shadow credit quality with the same
line. The pipeline block comes with it. A minimal-family run keeps its own
document under file reads.

A submission only the pipeline knows (its own document) reports `status` in
`main`'s vocabulary, so the contributor daemon's history counts and its
held-for-review check recognize it. The pipeline's own state stays in the
pipeline block (`processing_state`). The mapping:

| Submission status in `trace_submissions` | Run | `status` |
|---|---|---|
| `accepted`, `rejected`, `revoked`, `expired`, `purged`, `quarantined` | any | the same value |
| `received` (Review has not decided) | waiting for a human review, or Admission quarantined it | `quarantined` |
| `received` | Admission rejected it | `rejected` |
| `received` | any other state, a failed run included | `accepted` |

Its pending points are 0 when its Trace Credit leg will not be paid:
forfeited, failed, withheld by one of `main`'s credit checks, or a
`NoveltyUtility` leg that `main` never settles.

`main`'s gate evaluate route (`POST /v1/workers/gate/evaluate`) refuses a
submission that has a pipeline run with `409` `pipeline_run_owns_submission`,
before it scores anything, so `main`'s gate path cannot award a second
`NoveltyUtility` credit for a trace the pipeline credits.

The delta is pinned in the signed bundle package
(`novelty_utility_microcredits`, in microcredits). The pipeline does not read
`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` at run time. For the
pipeline to credit what the legacy path credits, the package's delta must
equal that variable times 1,000,000 (a points delta of `2.5` is
`2500000`). The default is `0` in both places: no award, no settlement leg,
and no ledger event. A different delta is a different package, with its own
bundle id, and it applies only to runs bound to that package.

## DB reconciliation of a pipeline tenant

`main`'s DB reconciliation (`/v1/admin/db-reconciliation-drill`, and a
maintenance run with `reconcile_db_mirror=true`) compares `main`'s file
mirror with the database. The pipeline writes database rows only, never a
file record, so the reconciliation leaves the tenant's pipeline rows out of
each comparison they would fail:

- the runs' submission rows and their derived records (the missing-in-files
  checks);
- the credit events whose `pipeline_run_id` is one of the tenant's runs;
- the settlement batches the runs' Trace Credit legs carry, and the NEAR
  outbox lines of those batches;
- the contributor-credit, reviewer-metadata and analytics reader parity
  checks, whose database side then reads without the pipeline rows;
- the check that an accepted submission's envelope object reads back: a
  pipeline submission's objects are the pipeline's own source and approved
  revision, which the pipeline reads and checks itself.

Each row is found through its pipeline run, not by its shape. `main`'s own
rows keep every check: a legacy row with no file record is still a blocking
gap. The database counts in the report (`db_submission_count` and the
others) still include the pipeline rows, so they can be larger than the file
counts on a clean report. The pipeline rows are found only while a pipeline
runtime is injected; without one, a tenant's earlier pipeline rows are
reported as gaps.

## Receipt staging and the orphan sweep

A pipeline receipt records its envelope object as a `staged` row in
`pipeline_receipt_artifacts` before it writes the object. The transaction that
stores the receipt marks the row `committed`. If the receipt fails after the
write, the row stays `staged`. On each pass, the pipeline worker sweeps every
listed tenant: for up to 32 `staged` rows whose `cleanup_after` (one hour after
staging) has passed, it deletes the object and then the row. If a delete
fails, the row stays for the next pass and the worker logs
`pipeline_receipt_sweep_delete_failed`. A failed receipt stays counted against
the quota. A retry with the same submission id is not counted again.

The sweep runs over the tenants the worker drains: those on
`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` or
`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS`. A tenant on neither list is not
swept: its staged objects (and their rows) stay exactly as they were until
the tenant is listed again or an operator removes them by hand.

After a failed receipt attempt leaves a staged row, a later receipt with the
same idempotency key and different content is refused with the
content-conflict 409 until the sweeper removes that staged row -- up to
about one hour after it was staged. A retry with the *same* content is
unaffected: the failed attempt never created a run, so the retry is a new
attempt that creates the run itself, and the caller gets the same 200 it
would have gotten on a first success.

## Submission quota at switch-over

The pipeline counts only pipeline receipts against the hourly submission
quota. It does not count legacy submission records. In the first hour after a
tenant moves to the pipeline, that tenant can therefore receive up to one
extra hourly quota. This is accepted behavior.

The legacy quota and tombstone checks still run before ingest routes a
receipt to the pipeline. A tenant with recent legacy submissions can
therefore receive a legacy 429 for a pipeline receipt.

## Rehearse the switch

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database.
Then run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg pipeline_activation
```

The tests cover mixed legacy and pipeline records, exact replay of a
pre-switch receipt, unique ledger sources, tenant expansion gates,
rollback, containment, suspension instead of rebinding, and writer
retirement after pending work completes.

The [local pipeline lab](pipeline-lab.md) `qualify` command runs this suite as part of pipeline qualification. Lab corpus and package evidence remains in local files;
these PostgreSQL integration tests remain separate schema and recovery checks.

## Local operator routes

`trace-commons-pipeline-local` adds these routes:

- `POST /v1/pipeline/switched-submissions`
- `GET /v1/admin/pipeline-routing`
- `POST /v1/admin/pipeline-contain`
- `POST /v1/admin/pipeline-retire-legacy-writer`

The existing corpus paths still submit directly to the pipeline executor.
The switched route is the dual-path receipt used during migration.

## Current completion

None of the redesign is in this repository yet. The versioned pipeline
contracts are defined; the runtime, qualification, and activation work
follows in later changes. `SCR-005` stays deferred until the external
valuation protocol exists. New valuation rules use a later bundle through
the same qualification and activation process.
