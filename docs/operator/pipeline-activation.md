# Pipeline activation and migration

> **Status: activation is in the repository, and production routing is off.**
> The routing record, the nine admin routes, containment, rollback, policy
> suspension, and the legacy drain report are built and tested. No tenant is
> routed to the pipeline until an operator activates it. A deployment can
> activate a tenant only with a full set of signed check results. This
> repository's local tooling cannot produce that set on its own (see "Current
> completion").

Pipeline activation routes the qualified pipeline for new submissions. Retained
legacy records stay readable. The switch assigns each receipt to one
implementation.

The pipeline now has these properties:

- A committed routing row decides where a tenant's new uploads go: the legacy
  path, the pipeline, or nowhere (contained). The two tenant lists are the
  scope of a process, and a row cannot widen it.
- Each receipt has one permanent owner, the legacy path or the pipeline. A
  retry goes to its first owner, whatever the routing row says now.
  Timestamps are audit metadata: they select no bundle and no owner.
- Activation needs a qualification of the bundle on the deployed code
  revision, a full set of signed check results, the deployed revision, the
  dependency profile of the process, four runnable policies, and the tenant's
  readiness. The server builds these inputs itself, from the verified signed
  results and its own state. A request body cannot carry a decision, a
  readiness, a dependency profile, or a revision.
- After a deploy, a process takes a new upload of a tenant whose row says
  `pipeline` only when the tenant's active bundle has a qualification on the
  code revision of that process. Until then the upload is refused with `503`.
- Rollback selects an earlier bundle that was active for the tenant, for later
  runs only. It runs the same gate as an activation, without the readiness
  check.
- Containment refuses each new upload of the tenant whose route is decided
  after it, with `503`, and keeps the worker: runs in flight, invalidations,
  and payouts still finish. An upload that is in flight on the legacy path can
  still finish ("Scope lists and the routing row").
- A suspended policy keeps its run. The run waits, uncharged, under the same
  bundle, and goes on after the resume.
- The legacy drain report shows what the legacy path still owes a tenant. It
  disables nothing: this release retires no legacy writer.
- Destructive schema cleanup is not part of this phase.

## Scope lists and the routing row

Two environment variables list tenants. They are the scope of one process.
Both need a pipeline runtime in the build, and the repository binary injects
none.

`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` is the receipts list: the tenants
whose new uploads this process may route to the pipeline. Unset or empty (the
default), this process routes no upload to the pipeline. A tenant listed
without a runtime: ingest refuses to start with
`pipeline_receipts_configured_without_runtime`.

`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` lists tenants whose pipeline work
the worker still processes while none of their new receipts is routed to the
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
tenant of a submission id a pipeline run owns replays its pipeline receipt,
as it did while the tenant was routed, with or without account admission
(a static-token tenant too); a new upload takes the legacy path. An upload
of a submission id a pipeline run owns never reaches the legacy record: the
same body from the recorded principal replays the receipt, a different body
is refused with `409` (`receipt id reused with different content`), another
principal with `409` as `main` refuses one, and a tenant on neither list, or
a build with no pipeline runtime, answers `409`
(`submission_owned_by_pipeline_run`). Between two processes the ownership
row holds this (`pipeline_receipt_ownership`, V110): the legacy path claims
the submission id before its first write, the pipeline commits its own
ownership row with the run, and only one of the two commits for one id. One
case stays open: a replica that lists the tenant on neither list makes no
claim while the tenant has no routing row. "Run one build and one
configuration" below gives the rule that closes it.

The lists do not decide where an upload goes. The tenant's routing row
decides, inside the scope. The row (`pipeline_tenant_routing`, one for each
tenant) holds one state: `legacy`, `pipeline`, or `contained`. A tenant has no
row until an operator acts on it. The routes in "Activate, roll back, contain,
deactivate" write it. A process reads the row once for each new upload:

| Routing row | On the receipts list | A new upload | Answer |
|---|---|---|---|
| none | no | takes the legacy path | none |
| none | yes | takes the legacy path (see the test-only rule below) | none |
| `legacy` | either | takes the legacy path | none |
| `pipeline` | yes | goes to the pipeline, when the tenant's active bundle is qualified on this build's revision | none |
| `pipeline` | yes, and the active bundle has no qualification on this build's revision | is refused, and nothing is stored | `503` `pipeline_bundle_not_qualified` |
| `pipeline` | yes, and this build has no revision | is refused, and nothing is stored | `503` `bundle_runtime_revision_unknown` |
| `pipeline` | no | is refused, and nothing is stored | `503` `pipeline_tenant_not_served` |
| `contained` | either | is refused, and nothing is stored | `503` `pipeline_receipt_intake_contained` |
| cannot be read | either | is refused | `503` `pipeline_routing_unavailable` |

The two rows about the qualification are in "After a deploy: the qualification
is read again for each new upload". They do not apply to a process that was
started with the test-only rule below.

The last row also covers a build with a runtime and no routing store. Ingest
never falls back to the legacy path for a row it cannot read.

A process with no pipeline runtime (the repository binary) reads the row too,
when it has a database. It serves no tenant on the pipeline, so every tenant is
"not on the receipts list" there:

- no row, or `legacy`: the legacy path;
- `pipeline`: `503` `pipeline_tenant_not_served`;
- `contained`: `503` `pipeline_receipt_intake_contained`.

Such a process already asked the database, for each new upload, whether a
pipeline run owns the submission id. It now reads the routing row in that same
statement, so the read adds no transaction. When that statement fails, the
answer is the one from before this rule (`500`, `trace commons operation
failed`), and nothing is stored. A process with no database has no routing
store: it reads nothing and sends every upload to the legacy path. A build
with no runtime from before this rule ignores the row ("Run one build and one
configuration" below).

A row cannot widen the scope: a tenant whose row says `pipeline` and that is
not on this process's receipts list is refused, not sent to the legacy path.
A tenant whose row says `contained` is refused whatever the lists say, on a
process with a runtime and on one with none. This includes a tenant that has
no pipeline work.

The same holds for a remediation: an upload, by its contributor, of a
corrected body for a submission id whose legacy record is quarantined. The
process reads the row for it first. A contained tenant's remediation is
refused (`503` `pipeline_receipt_intake_contained`), and so is the remediation
of a tenant whose row says `pipeline` on a process that does not serve the
tenant (`503` `pipeline_tenant_not_served`), and one whose row cannot be read
(`503` `pipeline_routing_unavailable` on a process with a runtime, `500` on
one with none, as for a new upload). In every other case a remediation
stays on the legacy path, which owns the id, also for a `pipeline` tenant that
the process serves; the qualification and the Admission policy are not read
for it. So the remediations of a contained tenant wait until its intake opens
again. An operator's rescrub of a quarantined record is not an upload: it
reads no routing row, and it stays open while the tenant is contained.

`contain` stops each routing decision that is made after it returns, on each
process that has a database and a build with this rule, with a runtime or
without one. It does not stop an upload whose route was decided before it:

- An upload that was decided for the legacy path before the `contain` can still
  write its legacy record after the `contain` returned. The legacy claim reads
  no routing row. The window is the time of one request in flight.
- An upload that was decided for the pipeline is checked again for its
  receipt: one read before the receipt's transactions, which waits for
  nothing, and one in each of the two transactions, which wait for the
  `contain`. It is refused when the `contain` commits before the check reads
  the routing. This includes an upload whose submission id the legacy path
  already owns (a legacy claim whose write failed): the check answers the
  containment before the owner. If the first check answered the legacy owner
  before the `contain` committed, the upload is on the legacy path, and the
  first case applies to it.

Each submission id still has one owner in both cases.

Before its first write the legacy path claims the submission id. The claim is
an ownership row with the owner `legacy` (`pipeline_receipt_ownership`),
made in one transaction that first checks that no pipeline run has the id. The
claim happens for a tenant on either list or with a routing row. A tenant
with no row that is on neither list claims nothing. The pipeline commits its
own ownership row (owner `pipeline`) in its receipt transaction, with the run.
So each submission id has one owner for good:

- A retry of an existing receipt is answered before the route is decided. It
  goes to its first owner, whatever the row says now, and it is checked against
  no qualification.
- An id that the legacy path claimed stays legacy, even when its first
  attempt failed before it wrote a record. A later upload of it goes to the
  legacy path while the row says `pipeline` or `legacy`. A tenant that is
  contained, or not served by the process, refuses it with `503`, as it
  refuses every new upload: the refusal comes before the owner is read. For
  the same reason, a tenant whose row says `pipeline` refuses it with `503`
  `bundle_policy_not_runnable` while the Admission policy of its active bundle
  is suspended, and with `503` `pipeline_bundle_not_qualified` while that
  bundle has no qualification on the build's revision: the policy and the
  qualification are read before the owner too.
- An id that a pipeline run owns is never taken by the legacy path. The
  pipeline's library refuses a second idempotency key for it with `submission
  identity is already bound to another receipt`. Ingest uses the submission id
  as the key, so over HTTP the same upload replays instead.

The route is decided after authentication, the submit rate limit, envelope
validation, the admission reservation, the tenant-access check, and the check
for a retry. It is decided before the server re-scrub, the tombstone check, and
the submission quota. So a refusal costs no classifier call. It also releases
the admission attempt: the attempt is not left processing, so a retry is not
`409` in progress, and a bounded account gets its charge back. A pipeline
upload still passes the tombstone and quota checks before the pipeline takes
it. A tenant that is contained or not served gets its `503` before a tombstone
or quota refusal.

The decision also reads whether the tenant's active bundle has a qualification
on the build's revision, and the Admission policy of that bundle, in the same
statement as the row. An upload for the pipeline is refused there in this
order: a build with no revision (`503` `bundle_runtime_revision_unknown`), a
bundle with no qualification on the revision (`503`
`pipeline_bundle_not_qualified`), a suspended Admission policy (`503`
`bundle_policy_not_runnable`, for as long as the suspension lasts). The attempt
is released as above.

One case is different. A `contain`, or a `suspend` of the Admission policy, can
commit while an upload is in flight, after its route was decided. The receipt
transaction then refuses the upload (`503` `pipeline_receipt_intake_contained`
or `bundle_policy_not_runnable`), and nothing is stored. That refusal comes
after the admission attempt was marked processing, and `main`'s admission
ledger releases an attempt only before that mark. So the attempt stays
processing until its lease ends: a retry inside the lease is `409` in progress,
a bounded account's charge is not given back, and a retry after the lease is
charged again. Only the uploads in flight at that moment are affected. (A
`deactivate` in flight refuses nothing: the upload is claimed for the legacy
path and goes on there.)

An upload that carries anchored evidence has one more limit. A released
reservation can be retried only while its evidence is unexpired. A containment
that lasts longer than the evidence stays valid makes that submission id
unusable on that path: fresh evidence under the same id is a conflict. The
account-trust path relaxes the time limit for an existing binding.

The test-only rule. `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` also
sends a tenant on the receipts list that has no routing row to the pipeline.
That is for tests and local development only. Production must never set it.
Setting it together with `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED` refuses
startup (`pipeline_test_dependencies_not_allowed_when_required`; the routing
setting has its own refusal for any other caller of the assembly,
`pipeline_unqualified_routing_not_allowed_when_required`). An assembly whose
service does not hold the same setting is refused with
`pipeline_runtime_unqualified_routing_mismatch`. A runtime whose dependencies
are production-qualified refuses to start with the variable, with or without
`TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED`:
`pipeline_unqualified_routing_with_production_runtime`. A runtime that starts
with the variable logs one warning, `pipeline_unqualified_routing_allowed`, and
`GET /v1/admin/config-status` reports `pipeline_unqualified_routing_allowed:
true`. In that process, `routing_state: null` in `GET
/v1/admin/pipeline/routing` does not mean the legacy path for a listed tenant.

The cost. On a process with a runtime, each new upload of a tenant on either
list reads the routing row once, in a short transaction of its own with one
query by primary key: the routing row together with the Admission policy and
the qualification of the tenant's active bundle. For every other upload of a
process with a database (a process with no runtime, or a tenant on neither
list) the same query also answers whether a pipeline run owns the submission
id, and it takes the place of the statement that asked only that: the routing
read adds no transaction and no statement there. Each new upload that takes
the legacy path of a tenant in scope, or with a row, adds the claim: four
statements in one transaction (the run check, the tenant upsert, the ownership
insert, and the owner read). A new upload of a tenant on neither list with no
row adds nothing. A retry adds nothing. A pipeline receipt's
own transactions then check the routing and the ownership again, in one
statement each time, because the receipt transaction is the authority on a
race: a row that changed after the decision is answered by what the
transaction finds.

A receipt that the commit transaction refuses keeps the quota row that its
staging transaction wrote (`pipeline_admission_usage`). So the hourly submission
quota counts an upload that a routing change, a legacy claim, or a suspension
refused at commit. A receipt that is refused earlier is not counted.

To send a tenant back to the legacy path, deactivate it first
(`POST /v1/admin/pipeline/deactivate`, with the `activation_record_id` that you
read from `GET /v1/admin/pipeline/routing` as `expected_record_id`): its row
says `legacy`, and its new uploads take the legacy path at once. Then move it from the receipts list to
the drain list and restart ingest. Do not remove a tenant from the receipts
list while its row says `pipeline`: every new upload of the tenant is then
refused with `503` `pipeline_tenant_not_served`, and none goes to the legacy
path. Keep the tenant on the drain list until the operational summary shows no
pending runs, invalidations, or payouts for it, and for as long as its
contributors need their pipeline submissions' statuses: the status route
(`POST /v1/contributors/me/submission-status`) reads the pipeline's view
only for a tenant on either list. A tenant on neither list is not processed
at all: its in-flight runs stop, a later withdrawal still queues an
invalidation (the withdrawal needs only a runtime), and queued invalidations
and payouts wait, unprocessed, until the tenant is listed again. Its status
answers are `main`'s alone: a submission only the pipeline recorded is not
described, and a document `main` holds carries no pipeline block.

Run one build and one configuration on every replica of ingest:

- A replica whose build has no pipeline runtime reads the routing row, when
  the build has this rule. It refuses the uploads of a tenant whose row says
  `pipeline` (`503` `pipeline_tenant_not_served`) or `contained`, and takes the
  uploads of every other tenant on the legacy path, with a legacy claim for a
  tenant that has a row. It can `contain` and `deactivate` a tenant: these two
  routes need only the routing store. So do not mix builds with and without a
  runtime while any tenant's row says `pipeline`: the replicas with no runtime
  refuse that tenant's uploads. To run such a tenant on a build with no
  runtime, `deactivate` it first (that build can do it too).
- A replica whose build has no pipeline runtime and is from before this rule
  (a build of `main` from before the activation routes, for example) reads no
  routing row. It sends every new upload of a tenant to the legacy path, a
  tenant whose row says `pipeline` or `contained` included. Do not run such a
  build while any tenant's row says `pipeline` or `contained`.
- A replica whose build has a pipeline runtime and is from before the routing
  row existed (a build without V110's code) also reads no routing row. It
  routes by its receipts list alone: every new upload of a listed tenant goes
  to the pipeline, a tenant whose row says `contained` or `legacy` included,
  and a listed tenant with no row too. Its receipt transaction checks no
  routing. It also has no policy guard at a phase commit or at the payout
  dispatch: a suspension stops a phase only where that build reads the policy,
  when it stages a receipt and when it starts a phase, and it does not stop a
  payout. Read "Binary rollback to an older build" in
  [deployment.md](deployment.md) before you install such a build.
- A replica whose receipts list lacks a tenant whose row says `pipeline`
  refuses that tenant's uploads with `503` `pipeline_tenant_not_served`. While
  a list change rolls through the replicas, some uploads are refused until
  every replica holds the new list. A retry of a receipt that the pipeline
  already owns gets another answer on such a replica when the tenant is on
  neither of its lists: `409` `submission_owned_by_pipeline_run`. So during a
  list rollout a client can see `503` for a new upload and `409` for a retry.
  Both end when every replica lists the tenant.
- Every replica that has a runtime must list every tenant that can be
  activated, on the receipts list or on the drain list, before the first
  activation of that tenant. This rule protects ownership, not only
  availability. A replica that has a runtime and lists the tenant on neither
  list makes no legacy claim for it while it has no routing row. An upload that
  such a replica decided for the legacy path before the activation can then
  write its legacy record after another replica committed a pipeline receipt
  for the same submission id (a client retry that reaches both). The
  submission id then has two owners.

`GET /v1/admin/config-status` reports the receipts list's size as
`tenant_rollout_gate_counts.pipeline_receipts` (a count), and the rollback
drill (`POST /v1/admin/rollback-drill`) names `pipeline_receipts` in
`active_rollout_flags` for a tenant on the list. Both now mean "in scope". They
no longer mean "routed to the pipeline": a tenant on the list is routed only
while its row says `pipeline`. Read the routing row (`GET
/v1/admin/pipeline/routing`) for that.

Config-status also reports four booleans about the start inputs of the routes
below. It never reports a revision, a path, a key id, or a key:

| Field | True when |
|---|---|
| `pipeline_code_revision_configured` | the binary was built with a code revision |
| `pipeline_package_trust_store_loaded` | the package trust store was loaded at start |
| `pipeline_check_trust_store_loaded` | the check trust store was loaded at start |
| `pipeline_unqualified_routing_allowed` | the test-only rule above is on |

The pipeline's own tables (V92 to V95 and V105 to V113) grant the ingest runtime group,
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
V95: the pipeline tables", "V105 and V106: review, invalidation, and export
tables", "V107 and V108: qualification and attempt artifact tables", and "V110
to V113: activation, policy interventions, the activation gate, and the
rebuild fence").

V109 adds no table and changes no grant. It marks the payout of a leg
seeded `pending` by V94-era code `disabled` (see "NEAR payout"), and adds
four checks: an export snapshot's requester is `principal_sha256:` or
`exporter_sha256:` and 64 lowercase hex digits, an export item's outcome and
view schema ids are labels, and an assessment's resolved quarantine reasons
are a JSON array. The code already writes only such values. It also
indexes two foreign keys that had no index on the referencing side: the
index invalidations by submission and the export items by run. Apply V109
by hand only with `psql --single-transaction -v ON_ERROR_STOP=1`: it lifts
forced row security on `pipeline_run_settlements` for one statement
([deployment.md](deployment.md), "V109: pipeline follow-ups").

## Activate, roll back, contain, deactivate

These nine routes qualify a bundle and move a tenant between the legacy path
and the pipeline: eight paths, and `policy-interventions` has a `GET` and a
`POST`. (The operational summary, the forensic trace, and the index requeue
route sit under the same prefix. They are older.) Each of the nine is under
`/v1/admin/pipeline/` and needs an admin credential of the tenant it acts on
(`403` `admin token required` otherwise). The tenant and the actor come from the
credential alone. No body names a tenant, and each body refuses any field it
does not list. A body that does not parse answers `pipeline_request_invalid`
with the parser's status: `422` for an unknown, missing, or mistyped field,
`400` for JSON that is not well formed, and `415` without a JSON content type
(a query string that does not parse is `400`). A body is at most 1 MiB (`413`
`pipeline_request_too_large`). A `reason_code` is a label: lowercase
letters, digits, and `_`, 1 to 64 characters. A bundle id is `sha256:` and 64
lowercase hex digits. A malformed one is `422` `pipeline_request_invalid` on
`activate`, `rollback`, and `GET policy-interventions`, and `409`
`policy_intervention_invalid` on `POST policy-interventions`.

| Route | Body | Answer |
|---|---|---|
| `POST /v1/admin/pipeline/qualifications` | `{signed_package, attestations}` | the qualification record: `bundle_id`, `package_hash`, `signing_key_id`, `signature_hash`, `metadata` (six digests), `qualified_at` |
| `POST /v1/admin/pipeline/activate` | `{bundle_id, reason_code, attestations, expected_record_id}` | the routing row |
| `POST /v1/admin/pipeline/rollback` | `{bundle_id, reason_code, attestations, expected_record_id}` | the routing row |
| `POST /v1/admin/pipeline/contain` | `{reason_code, expected_record_id?, expected_state?}` | the routing row |
| `POST /v1/admin/pipeline/deactivate` | `{reason_code, expected_record_id?, expected_state?}`; `expected_record_id` is required for a contained tenant | the routing row |
| `POST /v1/admin/pipeline/policy-interventions` | `{bundle_id, phase, action, reason_code}` | the intervention record ("Suspend a policy") |
| `GET /v1/admin/pipeline/policy-interventions?bundle_id=...` | none | `{interventions: [...]}`, oldest first |
| `GET /v1/admin/pipeline/routing` | none | `{routing_state, activation_record_id, active_bundle_id, active_bundle_qualified_on_revision, events}`: `routing_state` and `activation_record_id` are null for a tenant with no row, `activation_record_id` is the record id of the routing row in force, `active_bundle_qualified_on_revision` says whether the active bundle has a qualification on the revision of the process that answers (null with no active bundle, and on a build with no revision), and `events` are the newest 100, newest first, read in one snapshot |
| `GET /v1/admin/pipeline/legacy-drain` | none | the drain report ("Legacy drain report") |

The routing row has the fields `routing_state`, `activation_record_id`,
`actor_principal_ref`, `reason_code`, `evidence_hash`, and `recorded_at`.
`attestations` are signed check results (see
[pipeline-qualification.md](pipeline-qualification.md), "Signed check
results"): at most 64 in one request (`413` `pipeline_evidence_too_large`).

The expectation of a routing change. Each routing change gives the routing row
a new record id (`activation_record_id`, which is also the `event_id` of the
change's event). Read it from `GET /v1/admin/pipeline/routing` (the field
`activation_record_id`), or from the answer of your last change, which is the
routing row. A change that names the id is a compare-and-set. The store
compares the id with the row that it reads under the tenant's routing lock,
before the readiness and the gate. If another change came first, the answer is
`409` `pipeline_routing_state_changed`, and nothing is written: no routing row,
no bundle change, no event. Read the routing again and decide again.

- `activate` and `rollback` need `expected_record_id`: the id that you read,
  or the string `"none"` for a tenant with no row (`activation_record_id:
  null` in the routing view). A body without the field is `422`
  `pipeline_request_invalid`, and so is a `null` or any other value: do not
  copy the `null` of the routing view into the body. These two routes do not
  take `expected_state`: a body that sends it is `422`
  `pipeline_request_invalid` (an unknown field). So a request that you
  prepared before another operator's `contain` cannot open the tenant's
  uploads again after that `contain` returned. The id is compared, not the
  state: a request that you prepared while the tenant was contained is also
  refused after the tenant was opened and contained again, and a rollback is
  refused after another operator's rollback.
- `contain` needs no expectation: an emergency stop must not need a read
  first. It takes `expected_record_id` (an id, or `"none"`), `expected_state`
  (`legacy`, `pipeline`, `contained`, or `none` for a tenant with no row), or
  both. When both are sent, both must hold. A value that does not parse, a
  `null` too, is `422` `pipeline_request_invalid`.
- `deactivate` takes the same two optional fields. For a tenant whose row says
  `contained`, it needs `expected_record_id`: send the record id; the state
  alone is refused. Without the id the answer is `409`
  `pipeline_routing_expectation_required`, and nothing is written, also when
  the body says `"expected_state": "contained"`. `expected_state` compares the
  state only, and the state is the same in every containment: a request that
  you prepared for one containment would also pass after the tenant was opened
  and contained again. When both fields are sent, both must hold. So a
  `deactivate` that you prepared for a `pipeline` tenant, or in an earlier
  incident, cannot send a contained tenant back to the legacy path. For a
  `pipeline` tenant the expectation is optional. Send `expected_record_id` on
  every planned change.

Five routes need a pipeline runtime in the build (`404` `pipeline runtime not
configured` without one): `qualifications`, `activate`, `rollback`, and the two
`policy-interventions` routes. The other four need only the routing store:
`routing`, `legacy-drain`, `contain`, and `deactivate`. So a process with no
runtime can stop a tenant's intake and can return a tenant to the legacy path.
It needs both, because it refuses the uploads of a tenant whose row says
`pipeline` or `contained`. A route that reads or writes routing needs the
routing store, which exists only with the database mirror (`404`
`pipeline_routing_store_missing`).

What each action does:

- `qualifications` records the signed package's qualification for the tenant on
  this build's code revision, and registers the package for the tenant (the
  package and its four policy rows, each runnable; a package that is already
  registered keeps its rows as they are, so a suspended policy stays
  suspended). It changes no routing.
  The rules are in [pipeline-qualification.md](pipeline-qualification.md).
- `activate` routes the tenant's new uploads to the pipeline with the bundle,
  from any state (no row, `legacy`, `pipeline`, or `contained`). It switches the
  tenant's active bundle for later runs. A run that exists keeps its bundle.
  The bundle can be the one that is already active: that activation changes
  only the state, and it is how a contained tenant is opened again.
- `rollback` selects an earlier bundle for a tenant whose row says `pipeline`
  or `contained`: one that an `activate` or a `rollback` of this tenant
  selected before, and that is not the active bundle now. The routing state
  does not change. A `pipeline` tenant stays `pipeline`, and its new uploads
  bind the earlier bundle. A `contained` tenant stays `contained`: the bundle
  changes and its uploads stay stopped. A tenant routed to `legacy`, or with no
  row, is `409` `activation_state_invalid`. A bundle that was never active for
  the tenant, or is active now, is `409` `earlier_qualified_bundle_required`. A
  rollback runs the gate below with its own fresh evidence, and reads no
  readiness. A run that exists keeps its bundle.
- `contain` stops the tenant's new uploads (`503`
  `pipeline_receipt_intake_contained`, see "Scope lists and the routing row",
  which also says what an upload in flight can still do).
  It works from any state, a tenant with no row included. It changes no bundle,
  no run, and no receipt that already has an owner. The worker keeps going:
  runs in flight, invalidations, withdrawals, payouts, and confirmations of a
  listed tenant still finish, and a retry of an earlier receipt is still
  answered.
- `deactivate` returns a tenant that has a row to the legacy path: the row
  says `legacy`. It changes no bundle. A tenant with no row, or whose row
  already says `legacy`, is `409` `activation_state_invalid`. A contained
  tenant needs `expected_record_id` in the body (`409`
  `pipeline_routing_expectation_required` without it, see above).

Every action writes one row to `pipeline_activation_events`, in the same
transaction as the routing row. The row's `activation_record_id` is the event's
`event_id`. The event holds the action, the previous and the
resulting state, the previous and the resulting bundle, the actor's
`principal_ref`, the reason, an evidence hash, and the time. For a tenant that
had no row, the previous state is `unselected` in the table and null in the
route's answer. The rows are immutable: a trigger refuses `UPDATE` and a direct
`DELETE`. They are the record of a routing change: the bundle, the reason, and
the evidence hash are there. Each action also adds one row to `main`'s audit
log, which names the actor, the time, and the action only (see "The audit row
of an action" below).

The evidence hash of an `activate` or a `rollback` names the change. It is the
hash of the canonical JSON of the evidence that the change used and the bundle
that it selected. For an `activate`, the JSON is `{"bundle_id", "promotion",
"readiness"}`: the bundle id, the evidence hash of the promotion decision, and
the hash of the readiness. For a `rollback`, which reads no readiness, it is
`{"bundle_id", "promotion"}`. The routing row holds the same hash. The hash of a
`contain` or a `deactivate` covers the action, the tenant, and the reason.

An admin action waits at most 5 seconds for a lock. The four routing changes
wait for the tenant's routing lock, which each receipt transaction of the
tenant holds for a moment. A policy intervention waits for its policy row. When
the wait is longer, the answer is `503` `pipeline_routing_busy` (a routing
change) or `503` `policy_intervention_busy` (an intervention), and nothing is
written. Send the request again. A receipt has no such limit: it never fails
because an admin action waits.

Each action also logs one line, `pipeline admin action recorded`, with the
tenant's storage reference, an action label, and the evidence hash. The line
is written when the change committed, before the audit row below. Each `GET`
route appends one control-plane read audit row (the surfaces `pipeline_routing`,
`pipeline_policy_interventions`, and `pipeline_legacy_drain`). A refused read
appends none. A reason that is not a label is `409` `activation_actor_invalid`.

The audit row of an action. Each of the six write routes (`qualifications`,
`activate`, `rollback`, `contain`, `deactivate`, and `policy-interventions`)
appends one row to `main`'s audit log after its change committed. A refused
request appends none. The row is an event of the kind `pipeline_activation`
in the tenant's audit file, and a `policy_update` row in the database whose
metadata has the surface `pipeline_activation`. It holds the actor's
`principal_ref`, the time, and one action label with the count 1:
`pipeline_qualify`, `pipeline_activate`, `pipeline_rollback`,
`pipeline_contain`, `pipeline_deactivate`, `pipeline_policy_suspend`, or
`pipeline_policy_resume`. Its purpose hash is the hash of that label. It holds
no bundle, no reason, and no record id: read those from the routing events, the
qualification, or the interventions. A repeated `qualifications` request that
answers the existing row appends a row too. `contain` and `deactivate` append
their row on a process with no runtime.

The row is appended after the change, not in its transaction. If the change
committed and the append failed, the answer is `500`
`pipeline_change_committed_audit_failed`, and the log has one error line
(`pipeline admin action committed and its audit row was not appended`) with
the tenant's storage reference, the action label, the evidence hash, the hash
of the error, and, for a routing change, `activation_record_id`: the record id
that the change put in force. The change is in place. Do not send the request
again: a second `contain` appends a second event, a second `deactivate`
answers `409` `activation_state_invalid` (or `pipeline_routing_state_changed`
when it names the record id), and a second `activate` or `rollback` answers
`409` `pipeline_routing_state_changed`. Read the routing (`GET
/v1/admin/pipeline/routing`) to see the state and the record id in force. If
that read answers `500` too, the tenant's audit chain cannot take a row: each
audited request of the tenant fails until the chain is repaired. Do the repair
in [audit-trail-forensics.md](audit-trail-forensics.md) (`POST
/v1/admin/audit-chain-repair`), and then read the routing. The repair restores
a file that is behind the database. After a database restore the file is
ahead, and the repair refuses (`409`, the label `file_head_not_in_db`): the
database mirror backfill makes the two level, see
[backup-restore.md](backup-restore.md), step 3. Until the routing read
answers, the record id in force is in the error line. A change that committed
while the chain could not take a row gets no audit row later, unless the
repair restores it from the database. Its record is its routing event (or its
qualification or intervention row), its `pipeline admin action recorded` line,
and the error line in the log. The uploads of the tenant follow the routing
row at all times: a containment that answered this label stops intake.

The ingest login's grants would let a direct statement write
`pipeline_tenant_routing` and append events (V110) and update the active bundle
(V112). The database enforces the record of such a write (V110): the routing
row must name an event of its tenant, each change of the row must name a new
event, and at the commit that event must have the row's state and the row's
generation (`routing_generation`, a counter that a trigger raises on each update). An event
matches one version of the row and cannot be named again. So every change of
the routing row, a direct one too, appends one matching event. The database
does not enforce the gate: a direct write can append an event and a matching
row with no qualification and no evidence, and it can append an event that no
row names. The row's `activation_record_id` says which event is in force. Only
the code runs the gate: it changes routing through these routes, and the bundle
through the gate. Change routing only through the routes. The details are in
[deployment.md](deployment.md), "V110 to V113".

### What the process needs

The two trust stores. `TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH`
names a JSON file of the keys whose signatures make a bundle package trusted.
`TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH` names a JSON file of the keys
whose signatures make a check result count. Each file is a JSON array, not a
single object, of `{"key_id": "...", "public_key_base64url": "..."}`: at least
one key, distinct key ids, and each a 32-byte Ed25519 public key.
`pipeline.py keygen` writes one such object, so wrap it (or several) in
`[...]`. Both files are read once, at start.

- A variable that is set refuses the start with `pipeline_trust_store_invalid`
  in these cases: the file cannot be read, it is not a non-empty JSON array of
  keys, it repeats a key id, a key id is not an identifier, or a key is not a
  32-byte Ed25519 public key. The message names the variable for the first
  three (a read or parse failure). It does not name the variable for a bad key
  id or bad key bytes in a well-formed file. No message names a path or a key.
- A check key that is also a package key, by public key under any id or by
  key id, refuses the start with `pipeline_trust_store_overlap`. The two
  stores are two key pairs on purpose: a key that signs packages does not
  vouch for check results.
- A variable that is unset leaves its store empty, and only then do
  `qualifications`, `activate`, and `rollback` answer `503`
  `pipeline_trust_store_missing`. A variable that is set to the empty string, or
  to blanks, counts as unset. A variable that names a file never gives that
  `503`: its file is read at every start, and a file that cannot be read refuses
  the start.
- `activate` and `rollback` use the package trust store too. A stored package
  has no signature of its own. The qualification verified the signature and
  recorded the id of the key that made it. The two routes refuse a bundle whose
  qualification on the running revision records a key id that the package
  trust store no longer holds: `409` `bundle_package_signer_untrusted`. So to
  stop a package key, remove it from the file and restart every process. The
  check is by key id: do not use the id of a removed key again for another key.

  A removed key closes `activate` and `rollback` for every bundle that the key
  signed, on the revision that runs. Such a bundle cannot be qualified again on
  that revision with another key. A qualification row is append-only, and it
  names the key that signed the package: a second `qualifications` call for the
  same bundle and revision with a package that another key signed is `409`
  `bundle_qualification_identity_conflict`. So plan a key rotation. The two
  routes stay closed for those bundles until you put the key back in the file
  (and restart), or you qualify the bundle, signed by the new key, on a new
  code revision and deploy that revision. Until then `contain` and `deactivate`
  still work.
- The check-signing key must not be held by anyone who holds the tenant's admin
  credential. Nothing in the server enforces this: the server cannot tell who
  holds a key. Who holds it is decided when the deployment is promoted. This
  repository's CI signs its `qualify` run with a key that the job makes and
  discards, which no trust store holds.

The deployed code revision. `TRACE_COMMONS_BUILD_CODE_REVISION_HASH` is a
build-time variable. Set it when you compile `trace-commons-ingest`, to the
output of `python3 scripts/operator/pipeline.py revision` for the same tree
(`sha256:` and 64 lowercase hex digits). The tool hashes the path and content
of every file in the checkout that git tracks, and of every untracked file that
the repository's own `.gitignore` files do not ignore, except the top-level
`.local`, `.vscode`, and `target` directories, and an untracked `.cargo`
directory at any depth (a developer's local cargo configuration). A `.cargo`
file that git tracks is part of the revision. A host's `.git/info/exclude` and
a user's global excludes file hide nothing from it, so one checkout gives one
revision on every host. So any edit, a document included, and any stray
untracked file, changes the revision, and the command needs a git checkout.
Qualify and build from the same clean tree.

A binary built without the variable has no revision: `qualifications`,
`activate`, and `rollback` answer `409` `bundle_runtime_revision_unknown`. A
variable that is set to the empty string, or to blanks, counts as unset, as an
empty trust store variable does. A binary built with any other value that is
not such a digest refuses to start: `pipeline_code_revision_invalid`. `GET
/v1/admin/config-status` shows whether the binary has a revision
(`pipeline_code_revision_configured`).

Nothing in this repository sets the variable for you. The revision matters only
for a production distribution's own build, which injects a pipeline runtime.
The binary that this repository builds, `cloudbuild.yaml` included, has no
runtime: `qualifications`, `activate`, and `rollback` answer `404` `pipeline
runtime not configured` there, before the revision is read, so a revision
changes no upload and no change route there. It changes one field of `GET
/v1/admin/pipeline/routing`: `active_bundle_qualified_on_revision` is null
without a revision. A distribution's build must pass
`TRACE_COMMONS_BUILD_CODE_REVISION_HASH` itself, with the value that
`pipeline.py revision` printed in a checkout of the tree it compiles. (This
repository's `.gcloudignore` drops `.git/` from the uploaded source, so a Cloud
Build cannot compute the value from its own tree.)

The infrastructure profile. The routes derive it from this process's own
configuration (the fields `GET /v1/admin/config-status` reports). A request
cannot change it. Only a production value counts as production, and a value the
process cannot read is not production. A bundle cannot be qualified or
activated while any of these holds:

- the database mirror is absent, or its writes are not required
  (`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`): `authoritative_metadata_not_production`,
  and `best_effort_database_mirror_enabled` for a mirror that is on and best
  effort;
- the artifact store is not a GCS store with object IO (the file-system and the
  local encrypted stores are development stores): `artifact_store_not_production`;
- the artifact store allows plaintext compatibility, or none is configured:
  `plaintext_fallback_enabled`;
- the key wrapper is not Cloud KMS (a local master key is a development
  wrapper): `key_wrapper_not_production`;
- signed tokens are not verified, or managed EdDSA tokens are not required
  (`TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS`):
  `authentication_not_production`;
- any static bearer token is configured: `static_bearer_authentication_enabled`;
- the verifier holds an HS256 key besides EdDSA keys:
  `hs256_bridge_authentication_enabled`;
- the pipeline runtime holds a payout whose settlement mode moves real money:
  `live_external_payout_enabled`. Only the mode `http`
  (`TRACE_COMMONS_NEAR_SETTLEMENT_MODE=http`, where the payout adapter pays)
  sets it. A payout in the mode `disabled` (nothing advances) or `dry_run` (an
  in-process adapter, no network and no funds) does not set it, and a runtime
  with no payout does not. In this release a live payout therefore blocks
  `qualifications`, `activate`, and `rollback` for every tenant of the process:
  such a deployment can change routing only with `contain` and `deactivate`.
  Whether a live payout must block these routes is the owner's decision at
  promotion.

The bundle's own dependencies add labels of their own, but only a
qualification shows them: `runtime_scorer_not_production`,
`runtime_embedder_not_production`, `runtime_index_reader_not_production`,
`runtime_index_writer_not_production`, `runtime_settlement_not_production`,
`runtime_authority_not_production`, `runtime_privacy_not_production`,
`runtime_payout_not_production`, and `bundle_configuration_not_qualifiable`. A
qualification answers (`409`) the first label that applies, in this order: the
bundle's own labels, then the four `_not_production` labels (metadata, artifact
store, key wrapper, authentication), then the flags (plaintext, best-effort
mirror, static tokens, HS256 bridge, live payout). An activation or a rollback
never reaches the bundle's own labels. Its startup checks (step 5 below) refuse
the same conditions first, as `pipeline_runtime_dependencies_not_production_qualified`,
and a dependency that the runtime does not hold as
`pipeline_tenant_bundle_dependency_missing`. For these two routes the answer from
the profile is one of the infrastructure labels above, in the same order.

### The order of checks

An activation or a rollback passes these steps in order. A refusal at any step
writes nothing.

1. The request. The body must parse (`pipeline_request_invalid`, with the
   parser's status above), the bundle id must have its shape (`422`), and the
   attestations must number at most 64 (`413`).
2. The scope. The tenant must be on this process's receipts list (`409`
   `pipeline_tenant_not_in_scope`). A tenant on the drain list only is not in
   scope. Without this check, an activation on a process that does not list the
   tenant made every new upload of the tenant `503`
   `pipeline_tenant_not_served` there. The check sees only the process that
   answers: list the tenant on every replica first.
3. The process. Both trust stores must exist (`503`
   `pipeline_trust_store_missing`), and the build must have a revision (`409`
   `bundle_runtime_revision_unknown`).
4. The evidence. Each attestation must verify against the check trust store:
   `check_attestation_invalid` (the shape is wrong, or a check appears twice),
   `check_attestation_signature_invalid`, or `check_attestation_signer_untrusted`
   (all `409`). An attestation whose maximum age is more than seven days is
   `409` `bundle_qualification_evidence_age_above_ceiling`. An attestation for
   a check outside the 22 required checks is `409`
   `qualification_evidence_invalid`. The server then evaluates the promotion
   over the verified results, now. The evidence of a bundle is the output of
   one `qualify` run plus the three promotion-only results. The 19 results of
   `qualify` must carry one run id, at `qualifications`, `activate`, and
   `rollback`: a set that mixes two runs is blocked with
   `qualification_evidence_mixed_run`. The three promotion-only results can
   come from other runs.
5. The package. The tenant must have a stored package of the bundle (`404`
   `bundle_package_missing`). A stored package that no longer validates (an
   altered package) is `409` `bundle_package_missing`. The key that signed the
   package must still be in the package trust store (`409`
   `bundle_package_signer_untrusted`, see "What the process needs"). The
   startup checks of a tenant bundle (see "Each tenant's bundles at startup")
   then run on the package with this process's configuration:
   `pipeline_tenant_bundle_dependency_missing`,
   `pipeline_tenant_bundle_not_runnable`,
   `pipeline_runtime_main_gate_config_mismatch`,
   `pipeline_credit_issuer_principal_missing`, and
   `pipeline_runtime_dependencies_not_production_qualified` (all `409`). The
   last one is the answer for any bundle that is not production qualified: a
   development dependency, or a configuration that is not qualifiable. The
   dependency profile is built next.
6. The routing lock and the expected record. One transaction does this step
   and the two after it. It waits for the tenant's routing lock, for at most
   5 seconds (`503` `pipeline_routing_busy`). It then compares
   `expected_record_id` with the record id of the tenant's routing row (`409`
   `pipeline_routing_state_changed`). A rollback then checks the tenant's state
   and the bundle (`activation_state_invalid`,
   `earlier_qualified_bundle_required`).
7. The readiness, for an activation only: `409` `activation_readiness_failed`.
   The route reads it before the transaction, and the transaction evaluates it.
8. The gate.

The readiness reads the tenant's pipeline operational summary
(`GET /v1/admin/pipeline/operational-summary`) just before the call. It passes
only when all of these hold:

- the tenant-isolation and audit-immutability controls pass
  (`tenant_isolation_control_passed`, `audit_immutability_control_passed`);
- `retryable_error_count` is 0: no run is in the state `retry`;
- no run in the state `pending`, `retry`, or `leased` has waited in its phase
  for more than 300 seconds;
- `held_credit_count` and `delayed_credit_count` are both 0;
- `pending_index_command_count` and `failed_index_command_count` are both 0;
- `pending_invalidation_count` and `failed_invalidation_count` are both 0.

So a tenant cannot be activated while any of these is true:

- a run is in `retry` (a suspended policy puts its runs there);
- a run has waited more than 300 seconds in `pending`, `retry`, or `leased`;
- a settlement leg is held, pending, in `retry`, or leased;
- an index write is pending or failed;
- an invalidation is pending or failed.

A run in `pending` or `leased` that is younger than 300 seconds, with no such
leg, index write, or invalidation, passes. Activate a tenant when it is quiet.

The readiness is strict, and it covers the whole tenant, not one bundle. While
a suspended policy holds any run of the tenant in `retry`,
`retryable_error_count` is not 0, and every activation of the tenant is
refused, for every bundle. So a fix-forward to a new bundle is not possible
while runs wait under a suspended policy. "Suspend a policy" gives the ways
forward. A rollback reads no readiness.

The gate holds when every term holds, in this order. Each `409` label names
the first term that fails:

1. The promotion is ready and carries no blocker
   (`bundle_activation_promotion_not_ready`). The answer names what blocks it:
   the body has a second field, `blockers`, with the decision's blockers as
   labels, for example `["qualification_evidence_stale:pipeline_crash_matrix"]`.
   A blocker about one check has the form `<label>:<check_id>`. A blocker
   about the whole set is a label alone, for example
   `qualification_evidence_mixed_run`. The
   qualification route answers its own refusal of such a decision the same way
   (`bundle_qualification_promotion_not_ready` with `blockers`). No other answer
   has the field. See "The result contract, and what makes a result invalid" in
   [pipeline-qualification.md](pipeline-qualification.md) for what a ready
   decision needs.
2. The promotion was evaluated at most 15 minutes before the gate, and not
   after it (`bundle_activation_promotion_stale`).
3. The build's revision is a `sha256:` digest
   (`bundle_runtime_revision_unknown`), and it is the revision of the results
   (`bundle_runtime_revision_mismatch`).
4. The tenant has a qualification of the bundle
   (`bundle_qualification_missing`), on the build's revision
   (`bundle_runtime_revision_mismatch`).
5. The stored package loads and validates (`bundle_package_missing` when it is
   gone, or when it no longer validates by the time the gate loads it), and the
   results name exactly its three digests (`bundle_activation_package_mismatch`).
6. The dependency profile was built for this bundle
   (`bundle_qualification_profile_mismatch`), has no blocker (the label of its
   first one: an infrastructure label from the list above), and its runtime
   identity is the one the qualification recorded
   (`runtime_dependency_identity_mismatch`).
7. The bundle's four policies (Admission, Review, Score, Settle) are runnable
   (`bundle_policy_not_runnable`).

Then the gate switches the active bundle, and the routing row and the event
are written in the same transaction. One qualification covers one code
revision. After a deploy to a new revision, every bundle of the tenant needs a
qualification on it, an earlier bundle included, before it can be activated or
rolled back to.

### After a deploy: the qualification is read again for each new upload

A deploy changes the code revision and changes no routing row. So each process
checks one term of the gate again, for each new upload of a tenant whose row
says `pipeline`: the tenant's active bundle must have a qualification row for
the revision that the process was built from.

- No such row: `503` `pipeline_bundle_not_qualified`.
- A build with no revision: `503` `bundle_runtime_revision_unknown`. No
  qualification can be recorded on such a build, so `qualifications` does not
  help: deploy a build that has a revision.

Both refusals come with the route decision ("Scope lists and the routing
row"): nothing is stored, the admission attempt is released, and no classifier
call is made. No state changes. The tenant's row still says `pipeline`, and
its intake returns when the qualification is recorded on that revision, with no
second `activate`. The process does not contain the tenant.

The check applies to a new upload only:

- A retry of a receipt that exists is answered from its run.
- A run that exists goes on with its bundle. The worker reads no qualification.
- A remediation of a legacy record stays on the legacy path and is not checked.
- A process that was started with
  `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` (tests only) skips the
  check, as it skips the gate.

What the check proves. It proves that a qualification row exists for the
tenant, its active bundle, and the revision of this build, and nothing more.
These terms of the gate are checked only by `activate` and `rollback`, at one
moment, on the process that answers the route:

- the promotion over fresh signed results (terms 1 and 2);
- the stored package and its digests (term 5);
- the dependency profile of the process, its infrastructure blockers, and the
  runtime identity that the qualification recorded (term 6);
- the Review, Score, and Settle policies (the route decision reads the
  Admission policy only).

So a tenant that is activated keeps taking receipts, and its runs keep scoring
and settling, in each of these cases:

- a restart with weaker settings (a development store, a static token, a live
  payout), which blocks a new qualification;
- a build of the same revision whose runtime dependencies differ from the
  ones that the qualification recorded;
- another replica whose settings differ from the replica that answered the
  activation.

No refusal and no log line shows any of them. After a restart with other
settings, read `GET /v1/admin/config-status` on every replica and compare it
with the replica that answered the activation.

Where the check is made. Only the route decision reads the qualification. The
receipt's own transactions bind the active bundle and check the routing and the
Admission policy again; they read no qualification. So one upload can start a
run on a bundle that is not qualified on its process's revision in this case
only: the deployment runs two revisions, and an `activate` or a `rollback`
switches the tenant's bundle between that upload's route decision and its
staging transaction. The window is one request in flight.

How to see it:

- `GET /v1/admin/pipeline/routing` on a process of the build answers
  `active_bundle_qualified_on_revision` for the credential's tenant: `true`,
  `false`, or null (no active bundle, or a build with no revision). A process
  answers for its own revision. The route needs no runtime.
- At start, a process that is not started with the test-only rule reads the
  same fact for each tenant on its receipts list, and logs one warning,
  `pipeline_active_bundle_not_qualified`, with the tenant's storage reference,
  for each listed tenant whose row says `pipeline` and whose active bundle has
  no qualification on the build's revision. A build with no revision and a
  receipts list that is not empty logs one warning in all,
  `pipeline_code_revision_unset`. A read that fails logs
  `pipeline_qualification_start_check_incomplete` for its tenant. All the
  reads together have a limit of 5 seconds, so the check holds the start for
  5 seconds at most: when the limit ends, the check stops and logs
  `pipeline_qualification_start_check_incomplete` one time, with
  `tenants_not_read`, the count of the listed tenants that it did not read.
  None of these stops the start: the upload path makes the refusal.

The deploy procedure. `qualifications` records a qualification for the revision
of the process that answers it, and a request cannot name another revision. So
the running fleet cannot record a qualification for the next revision. For
each deploy of a new revision B, and for each tenant whose row says `pipeline`:

1. Run `python3 scripts/operator/pipeline.py revision` on tree B, and build B
   with that value.
2. Run `pipeline.py qualify` on tree B for the package of the tenant's active
   bundle, and get the three promotion-only results for B. One run covers one
   package: a tenant on another bundle, and each earlier bundle of step 4,
   needs its own run (a set for another package is `409`
   `bundle_qualification_package_mismatch`). Use the 19 results of that one
   run: do not replace one of them with a result of another run
   (`qualification_evidence_mixed_run`). Sign the set shortly before you use
   it.
3. Start one process of build B with a runtime, both trust stores, and the
   production settings, outside client traffic. Leave both scope lists unset
   on it (`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` and
   `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS`): `qualifications` and `GET
   routing` check no list, and with no list the worker of this process drains
   no tenant. With the fleet's lists its worker would process the listed
   tenants' runs on build B before the roll (the worker reads no
   qualification).
4. Send one `POST /v1/admin/pipeline/qualifications` for each tenant and for
   its active bundle to that process, with that tenant's admin credential.
   Make sure that `metadata.code_revision_hash` in the answer is the revision
   of B. A request that reaches a process of revision A is refused with `409`
   `bundle_qualification_code_revision_mismatch` and records nothing: the
   results name revision B, and a process records a qualification for its own
   revision only. Then read `GET /v1/admin/pipeline/routing` on that process:
   `active_bundle_qualified_on_revision` must be `true`. Qualify each earlier
   bundle that you want to be able to roll back to with steps 2 and 4 for
   that bundle.
5. Roll the fleet to build B.

If you cannot do steps 2 to 4 before the roll, contain the tenant before the
deploy (`POST /v1/admin/pipeline/contain`, with `"expected_state":
"pipeline"`), or accept that its new uploads are refused until step 4 is done.
Open a contained tenant again with `activate` after the bundle is qualified on
the revision that runs. That `activate` names the record id of the containment
as `expected_record_id`: the `activation_record_id` in the answer of the
`contain`, or in `GET /v1/admin/pipeline/routing`.

A rolling deploy without steps 3 and 4. While the fleet runs both revisions,
an upload that reaches a process of B is refused with `503`
`pipeline_bundle_not_qualified`, and one that reaches a process of A is
accepted. After the roll, every new upload of the tenant is refused until its
`qualifications` request reaches a process of B.

A binary rollback to an earlier revision A. The qualification rows of A stay:
the table is append-only and holds one row for each bundle and revision. So
every bundle that was qualified on A is served again. A bundle that was first
qualified while B ran has no row for A: its tenant's new uploads are refused
on A until a full set for tree A is signed and posted to a process of A. A
build from before this rule does not make the check at all
([deployment.md](deployment.md), "Binary rollback to an older build").

A commit that changes only a document. The revision of the tree changes,
because every tracked file is hashed. The revision of a process is fixed when
its binary is built. So intake changes only when a binary that was built from
the new tree is deployed, and then steps 1 to 5 apply.

### Evidence age, rollback, and a first rollout

The 15-minute rule. Both the readiness (an activation) and the promotion
(an activation or a rollback) must be at most 15 minutes old at the gate. The
route evaluates both within the request, shortly before the gate, so these two
rules refuse a request that stalls for more than 15 minutes in between (waiting
for the tenant's routing lock, for example). They do not refuse a result that
is old. A result's own age is the
maximum age its signer gave it (see
[pipeline-qualification.md](pipeline-qualification.md)): a result older than
that blocks the promotion (`bundle_activation_promotion_not_ready`, with the
stale check in `blockers`). The three routes refuse a maximum age of more than
seven days (`409` `bundle_qualification_evidence_age_above_ceiling`): the
signer chooses the age, and the server bounds it on every route that verifies
signed results. Sign the set shortly before you use it.

Rollback and containment.

- After a deploy, a rollback needs the earlier bundle qualified on the deployed
  revision, which takes a full qualification run on that revision. So
  `contain` is the emergency stop, and rollback is the controlled path back:
  contain a tenant when you doubt it (in an emergency, with no expectation),
  and roll back once the earlier bundle is qualified on the revision that runs
  (with the `activation_record_id` that you read from `GET
  /v1/admin/pipeline/routing` as `expected_record_id`).
- A rollback changes the bundle and keeps the routing state. From `pipeline`
  the row stays `pipeline`. From `contained` the row stays `contained`: the
  earlier bundle is selected and the tenant's uploads stay stopped. A rollback
  never opens uploads. To open a contained tenant after a rollback, `activate`
  the bundle that is now active. That activation makes the readiness check.
- A routing change takes the tenant's routing lock exclusively. A receipt
  that is in its staging or commit transaction holds the lock shared, so a
  change waits for it (for at most 5 seconds, see above), and a receipt that
  starts afterwards sees the change. A receipt that staged before an
  activation and commits after it keeps the bundle it staged under.
- The same holds for a rollback. A receipt that staged under the bundle that
  you roll back from can commit a new run on that bundle after the rollback
  returned. The commit checks the routing state and that bundle's Admission
  policy. It does not check which bundle is active. To stop new runs on that
  bundle at once, suspend its Admission policy before the rollback ("Suspend a
  policy"): the commit then refuses the receipt.

The order of a first rollout:

Before each change of routing in this procedure, read the tenant's routing
(`GET /v1/admin/pipeline/routing`, the fields `routing_state` and
`activation_record_id`). Then send that record id as `expected_record_id` in
the body of the change, or `"none"` when the field is null. `activate` and
`rollback` are refused without it (`422` `pipeline_request_invalid`). The
answer of each change is the new routing row: its `activation_record_id` is
the id to send with your next change, if no other operator made a change in
between. If the answer is `409` `pipeline_routing_state_changed`, another
change came first: read the routing again before you decide. If the answer is
`503` `pipeline_routing_busy`, send the request again.

1. Build `trace-commons-ingest` with the revision of the tree, with a pipeline
   runtime assembly. Set both trust store variables. List the tenant on
   `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` on every replica and start every
   replica. Every replica that has a runtime must list the tenant before step 3
   ("Run one build and one configuration" says why). With no routing row, the
   tenant stays on the legacy path.
2. Qualify. Produce a full set of signed results on the revision that
   runs, and `POST` it with the signed package to `qualifications`.
3. Activate one tenant: `POST` a fresh set to `activate`, with
   `"expected_record_id": "none"` for a tenant that has no row yet. A request
   activates one tenant, the credential's. Activate another tenant with its own
   credential, after the first one has run clean.
4. Watch the tenant. Read `GET /v1/admin/pipeline/operational-summary` (work by
   state, `retryable_error_count`, `terminal_error_count`,
   `suspended_policy_count`, the index and invalidation counts, the NEAR outbox
   by state) and `GET /v1/admin/pipeline/routing` (the events).
5. Contain on doubt: `POST` `contain`, with the `expected_record_id` that you
   read when you have time to read the routing first. In an emergency, send
   `contain` with no expectation: it then works from any state. New uploads
   are refused with `503`, and the worker keeps processing what the tenant
   already has.
6. Roll back or deactivate, each with the `expected_record_id` that you read.
   `rollback` selects an earlier qualified bundle for later runs and keeps the
   routing state: a contained tenant stays contained. To open it again,
   `activate` the bundle that is now active, with the record id that the
   rollback answered. `deactivate` returns the tenant to the legacy path. A
   `deactivate` of a contained tenant without `expected_record_id` is refused
   (`409` `pipeline_routing_expectation_required`). Then move it to the
   drain list as "Scope lists and the routing row" says.

A fix-forward is an activation of a new bundle (steps 2 and 3 for that bundle).
It needs the readiness. It is refused while a suspended policy holds a run of
the tenant in `retry`: see "Suspend a policy".

## Suspend a policy

An operator suspends one policy of one bundle of the tenant, and resumes it
later. A suspension is the lever for a bound policy that has become unsafe: the
runs bound to it keep their package.

`POST /v1/admin/pipeline/policy-interventions` takes `{bundle_id, phase,
action, reason_code}`. `phase` is `admission`, `review`, `score`, or `settle`.
`action` is `suspend` or `resume`. The answer is the intervention record:
`intervention_id`, `bundle_id`, `phase`, `action`, `actor_principal_ref`,
`reason_code`, `previous_status`, `resulting_status`, `evidence_hash`, and
`recorded_at`. `GET /v1/admin/pipeline/policy-interventions?bundle_id=...` lists
the records of one bundle, oldest first. The record is in
`pipeline_policy_interventions` and is immutable. The operational summary counts
the tenant's suspended policies as `suspended_policy_count`.

| Answer | Meaning |
|---|---|
| `409` `policy_intervention_not_supported` | the action is `terminate`: the status exists in the schema, and no behavior is defined for a run bound to a terminated policy, so it is refused and writes nothing |
| `409` `policy_intervention_no_transition` | a `suspend` of a policy that is not runnable, or a `resume` of one that is |
| `409` `policy_intervention_invalid` | an unknown action, a malformed bundle id, or a reason that is not a label |
| `404` `bundle_package_missing` | the tenant has no policy row for that bundle and phase |
| `503` `policy_intervention_busy` | the intervention waited 5 seconds for its policy row (a phase commit, an index dispatch, or another intervention holds it) and wrote nothing: send it again |

What a suspended policy shows:

- A new upload under a suspended Admission policy of the tenant's active bundle
  is refused with `503` `bundle_policy_not_runnable`, and nothing is stored. The
  route decision refuses it, so the admission attempt is released ("Scope lists
  and the routing row" says what happens to an upload that is in flight when the
  suspension commits).
- A run bound to a suspended Review, Score, or Settle policy waits in `retry`
  with the label `bundle_policy_not_runnable` (the operational summary shows it
  as the work item's reason). The retry is uncharged: the wait does not count
  against the run's attempts. The run keeps its bundle and goes on from what it
  has stored after the resume.
- An activation of a bundle with a suspended policy is refused (`409`
  `bundle_policy_not_runnable`).
- While a suspended policy holds any run of the tenant in `retry`, every
  activation of the tenant is refused, for every bundle (`409`
  `activation_readiness_failed`). The readiness counts each run in `retry`, and
  a run older than 300 seconds in its phase. This check stays strict (owner
  decision). So a fix-forward to a new bundle needs one of these first:
  - Resume the suspended policy. Its waiting runs then run under it, each after
    its backoff. Activate the new bundle when no run is in `retry` and the other
    readiness counts are clear.
  - Keep the policy suspended, and contain or deactivate the tenant. Its new
    uploads stop, or go to the legacy path. The waiting runs stay, and no
    activation is possible until the policy is resumed.

  A rollback reads no readiness. It needs a bundle that the tenant selected
  before and whose four policies are runnable.
- The runs that wait under a suspended Settle policy keep their index commands
  in the neighbour set of each later compatibility Score of the tenant, for
  every bundle. So a later near-duplicate of such a run gets no novelty award
  during the suspension, and none at all if the policy is never resumed.

What a suspension stops. Each phase's commit, in every policy state it guards:
Admission at the receipt, Review, Score, and Settle. In Settle: a new
selection, the index write, each leg's external call, and the credit event with
its settlement batch. And a payout dispatch (a submit to the NEAR adapter): a
payout leg waits while its Settle policy is suspended, and a payout that is
already submitted is still confirmed, because a confirmation is a lookup and not
a dispatch.

What it does not stop:

- An external effect that already runs. A guard does not retract an accepted
  operation: an index write that holds the policy row finishes first, a leg's
  adapter call that was accepted stays accepted, and a NEAR submit already made
  stays made. A Settle commit holds the policy row only for its own short
  transaction, so a suspension refuses a commit and stops nothing that has
  started; the run retries and resumes from its stored selection.
- The failure and withdrawal paths. They write no outcome and run in every
  policy state.
- The index rebuild. It upserts the index entries of complete runs in every
  policy state, because it replays committed Settle effects.

Three things an operator must not assume:

- `resume` does not wake a suspended run at once. The run retries on its backoff:
  the age of its phase, from one second up to one hour.
- A suspension of a Settle policy can meet an index dispatch. A dispatch holds
  the run, the submission, and the policy row until its write commits, for at
  most the earlier of the Settle lease's end and 30 seconds (the dispatch
  budget). The intervention waits for the row for at most 5 seconds. Then it
  answers `503` `policy_intervention_busy` and writes nothing. Send it again
  until it returns the record: overlapping Settle transactions of one bundle can
  hold the row for longer than one dispatch.
- Do not read "suspend has returned" as "no dispatch will start". Two short
  transactions lie between a dispatch's guard read and its external submit.

## Legacy drain report

`GET /v1/admin/pipeline/legacy-drain` shows what the legacy path still owes the
tenant for the receipts it took. It is a report. `drained` disables nothing,
and this release has no route that retires a legacy writer.

The cost of the report grows with the tenant's whole history. Each count reads
every submission, batch, or outbox row of the tenant, in one snapshot, on a
pooled connection that other tenants share. Each statement can run for at most
30 seconds. A statement that runs longer is cancelled, and the route answers
`503` `legacy_drain_report_timeout` with no report (never a partial one). Read
the report when the database is quiet. Do not poll it.

The answer has these fields: `generated_at`, `routing_state` (the tenant's row
in the same snapshot, null with none), `gate_driver_enabled`, `pending`,
`not_blocking`, `drained`, and `evidence_hash`. `pending` holds ten counts,
zeros included. `drained` is true only when all ten are 0. A count is the
selection of the legacy worker that does the work, limited to what the pipeline
does not own: by submission with no pipeline run for the first seven, by
payout instrument (none) for the two NEAR counts, and tenant-wide for the
withdrawals. A count that cannot copy its worker exactly counts more, never
less.

| Count in `pending` | What it counts | The legacy work that clears it |
|---|---|---|
| `awaiting_pii_backstop` | submissions in the status `awaiting_pii_backstop` | the PII backstop's verdict |
| `gate_decision_pending` | with the gate driver on: submissions with an active submitted envelope and no gate decision, below the driver's attempt ceiling | the gate driver, or a `POST /v1/workers/gate/evaluate` that names the submission |
| `gate_decision_exhausted` | with the gate driver on: the same, at or above the ceiling | a person: reset the attempt row or accept the loss |
| `quarantine_review_pending` | quarantined submissions | a reviewer's decision |
| `vector_index_pending` | the current precheck records of accepted submissions that have no active vector entry | the vector index worker |
| `delayed_credit_unsettled` | positive credit events of the four settlement-eligible types on accepted submissions that no finalized settlement batch names | credit settlement |
| `revocation_propagation_pending` | revocation propagation items `pending`, `in_progress`, or `failed` | the revocation propagation worker |
| `near_outbox_pending` | `main`'s NEAR outbox rows `pending`, `failed`, or `submitted`, for the whole tenant | the NEAR submitter and confirmer |
| `near_payout_unqueued` | line items of finalized `main` settlement batches that have a NEAR contract and no outbox row: a payout held for want of a payout target, or an outbox row that is missing | enrolling or designating a payout target, then a live settlement run |
| `withdrawal_completion_pending` | submissions of a withdrawn source session whose withdrawal is not complete, for the whole tenant, a version with a pipeline run included | the revocation propagation worker |

Read these rules before you trust a number:

- `near_outbox_pending` counts a row until the NEAR workers confirm it. A
  tenant with a NEAR contract cannot read `drained` before they have run.
  `near_payout_unqueued` is 0 for a deployment that never had a NEAR contract on
  a batch.
- `withdrawal_completion_pending` stays above zero until the revocation
  propagation worker has run for the tenant. Its in-process scheduler covers
  only the tenant of its own token. A legal hold keeps its attachments out of
  the count (`TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES`).
- Several counts can stay above zero with no legacy code that clears them. A
  person has to act:
  - `gate_decision_exhausted`, and `awaiting_pii_backstop` after its attempts
    run out;
  - `quarantine_review_pending`;
  - `revocation_propagation_pending`, for an item that a crashed run left
    `in_progress` (the worker never lists it again);
  - `delayed_credit_unsettled`, for an account with a credit hold or for credit
    that a ranking gate or the per-account cap holds back;
  - `vector_index_pending`, for a record that a consent check refuses;
  - `near_outbox_pending`, for a line `submitted` with no transaction hash;
  - `near_payout_unqueued`, for an account with no payout target.

The gate driver has two modes, and `gate_driver_enabled` says which one you read.
The default deployment has the driver off.

- Driver on (`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED`): `gate_decision_pending`
  and `gate_decision_exhausted` are counted as above, and they block `drained`.
- Driver off: both read 0, and `not_blocking.gate_decision_absent` is the number
  of legacy submissions that have an active submitted envelope and no gate
  decision, whatever their attempts. It never changes `drained`. So
  `drained: true` with the driver off does not mean that those submissions have
  a decision. They have none. It means that no legacy code owes one. A gate run
  also awards `NoveltyUtility` credit, so the report does not ask for one.
  `POST /v1/workers/gate/evaluate` can still score a submission that its caller
  names, in either mode. To get the decisions, turn the driver on and read the
  report again.

The route reads the gate driver setting of the replica that answers. Run one
gate driver setting on every replica, or two replicas answer in two modes. The
`evidence_hash` belongs to one mode: the same rows in the two modes hash
differently, so compare hashes only within a mode.

What the report does not count, because none of it is follow-up that the legacy
path owes for a receipt it took. This list is the owner's, and nothing proves it
complete:

- retention and purge: time-driven maintenance over every submission, which
  covers the pipeline's submissions too;
- export jobs: snapshots that an operator requests and completes;
- benchmark and process-evaluation work: it runs when an operator calls its
  route, with the inputs of the request;
- the database mirror backfill: it copies file records into the tables, as an
  operator's maintenance over the file store;
- `/v1/workers/utility-credit`: it takes its submission ids from its caller, so no
  table lists what it still has to do.

The precondition. The report reads the database tables only. It is a drain
signal only when the tenant's legacy records in the database are authoritative,
and the route checks that this process is configured so:

- the legacy path's database writes are required
  (`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`, or account admission is on), so a
  failed write fails the legacy operation instead of leaving a file-only
  record; and
- the tenant's review, credit, settlement, and outbox reads come from the
  database (`TRACE_COMMONS_DB_REVIEWER_READS`, or the tenant on
  `TRACE_COMMONS_DB_REVIEWER_READS_TENANT_IDS`).

A tenant that fails this is refused with `409`
`legacy_drain_records_not_authoritative`. The check reads this process's
configuration. It does not read the tenant's history. A record that exists only
in the file store, from before the writes were required, is owed work that the
report cannot see, and it reads zero with `drained` true. Run the database
mirror backfill for the tenant before you trust the report. The call is `POST
/v1/admin/maintenance`, with the tenant's admin credential and the body
`{"backfill_db_mirror": true, "prune_export_cache": false}`. It is a retention
call, and the backfill is only one of its effects. With `dry_run` false it also:

- marks the tenant's file records that are past their `expires_at` as expired
  (a record on a legal hold is left alone), sets each one's final credit figure
  (0 when it had none), and mirrors the expiry to the database;
- marks the file records that a tombstone covers as revoked, and mirrors the
  revocation;
- marks derived records revoked or expired to match;
- invalidates the export provenance and the benchmark artifacts that name a
  revoked or expired source;
- prunes the export cache, unless the body says `"prune_export_cache": false`;
- appends a maintenance audit event and writes a retention job record, with
  an item for each change. A dry run appends the event and the job record too,
  marked as a dry run.

It purges nothing unless the body names `purge_expired_before`. With exactly
this body the call is a backfill-only request, and it leaves the pipeline's own
submissions alone. A call with another field set, or without `"prune_export_cache":
false`, also expires the pipeline's submissions. Send the call
first with `"dry_run": true` added, and read the counts in the answer
(`records_marked_expired`, `records_marked_revoked`, `derived_marked_expired`,
`derived_marked_revoked`, `export_provenance_invalidated`,
`benchmark_artifacts_invalidated`, `db_mirror_backfilled`,
`db_mirror_backfill_failed`). Send it again without `dry_run` only when you accept
those changes. If `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN` is set, the
body must also say `"reconcile_db_mirror": true`, and then it is no longer a
backfill-only request.

The report runs as the ingest runtime login in one read-only, single-snapshot
transaction (`REPEATABLE READ`): nine or ten statements, one for each count the
mode takes, each a `COUNT` filtered by the tenant, with no row lock and no write. It
reads these tables: `pipeline_runs`, `pipeline_tenant_routing`,
`trace_submissions`, `trace_object_refs`, `trace_gate_decisions`,
`trace_gate_evaluation_attempts`, `trace_derived_records`,
`trace_vector_entries`, `trace_credit_ledger`,
`trace_credit_settlement_batches`, `trace_near_credit_outbox`,
`trace_revocation_propagation_items`, `trace_withdrawals`,
`trace_submission_sessions`, `trace_source_sessions`, and
`trace_token_attachments`. Eleven of them are older than V62, and the login holds
`SELECT` on those only through the pilot's V62-era table grants (see
[deployment.md](deployment.md)). V90 grants the three session and attachment
tables, and V92 and V110 grant `pipeline_runs` and `pipeline_tenant_routing`. A
deployment without the V62-era grants answers with a `500`, not with a zero. Each call appends one control-plane read audit row.

The rehearsal runs the legacy work through the legacy routes and reads the
report at each step:
`tests::pipeline_activation_pg_tests::the_legacy_drain_report_counts_real_pending_work_and_reaches_zero`
(see "Rehearse the switch").

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

This check is scoped to one bundle, not to every dependency the service
happens to hold (decision P4-D7): `pipeline_runtime_is_production_qualified`
inspects only the dependencies `PipelineService::default_package()` --
the package this service registers as every rollout tenant's active bundle
-- actually uses: the scorer and embedder it names, the held index reader
and writer, one settlement-adapter check per instrument the package pins,
the authority provider, and the privacy boundary. A non-production-qualified
scorer, embedder, or settlement adapter the default bundle never touches
(for example, a scorer registered for a different bundle that is not yet
the active one) does not block startup.

The NEAR payout adapter is the exception: it is checked whenever payout is
enabled, whatever the default package pins. Payout is service-wide, like
the index writer: the payout pass pays the complete runs of every bundle,
including runs bound to an earlier default package. An unqualified payout
adapter with payout enabled refuses startup under the same conditions as
any other unqualified dependency.

The same per-bundle check, with payout, is available as
`PipelineService::bundle_qualification` for any package, which is what the
`pipeline_bundle_qualification` required check
(`qualification_inspects_the_objects_the_constructor_receives`) exercises.

## Per-phase claim lease

Each pipeline phase claims its run under its own lease length, sized for how
long that phase can actually run rather than one fixed lease every phase
shared. Score, in particular, runs the injected scorer and embedder inside
the lease -- a chunked NEAR AI perplexity scorer or a CPU-bound embedder can
exceed a short lease on the pilot.

The scorer, the embedder, the index, and the object store are synchronous,
so the worker calls them on the blocking thread pool, never on a runtime
worker thread that ingest's HTTP routes share: Score's whole evaluation,
Settle's index writes, the invalidation pass, and every object read, write,
and delete of a phase.

Settle's index writes run in a task of their own that holds the run and
submission rows locked until the write commits, so a Settle that is stopped
(a shutdown past its grace period) does not release them while the writes
go on. The rows are held at most until the Settle lease's end or 30 seconds
after the write starts, whichever is earlier: past that, the write rolls
back, the rows are free, and once the index call in flight returns the run
waits in `retry`, uncharged, as `index_unavailable`. The index cannot see
the locks, and a lost database session or a process exit still releases
them during a write. So the writes also stop at that deadline: no index call
starts after it, and the index writer must return from each call within 60
seconds. A withdrawal that finds an unfinished write on a run whose lease is
still live queues the revision's removal no earlier than that lease's end
plus those 60 seconds.

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

`max_attempts` (5) is a budget per phase, not per run. Each claim charges
one attempt; the Review commit that approves a run, and the Score commit,
reset `attempt_count` to 0. So Review, Score and Settle each get the whole
budget, and a phase that commits on its last attempt leaves the next phase
claimable.

While a phase runs, a background task renews its lease: every
`max(lease / 3, 100ms)`, it extends the live claim's lease, stopping as soon
as the phase ends, the lease is lost (reclaimed by someone else, or already
cleared), or renewal reaches its cap. The cap is
`PIPELINE_LEASE_RENEWAL_CAP_FACTOR` (4) times the phase's own configured
lease, measured from the moment the phase claimed the run: an honest phase
that is merely slow keeps being renewed, but a phase that never comes back
at all still surrenders its claim within a bounded multiple of its own
lease rather than being renewed forever. The commit fences
(`ensure_current_lease`, `ensure_live_lease`, every lease-checked `UPDATE`)
stay the only authority over what a phase is allowed to write; renewal only
keeps an honest slow phase from being reclaimed out from under it before it
finishes. Renewal closes only the second gap above, and only for a live
worker: it renews its own lease before `lease_expires_at` passes, so no
other worker reclaims a phase that is merely slow. The first gap stays: a
crashed worker still records nothing. The second gap reopens when a live
worker's lease expires anyway -- its phase runs past the renewal cap, or
its renewal does not get to run before the lease expires.

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
are routed or drained (the section above). With
`TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` set, ingest also refuses to start a
runtime whose privacy boundary does not run a prose-PII classifier, or that
has none (`pipeline_privacy_filter_required`), as `main` refuses to start
with no filter backend. This is judged by what the boundary does, not by
whether it reports itself qualified. The classifier-backed boundary
(`ClassifierRedactorPipelinePrivacyBoundary`) is built with its adapter's
backend tag, as `main` pairs them (`TRACE_PRIVACY_FILTER_BACKEND`). Over the
no-op adapter, which is the `none` backend's, it neither classifies prose
PII nor counts as qualified, so both refusals apply to it.

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
  (`quarantine reason is unresolved`). A stale claim is `409`. The route
  applies `main`'s privileged-action consent check, as `main`'s review
  decision does: when the reviewer's credential or the tenant's current
  policy allows none of the submission's consent scopes or uses, it answers
  `403` and records nothing. A policy narrowed after the receipt applies.

An assessment moves the run back to `pending`, due at once, in the same
transaction. The worker then runs Review with the assessment: an approval
continues to Score, a rejection ends the run.

The claim and assessment routes append their audit rows after the claim or
the assessment commits. When that append fails, the route still answers the
committed result (the lease token, the assessment id), and logs
`pipeline_review_audit_append_failed` with the tenant's storage reference, a
hash of the run id and the route (`claim` or `assessment`). The audit trail
then has no row for that claim or decision; the decision itself is in
`pipeline_review_assessments`.

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
- `artifact_store_unavailable` (Review and Score): an object-store call of
  the run failed for a transport or availability reason -- Review's source
  read or approved write, or Score's approved read, its object keys'
  derivation, or its object writes. When the store reached the object and
  found it missing (a missing file on the local and file stores, a 404 from
  Google Cloud Storage) or not what its receipt names (a hash or reference
  mismatch, a decode or decrypt failure, a wrapped data key the key wrapper
  cannot decode, size or authenticate), the attempt is charged instead, as
  `artifact_integrity_failed`, and the phase's attempt budget ends the run.
  Charged attempts under this label are one hour apart, not the short
  backoff of the other charged labels: with the default budget of 5
  attempts, the run fails about four hours after the first failure. A store
  configuration fault (a root that is not mounted, a wrong
  `TRACE_COMMONS_ARTIFACT_KEY_HEX`, a wrong bucket) looks like an integrity
  failure, so correct the store within that time; a run that fails is not
  put back. This time holds for a run in Review or Score only.
  Any other Google Cloud Storage fetch failure (credentials, network, 429,
  5xx), a key-wrap service call that fails, and a record wrapped by another
  kind of key wrapper (what a key-provider migration shows) wait here,
  uncharged, retried at most once an hour. A cloud KMS that refuses a
  corrupt wrapped key answers through that same call, so on a cloud KMS
  that case waits uncharged too: the client cannot tell a refusal from an
  outage.
  Settle's read of the stored index command is always charged
  (`index_command_invalid`), a store failure of that read included, with
  the short backoff: a run in Settle can fail about one second after such a
  fault, and its open legs are then forfeited.
- `serialized_json_object_key_unavailable` and
  `pipeline_attempt_object_key_mismatch` (compatibility Score): the same
  rule, under the store's own label -- a store that cannot derive an object
  key, or one that prepares an object under a key other than the one it
  derived (see "The attempt artifact sweep" below). The store, not the
  trace, is at fault, so neither is charged.

An amount above a configured cap is different: the cap refuses the payment,
the leg fails as `credit_cap_exceeded`, and the attempt is charged.

The index holds only revisions of runs that completed. A run that fails for
good at Settle after its index write may have written entries (the write is
`pending`, `complete`, `failed`, or `cancelled`) -- a crash after the write on
its last attempt, or legs that exhaust its attempts -- queues the
invalidation of its revision in the transaction that fails it, through the
queue a withdrawal uses (reason `run_failed`); a `pending` write is
cancelled and the run excluded. The worker's invalidation pass removes the
entries.

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
  pipeline run. Otherwise it takes `main`'s path. On a build with no
  runtime injected, that path still queues the pipeline's follow-up for
  each withdrawn submission with a pipeline run, through the database,
  after the tombstones and before the bytes: the revision's invalidation
  (reason `withdrawn`), a payload deletion per live object, and the end of
  its runs' work. A runtime processes them when it runs.

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
revoked as before, and, when the submission has a pipeline run, then make
the pipeline's follow-up in one transaction (with no runtime injected,
through the database, for a later runtime to process): the export snapshot invalidations and payload deletions above,
the run's index invalidation (reason `revoked`), and the release of a run
parked in `awaiting_review`. Settle reads the revoked status and forfeits
every leg it has not completed. `main`'s completion of a source-session
withdrawal on its own path -- at an account merge confirm, for a version the
merge joined to a withdrawn session, and in the revocation-propagation
worker's reconciler -- makes the same follow-up for a version with a
pipeline run (reason `withdrawn`), so that version leaves the reconciler's
incomplete list once completed. The follow-up runs before `main` deletes the
version's content. A follow-up that fails (a database error, for example)
therefore delays that deletion until the reconciler's next pass, which
retries both; the log line is `Trace Commons source-session withdrawal
completion failed; the next reconcile retries it`. The order is deliberate:
the reconciler retries only versions that are still incomplete, so deleting
the content first would leave a failed follow-up with no retry.

`main` marks the submission in one transaction and the follow-up runs in
another, so a process that stops between the two loses the follow-up. The
worker recovers it: once a minute for each tenant (and on the tenant's
first pass after a start), it finds up to 32 of the tenant's revoked or
withdrawn submissions, in submission id order starting just after the last
one the previous run read and wrapping round to the lowest (the position is
kept in memory, so a restart begins at the lowest), with a run whose index
write started and no queued
invalidation, or with an export snapshot item that is not invalidated (a
run that Settle keeps out of the index has no index work, and a snapshot
can still hold it). It makes the follow-up for each (reason `withdrawn`
when a withdrawal row exists, `revoked` otherwise, actor
`pipeline_worker`); the invalidations it queues are processed in the same
pass. So a lost follow-up waits at most about a minute once a worker runs.
The read checks every revoked or withdrawn submission of the tenant on each
run, recovered or not, which is why it does not run on the 10-second
invalidation step. A listed tenant that has no pipeline run is answered from
one read of `pipeline_runs`; its submissions are not read. One follow-up that fails is logged as
`pipeline_lost_follow_up_failed` (with the tenant's `tenant_storage_ref`
and a hash of the submission id), does not stop the others of the pass,
and is retried when a later run wraps round to it. Because each run starts
after the last one, 32 or more follow-ups that fail every time cannot hold
the window: the submissions after them are reached on the next run. A recovery that recovers nothing because
of a failure is logged as `pipeline_worker_lost_follow_up_recovery_failed`
and retried a minute later.

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
by building the service with a payout (`PipelineServiceBuilder::with_payout`),
and a service built without one has payout disabled; the payout
configuration has no other switch (`main`'s settlement mode still applies,
below). With payout disabled, nothing is submitted to NEAR.

- Payout takes only a complete run's `trace_credit` legs that have the
  `near` payout rail and a settlement batch. A compatibility bundle's
  `NoveltyUtility` leg has no batch, so it is never paid, as on `main`.
  Other instruments never use the NEAR outbox.
- Score records a leg's payout state when it adds the leg. A run scored
  while payout was disabled has payout `disabled` and is never paid, even
  after payout is turned on. Only a Trace Credit leg that settles into a
  batch is seeded `pending`; any other instrument on the `near` rail is
  `disabled`. A leg is paid only when Score marked it payout-eligible
  (`payout_eligible`, V105): a leg seeded by earlier code, whose batch line
  has no account settlement key or hold, is never paid, and V109 marks the
  payout of such a leg, of any instrument, `disabled` where it read
  `pending`.
- A leg Settle completed and ledgered is paid even when its run later fails
  for good (attempts exhausted, or a crash before Settle's own commit on its
  last attempt), as a withdrawal does not stop it either. The contributor
  status then reads the leg's payout state.
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
  | `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` | applied at every payout | Ingest hands the mode to the runtime and refuses one that holds another (`pipeline_runtime_near_payout_controls_mismatch`). `disabled` (the default): no outbox row is written and nothing is submitted or confirmed; each leg stays `pending`, as `main`'s rows do. `dry_run`: the full outbox state machine runs in process, with synthetic transaction hashes from each call's idempotency key, no network and no funds, and the injected adapter is not called. A leg `dry_run` confirms ends `confirmed` for good, as on `main`: a later switch to `http` does not pay it, because the payout skips a `confirmed` leg. Use `dry_run` only for legs that need no real payment. `http`: the injected adapter pays. A line is confirmed only in the mode that submitted it (recorded in its stored call as `pipeline_submission_mode`): after a switch between `http` and `dry_run`, a line the other mode submitted stays `submitted` until that mode returns, so a synthetic hash never replaces a real one. A `submitted` line with no recorded mode (code from before this rule submitted it) reads as `http`: `http` confirms it, and `dry_run` leaves it `submitted`. A build of `main` from before this rule also submits lines with no recorded mode. Such a line that `dry_run` submitted there stays `submitted` after the upgrade and is not confirmed; it never reached NEAR, so no money moves. Before a change between `http` and `dry_run`, stop the worker and check that no pipeline outbox line is `pending` or `failed`: the mode is recorded only after the submit, so a line in those states may have reached NEAR, and the other mode would submit it again as its own. |
  | `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH` | refuses an enabled payout on an adapter without a credential | As `main` refuses to start its NEAR adapters without their bearer tokens, whatever the mode: `near_payout_adapter_auth_missing`. The runtime must hold the same flag (`pipeline_runtime_near_payout_controls_mismatch`). |
  | Credit holds (`credit_holds`) | applied at Settle, to settled legs only | A held principal's leg that settles into a batch (the minimal family's `accepted` event) is `held` and is not settled, as `main` leaves held accounts out of its batches and payouts. The `accepted` event is not one of `main`'s settlement-eligible event types (benchmark conversion, regression catch, training utility, ranking utility); the pipeline batches that leg itself. A compatibility run's `NoveltyUtility` leg ignores holds and writes its ledger row, as `main` writes `NoveltyUtility` credit regardless of holds; that event never settles or pays. |
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
  is paid within one interval after the contributor enrolls or designates a
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
  status, the outbox line as `failed` in the pipeline operational summary's
  `near_outbox_by_state` (`GET /v1/admin/pipeline/operational-summary`), and
  the settled credit itself unchanged. `main`'s operational summary, its
  promotion gates, and the rollout-smoke readiness built from them read
  `main`'s outbox lines only, as `main`'s outbox listing does, so a failed
  pipeline line never holds them. The payout stays
  `failed`: nothing in this release takes it up again. A later release adds
  an operator retry route. A failed submit may still have reached NEAR, so
  until then check a `failed` payout's outbox line against NEAR by hand.

The pipeline operational summary also reports two controls from the
database catalog. `tenant_isolation_control_passed` checks that every
pipeline table enables and forces row-level security with the tenant
policy, that no other permissive policy applies to the reading role (one
for another role, such as `trace_gate_driver`'s, does not), and that the
role cannot bypass row-level security. `audit_immutability_control_passed`
checks that both of `phase_outcomes`' immutability triggers exist, fire for
ordinary sessions as row triggers before the update or delete, on every
column and with no `WHEN` condition, and call
`reject_phase_outcome_mutation`. Neither checks a function's body: the
database owner can replace any function, so a replaced body is outside what
a health check can show.

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
`export_snapshot_invalidated_create_new_snapshot` once the snapshot is
invalidated, and `export_snapshot_stale_create_new_snapshot` when one of its
submissions passed its expiry date or was revoked and no follow-up has
invalidated the snapshot yet. Every snapshot that holds a submission is
invalidated, a delivered (`complete`) one included, by a withdrawal, by a
revocation through `main`'s routes, and by `main`'s retention when it
expires or purges the submission; the item records which (`withdrawn`,
`revoked`, `expired`, `purged`). Each create and each delivery
appends one hash-only `export` audit event; a refused request appends none.

## Compatibility credit

The compatibility bundle reproduces `main`'s gate-path credit. Its
configuration is validated as `main` validates its gate at startup: a
production-compatible configuration with every floor zero is refused
(`compatibility_zero_floor`), and a configuration with at least one positive
floor is accepted. The pilot template
(`deploy/pilot-gcp/ingest.env.template`) sets a perplexity floor of 0, a
tail-fraction floor of 0 and a novelty floor of 500000, which is accepted. A runtime that routes or
drains a tenant must bind a qualifiable configuration: the local reference
configuration (all floors zero) fails the qualification gate
(`pipeline_runtime_dependencies_not_production_qualified`) unless
`TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is set. The gate reads it
as the bundle qualification's configuration term
(`bundle_configuration_not_qualifiable`), the same one `qualify_bundle`
refuses a package on, so a signed package whose configuration is not
qualifiable cannot be recorded as qualified either. The configuration holds
`main`'s gate configuration, as
ingest parses it, when a pipeline runtime is assembled: the three floors
(`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS`,
`TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS`; with a floor unset, no
compatibility bundle matches), top-k (`TRACE_COMMONS_GATE_TOP_K`, 5 unless
configured), the four chunk knobs (`TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS`,
`..._CHUNK_MAX_TOKENS`, `..._CHUNK_CAP`, `..._CHUNK_MIN_TOKENS`), the
index-insert threshold (`TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS`,
50000 unless configured; Score inserts a chunk into the index under it, never
under the novelty floor), and the `NoveltyUtility` delta (below). Ingest hands
them to the runtime as one value and refuses one whose default package, or
any bundle a routed or drained tenant may run (see "Each tenant's bundles at
startup"), holds any other (`pipeline_runtime_main_gate_config_mismatch`). When both gate
floors pass, Score awards the `NoveltyUtility` delta to `trace_credit`, and
Settle records it as one `NoveltyUtility` ledger event, written as `main`
writes that event: settlement state `final`, actor role `vector_worker`, the
actor the pipeline's issuer (`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`,
where `main` records its issuing gate worker), and the reason
`novelty_utility:compatibility_quality_novelty_v1`. A runtime that routes or
drains a tenant through the compatibility bundle refuses to start without
that issuer (`pipeline_credit_issuer_principal_missing`), and with no issuer
the leg is withheld (`credit_check_error`, below): the contributor never
stands in for the issuer. That event type does not settle on `main`, so the
pipeline never batches or pays it, and the contributor status reports the
leg as `not_settlement_eligible`.

Every credit event a Trace Credit leg writes records the witness provenance
label `main` records (`unattested` when the submission has no verified
witness evidence; nothing when the evidence cannot be read, as on `main`).
The worker then appends `main`'s hash-only `CreditMutate` audit event for it
through `main`'s audit log, at once on the replica that settled the leg and
within 10 seconds on any other: the event's id is the credit event's, its
actor is the issuer in the role `vector_worker` (for a minimal-family
`accepted` event, the pipeline worker, role `system`), and its metadata
holds the event type, the delta, and hashes of the reason and the source
key. The leg is marked audited (`credit_audited_at`) once the event is
appended. An event the worker cannot append, mirror or verify is logged as
`pipeline_worker_credit_audit_item_failed` (with the tenant's
`tenant_storage_ref` and a hash of the event id); its leg stays unmarked and
is tried again on each pass, and the tenant's later events are not held
back. When the database refuses the mirror of an event that is in the file
log only on each pass, the tenant's database audit chain is past that event:
run the audit-chain drill and follow
[audit-trail-forensics.md](audit-trail-forensics.md); the audit-chain repair
route restores file lines from database rows and does not add the missing
database row. Such a leg keeps its place in each pass, which takes the
tenant's 32 oldest unmarked legs: 32 refused legs of one tenant stop its
later `CreditMutate` events. This can occur only with a database mirror that
is not required.

A credit hold on the contributor does not stop this event, as it does not on
`main`: holds gate settlement batches and payouts only.

Near-duplicate traces earn once. A compatibility Score runs one at a time per
tenant, across every worker and replica: it holds a per-tenant advisory lock
from its neighbour read through its commit, and it counts as neighbours both
the index and the index commands of the tenant's runs that Score committed and
Settle has not applied yet (a run still to settle, its submission operable, no
invalidation queued). So of several near-duplicate traces received together,
only the first earns `NoveltyUtility` and gets index entries; each later one
completes with no award and no index entries, as on `main`, whose gate inserts
a trace's entries before it scores the next one. A tenant's compatibility
Score throughput is therefore one Score at a time, across every replica. A
Score that finds the lock held does not wait for it: its run waits in `retry`,
uncharged, as `score_lock_busy`, for 2 seconds, and that replica ends the
tenant's batch for the pass and goes on to its other tenants. Other tenants
and the other phases are not serialized.

A Score that cannot read one of those unapplied index commands fails closed:
its run waits in `retry`, uncharged, as `index_unavailable`, since leaving
the command out could credit a near-duplicate twice. Until that command can
be read, or its run settles or fails, every compatibility Score of the tenant
waits the same way. The read never selects a run whose command was removed on
purpose (its submission withdrawn, revoked, purged or expired, its revision's
invalidation queued, or the run failed for good). The worker logs
`pipeline_unapplied_index_command_unreadable` with `run_ref_hash`, the
SHA-256 of the run id's text, and the tenant's `tenant_storage_ref`. Find
the run in one session. Set the tenant first: `pipeline_runs` forces row
security, so without it a role that is not a superuser gets no row and no
error.

```sql
SELECT set_config('trace_commons.trace_tenant_id', '<tenant>', false);
SELECT run_id FROM pipeline_runs
 WHERE tenant_id = '<tenant>'
   AND 'sha256:' || encode(sha256(run_id::text::bytea), 'hex') = '<run_ref_hash>';
```

`<tenant>` is the routed or drained tenant for which
`'tenant_sha256:' || left(encode(sha256('<tenant>'::bytea), 'hex'), 32)`
equals the logged `tenant_storage_ref`.

The Score holds its transaction (and the lock) open while the scorer and the
embedder run, so the ingest login must not have an
`idle_in_transaction_session_timeout` (or a `statement_timeout`) shorter than
the Score lease (`TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SCORE`, 30 minutes by
default). A shorter timeout ends the transaction mid-Score; the Score then
fails as `database_unavailable`, uncharged, and is retried without end.

Before Settle writes the ledger event, it applies `main`'s `NoveltyUtility`
credit checks, in `main`'s order. A check that refuses the credit withholds
the leg: the leg completes with no ledger event, its `last_error_label` is
`main`'s reason, and the contributor status reports `withheld` with that
reason. The Score decision does not change, and a withheld leg is not a
charged Settle error. The exception is a leg an earlier Settle attempt
already dispatched (its adapter answered `Unavailable`, the process stopped
after the call, or the Settle policy was suspended while the call was in
flight, so the effect may have happened): a check that withholds it later
fails it as `settlement_unreconciled` instead, with no further adapter call.
That retry is charged, the run fails when Settle's attempts run out, and the
leg waits for an operator to reconcile it against the adapter's records by
`operation_ref_hash`, as for any other `settlement_unreconciled` leg.

| Check | Withheld as |
|---|---|
| With `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE` set, the runtime's scorer and embedder must be production-qualified (where `main` requires a production gate service). | `non_production_gate` |
| With `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` set, the pipeline's issuer, `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`, must be on that list (where `main` checks its calling gate worker). With the list set and no pipeline issuer, every award is withheld. With the list unset, every award passes this check, as on `main`. | `central_issuer_denied` |
| The pipeline's issuer must be configured (with the list set and no issuer, the row above withholds first). The tenant's authority must exist, and so must its policy when the authority requires one. The policy comes from the runtime's authority provider, the source the receipt uses. | `credit_check_error` |
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
document with contributor reads from files and from the database alike, so
its Trace Credit award reads as points in both (`main` keeps no ledger event
of the `accepted` type its leg writes).

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
`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` at run time; at startup,
ingest requires the package's delta to equal that variable times 1,000,000
(a points delta of `2.5` is `2500000`), as part of the gate configuration
above. The default is `0` in both places: no award, no settlement leg, and no
ledger event. A different delta is a different package, with its own bundle
id, and it applies only to runs bound to that package; to change it for a
routed tenant, follow "Each tenant's bundles at startup".

### Each tenant's bundles at startup

A tenant keeps its first active bundle when the default package changes
(registration activates the default package only for a tenant that has no
active bundle), and a run stays bound to the bundle of its receipt. So
before ingest serves, it checks every bundle a worker may run for each
routed or drained tenant -- its active bundle and the bundle of each run not
yet `complete` or `failed` -- as it checks the default package, and refuses
to start on the first failure:

| Check | Refused as |
|---|---|
| The runtime holds the scorer and embedder the package names. | `pipeline_tenant_bundle_dependency_missing` |
| The package is a policy family the runtime runs. | `pipeline_tenant_bundle_not_runnable` |
| A compatibility package holds `main`'s gate configuration (floors, top-k, chunk knobs, index-insert threshold, delta). | `pipeline_runtime_main_gate_config_mismatch` |
| A compatibility package has the pipeline's issuer configured. | `pipeline_credit_issuer_principal_missing` |
| The package passes the default package's qualification gate, unless `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is set: every dependency it names is production-qualified (the runtime may hold one that is not, as long as its default package does not name it), and a compatibility package's configuration is qualifiable. | `pipeline_runtime_dependencies_not_production_qualified` |
| The tenant's bundles can be read. | `pipeline_tenant_bundle_unreadable` |

The activation and rollback routes run the same checks on the bundle they
select, with the process's own configuration, before their gate (see "The order
of checks" in "Activate, roll back, contain, deactivate"). The gate holds no
service. So the routes are what keeps these checks true for a bundle that is
selected after start.

To change a value the default package carries (a floor, the threshold, the
delta, the scorer or the embedder), every routed or drained tenant must
leave its old bundle first, since startup refuses the new configuration
while a tenant can still run the old one. For each such tenant:

1. Under the old configuration, deactivate the tenant
   (`POST /v1/admin/pipeline/deactivate`, with the `activation_record_id` that
   you read from `GET /v1/admin/pipeline/routing` as `expected_record_id`),
   move it from the
   receipts list to the drain list, and wait until the operational summary
   shows no run of it in flight. Deactivating first matters: a tenant whose row says `pipeline`
   and that is only moved off the receipts list is refused with `503`
   `pipeline_tenant_not_served`, and is not sent to `main`'s path.
2. As the database owner, remove its selection. The table forces row-level
   security, also for its owner, so a plain `DELETE` with no tenant set deletes
   no row and reports no error (`DELETE 0`). Set the tenant in the same
   transaction:

   ```sql
   BEGIN;
   SELECT set_config('trace_commons.trace_tenant_id', '<tenant>', true);
   DELETE FROM pipeline_active_bundles WHERE tenant_id = '<tenant>';
   COMMIT;
   ```

   Make sure that the `DELETE` reports `DELETE 1`. With `DELETE 0`, step 3
   still finds the old bundle and refuses the start. (A superuser, or a role
   with `BYPASSRLS`, needs no `set_config`.) The runtime login has no `DELETE`
   on the selection. It does hold `UPDATE (bundle_id, selected_at)` (V112),
   which the code issues only in the activation gate, for a bundle that is
   qualified on the running revision. Do not change a selection with a direct
   statement of the ingest login.
3. Start ingest with the new configuration and the tenant back on the
   receipts list. Startup finds no old bundle to check for it, registers the
   new default package, and selects it as the tenant's active bundle. The
   tenant's routing row still says `legacy`, so its uploads stay on the legacy
   path. Qualify the new bundle on the new build's revision and activate the
   tenant again (`POST /v1/admin/pipeline/qualifications`, then `POST
   /v1/admin/pipeline/activate` with `expected_record_id` set to the
   `activation_record_id` that the deactivation of step 1 answered and that
   `GET /v1/admin/pipeline/routing` shows).

A tenant left on the drain list for good keeps no active bundle and runs no
new receipt, so step 3 may leave it on the drain list.

## Retention of pipeline submissions

`main`'s retention maintenance covers the submissions with a pipeline run,
which are in the database only. Every trigger of it does: the retention
scheduler (`TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_*`), the retention
worker route (`POST /v1/workers/retention-maintenance`), and the admin
maintenance route (`POST /v1/admin/maintenance`). Each run reads the tenant's
pipeline submissions from the database and applies `main`'s rules with the
same request:

- A submission whose expiry date has passed is marked `expired`, unless its
  retention policy is on a legal hold
  (`TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES`). An expired submission is
  marked `purged` only by a run with a purge cutoff (`purge_expired_before`,
  from the request or the scheduler's setting) later than its expiry date,
  and not while it is on a legal hold.
- A dry run counts what it would change and changes nothing.
- A purge request applies `main`'s privileged-action consent check to every
  purge candidate, pipeline submissions included: one the tenant's policy
  does not allow refuses the whole request with `403`.
- The rows are marked through `main`'s own expiry and purge writes, with
  `main`'s lifecycle audit, invalidations, and retention ledger items, and
  are counted in the maintenance response as `main`'s are.

Before `main` marks a row, the pipeline's follow-up is queued in the
database, in one transaction: the export snapshots that hold the submission
are invalidated (reason `expired` or `purged`), the index invalidation of its
runs' revisions is queued (reason `retention_expired` or `retention_purged`)
for the pipeline worker, and a run parked in `awaiting_review` is released.
An expiry deletes no payload, as `main`'s does not. A purge also invalidates
the submission's object refs and queues one payload deletion per live object
(reason `pipeline_retention_purge`) for `main`'s revocation-propagation
worker, as a pipeline withdrawal does; the maintenance response counts no
deleted file for it. This works with no pipeline runtime injected: a runtime
processes the queued invalidations when it runs.

`main`'s rollback-flag drill (`POST /v1/admin/rollback-drill`) leaves the
database-only rows of pipeline submissions (the submissions and their
tombstones) out of `db_submissions_not_in_file_fallback` and
`db_tombstones_not_in_file_fallback`, as the DB reconciliation below does,
and reports them as `pipeline_db_only_submission_count` and
`pipeline_db_only_tombstone_count`, which never block. As there, they are
found only while a pipeline runtime is injected. `main`'s replay export with
database replay reads (`TRACE_COMMONS_DB_REPLAY_EXPORT_READS`) leaves the
pipeline submissions out of its sources: they are exported through pipeline
snapshots.

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

### The attempt artifact sweep

A second, parallel table, `pipeline_attempt_artifacts` (V108), stages the
objects a phase attempt writes mid-phase -- Review's approved revision,
and Score's index command and neighbour set -- the same way
`pipeline_receipt_artifacts` stages the receipt's envelope. Who owns
deleting which row is a fixed split (controller ruling R2-1):

- A `staged` row's object is named by no object ref yet, so no withdrawal
  can ever reach it. `PipelineService::sweep_attempt_artifacts` owns it: on
  each pass, for up to 32 of the tenant's `staged` rows whose
  `cleanup_after` has passed, oldest first, it deletes the object (skipping
  the delete only when the store confirms the object is already absent) and
  then the row. `cleanup_after` is set when the row is staged, to
  `PIPELINE_LEASE_RENEWAL_CAP_FACTOR` times that phase's configured lease
  plus one hour of margin for the commit to land -- the same bound a live
  lease renewal is capped at, so an attempt that is still legitimately
  renewing its lease never has its own object swept out from under it. A
  delete failure logs `pipeline_attempt_sweep_delete_failed` (with the
  store's own refusal label beside it, `store_label`, when the store gave
  one) and keeps the row for the next pass. A kept row keeps its
  `cleanup_after`, but it does not stop the pass: the pass goes on to the
  next due rows, a page at a time, until it has removed 32 rows or examined
  128 (four for each row it may remove). The worker keeps where the pass
  stopped, and the tenant's next pass resumes there; a pass that reaches
  the last due row makes the next one start over at the oldest. So however
  many rows are kept, kept rows never stall a tenant's sweep. A pass
  removes at most 32 rows and examines at most 128, so clearing N due rows
  takes between about N / 128 passes (most rows kept) and N / 32 passes
  (most rows removed). The position is held in the worker's memory only: a
  restarted worker starts over at the oldest. One pass is one database
  transaction: it examines up to 128 rows and makes up to about 256 object
  store calls (a presence check or key derivation, then a delete, for each
  row), while it holds a pooled connection and the row locks of the rows it
  has read.
- A `committed` row's object is an object ref of the submission, recorded
  by the same phase commit that committed the row. Deleting it belongs to
  the withdrawal, not this sweep: a withdrawal invalidates the object ref
  and queues its payload deletion in the same transaction as the tombstone
  (see "Withdrawal follow-ups and index invalidation" above), and `main`'s
  revocation-propagation worker deletes it. This sweep never touches a
  `committed` row.
- A phase attempt whose commit is refused, for any reason (an inoperable
  submission, a stale lease, a missing settlement adapter), deletes the
  objects it wrote itself, best effort (Review its approved object, Score
  its index command and neighbour set), and so does a Score attempt whose
  second write fails after its first. Its `staged` row stays either way;
  this sweep later finds the object already absent and drops the row with
  no delete, or deletes an object that path failed to clean up. The
  objects only this sweep deletes are those of an attempt that stopped
  after writing and before any commit or refusal -- a crashed process --
  and those kept when the connection was lost during the commit, which
  may have landed, and did not.

Review and the minimal bundle's Score stage each row just before they
write its object, with the object's ciphertext hash. A compatibility Score
stages its two rows earlier, before it takes its tenant's Score lock (see
"Compatibility credit" above), so that it holds one database connection at
a time while it holds the lock. At that point the object key
is already fixed -- the store derives it from the tenant, the run, the
attempt's lease token and the artifact -- but the content, and so its hash,
is not. Those rows are staged with no hash:

- The Score commit sets each written row's hash, from the object ref it
  records, as it moves the row to `committed`, and deletes the row of an
  artifact the Score did not write (a duplicate at Score writes no index
  command). A `committed` row always has its hash; V108's guard lets only
  the commit set a missing hash. Review stages its `approved` row with its
  hash, and V108 refuses an `approved` row without one, so a row with no
  hash only ever names a compatibility Score's object.
- Every commit that records an attempt's object must move exactly the
  `staged` row that names it: the same object key, and no hash yet or the
  same hash. The Score commit moves one row for each object it wrote; a
  Review approval moves its one `approved` row, and a rejection moves none;
  a receipt's final transaction moves its one receipt row. Anything else --
  the row gone, or naming another object or hash -- refuses the commit as
  `pipeline_attempt_artifact_missing`. The lease was live when the commit
  checked it, and the sweep removes a row only past any lease the attempt
  could hold, so this is an out-of-band change or a defect, not a lease
  expiry: a phase records it as a charged retry under that label (it ends
  in `failed`/`attempts_exhausted` if it persists), and the refusal deletes
  the attempt's objects like any other.
- For a due `staged` row with no hash, the sweep first derives the key
  again, through the store, from the row's artifact, run and lease token.
  Only when the row's key is that key does it delete whatever object is
  stored there, and then the row. It compares no hash, and needs none to
  pick the object: the key carries the attempt's own lease token, so no
  other object is ever stored there. If the attempt wrote nothing, the
  store answers that nothing was there and the row goes. A row whose key
  is not its derived key is kept, the object at that key is not touched,
  and the sweep logs `pipeline_attempt_sweep_key_mismatch` on each pass.
- A compatibility Score publishes nothing after the latest moment its lease
  could still be live (four leases after its rows were staged), so an object
  is never written after its row could have been swept. It stops as an
  uncharged `lease_expired` instead.
- A compatibility Score that finds its tenant's Score lock held
  (`score_lock_busy`, see "Compatibility credit") has staged its two rows
  and published nothing. The transaction that releases its run deletes
  them, so a busy try, which repeats every 2 seconds while another Score of
  the tenant runs, leaves no rows for this sweep.
- The artifact store must be able to derive an object key before the
  content exists and delete at a key alone. The local store, the
  filesystem-remote provider and the GCS provider can. Three labels name a
  store that cannot, and only the first, and a key mismatch, stop a Score:
  - `serialized_json_object_key_unavailable`: the store cannot derive a
    key. A compatibility Score stops before it scores, and its run waits in
    retry under this label without being charged, as for any other failed
    store call (`artifact_store_unavailable`, ruling FR3). So does a Score
    whose store prepares an object under a key other than the one it
    derived (`pipeline_attempt_object_key_mismatch`); it publishes nothing.
  - `artifact_delete_at_object_key_unavailable`: the store cannot delete at
    a key. A compatibility Score still runs and publishes.
  - `remote_trace_artifact_delete_at_key_unavailable`: the remote provider
    behind the service-owned store cannot delete at a key. The service-owned
    store still derives keys, so a compatibility Score runs and publishes as
    usual.

  The sweep is affected in each case: for a due row with no hash, the store
  refuses with that label, and the sweep keeps the row and logs
  `pipeline_attempt_sweep_delete_failed`, with that label as `store_label`,
  on each pass that reaches it, until the store is fixed. The sweep goes
  on past such rows to later ones (see the first bullet above), so the
  tenant's other due rows, hashed rows included, are still swept. For the
  last two labels, a Score itself is not affected.

Score's withdrawal rule follows from the same split: withdrawing a
submission whose run already committed Score deletes the index command and
neighbour set through the ordinary object-ref invalidation path above, the
same as Review's approved revision -- never through the attempt sweep. A
run withdrawn before Score commits has nothing there yet; a run whose Score
attempt staged an object and then stopped before any commit or refusal -- a
crashed process -- leaves that object to the attempt sweep, not to any
withdrawal, because no object ref names it yet. A refused Score commit
deletes its own object itself, the same as any other refusal; the sweep's
part there is only to remove the row once it finds the object already gone.

## Submission quota at switch-over

The pipeline counts only pipeline receipts against the hourly submission
quota. It does not count legacy submission records. In the first hour after a
tenant moves to the pipeline, that tenant can therefore receive up to one
extra hourly quota. This is accepted behavior.

The legacy quota and tombstone checks still run before ingest routes a
receipt to the pipeline. A tenant with recent legacy submissions can
therefore receive a legacy 429 for a pipeline receipt.

## Rehearse the switch

The switch has four exact tests, two in the runtime suite and two in the
ingest binary. Each needs a PostgreSQL test database: use a fresh database
named `admission_test_<name>` on the literal host `127.0.0.1` for each command,
and set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to it. Run the ingest binary's tests
with `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` set too (a login that is a
member of `trace_login_resolver`, see
[login-resolver-role.md](login-resolver-role.md)), and single-threaded, because
its tests share a fixture tenant. Then run these
commands:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- \
  rollback_selects activation_requires
cargo test -p trace-commons-server --bin trace-commons-ingest -- --test-threads=1 \
  containment_refuses_new_receipts the_legacy_drain_report_counts_real_pending_work_and_reaches_zero
```

The tests cover:

- `activation_requires_every_term_of_the_gate`: each term of the activation gate
  refuses on its own, with its label, and a refused activation writes no
  routing row, no event, and no active bundle.
- `rollback_selects_an_earlier_bundle_for_new_runs_only`: a rollback selects an
  earlier bundle for new runs, and the runs that exist keep their bundle and
  their outcomes. A rollback to the active bundle, or to one that was never
  active for the tenant, is refused. A contained tenant rolls back to the
  pipeline. It emits the check `pipeline_activation_rollback`.
- `containment_refuses_new_receipts_and_keeps_pending_work`: with two replicas, a
  receipt taken before containment stays answerable on both and the worker
  completes it while the row says `contained`. A new upload through either
  replica is a `503` that writes no record, no ownership row, and no run, and
  it is accepted when the row says `pipeline` again. It emits the check
  `pipeline_activation_containment`.
- `the_legacy_drain_report_counts_real_pending_work_and_reaches_zero`: the legacy
  work is done through the legacy routes (a review decision, gate evaluation,
  the vector index worker, and credit settlement), the report counts what is
  pending at each step and reaches zero, the pipeline's own runs do not count,
  and the legacy writer still takes a receipt afterwards. It emits the check
  `pipeline_legacy_drain`.

The other tests of these areas run in the same two targets: mixed legacy and
pipeline receipts and their replays across every switch, the ownership races,
the route guards, policy suspension instead of rebinding, and the rebuild fence.

[`pipeline.py qualify`](pipeline-qualification.md) runs ten exact tests from the
`versioned_pipeline_runtime_pg` suite, four from the ingest binary, and one from
the library as required database checks, among them the three above that emit the
activation checks; `pipeline.py test --check postgres` runs the whole suite. Corpus and package
evidence stays in local files; these PostgreSQL integration tests remain
separate schema and recovery checks.

## Current completion

The versioned pipeline contracts, runtime, compatibility capabilities,
qualification tooling, and activation are in this repository: `pipeline.py
test`, `run`, `package`, `restore-drill`, `qualify`, `keygen`, and `revision`
(see [pipeline-qualification.md](pipeline-qualification.md)) exercise the
pipeline against a real PostgreSQL server, and the nine admin routes above
qualify, activate, roll back, contain, and deactivate a tenant and suspend a
policy. Production routing is off until an operator activates a tenant. The
repository binary injects no runtime, so it serves no tenant on the pipeline
and no route of it can write a `pipeline` row. With a database it reads the
routing row: it refuses the uploads of a tenant whose row says `pipeline` or
`contained`, and every other upload takes the legacy path.

What remains for promotion:

- An activation through the route needs a full set of 22 verified results: the
  19 that `qualify` produces and the three promotion-only checks
  (`pipeline_production_adapters`, `pipeline_remote_restore`,
  `pipeline_hf_network_canary`). Those three need the production assembly, no
  code in this repository emits them, and a local run's restore drill result
  carries the blocker `filesystem_restore_local_only`. So no tenant can be
  activated with the results of a local `qualify` run alone.
- The production assembly (a production scorer, embedder, index, settlement
  adapter, and payout), a remote object-store restore drill, and the Hugging
  Face network canary are promotion work. So is the choice of who holds the
  check-signing key.
- `terminate` of a policy is not supported. The specification that says what a
  terminated policy does to a run opens later, with promotion.
- No legacy writer is retired. The drain report shows what the legacy path still
  owes a tenant, and nothing acts on it.
- CI's `pipeline qualification and restore` job is not a required check. It
  signs its `qualify` run with a key that the job makes and discards. No trust
  store holds that key, so the signed set of a CI run activates nothing.

`SCR-005` stays deferred until the external valuation protocol exists. New
valuation rules use a later bundle through the same qualification and
activation process.
