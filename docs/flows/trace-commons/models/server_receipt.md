# Receipt acceptance, routing and storage

This is a finite current-behavior abstraction of baseline
`8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. Typechecking establishes syntax,
types and effects only. No product invariant, temporal property, guarantee test,
TLC run or exhaustive exploration has been selected or performed here.

The model covers the pipeline library's receipt protocol and its ingest replay
and response boundary. `TheAuthenticatedRequestArrives` starts **after** HTTP
authentication and upload-admission authorization. It does not represent a public
unauthenticated endpoint. Auth-derived tenant, principal and role are fixed in
the request and cannot be changed by its body.

## Variables and bounds

- `s.requests`: four lifetime request/attempt identities (`0..3`), independently
  in flight. Each identity owns its own staging row and object key. They may
  share tenant/key/bytes, or race with a different raw byte identity.
- Two tenants, two principals, three idempotency keys, two submission ids per
  tenant, two immutable bundle ids. `slot(t,k) = 3*t+k` scopes every key,
  owner, quota and source-session identity to its tenant. Contributor, Reviewer
  and Admin are modeled roles; Reviewer/Admin share `can_review` ownership
  access and do not count toward receipt quota.
- `bytes` is the immutable raw request hash class (`0,1`). `sourceBytes` is a
  distinct post-deterministic-rescrub identity class (`2,3`), represented by the
  fixed mapping `bytes+2`; this mapping illustrates byte identities rather than
  implementing the redactor or its possible many-to-one outputs.
- `s.receipts`: durable records indexed by tenant/key, including original
  principal, raw byte identity, stored-source identity, source attempt/object
  pointer, pinned bundle and synchronous Admission result.
- `s.owner`: durable submission routing ownership, separate from the routing of
  **new** receipts in `s.routing`. Legacy, pipeline and contained routing are
  explicit; the initial qualified runtime serves both tenants via pipeline.
- `s.activeBundle` can change while an attempt is writing. The staging
  transaction snapshots it into the request; neither activation nor rollback
  edits that request's bound bundle or an existing receipt.
- `s.runnable` is the tenant/bundle Admission policy intervention flag.
  `controlsPresent` joins authority lookup/read success and privacy-control
  availability at submission entry. These represent configuration/operational
  inputs, not measured deployment state.
- `usage` and `usagePrincipal`: a single symbolic hourly window, tenant cap 2,
  principal cap 1. Staging charges once per tenant/key; failures and cleanup do
  not refund it. The model bounds the window rather than deleting old ledger
  rows or implementing wall-clock rollover.
- `sourceWithdrawn` and `tombstones` are irreversible input events. A source
  session is represented by tenant/principal (one session per principal here),
  rather than a full account/session digest pair.
- `phase` describes transient request work; `staged`, `object` and `due` are
  independent. There can be a row without an object, an object before receipt,
  or an object whose row was swept while publication was in flight.
- `frame` and `outcome` are review-panel projections of the latest event, not
  durable database fields. All panels retain independent operator API controls;
  per-object guards decide eligibility. Navigation is not an authorization
  restriction on background server work.

## Source mapping and action meanings

All paths below are relative to the repository root and line numbers refer to
this pinned baseline.

| Actions / fields | Exact implementation anchors | Modeled behavior |
| --- | --- | --- |
| `TheAuthenticatedRequestArrives`; fixed tenant/principal/role | `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:16069-16105`, `3982-4028`, `63708-63710` | Ingest constructs the service request from `TenantCtx`, counts Contributor quota, and permits cross-principal receipt reads only for a reviewing role. HTTP uses `submission_id.to_string()` as the key. |
| `ThePrecheckReturns`; `controlsPresent` | `crates/trace-commons-server/src/versioned_pipeline.rs:11385-11440`, `11725-11785`; `crates/trace-commons-server/src/versioned_pipeline_authority.rs:16-45` | Resolve authority and require privacy control before write-path replay or any staging. Read failure/missing control fails closed. Early refusals do not stage or write. |
| `refusal`; replay bytes/owner | `crates/trace-commons-server/src/versioned_pipeline.rs:11654-11714`; `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:16126-16162`, `15660-15696` | Same key plus same raw bytes replays; other bytes conflict. The library result and subsequent ingest ownership check are projected into one caller-visible result. |
| `TheCompletedAdmissionRetryReadsTheExistingReceipt` | `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:15660-15696`; `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/admission.rs:541-607`; `crates/trace-commons-server/src/versioned_pipeline.rs:11654-11714` | Completed-admission retry is a read-only lookup: no new quota, source-session/tombstone/routing check or rescrub. Its triggering event assumes the upstream admission ledger reported a completed reservation; ledger internals are coarse inputs here. |
| `TheLocalRescrubAndPreparationFinish`, `TheLocalRescrubOrPreparationFails`; `sourceBytes` | `crates/trace-commons-server/src/versioned_pipeline.rs:11440-11525`; `crates/trace-commons-server/src/versioned_pipeline_authority.rs:94-143` | Deterministic local rescrub and encryption preparation precede staging, outside a pooled transaction. Raw bytes remain replay identity. The receipt never calls the classifier. |
| `TheStagingTransactionRuns`; bundle and quota | `crates/trace-commons-server/src/versioned_pipeline.rs:11787-11950` | Serialized tenant/key and quota transaction repeats refusal checks, takes the routing lock, binds active bundle, verifies Admission runnable, counts quota once/key, stages this attempt, then releases locks before object I/O. |
| `TheAttemptObjectIsPublished`, `TheSynchronousAdmissionFinishes` | `crates/trace-commons-server/src/versioned_pipeline.rs:11532-11614`; `crates/trace-commons-server/src/versioned_pipeline_bundle.rs:74-170` | Publish the attempt's encrypted source outside the transaction, then evaluate Admission synchronously. Admit/Quarantine/Reject are bounded policy-output classes. Grant denial is ordinary Reject; missing authority already refused the request. |
| `TheFinalReceiptTransactionRuns` | `crates/trace-commons-server/src/versioned_pipeline.rs:11953-12153`, `8376-8401` | Source-session withdrawal is checked **before** the existing-run lookup in this final transaction. Existing receipt then replays; otherwise routing/owner/tombstone/staging deadline/bound-policy are rechecked. Ownership, run, submission, source object ref and Admission outcome commit atomically. Reject creates a Complete run with no next phase; other decisions create Pending Review. |
| Same-submission/different-key conflict | `crates/trace-commons-server/src/versioned_pipeline.rs:11975-11981`, `8350-8374` | An id already owned by a pipeline run under another key is an error, not a LegacyOwned result. Its staged object is left for sweeping on transaction error. The model leaves this attempt in the orphan branch until it becomes due for sweeping; it does not assert immediate deletion. |
| `TheReceiptResponseIsDelivered`, `ThePostCommitAuditOrResponseFails` | `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:16112-16125` | After durable Created, append failure can answer 500; a lost response likewise leaves a durable run. A retry replays and does not append the original event again. |
| `TheRequestCrashesOrStorageFails`, due/discard/sweep actions | `crates/trace-commons-server/src/versioned_pipeline.rs:12156-12252`, `7560-7685` | Per-attempt orphan objects/rows outlive failed work. Cleanup deletes object then row; failed cleanup retains/restages a due row. Sweeper takes only due staged rows, not committed object refs. Publication can finish after a row was swept; final commit refuses and cleanup handles the late object. |
| `TheSourceSessionIsWithdrawn`, `TheSubmissionIsTombstoned`, `TheLegacyPathClaimsAnUnownedSubmission` | `crates/trace-commons-server/src/versioned_pipeline.rs:12007-12072`; `crates/trace-commons-server/src/versioned_pipeline_activation.rs:1515-1591` | World events can intervene between staging and final acceptance. An ownership claim cannot take a pipeline-owned id. |
| `OperatorChangesRouting`, `OperatorSelectsBundle` | `crates/trace-commons-server/src/versioned_pipeline_activation.rs:80-104`, `341-377`, `1594-1843`; `crates/trace-commons-server/src/versioned_pipeline.rs:18012-18024` | Input represents a successful authorized qualification/activation/rollback/containment/deactivation operation. Actual expectation-id and qualification gates remain a coarse interface. Shared/exclusive routing locking is represented by atomic transactions. Receipt ownership survives routing changes. |
| `OperatorChangesAdmissionControl`; immutable packages | `crates/trace-commons-server/src/versioned_pipeline.rs:2158-2254`, `2395-2530`, `12074-12082` | Bundle bytes are immutable for an id; policy runnable flags are mutable interventions. A suspension while writing refuses final commit under the originally selected bundle. |
| Runtime precondition | `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/production_assembly.rs:42-108`; `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:15485-15530` | Runtime unset/blank selects None; production requires the scorer feature and `enclave_near_ai`. Runtime/receipt-list/drain-list and startup readiness are **inputs**, not inferred live enablement. This model starts with a serving qualified runtime. |

## Composition correspondence

- `TheAuthenticatedRequestArrives` consumes the client upload boundary: fixed
  tenant/principal, key and raw bytes. The HTTP composition must constrain key
  to submission id; the detailed library model permits distinct keys to show
  the implemented already-bound error.
- `TheFinalReceiptTransactionRuns` with `outcome == "created"` owns durable
  receipt acceptance and immutable `receipts[cell].bundle`, `.admission`,
  `.principal`, `.bytes`, `.sourceAttempt`. It corresponds to
  `server_pipeline.TheReceiptCommitsItsAdmission`, which starts at Review or
  completes an Admission Reject. No worker Admission action exists.
- `TheCompletedAdmissionRetryReadsTheExistingReceipt` and `ThePrecheckReturns`
  with `outcome == "replayed"` correspond to receipt read-back without a new
  processing run. `TheReceiptResponseIsDelivered` is separate from commit.
- `OperatorChangesRouting` changes new-receipt routing only. Existing owner and
  receipt data stay put. `OperatorSelectsBundle` changes future staging input;
  processing reads the bundle stored on the receipt.
- Withdrawal/tombstone inputs correspond to the lifecycle model's irreversible
  admission boundary. A withdrawal after this receipt commits requires its
  lifecycle follow-up; this model does not undo the committed receipt.

## Explicit coarseness and omissions

Crypto, serialization, storage-key derivation, trust-provider verification,
RLS/PostgreSQL internals and byte-content semantics are not proved. Admission
policy output is nondeterministic over classes of inputs; Low/Medium/High,
medium-risk acceptance basis and individual schema/consent/grant reasons are
source-mapped above but not separate input variables. Receipt object mismatch,
object-store tasks and unexpected I/O errors share the storage-failure branch.
The four request identities bound the workload; this is not unbounded replay
proof. No hourly rollover, more than two devices per tenant, no-row/unqualified
routing, default-routing cache, qualification-record generation, operator
expected-record races, or full legacy storage handler is expanded here.

Upload evidence and account allowance reservation remain an explicit coarse
upstream interface. The current evidence path verifies witness/provider/policy
and immutable body hashes before reserving (`admission.rs:574-679`); source
session/account allowance is the separate path (`admission.rs:968-1025`). Their
window math, lease reservation release, device revocation and enrollment policy
are not silently represented by the pipeline quota. The client/identity and
service-interface models must account for those domains separately.

Frames are internal explanatory panels, not proposed application screens.
Rendering merges many object states into five labels; all request, receipt,
quota, owner and object variables remain independent in the model. No chart or
screen budget was used to remove those transitions.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees. Owner selection is now complete; TLC check outcomes are recorded separately.

## Selected safety monitors

The owner selected all thirteen candidates. This model configures P5, P8 and P11
as observation-only safety invariants. See [receipt and processing proof
notes](../proofs/server-ingest-notes.md) for the exact projections, decision-time
boundaries, source correspondence and limitations. The observers preserve all
source guards and finite domains; typechecking and frame/control checks pass.
This documentation does not claim a completed exhaustive check.
