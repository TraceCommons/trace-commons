# Versioned Pipeline Processing Design

> A small model for processing traces, explaining outcomes, and changing policy safely.

- **Status:** Proposed target design
- **Date:** 2026-09-09
- **Scope:** Domain model, workflow, persistence, policy development, and rollout

## Review guide

This iteration replaces the previous independent-workflow design with one
pipeline and four phases:

1. Admission
2. Review
3. Score
4. Settle

One immutable bundle selects all four policies. Each phase stores one outcome.
The outcome contains the decision, the evidence, and the evaluation.

The design does not add durable observations, facts, deployment assignments,
effect records, vector epochs, or a generic schema registry.

## 1. Goals

The design has four goals:

- Explain why a trace produced each pipeline outcome.
- Let one Score decision award multiple instruments at the same time.
- Make each policy and complete bundle easy to test.
- Keep policy development separate from policy execution.

The design does not require deterministic replay. An outcome must explain past
behavior, even when its external inputs are no longer available.

The design does not define one true value for a trace. A Score policy combines
one or more valuations into the credit decision for its bundle.

## 2. Domain model

The design uses seven domain concepts.

### Phase

A **phase** is one step in the pipeline:

- **Admission** decides whether processing can continue.
- **Review** transforms and approves the trace for the registry.
- **Score** assigns ordered, instrument-keyed awards.
- **Settle** decides index membership and applies every instrument operation.



### Policy

A **policy** is the versioned implementation of one phase. Examples include:

- An Admission policy that accepts every valid request.
- A Review policy that removes PII.
- A Score policy that assigns a fixed amount.
- A Score policy that aggregates valuations from several participants.
- A Settle policy that updates the index and uses the existing credit process.

Each policy owns its internal algorithm and dependencies. An observer, scorer,
embedder, or vector index is a policy dependency, not a durable domain concept.

### Bundle

A **bundle** is the complete pipeline configuration. It selects one policy for
each phase and all immutable deployed inputs for those policies.

One bundle processes one pipeline run. A policy change creates another bundle.
An active bundle change affects new runs only.

### Projection

A **projection** is the versioned input that a policy gives to an observer.
It has no independent record or lifecycle. Its identifier and input hash appear
in evidence when they affect a decision.

### Decision

A **decision** is the typed result of a phase:

```rust
enum AdmissionDecision {
    Admit,
    Quarantine { reason: ReasonCode },
    Reject { reason: ReasonCode },
}

enum ReviewDecision {
    Approved { registry_revision_id: RevisionId },
    Rejected { reason: ReasonCode },
}

struct ScoreDecision {
    awards: InstrumentAwards,
}

struct InstrumentAward {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits, // u128; a decimal string on the wire
}

enum IndexMembershipDecision {
    Exclude { reason: ReasonCode },
    Include { command_hash: ContentHash, entry_count: u32 },
}

struct SettleDecision {
    index_membership: IndexMembershipDecision,
    settlement_operations: Vec<InstrumentSettlement>,
}

struct InstrumentSettlement {
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
    operation_ref_hash: ContentHash,
    result_ref_hash: ContentHash,
}
```

`instrument_id` is a bounded safe label. Award and settlement collections sort
by that identifier and reject duplicates. An empty award collection is the
completed zero-award result. Each amount uses checked integer atomic units.
Each award names an instrument that the bound bundle pins (section 3).

`AtomicUnits` is a `u128`, which holds any NEP-141 balance. One NEAR is
10^24 yoctoNEAR, so `u64` cannot hold even 0.0001 NEAR. On the wire an amount
is a canonical decimal string, as NEAR's `U128` is: ASCII digits only, with no
sign and no leading zero. A JSON number is refused, because JavaScript and
decoders that read numbers as `f64` lose precision above 2^53. The award-set
identity encodes each amount as 16 big-endian bytes.

Trace Credit is the `trace_credit` instrument. One Trace Credit equals
1,000,000 microcredits. Trace Credit is a NEP-141 token with `decimals = 6`, so
one atomic unit is one microcredit. The existing `BIGINT` microcredit ledger
maps one to one to the token, with no scale conversion. This is also the
6-decimal convention of USDC and USDT on NEAR. `Microcredits` stays `u64`. A
`trace_credit` amount cannot exceed `MAX_TRACE_CREDIT_MICROCREDITS`, which is
`i64::MAX`: about 9.2 trillion credits. Its adapter converts microcredits to
atomic units exactly, without floating point. Decimal conversion rejects
excess precision and overflow.

An operational error is not a decision. The run remains retryable or moves to
a failed state with a safe error label.

### Evidence

**Evidence** records the state that the policy used. Evidence can contain:

- Rate-limit state and PII findings for Admission.
- The source revision and transformation report for Review.
- Measurements, index state, and valuation evidence for Score.
- The index-membership result and internal settlement batch for Settle.

Large or sensitive evidence stays in encrypted object storage. The outcome
contains bounded values and content hashes.

### Evaluation

An **evaluation** explains how the policy mapped its evidence to its decision.
It uses structured fields and stable reason codes, not free-form prose.

For example, Score evidence can contain an index snapshot, measurements, and
coverage. Its evaluation records the rule that produced credit and index
membership.

### Outcome

An **outcome** is the immutable record that groups these concepts:

```rust
struct Outcome<D, E, V> {
    outcome_id: OutcomeId,
    tenant_id: TenantId,
    run_id: RunId,
    trace_id: TraceId,
    phase: Phase,
    bundle_id: BundleId,
    outcome_schema: SchemaRef,
    decision: D,
    evidence: E,
    evaluation: V,
    recorded_at: DateTime<Utc>,
}
```

One outcome schema identifies the three payload types for a policy family. The
`bundle_id` resolves the selected policy implementations, configuration, and
data. It does not identify the code revision that ran; see section 3.

This diagram shows the complete reasoning boundary:

```mermaid
flowchart LR
    B[Bundle] --> P[Phase policy]
    PR[Projection] --> P
    P --> E[Evidence]
    P --> V[Evaluation]
    P --> D[Decision]
    E --> O[Outcome]
    V --> O
    D --> O
    B --> O
```





## 3. Bundle identity

A bundle manifest names the deployed inputs that can change a decision:

```rust
struct BundleManifest {
    format_version: u32,
    admission: PolicyRef,
    review: PolicyRef,
    score: PolicyRef,
    settle: PolicyRef,
    instruments: BTreeMap<InstrumentId, InstrumentDescriptor>,
}

struct PolicyRef {
    policy_id: PolicyId,
    implementation_id: ImplementationId,
    configuration_hash: ContentHash,
    data_artifact_hashes: Vec<ContentHash>,
    projection_ids: Vec<ProjectionId>,
}

struct InstrumentDescriptor {
    kind: InstrumentKind, // nep141 | erc20 | credit_account
    network: String,      // NEAR network, EIP-155 chain id, or ledger label
    contract: String,     // NEAR account id, 0x address, or account label
    decimals: u8,         // one whole token is 10^decimals atomic units
}

impl BundleManifest {
    fn to_bundle_id(&self) -> BundleId {
        BundleId::from_hash(canonical_hash(self))
    }
}
```

Configuration includes thresholds and other parameters. Data artifacts include
models, bootstrap data, and fixed reference data when a policy uses them.

Policies are Rust code in the server binary. The `implementation_id` selects
one of them. The manifest does not hash policy code, and the package does not
contain it. Qualification binds a package to the code revision that it tested,
and activation checks that revision. See
[package qualification](2026-09-14-versioned-pipeline-package-qualification.md).

The bundle package contains the manifest and deployable artifacts. The ingest
service makes sure that the package matches its bundle identifier before use.

An outcome stores only the bundle identifier. The bundle registry retains the
package. The policy lab retains development runs and reports that produced the
bundle.

The bundle hash excludes mutable external state. A policy records the exact
external state that it reads as evidence.

Golden tests protect bundle identity. A change to a policy or implementation
identifier, configuration, data, projection identity, or pinned instrument
descriptor must change the bundle identifier.

### Pinned instruments

The manifest pins one descriptor for each instrument that the bundle can
award. A signed award then says exactly which token it pays and at what
scale. The descriptor gives:

- `kind`: `nep141` for a NEAR token, `erc20` for an EVM token, or
  `credit_account` for an off-chain credit account that is not a token.
  Inference credits can use `credit_account`.
- `network` and `contract`: a NEAR network and account id, an EIP-155 chain id
  and lowercase `0x` contract address, or a ledger label and account label.
- `decimals`: the scale of the atomic units, at most 38.

Each kind accepts one spelling of its network and contract, so equal
descriptors give equal bundle identifiers. The descriptors are part of the
canonical manifest bytes, so they are part of the bundle identifier.

The `trace_credit` descriptor must pin a `nep141` token with 6 decimals. A
manifest that repeats an instrument, or that has no `instruments` field, fails
to load. Loading a manifest applies every check that the bundle identifier
applies, so a manifest with a malformed descriptor also fails to load. A
reader that uses a loaded manifest's descriptors without its bundle identifier
gets only valid descriptors.

An award for an instrument that the bound bundle does not pin is refused. The
runner checks Score's awards with `BundleManifest::require_pinned` before the
Score outcome commits, so no settlement starts for an unpinned instrument.

A descriptor never changes for an instrument identifier. A change of
contract, network, kind, or `decimals` is a new instrument with a new
identifier. An award that is already signed never gets a new meaning. The
bundle registry enforces this rule: it refuses a package that pins a
registered instrument identifier to a different descriptor.

## 4. Workflow

The server binds the active bundle when it creates a run. Every phase in that
run uses the same bundle.

```mermaid
flowchart TD
    T[Receive trace] --> A[Admission: synchronous]
    A -->|Reject| X[Complete without registration]
    A -->|Admit| RQ[Queue Review]
    A -->|Quarantine| RQ
    RQ --> R[Review: asynchronous]
    R -->|Reject| X
    R -->|Approve| G[Commit reviewed revision to registry]
    G --> S[Score: asynchronous]
    S --> L[Settle: asynchronous]
    L --> I[Decide and apply index membership]
    L --> B[Use credit settlement if required]
    I --> O[Record Settle outcome]
    B --> O
    B --> N[Instrument-specific outbox]
    O --> C[Complete]
```





### Admission

Admission runs in the request path. It uses only bounded local work so that it
can complete before the response.

The policy can reject a request because of rate limits. It can quarantine a
trace because of synchronous PII risk. It can also admit a valid trace.

Admission validates the contribution path, schema, tenant grant, consent, and
allowed uses. Model-based substance and novelty valuation belongs in Score.

Admission can read indexes but cannot make external writes. This restriction
makes a repeated request safe before its outcome commits.

The evidence stores the hashes of any index, detector configuration, or
projection that affected the decision. Audit logs repeat hashes and safe labels
only.

An admitted trace still passes through Review. Quarantine requires Review to
address the Admission reason before approval.

### Review

Review runs asynchronously. It transforms the stored trace before the server
commits the approved revision to the registry.

A static Review policy can pass the trace through during tests. A production
policy can scrub PII or apply another required transformation.

Review uses only the submitted contribution artifact and server-generated
evidence. It cannot request or collect more contributor-side data.

The field `request_content_hash` identifies the exact approved artifact. The
Review outcome binds this source hash to the approved registry revision.

Review can query an index when its policy requires one, but it cannot modify an
index.

The Review outcome identifies the source revision, transformation evidence,
and approved registry revision. A rejected trace does not enter the registry.

On approval, the Review policy returns the approved bytes in
`ReviewOutput` beside its result, as `ApprovedContent`. The bytes are
transient. `ReviewOutput::approved` requires the evidence to name their hash
and the worker that produced them. The server encrypts and stores the bytes,
then commits only their reference with the outcome.

### Score

Score runs after Review approves the registry revision. It determines an
ordered set of instrument awards. It does not define an objective trace value.

A Score policy can use:

- A fixed amount.
- Model-based substance and novelty measurements.
- A read-only query of the active index.
- One or more external valuations.

Score can read an index, but it cannot modify one. A shadow policy uses an
isolated index namespace and cannot affect an active decision.

If Settle can use an embedding, Score stores it as encrypted evidence. The
Score outcome stores only its artifact hash.

The Score policy returns proposed index entries as a `SealedIndexCommand` in
`ScoreOutput`, one entry for each chunk with its original chunk number, hash,
and exact vector. `ScoreOutput::new` requires `embedding_artifact_hash` to
equal the command hash, and `neighbor_artifact_hash` to equal the hash of the
neighbor artifact. The server stores both encrypted before the Score outcome
commits. Settle receives the stored command in `SettleInput`. It does not
query the live index or compute new embeddings, so a restart cannot change
its membership decision.

```json
{
  "decision": {
    "awards": [
      {
        "instrument_id": "trace_credit",
        "atomic_units": "200000000"
      },
      {
        "instrument_id": "storage_rebate",
        "atomic_units": "5"
      }
    ]
  },
  "evidence": {
    "index_id": "novelty-active-v1",
    "index_snapshot_id": "snapshot-42",
    "index_snapshot_hash": "sha256:index-snapshot",
    "index_cardinality": 8421,
    "projection_id": "canonical-summary-v3",
    "projection_input_hash": "sha256:reviewed-revision",
    "neighbors_requested": 10,
    "neighbors_returned": 10,
    "neighbor_summary_hash": "sha256:neighbor-summary",
    "embedding_artifact_hash": "sha256:embedding-artifact",
    "novelty_score_micros": 910000,
    "substance_score_micros": 870000,
    "coverage_micros": 1000000
  },
  "evaluation": {
    "rule": "compatibility-score-v1",
    "awards_id": "sha256:ordered-award-set"
  }
}
```

The bundle stores immutable thresholds and model configuration. The evidence
stores mutable index state and measured values that affected the decision.

Future Score policies can combine signed external valuations. A separate
protocol must define participant trust, quorum, and key management before
activation.

### Settle

Settle runs after every completed Score phase. It receives the committed Review
and Score outcomes and the bound bundle.

Settle decides index membership only from these immutable inputs. It cannot
query a mutable index or repeat valuation work to make this decision.

If Settle includes the revision, it creates deterministic entry keys. Each key
covers the tenant, index, reviewed revision, projection, model, and chunk.

Settle stores the encrypted index command and its hash before the index write.
A retry uses the stored command and does not repeat the membership decision.

An upsert with the same key and content is a successful no-op. A conflict with
different content fails closed. Queries exclude the same reviewed revision.

For each positive award, Score creates one eligible instrument operation with a
stable key for the tenant, run, Score outcome, and instrument. The instrument
adapter can group compatible operations into its own batches. The Trace Credit
adapter preserves existing holds, caps, issuer approval, source-list approval,
and duplicate-credit protection.

These rules apply to settlement:

1. A run can settle in several instruments. Score returns a set of awards,
   and the set can name several instruments. The design has no per-tenant or
   per-run instrument selector. `SettleDecision` carries one operation for
   each award.
2. A run has at most one award for each instrument. `InstrumentAwards` and
   `SettleDecision` refuse a second award or operation for an instrument. The
   settlement table key `(tenant_id, run_id, instrument_id)` refuses a second
   row.
3. Each instrument settles as an independent leg. Its operation has a stable
   key, so a retry is idempotent. A retry of one leg does not repeat a
   completed operation, in that leg or in another leg.
4. Settlement has no atomicity across instruments. One leg can complete while
   another is held, retried, or forfeited. No leg waits for another leg or
   reverses another leg. The Settle outcome records after every leg completes
   or is forfeited.

The server records the Settle outcome after all required index and instrument
operations complete. The decision returns every operation and its result
reference in deterministic instrument order. The runner persists and uses
those references; it does not reconstruct operations from mutable run state.
External payout remains in the instrument adapter's outbox. Its later state
does not change the Settle outcome.

Raw account references and transaction hashes do not appear in outcomes, audit
rows, or logs.

## 5. Policy contracts

The phase traits, result types, and decisions live in
`trace-commons-gate-api`. Policy implementations and persistence remain in
their server or gate crates. Client DTOs remain in
`trace-commons-protocol`.

Each phase has a small typed policy trait. This example shows the Score phase:

```rust
#[async_trait]
trait ScorePolicy: Send + Sync {
    // `ScoreOutput` is the `PhaseResult` plus the transient artifacts that
    // its evidence names by hash.
    async fn execute(&self, input: &ScoreInput) -> Result<ScoreOutput, PolicyError>;
}

#[async_trait]
trait SettlePolicy: Send + Sync {
    async fn execute(
        &self,
        input: &SettleInput,
    ) -> Result<PhaseResult<SettleDecision, SettleEvidence, SettleEvaluation>, PolicyError>;
}

struct PhaseResult<D, E, V> {
    decision: D,
    evidence: E,
    evaluation: V,
}

enum PolicyError {
    Transient(ReasonCode), // A dependency failed; retry without charging the trace.
    Permanent(ReasonCode), // This trace cannot be processed; charge the trace.
}

struct ScoreRunner {
    policy: Arc<dyn ScorePolicy>,
}

struct IndexedScorePolicy {
    index: Arc<dyn VectorIndexReader>,
}

struct DefaultSettlePolicy {
    index: Arc<dyn VectorIndexWriter>,
    instruments: Arc<dyn InstrumentSettlementRegistry>,
}
```

Admission, Review, and Settle use the same result shape with their own types.
Review returns it in `ReviewOutput` with any approved content. A policy
reports an outage of its scorer, embedder, index, or other backend as
`PolicyError::Transient`, so the runner does not charge the outage to the
trace. Runners hold policies as trait objects. Score policies receive read-only index
capabilities. Settle policies receive write capabilities.

Policies hold their scorers, embedders, vector indexes, and credit adapters as
trait objects. This boundary prevents Score from modifying an index.

Every phase input carries the tenant's `TenantStorageRef`, the derived key that
ingest uses for every index and storage write. A policy queries an index with
that key. It never receives the raw tenant identifier, so it cannot open an
empty shard by keying with the wrong value.

Tests use small policy implementations through the production trait:

```rust
struct FixedScorePolicy {
    awards: InstrumentAwards,
}

#[async_trait]
impl ScorePolicy for FixedScorePolicy {
    async fn execute(&self, _input: &ScoreInput) -> Result<ScoreOutput, PolicyError> {
        let result = PhaseResult {
            decision: ScoreDecision {
                awards: self.awards.clone(),
            },
            evidence: ScoreEvidence::Fixed,
            evaluation: ScoreEvaluation::FixedAmount,
        };
        ScoreOutput::new(result, None, None).map_err(|_| permanent("invalid_score_output"))
    }
}
```

The design needs no mock-only policy hierarchy. A test implementation satisfies
the same contract as a production implementation.

## 6. Persistence and recovery

The workflow needs three tables for run state and history. The bundle
registry, receipt, and routing records that sections 3, 4, and 7 describe are
separate. Existing trace, registry, tenant, and account storage remains
outside this model.

```text
pipeline_runs
  tenant_id, run_id, trace_id, bundle_id
  request_idempotency_key, request_content_hash
  next_phase: admission | review | score | settle | none
  state: pending | leased | retry | complete | failed
  lease_token, lease_expires_at
  attempt_count, next_attempt_at
  last_error_label
  index_membership: undecided | excluded | included
  index_command_ref, index_command_hash nullable
  index_write_state: none | pending | complete | failed
  created_at, updated_at
  unique (tenant_id, request_idempotency_key)

pipeline_run_settlements
  tenant_id, run_id, instrument_id
  atomic_units NUMERIC(39,0) CHECK (atomic_units > 0)
  CHECK (instrument_id <> 'trace_credit'
         OR atomic_units <= 9223372036854775807)
  operation_ref_hash, result_ref_hash nullable
  operation_state: pending | leased | retry | held | complete | forfeited | failed
  lease_token, lease_expires_at
  attempt_count, max_attempts, next_attempt_at
  last_error_label
  created_at, updated_at
  primary key (tenant_id, run_id, instrument_id)
  unique (tenant_id, operation_ref_hash)

phase_outcomes
  tenant_id, outcome_id, run_id, trace_id
  phase, bundle_id
  outcome_schema_id, outcome_schema_version
  decision JSONB
  evidence JSONB
  evaluation JSONB
  recorded_at
  unique (tenant_id, run_id, phase)
```

`pipeline_runs` is mutable queue state. `phase_outcomes` is immutable history.
The run records index progress. `pipeline_run_settlements` is the settlement
table: one row for each positive award of a committed Score outcome, created
in the Score transaction. It records that instrument operation's progress,
lease, and retry state, independently of every other instrument, so a
`storage_rebate` operation recovers without the Trace Credit path. A row ends
`complete` with its result reference, or `forfeited` when withdrawal commits
first; both count as complete for the Settle outcome.

`atomic_units` is `NUMERIC(39,0)`. Its 39 digits hold every `u128` value.
`BIGINT` is signed 64-bit and would refuse any token amount above `i64::MAX`.
The positivity check matches the rule that only a positive award creates a
row. A reader loads the value through `AtomicUnits`, which refuses a value
above `u128::MAX`. The second check
holds `trace_credit` rows to `i64::MAX` (9223372036854775807), the range of
the `BIGINT` credit ledger. The database enforces the ledger bound, and not
only the Rust contract.

Existing credit events, holds, batches, and outbox rows stay the Trace Credit
and payout records. A `trace_credit` settlement row links its credit event
and settlement batch. Moving those links to a Trace Credit extension table is
deferred.

The receipt path performs these operations:

1. Authenticate the request and derive the tenant.
2. Look up the request key. An identical replay returns the existing run
   here, before any limit is counted.
3. Refuse a tombstoned request-content hash, and enforce rate and quota
   limits. A refusal returns a safe label and stores nothing.
4. Store the encrypted trace body.
5. Resolve and validate the active bundle.
6. Execute Admission.
7. Commit the run, Admission outcome, and next phase in one transaction.

The request key is unique within the tenant and binds to the request-content
hash. A replay returns the existing run. Reuse with different content fails.

Step 3 keeps the order of the current ingest path, which refuses withdrawn
content and over-limit callers before any artifact write. Without it,
withdrawn content resubmitted under a new request key would be stored again,
and an over-quota caller could make the service encrypt and store a body for
every refused request. Admission still receives the tombstone and quota
facts and records them as evidence.

An asynchronous worker performs these operations:

1. Claim a run with a fenced lease.
2. Load the bundle that is already bound to the run.
3. Execute the next phase.

Review and Score commit their outcome and next phase in one transaction. Settle
uses the staged command process below. A retry retains the bundle identifier.
The unique phase constraint prevents duplicate outcomes from concurrent workers.

Review stores transformed content by content hash. Its transaction commits the
approved revision, registry entry, outcome, and Score transition together.

The Score transaction stores its outcome. Each positive instrument award also
creates one eligible operation with a stable instrument-keyed identity.

Settle stores its membership decision and any index command before the index
write. The stored command makes an index retry idempotent.

Settle applies the index command independently from each instrument settlement.
One instrument can retry without repeating a completed operation for another.
The Trace Credit adapter selects eligible credit events and creates approved
account-level batches. A batch can finalize credit for several runs.

The Settle outcome is per run. It records the completed index operation and
every instrument operation and result reference for that run. A disabled or
pending external outbox item does not delay this outcome.

Terminal infrastructure errors update the run with a safe label. They do not
create a phase decision or change an earlier outcome.

Each outcome has a stable schema identifier and version for all three payloads.
An incompatible change increments the version and retains the old reader. This
design needs no generic schema registry.

All tenant tables use forced PostgreSQL RLS. The active bundle is tenant-scoped.
Workers set tenant context from the trusted lease result, not envelope fields.

The cross-tenant claimer can update lease columns only. Policy execution,
artifact access, outcome writes, and settlement use a tenant-scoped transaction.

### Contributor status

`POST /v1/contributors/me/submission-status` combines the new run data with
existing credit and outbox data. No new read-model table is necessary.

```json
{
  "submission_id": "0198...",
  "processing": {
    "phase": "score",
    "state": "pending",
    "reason": null
  },
  "credit": {
    "microcredits": 0,
    "state": "unscored"
  },
  "payout": {
    "state": "not_available"
  }
}
```

The response derives these distinctions:

- `review` with `pending` means that Review has not produced an outcome.
- `score` with `pending` means that Review approved the trace and Score is due.
- `zero` credit means that Score completed with no credit.
- `held` credit means that an existing account hold blocks internal settlement.
- `finalized` means that an approved batch finalized internal credit.
- `held` payout comes from the existing payout-hold reason.
- Payout uses the existing `disabled`, `pending`, `submitted`, `confirmed`, and
`failed` outbox states.



## 7. Phase guards

A phase guard stops work when its required authority is no longer valid. Each
phase checks its guard at two boundaries:

1. Before it loads content or starts policy work.
2. Inside the transaction that commits its outcome or side effects.

The final check locks the applicable submission and policy-status rows. A
withdrawal or policy suspension that commits first prevents the phase commit.
If the phase commits first, its outcome precedes the intervention.

For an index write, Settle keeps these locks until it commits the command
result.

Review and Score require an operable submission. The submission is not
operable after withdrawal, revocation, or retention expiry. Its consent and
allowed uses must also authorize the phase.

The Settle index decision and write also require an operable submission. Credit
settlement uses the committed Score outcome and does not read the trace.

Every phase requires a runnable bound policy. The bundle registry stores this
operational status outside the immutable package. A suspension does not change
the bundle identifier or move the run to another bundle.

If withdrawal commits before the Settle decision, Settle excludes index
membership. If withdrawal commits later, Settle stops a pending command. The
existing revocation path invalidates an index write that completed first.

Withdrawal forfeits awards that are not settled. Settled awards stay.
"Settled" means carried by a finalized settlement batch, as on `main`. If
withdrawal commits after Score and before an instrument operation completes,
that operation ends as `forfeited`. Settle records the withdrawal and
completes; it does not wait for the operation. The withdrawal response
computes `credit_retained` with the same check as
`withdrawal_retains_all_credit` on `main`. This rule applies to every
instrument. It preserves the current contract: settled credit is never clawed
back, and pending credit is forfeited.

A suspended policy leaves the run retryable with a safe error label. Previous
outcomes stay immutable. The run resumes only if the same bound policy becomes
runnable again.

The NEAR outbox worker repeats the policy guard before dispatch. A suspension
keeps a pending item undispatched. A guard cannot retract an operation that an
external system already accepted.

Each withdrawal, suspension, resume, or terminal stop appends a hash-only audit
event. An intervention never rewrites an earlier outcome.

## 8. Policy development

Policy development occurs outside the ingest path.

```mermaid
flowchart LR
    C[Corpus] --> L[Calibration and simulation]
    L --> RP[Local outcome report]
    L --> BP[Bundle package]
    BP --> T[Policy and bundle tests]
    T --> BR[Bundle registry]
    BR --> A[Activate for new runs]
```



The [local pipeline lab](../../operator/pipeline-lab.md) provides corpus execution, reports, a file
catalog, package signing, and qualification drills. The existing calibration
tool targets the old gate. Pilot bootstrap downloads JSONL and submits to
ingest. Neither tool calibrates this pipeline. Pipeline calibration and a
bootstrap/holdout split remain later work. A new lab service or lab database
is not required by this design.

For an Admission policy, the lab can:

1. Split a selected corpus into bootstrap and holdout sets.
2. Configure the policy and its policy dependencies.
3. Calibrate thresholds against the holdout set.
4. Store a local outcome report.
5. Build a deployable bundle package.

For a Score policy, the lab can compare fixed, trusted-party, and multi-party
valuation rules. Model-based substance and novelty scores remain policy inputs
when a selected Score policy uses them.

The ingest database does not store lab run identifiers or calibration reports.
The lab catalog maps each bundle identifier to those development records.

### Test levels

Policy tests pass typed fixtures directly to one policy.

Phase-runner tests use the normal policy trait with small test implementations.
They cover retry and persistence behavior.

Bundle tests process golden traces through all four policies. They assert the
phase decisions, evidence shape, evaluation shape, and bundle identifier.

Integration tests use an isolated index and the existing settlement interfaces.
External payout stays disabled during these tests.

## 9. Rollout

The rollout has six steps:

1. Add bundle loading, pipeline runs, and phase outcomes beside current tables.
2. Wrap current behavior in one compatibility bundle.
3. Preserve current review, scoring, and settlement results across the new phase
  boundaries.
4. Compare compatibility outcomes with current results on a fixed corpus.
5. Activate the compatibility bundle for new submissions.
6. Keep old records readable until their retention period ends.

New valuation rules follow after the compatibility bundle and its transitions
are stable.

Routine activation changes affect new runs only. Existing runs retain their
bound bundle unless a phase guard stops them. A rollback does not rewrite old
outcomes.

This proposal does not define production reprocessing. Policy development and
comparison use the lab path. A later reprocessing design must prevent repeated
settlement before it can operate in production.

## 10. Required constraints

1. Each run binds one immutable bundle before Admission executes.
2. Each completed phase stores one immutable outcome.
3. Each outcome identifies its phase, trace, run, tenant, and bundle.
4. Evidence records external state that can affect a decision.
5. Evaluation explains the mapping from evidence to decision.
6. Missing required evidence fails closed.
7. Request keys bind to request content and are unique within a tenant.
8. Score can read an active index but cannot modify it.
9. Settle decides index membership only from the bound bundle and committed
  Review and Score outcomes.
10. Settle seals an index command before it writes to the index.
11. Settle uses deterministic index keys and self-exclusion.
12. Shadow comparisons use an isolated index namespace.
13. Score awards are ordered by bounded instrument identifier and reject
   duplicate instruments. A run has at most one award for each instrument.
14. Positive Score awards create one idempotent eligible operation per
   instrument. Each instrument settles as an independent leg, with no
   atomicity across instruments.
15. Settle returns every instrument operation and result reference. Runners
   persist and honor those values instead of recomputing them.
16. Existing Trace Credit batches, holds, approvals, and the NEAR outbox
   remain authoritative.
17. All awards use checked `u128` atomic units, carried as canonical decimal
   strings. Trace Credit pins 6 decimals, so the Trace Credit adapter converts
   exact microcredits. The settlement table holds `trace_credit` rows to
   `i64::MAX`.
18. Each phase checks submission and policy authority before work and commit.
19. Authentication supplies tenant scope. Envelope tenant fields provide
  attribution only.
20. Audit rows and logs use hashes and safe labels only.
21. Policy implementations hold scorers, embedders, vector indexes, and
  instrument adapters
  behind trait objects.
22. The bundle manifest pins one descriptor for each instrument that it can
  award. The descriptors are part of the bundle identifier. An award for an
  unpinned instrument is refused. A new contract, network, kind, or
  `decimals` is a new instrument.



## 11. Follow-up specifications

Implementation requires these narrow specifications:

- The payload types and reason codes for each phase.
- The bundle package format, signature, and retention policy.
- The external valuation request and attestation protocol.
- The vector adapter contract for idempotency and self-exclusion.
- The integration contract for existing settlement batches and the NEAR outbox.
- The operator controls for policy suspension, resumption, and termination.
- A migration qualification plan for crash recovery, isolation, and retries.

These specifications can add fields inside the defined boundaries. They do not
add another workflow layer or another provenance model.
