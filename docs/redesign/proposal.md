# Versioned Classifier Processing Design

> A minimal durable model for classifier evidence, observations, facts,
> decisions, and effects. The design supports safe retries, controlled
> reprocessing, policy evolution, and complete decision provenance.

- **Status:** Proposed target design
- **Date:** 2026-09-02
- **Scope:** Data model and processing workflow
- **Migration plan:** Not included

## Contents

1. [Motivation: current problems](#1-motivation-current-problems)
2. [High-level solution](#2-high-level-solution)
3. [New domain model](#3-new-domain-model)
4. [Rust pseudocode](#4-rust-pseudocode)
5. [New workflow model](#5-new-workflow-model)
6. [New database model](#6-new-database-model)
7. [Vector index model](#7-vector-index-model)
8. [System invariants](#8-system-invariants)



## 1. Motivation: current problems

The current schema grew through additive changes around specific classifiers.
It now mixes domain results, policy choices, workflow state, and external effects.

### No complete classifier identity

A score depends on a projection, model, configuration, calibration, and decision rule.
No single identifier binds all these parts.

### Schema and classifier coupling

Columns such as perplexity, tail fraction, and novelty encode one implementation.
New observations require new columns or changed field semantics.

### Mixed domain concepts

A gate row contains measurements, pass flags, evidence hashes, vector identifiers,
credit state, and later shadow scores.

### Mutable audit records

Some operations append decisions.
Other operations update the latest decision or add derived fields to old decision rows.

### Incomplete provenance

Gate decisions contain partial version stamps.
They do not identify the exact evidence, observations, facts, policy artifact,
and deployment selection.

### Effects occur inside classifiers

The current novelty path can insert vectors before the decision is durable.
A database error can leave unrecorded external state.

### Several vector planes

Gate novelty, general vector metadata, and shadow deduplication use separate paths.
Their roles and consistency rules are not explicit.

### No controlled reprocessing unit

A successful driver decision removes a trace from automatic work.
No run binds a trace revision to a complete bundle set.

> **Root problem:** The system cannot always identify the complete classifier
> and policy that produced a result. It also cannot always reproduce the result.



## 2. High-level solution

The new design separates immutable domain results from mutable operational state.
It persists only the boundaries needed for audit, recovery, and external effects.

### Complete version identity

Each processing job pins a bundle set and a deployment assignment.
Each result identifies its producer.

### Immutable evaluation

One evaluation records evidence, observations, facts, and a decision.
Reprocessing creates a new evaluation.

### Typed JSON documents

Variable classifier data uses JSONB with a schema identifier.
Stable identity and query fields remain relational.

### Minimal durable workflow

The database stores jobs, completed evaluations, effect intents, and effect receipts.
Telemetry records transient steps.

### Effects after decisions

The server persists an effect intent before it changes an external system.
An idempotency key makes retries safe.

### Explicit active selection

Deployment assignments select active and shadow bundle sets.
Timestamp order does not select production behavior.

### Durability principle

> **Persist domain results and irreversible boundaries.**
> Recompute transient pure processing.
> Send execution detail to logs, traces, and metrics.



### None Requirements

- A database event for each internal workflow step.
- A separate database row for each temporary calculation.
- One physical vector backend for all vector roles.
- Raw trace bodies, vector arrays, or model log probabilities in PostgreSQL.
- Classifier-specific SQL columns for each new observation field.



## 3. New domain model

The core reasoning chain has five immutable concepts.
Operational records coordinate the chain and apply its result.

1. **Evidence** is the exact trace revision, projection, corpus snapshot,
  index epoch, and other input used for measurement.
2. **Observations** are raw outputs from classifier roles.
  Observations do not contain accept or reject policy.
3. **Facts** are normalized policy inputs assembled from observations by a
  versioned fact assembler.
4. **Decision** contains the desired dispositions produced by a pure,
  versioned policy from one fact set.
5. **Effect** records a requested state change and the result of applying it.
  A decision does not perform the effect.



### Supporting concepts



#### Trace revision

An immutable reference to one stored form of a trace.
A remediation or content change creates a new revision.

#### Classifier bundle

A content-addressed unit that identifies behavior, projection, model,
configuration, calibration, and output schemas.

#### Bundle set

A compatible set of role bundles, a fact assembler, and a policy.
Compatibility is validated before deployment.

#### Deployment assignment

A durable rule that selects a bundle set for a tenant, mode, and time range.
The processing job stores the resolved assignment.

#### Processing job

A mutable lease and retry record.
The job coordinates work but does not provide the audit history.

#### Evaluation

The immutable aggregate that contains the complete reasoning chain for one
trace revision and one bundle set.

#### Effect intent

A durable request to register a trace, insert a vector, issue credit,
schedule review, or apply another disposition.

#### Effect receipt

An immutable result from an applied effect.
It references external identifiers and contains a result hash.

### Relationship diagram

```mermaid
flowchart LR
    TR[TraceRevision<br/>immutable stored input]
    DA[DeploymentAssignment<br/>active or shadow selection]
    BS[BundleSet<br/>roles + assembler + policy]
    PJ[ProcessingJob<br/>lease, retry, current status]
    EV[Evaluation<br/>immutable reasoning aggregate]
    ED[Evidence<br/>inputs and immutable references]
    OB[Observations<br/>role outputs with producer bundles]
    FA[Facts<br/>normalized policy inputs]
    DE[Decision<br/>policy dispositions and reasons]
    EI[EffectIntent<br/>durable idempotent request]
    ER[EffectReceipt<br/>immutable applied result]
    IX[IndexEpoch<br/>or watermark]
    ES[ExternalState]

    TR --> PJ
    DA --> PJ
    BS --> PJ
    PJ --> EV
    EV --> ED
    EV --> OB
    EV --> FA
    EV --> DE
    ED --> OB
    OB --> FA
    FA --> DE
    DE --> EI
    EI --> ER
    IX --> ED
    ER --> ES
```



The evaluation contains the reasoning chain.
Operational records coordinate processing and external state changes.

### Provenance path

The model explains why an effect occurred through one directed chain:

```text
EffectReceipt
  → EffectIntent
  → Decision
  → Facts
  → Observations
  → Evidence
  → TraceRevision + BundleSet + IndexEpoch
```



## 4. Rust pseudocode

The persistence payloads use JSONB.
Runtime code decodes each payload into a typed Rust structure before use.

### Identity and version types

```rust
struct SchemaRef {
    schema_id: String,
    version: u32,
}

struct BundleId(String);
struct BundleSetId(String);
struct DeploymentId(Uuid);
struct TraceRevisionId(Uuid);
struct EvaluationId(Uuid);
struct ContentHash(String);

enum ProcessingMode {
    Active,
    Shadow,
    Reprocess,
}

enum ClassifierRole {
    TenantDeduplication,
    GlobalDeduplication,
    Novelty,
    Substance,
}
```



### Classifier bundles and deployment

I feel like we've lost the plot here. The class structure here made more sense

```rust
struct ClassifierBundle {
    bundle_id: BundleId,
    role: ClassifierRole,
    artifact_digest: ContentHash,

    projection: ComponentRef,
    model: ComponentRef,
    configuration: VersionedDocument,
    calibration: Option<ArtifactRef>,
    certification: CertificationRef,

    evidence_schema: SchemaRef,
    observation_schema: SchemaRef,
}

struct BundleSet {
    bundle_set_id: BundleSetId,
    classifiers: BTreeMap<ClassifierRole, BundleId>,
    fact_assembler_bundle_id: BundleId,
    policy_bundle_id: BundleId,
    compatibility_manifest: VersionedDocument,
}

/*
Slow down, this implies that different bundles will be applied to different tenants. do we need that variance? Also the Deployment Assignment is mutable, so we cna't hash it, so it doesn't provide provanance
*/
struct DeploymentAssignment {
    deployment_id: DeploymentId,
    tenant_scope: TenantScope, // do we need tenant specific bundles? probably too complex
    mode: ProcessingMode,
    bundle_set_id: BundleSetId,
    effective_from: DateTime<Utc>,
    effective_until: Option<DateTime<Utc>>,
}
```

The deployment assignment records an operator choice.
The processing job also pins the resolved bundle set.
This prevents configuration drift during a run.

### Versioned domain documents

Remark: wtf is this. this feel very different than what we were workig with before

TODO: Review the initial workflow conversation

```rust
struct VersionedDocument {
    schema: SchemaRef,
    payload: serde_json::Value, // didn't we want to explciitely avoid serde_json::Value?
}

struct ProducerRef {
    bundle_id: BundleId,
    artifact_digest: ContentHash, 
}

struct Evidence {
    document: VersionedDocument,
    trace_revision_id: TraceRevisionId,
    projection_bundle_id: BundleId,
    reference_index: Option<IndexReference>,
}

struct Observation {
    role: ClassifierRole,
    producer: ProducerRef,
    evidence_index: usize,
    document: VersionedDocument,
}

struct FactSet {
    producer: ProducerRef,
    document: VersionedDocument,
}

struct PolicyDecision {
    producer: ProducerRef,
    document: VersionedDocument,
}
```



### Evaluation aggregate

XXX: This is directionally correct but missing precision due to context rot

```rust
struct Evaluation {
    evaluation_id: EvaluationId,
    job_id: Uuid,
    tenant_id: String,
    trace_revision_id: TraceRevisionId,
    deployment_id: DeploymentId,
    bundle_set_id: BundleSetId,
    mode: ProcessingMode,

    evidence: Vec<Evidence>,
    observations: Vec<Observation>,
    facts: FactSet,
    decision: PolicyDecision,

    evaluation_hash: ContentHash,
    created_at: DateTime<Utc>,
}
```

> **Hash policy:** The minimal design stores one hash over the canonical
> evaluation. Component hashes are optional at independent trust, reuse,
> signature, or cache boundaries.



### Typed policy interface

```rust
trait ObservationDecoder {
    type Output;

    fn decode(
        &self,
        document: &VersionedDocument,
    ) -> Result<Self::Output, SchemaError>;
}

trait FactAssembler: Send + Sync {
    fn assemble(
        &self,
        evidence: &[Evidence],
        observations: &[Observation],
    ) -> Result<FactSet, FactError>;
}

trait DecisionPolicy: Send + Sync {
    fn decide(
        &self,
        facts: &FactSet,
    ) -> Result<PolicyDecision, PolicyError>;
}
```

A policy never reads arbitrary JSON directly.
Its adapter validates the schema and returns a typed facts structure.

### Effects

```rust
enum EffectKind {
    RegisterTrace,
    QuarantineTrace,
    InsertVector,
    InvalidateVector,
    IssueCredit,
    WithholdCredit,
    ScheduleReview,
}

struct EffectIntent {
    effect_id: Uuid,
    evaluation_id: EvaluationId,
    decision_hash: ContentHash,
    kind: EffectKind,
    payload: VersionedDocument,
    idempotency_key: String,
    required: bool,
    created_at: DateTime<Utc>,
}

struct EffectReceipt {
    receipt_id: Uuid,
    effect_id: Uuid,
    executor_bundle_id: BundleId,
    external_resource_id: Option<String>,
    result: VersionedDocument,
    result_hash: ContentHash,
    applied_at: DateTime<Utc>,
}
```



## 5. New workflow model

The workflow persists two recovery boundaries.
Pure classifier work occurs between these boundaries and can restart after an error.

1. **Accept and queue.** Store the trace revision.
  Resolve the deployment.
   Create one processing job in the same database transaction.
2. **Compute in memory.** Load evidence.
  Produce observations.
   Assemble facts.
   Evaluate the policy.
   Do not apply external effects.
3. **Commit the decision.** Insert one immutable evaluation.
  Insert effect intents.
   Mark the job as decision committed.
4. **Apply effects.** Claim each effect.
  Apply it with its idempotency key.
   Insert a receipt and mark the effect complete.



### Processing job states

```mermaid
stateDiagram-v2
    [*] --> Pending
    Pending --> Leased: claim
    Leased --> Pending: lease expiry
    Leased --> Retry: retryable error
    Retry --> Leased: claim
    Leased --> DecisionCommitted: evaluation committed
    Leased --> TerminalFailure: non-retryable error
    DecisionCommitted --> Complete: required effects have receipts
```



The state machine remains explicit in code.
The database does not store a separate event for each transition.

### Crash and retry behavior


| Failure point             | Durable state                                | Recovery                                                                 |
| ------------------------- | -------------------------------------------- | ------------------------------------------------------------------------ |
| Before evaluation commit  | The trace revision and processing job exist. | The lease expires. Another worker recomputes the complete evaluation.    |
| After evaluation commit   | The evaluation and effect intents exist.     | The classifier does not run again. Effect workers continue the work.     |
| During an external effect | The effect intent and idempotency key exist. | The effect worker retries or queries the external system by key.         |
| After an effect succeeds  | The receipt identifies the external result.  | No recovery is necessary. A compensation uses a new decision and effect. |




### Selective checkpoints

The default workflow does not persist partial observations.
It recomputes them after a retry.

A classifier can use a content-addressed observation cache when recomputation
has high cost.
The cache key binds the bundle and exact evidence.

- Use a checkpoint for expensive or nondeterministic classifier calls.
- Use a checkpoint when independent services produce observations.
- Use a checkpoint when several policies reuse the same observation.
- Do not use a checkpoint for inexpensive deterministic calculations.



### Active, shadow, and reprocess modes

- **Active:** The evaluation creates effect intents after the policy decision.
- **Shadow:** The evaluation is durable, but it creates no production effects.
- **Reprocess:** A new job and evaluation reference an earlier evaluation.
Old records remain immutable.



### Workflow telemetry

Logs, traces, and metrics record lease renewals, model latency, retries,
and internal stage timing.
These details do not enter the audit model.

A small audit stream can record manual overrides, activation, rollback,
cancellation, terminal failure, and compensation.
Routine execution events do not require durable audit rows.

## 6. New database model

Stable identity, tenancy, status, and idempotency fields remain relational.
Evolving classifier content uses validated JSONB documents.

### Core tables



#### `classifier_bundles`

Immutable registry of classifier, assembler, and policy artifacts.
The bundle digest closes over behavior and dependencies.

#### `bundle_sets`

Immutable compatibility manifest for all roles used in one evaluation.

#### `deployment_assignments`

Time-bounded active and shadow selection by tenant scope.
The assignment provides deployment history and rollback identity.

#### `processing_jobs`

Mutable lease, retry, and completion projection.
This table is the work queue.

#### `evaluations`

Immutable JSONB aggregate for evidence, observations, facts, and the decision.

#### `effect_outbox`

Durable effect intent with mutable delivery state.
The requested payload remains immutable.

#### `effect_receipts`

Immutable result of a successful external or internal effect.

#### `vector_index_epochs`

Immutable novelty reference epochs and explicit generations for mutable online indexes.

### Illustrative SQL shape

These definitions show the model.
They are not a migration or final PostgreSQL syntax.

```sql
CREATE TABLE processing_jobs (
    tenant_id              text        NOT NULL,
    job_id                 uuid        NOT NULL,
    trace_revision_id      uuid        NOT NULL,
    deployment_id          uuid        NOT NULL,
    bundle_set_id          text        NOT NULL,
    mode                    text        NOT NULL,
    state                   text        NOT NULL,
    lease_owner_hash        text,
    lease_expires_at        timestamptz,
    attempt_count           integer     NOT NULL DEFAULT 0,
    next_attempt_at         timestamptz,
    last_error_code         text,
    evaluation_id           uuid,
    created_at              timestamptz NOT NULL,
    updated_at              timestamptz NOT NULL,

    PRIMARY KEY (tenant_id, job_id),
    UNIQUE (
        tenant_id,
        trace_revision_id,
        deployment_id,
        bundle_set_id,
        mode
    )
);

CREATE TABLE evaluations (
    tenant_id              text        NOT NULL,
    evaluation_id          uuid        NOT NULL,
    job_id                 uuid        NOT NULL,
    trace_revision_id      uuid        NOT NULL,
    deployment_id          uuid        NOT NULL,
    bundle_set_id          text        NOT NULL,
    mode                    text        NOT NULL,

    evidence_schema        text        NOT NULL,
    evidence               jsonb       NOT NULL,
    observations_schema    text        NOT NULL,
    observations           jsonb       NOT NULL,
    facts_schema           text        NOT NULL,
    facts                  jsonb       NOT NULL,
    policy_bundle_id       text        NOT NULL,
    decision_schema        text        NOT NULL,
    decision               jsonb       NOT NULL,

    evaluation_hash        text        NOT NULL,
    created_at              timestamptz NOT NULL,

    PRIMARY KEY (tenant_id, evaluation_id),
    UNIQUE (tenant_id, job_id)
);

CREATE TABLE effect_outbox (
    tenant_id              text        NOT NULL,
    effect_id              uuid        NOT NULL,
    evaluation_id          uuid        NOT NULL,
    effect_kind            text        NOT NULL,
    effect_schema          text        NOT NULL,
    payload                 jsonb       NOT NULL,
    payload_hash            text        NOT NULL,
    idempotency_key         text        NOT NULL,
    required                boolean     NOT NULL,
    status                  text        NOT NULL,
    attempt_count           integer     NOT NULL DEFAULT 0,
    next_attempt_at         timestamptz,
    last_error_code         text,
    created_at              timestamptz NOT NULL,
    updated_at              timestamptz NOT NULL,

    PRIMARY KEY (tenant_id, effect_id),
    UNIQUE (tenant_id, idempotency_key)
);

CREATE TABLE effect_receipts (
    tenant_id              text        NOT NULL,
    receipt_id             uuid        NOT NULL,
    effect_id              uuid        NOT NULL,
    executor_bundle_id     text        NOT NULL,
    external_resource_id   text,
    result_schema           text        NOT NULL,
    result                  jsonb       NOT NULL,
    result_hash             text        NOT NULL,
    applied_at              timestamptz NOT NULL,

    PRIMARY KEY (tenant_id, receipt_id),
    UNIQUE (tenant_id, effect_id)
);
```



### Job claim

```sql
WITH candidate AS (
    SELECT tenant_id, job_id
    FROM processing_jobs
    WHERE state IN ('pending', 'retry')
      AND next_attempt_at <= now()
    ORDER BY created_at
    FOR UPDATE SKIP LOCKED
    LIMIT 1
)
UPDATE processing_jobs AS job
SET state = 'leased',
    lease_owner_hash = $worker_hash,
    lease_expires_at = now() + $lease_duration,
    attempt_count = attempt_count + 1,
    updated_at = now()
FROM candidate
WHERE job.tenant_id = candidate.tenant_id
  AND job.job_id = candidate.job_id
RETURNING job.*;
```



### Evaluation commit

```sql
BEGIN;

INSERT INTO evaluations (...);

-- Active mode only. Insert zero rows for a shadow evaluation.
INSERT INTO effect_outbox (...);

UPDATE processing_jobs
SET state = 'decision_committed',
    evaluation_id = $evaluation_id,
    lease_owner_hash = NULL,
    lease_expires_at = NULL,
    updated_at = now()
WHERE tenant_id = $tenant_id
  AND job_id = $job_id
  AND state = 'leased';

COMMIT;
```



### Read models

Mutable views and projections support common queries.
These projections are not the decision audit source.

- The current processing state for each trace revision.
- The active evaluation under the current deployment assignment.
- Pending and expired job leases.
- Pending and retryable effects.
- Active and shadow decision differences.
- The full provenance chain for an effect receipt.



### JSONB rules

- Each JSONB document has a schema identifier and version.
- Application code validates the document before each write and read.
- A semantic field change creates a new schema version.
- Policies use typed decoded facts.
- Policies do not inspect arbitrary JSON paths.
- Large or sensitive evidence stays in encrypted object storage.
- The database stores references, hashes, and bounded safe metadata.



### Hash policy

The evaluation uses canonical JSON before hash calculation.
One final evaluation hash is sufficient for the minimal design.

A component receives its own hash only when another system reuses, signs,
caches, compares, or independently attests that component.

### Tenant isolation

Every table carries `tenant_id`.
PostgreSQL forces RLS on every tenant table.
Worker claims use narrow roles and bounded columns.

## 7. Vector index model

One control plane manages logical vector namespaces.
Different namespaces can use different physical backends and consistency rules.

### Novelty reference

This namespace uses immutable epochs.
Each observation references the exact epoch, corpus manifest, and embedding bundle.

### Online tenant deduplication

This namespace changes after accepted submissions.
Evidence records the state generation or watermark used by the successful attempt.

### Online global deduplication

This namespace supports anti-spam checks across tenants.
Its access policy and evidence surface remain separate from tenant novelty.

### Ranking features

This namespace supports downstream ranking.
It does not silently substitute for novelty or deduplication evidence.

### Required index records

```text
VectorNamespace
  namespace_id
  role
  tenant_scope
  backend_kind

VectorIndexEpoch
  epoch_id
  namespace_id
  embedding_bundle_id
  source_snapshot_id
  manifest_hash
  state: building | sealed | active | retired

VectorIndexMembership
  epoch_id
  trace_revision_id
  external_vector_id
  source_projection_hash
  inserted_by_effect_id
```

> **Classifier restriction:** A classifier can query an index, but it cannot
> insert into that index. A policy decision requests insertion through an
> effect intent.



## 8. System invariants

1. Each processing job pins one trace revision and one resolved bundle set.
2. Each evaluation is immutable.
3. Reprocessing creates a new job and evaluation.
4. Each document has a schema identifier.
5. Each observation identifies its classifier bundle.
6. Each fact set identifies its assembler bundle.
7. Each decision identifies its policy bundle and exact fact set.
8. Only active evaluations create production effect intents.
9. The database stores each external effect intent before application.
10. Each effect has a stable idempotency key.
11. Each successful effect has an immutable receipt.
12. Each novelty observation identifies an index epoch or state watermark.
13. Missing required evidence causes a closed decision or processing error.
14. Current state is a projection. It is not an overwritten audit record.
15. Classifier-specific fields remain inside validated JSONB documents.



### Result

The system can explain every durable effect without recording each internal
workflow step.
It can also retry incomplete work from the stored trace revision.

The model supports new classifiers and policies without classifier-specific
database columns.
Schema versions preserve the meaning of historical documents.