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
- Removing a tenant from the list also stops the worker for that tenant.
  Its in-flight pipeline runs stay unprocessed until the tenant is listed
  again.

Activation replaces this list with qualified routing.

The pipeline's own tables (V92 to V95, V101 and V102) grant the ingest runtime group,
`trace_ingest_runtime`, exactly what the pipeline reads and writes there. The
pipeline also reads and writes tables from V62 and earlier -- submissions,
object refs, derived records, tombstones, withdrawals, credit holds, the
Trace Credit ledger and settlement batches, export manifests, and the NEAR
outbox -- which no pipeline migration grants anything on; an ingest login in
the group needs the pilot's V62-era table grants for those too, or the
pipeline fails closed with `permission denied`. To withdraw a submission
that belongs to a source session, both withdrawal routes also need the
ingest login to be a member of `trace_account_admission_runtime`, as
`main`'s withdrawal already does ([deployment.md](deployment.md), "V92 to
V95: the pipeline tables" and "V101 and V102: review, invalidation, and export
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
- invalidates every export snapshot that carries the submission.

The response is `main`'s withdrawal response plus two follow-up states,
`index_invalidation` and `revocation_propagation`. Each is `not_required`,
`pending`, `complete`, or `failed`. `credit_retained` is false when the
withdrawal forfeits a Trace Credit leg that was not `complete`, or when
`main`'s rule finds settlement-eligible credit that has not settled. A
completed leg is never clawed back.

The worker processes the invalidations. On each pass, after a tenant's runs,
it takes up to 32 of the tenant's due invalidations and removes every index
entry of the withdrawn revision. An index outage (the index answers `Failed`
or `Uncertain`) does not charge an attempt: the invalidation stays `pending`
and is retried after a delay that grows from 1 second to at most 1 hour,
measured from when it was queued. There is no retry limit, so an
invalidation retries through an outage of any length and completes when the
index comes back. During an outage, the operational summary shows the
pending invalidations. Only a failure that waiting cannot fix is charged: the
run's Score evidence names no index, or the index answers with a content
conflict. When those attempts run out, the invalidation is `failed` with
`index_invalidation_failed`, and the revision's entries can still be in the
index. Remove them by hand.

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
- Before each submit, the payout checks again that the submission is
  operable.
- The payout uses `main`'s per-tenant NEAR submit lock, so a payout pass,
  a second ingest replica, and `main`'s NEAR submitter never submit for one
  tenant at once. When the lock is held, the pass skips the tenant's
  submits until the next pass, and a direct `process_payout` is refused
  with `payout_lock_held`. `main`'s NEAR worker never submits or confirms a
  pipeline outbox row.
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
  until then check a `failed` payout's outbox line against NEAR by hand, as
  for `settlement_unreconciled` below.
- When a submission stops being operable after its payout reached the NEAR
  outbox, the leg is marked `settlement_unreconciled`: the transfer may have
  happened. Find the leg's outbox row and reconcile it by hand. The
  exception is a payout whose every outbox line is already confirmed: it
  stays `confirmed`.

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

The compatibility bundle reproduces `main`'s gate-path credit. When both gate
floors pass, Score awards the `NoveltyUtility` delta to `trace_credit`, and
Settle records it as one `NoveltyUtility` ledger event, written as `main`
writes that event: settlement state `final`, actor role `vector_worker`, and
the reason `novelty_utility:compatibility_quality_novelty_v1`. That event type
does not settle on `main`, so the pipeline never batches or pays it, and the
contributor status reports the leg as `not_settlement_eligible`.

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

The delta is pinned in the signed bundle package
(`novelty_utility_microcredits`, in microcredits). The pipeline does not read
`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` at run time. For the
pipeline to credit what the legacy path credits, the package's delta must
equal that variable times 1,000,000 (a points delta of `2.5` is
`2500000`). The default is `0` in both places: no award, no settlement leg,
and no ledger event. A different delta is a different package, with its own
bundle id, and it applies only to runs bound to that package.

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

A tenant removed from `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` stops
being swept the same pass it stops being worked: the sweep runs only over
listed tenants. Its staged objects (and their rows) stay exactly as they
were until the tenant is listed again or an operator removes them by hand.

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
