# Trace Commons Behavioral Contracts

Status: Proposed redesign acceptance contract

This document defines the behavior that a redesigned Trace Commons system must provide.
It describes user and operator outcomes.
It does not preserve the current architecture by default.

## 1. Purpose

The current implementation mixes product behavior with pilot code, storage migration code, and operational controls.
A route or table can therefore exist without representing a lasting user need.

This document answers one question:

> Does the new system fulfill the required contracts of the old system?

Use this document to:

- design the replacement system;
- define black-box acceptance tests;
- review removed or changed behavior;
- identify required compatibility adapters;
- prevent old implementation details from becoming new requirements.

## 2. Contract classes

Each behavior belongs to one of these classes.

### Product contract

A product contract represents a user, security, governance, or operator need.
The redesign must preserve its outcome.
The redesign can use different APIs, services, tables, queues, or state machines.

### Compatibility contract

A compatibility contract preserves a published interface during an agreed transition.
It can be implemented by an adapter.
It does not control the internal design.

### Replace

A replaced behavior is a pilot shortcut, migration mechanism, or harmful architectural choice.
The redesign must not preserve it as a requirement.

### Conformance record

The redesign SHOULD maintain one record for each contract identifier.
Each record SHOULD contain:

- implementation status;
- responsible service or component;
- acceptance test identifiers;
- compatibility adapter, when required;
- approved exception, when required;
- evidence from the latest passing test run.

A contract is not complete only because matching code exists.
Completion requires passing evidence at the stated system boundary.

## 3. Normative terms

`MUST`, `MUST NOT`, `SHOULD`, and `MAY` are normative terms.

- `MUST` defines a required acceptance condition.
- `MUST NOT` defines a prohibited result.
- `SHOULD` defines a strong default.
- `MAY` defines optional behavior.

## 4. Product actors

### Contributor

A contributor chooses whether to share traces.
The contributor submits redacted traces, checks their status, manages consent, and receives credit.

### Consumer

A consumer uses approved traces for an allowed purpose.
Examples include evaluation, benchmark generation, ranking training, and model training.

### Reviewer

A reviewer examines traces that need human privacy or policy review.
The reviewer can approve, reject, or request remediation.

### Tenant administrator

A tenant administrator manages access, consent rules, allowed uses, and tenant operations.

### Platform operator

A platform operator deploys services, monitors workflows, recovers failed work, and runs safety checks.

### Classifier operator

A classifier operator creates, certifies, deploys, rolls back, and retires classifier bundles.

### Credit issuer

A credit issuer approves credit policy and settlement.
The issuer can hold settlement but cannot silently change source evidence.

### External adapter

An external adapter applies an effect outside the control plane.
Examples include object storage, a vector index, an evaluator, and a settlement network.

## 5. System boundary

The system accepts redacted trace envelopes.
It evaluates those envelopes under versioned classifier and policy bundles.
It exposes approved projections to authorized consumers.
It manages trace withdrawal, expiration, credit, and external effects.

The following items are outside the trust boundary:

- envelope tenant identifiers;
- client-provided scores;
- client-provided retention classes;
- external adapter responses;
- caller-provided ownership claims.

The system MUST derive authority from authenticated identity and stored policy.

## 6. Global contracts

### SYS-001: Tenant isolation

Class: Product contract

- Every read and write MUST use the authenticated tenant.
- Envelope tenant fields MUST be attribution only.
- A caller MUST NOT detect another tenant's resources.
- Database, object, vector, cache, and export access MUST apply the same boundary.
- Administrative cross-tenant work MUST use an explicit service role.
- Cross-tenant work MUST create an audit record.

### SYS-002: Least privilege

Class: Product contract

- Contributor, reviewer, administrator, and worker permissions MUST be separate.
- A worker credential MUST authorize only its assigned workflow.
- Worker roles MUST NOT inherit reviewer access.
- A missing permission MUST fail before sensitive data is read.
- Revocation and self-withdrawal MUST remain available after ordinary access is removed.

### SYS-003: Fail-closed controls

Class: Product contract

- A required security control MUST NOT have a permissive fallback.
- A missing classifier, policy, key, object store, or authorization source MUST stop the affected operation.
- The failure MUST identify a safe control name.
- Diagnostic and dry-run operations MAY remain available when live mutation is blocked.

### SYS-004: Privacy

Class: Product contract

- Trace contribution MUST be off by default.
- Raw local sessions MUST NOT leave the contributor device.
- The client MUST redact a trace before upload.
- The server MUST apply its own privacy checks.
- Unresolved privacy risk MUST prevent consumer access.
- Logs, errors, audit records, and metrics MUST NOT contain trace bodies or secrets.
- Operational references MUST use safe identifiers, labels, or hashes.

### SYS-005: Idempotency

Class: Product contract

- Every retryable mutation MUST have a stable idempotency key.
- A successful retry MUST return the existing result or continue the existing workflow.
- A retry MUST NOT duplicate credit, exports, vector entries, deletions, or external submissions.
- The system MUST detect an idempotency key used by another owner.
- A conflicting reuse MUST fail without revealing the existing owner.

### SYS-006: Provenance

Class: Product contract

- Every observation MUST identify its classifier operation and bundle.
- Every fact set MUST identify its input observations and fact schema.
- Every decision MUST identify its policy bundle and input facts.
- Every effect intent MUST identify its authorizing decision.
- Every effect receipt MUST identify its intent and external result.
- New processing MUST append records.
- New processing MUST NOT rewrite the reasoning from an earlier run.

### SYS-007: Schema evolution

Class: Product contract

- Each polymorphic payload MUST include a schema identifier.
- Evidence, observations, facts, decisions, and effect data SHOULD use versioned JSON documents.
- A reader MUST reject an unknown required schema.
- A reader MAY ignore a documented optional field.
- Semantic field changes MUST use a new schema identifier.

### SYS-008: Availability is a derived state

Class: Product contract

- Trace availability MUST be derived from accepted decisions and lifecycle events.
- A single mutable status column MUST NOT be the only source of truth.
- Views MAY provide the current state for efficient reads.
- A view MUST be rebuildable from authoritative records.

### SYS-009: Safe failure information

Class: Product contract

- Every API failure MUST include a stable machine-readable code.
- A failure SHOULD state whether the caller can retry.
- A failure MUST NOT include a token, raw URL, account reference, trace body, or adapter response.
- Unknown and inaccessible resources MUST have the same response.
- Internal failures MUST include a safe correlation identifier.

### SYS-010: Time and ordering

Class: Product contract

- Durable records MUST use server-assigned timestamps.
- Workflow records MUST have stable identifiers.
- Concurrent decisions MUST have an explicit ordering or conflict rule.
- A stale worker MUST NOT overwrite a newer result.

## 7. External API contracts

The external API is a product boundary.
Internal route layout is not a product boundary.

### API-001: Versioned wire formats

Class: Product contract

- Every public request and response type MUST have a published schema.
- Breaking changes MUST use a new schema or API version.
- The trace envelope MUST keep a stable version identifier.
- A server MUST reject an unsupported required version.
- Public schemas SHOULD live in `trace-commons-protocol`.

Compatibility:

- The redesign MUST accept `ironclaw.trace_contribution.v1` during the declared compatibility period.
- The current response formats MAY be provided by an adapter.

### API-002: Common HTTP behavior

Class: Product contract

- Missing authentication MUST return `401`.
- Valid authentication with insufficient authority MUST return `403`.
- Invalid input MUST return `400`.
- Unknown or inaccessible owned resources MUST return `404`.
- State or idempotency conflicts MUST return `409`.
- Rate or quota limits MUST return `429`.
- A missing required dependency MUST return `503`.
- A successful idempotent deletion MAY return `204`.
- Explicit list limits outside the supported range MUST fail.
- The server MUST NOT silently increase a caller's requested scope.

The new error shape SHOULD contain:

```json
{
  "error": {
    "code": "stable_error_code",
    "message": "Safe description.",
    "retryable": false,
    "correlation_id": "safe-opaque-id"
  }
}
```

The current `{ "error": "message" }` shape is a compatibility concern.
It is not the target error model.

## 8. Identity, onboarding, and authentication

### AUTH-001: Contributor onboarding

Class: Product contract

User need: A contributor can join an authorized tenant without receiving a shared service secret.

- An onboarding credential MUST be bounded by use count, expiry, or both.
- Onboarding MUST bind a contributor device key to a tenant and principal.
- The device private key MUST remain on the contributor device.
- A repeated completed request MUST have a defined result.
- The response MUST provide the service endpoints and allowed scope ceiling.
- An invalid or consumed invitation MUST fail with a stable code.
- Onboarding MUST NOT grant a wider scope than the invitation permits.

Current compatibility mapping:

- `POST /v1/onboard`
- `POST /v1/enroll`

Campaign-specific NFT onboarding routes are optional plugins.
They are not core Trace Commons contracts.

### AUTH-002: Upload claim generation

Class: Product contract

User need: A contributor can obtain a short-lived credential for trace operations.

- The issuer MUST authenticate the workload or registered device.
- The claim MUST identify tenant, principal, role, issuer, audience, issue time, expiry, and unique token identifier.
- The claim MUST carry consent and allowed-use ceilings when they apply.
- The claim lifetime MUST have a server-controlled maximum.
- The ingest service MUST verify the signature, issuer, audience, expiry, and key identifier.
- Claim authority MUST be intersected with the current tenant access grant.
- A client SHOULD refresh a claim before expiry.
- A client MAY refresh once after an authentication failure.

Current compatibility mapping:

- `POST /v1/trace-upload-claim`
- `GET /.well-known/trace-commons-ed25519-keyset.json`

### AUTH-003: Key discovery and rotation

Class: Product contract

- A verifier MUST be able to fetch current public verification keys.
- A keyset MUST NOT expose private material.
- Rotation MUST support old claims for their remaining valid lifetime.
- A stale or unavailable keyset MUST fail closed after the allowed cache window.
- Rotation MUST be testable before production promotion.

The current single-key restart process is a behavior to replace.

### AUTH-004: Human account sessions

Class: Product contract

User need: A person can manage traces and payout identities through strong authentication.

- Human sessions MUST be separate from unattended upload claims.
- Login codes MUST be single use and short lived.
- Native application login MUST bind authorization to a proof key.
- Sensitive changes MUST require recent strong authentication.
- Logout MUST end the current session.
- Revoke-all MUST invalidate all active account sessions.
- Cross-account reads MUST return the same result as unknown resources.

Passkeys and NEAR login are supported authentication methods.
No single method is a permanent internal architecture requirement.

### AUTH-005: Access grant management

Class: Product contract

- An administrator MUST be able to create, inspect, expire, and revoke a tenant access grant.
- A grant MUST bind tenant, principal, role, and permitted scopes.
- Grant removal MUST take effect on new requests.
- Grant removal MUST NOT block self-withdrawal.
- Grant changes MUST be audited without storing raw credentials.

Long-lived static bearer tokens and HS256 bridge tokens are behaviors to replace.

### AUTH-006: Operator and service credentials

Class: Product contract

User need: Operators and automated services can authenticate without sharing one administrator secret.

- A service credential MUST identify tenant, principal, role, issuer, audience, and expiry.
- Credential authority MUST be no wider than the assigned workflow.
- Operators MUST be able to issue, rotate, and revoke service access.
- Rotation MUST allow in-flight work to finish within a bounded period.
- Credential issuance and revocation MUST create safe audit events.
- Raw credentials MUST NOT appear in configuration reports, logs, commands, or audit records.
- Production service credentials SHOULD be short lived.

The current environment-based worker tokens are compatibility inputs.
They are not the target issuance or rotation model.

## 9. Trace submission workflow

### SUB-001: Local consent and redaction

Class: Product contract

User need: A contributor controls what leaves their device.

- Contribution MUST require explicit opt-in.
- The client MUST create a redacted envelope before upload.
- Message text and tool payload inclusion MUST be explicit.
- The envelope MUST state its consent policy and redaction pipeline versions.
- Raw detector spans and original private text MUST NOT be uploaded.
- The contributor MUST be able to inspect the planned submission.

### SUB-002: Submit a trace

Class: Product contract

User need: A contributor can submit once and learn whether the system accepted custody.

- Submission MUST require an authenticated contributor claim.
- The service MUST enforce the claim, grant, and tenant policy intersection.
- The service MUST validate the envelope schema and consent.
- The service MUST correct under-reported content declarations before risk processing.
- The service MUST re-scrub or independently validate submitted content.
- The service MUST store the accepted input durably before confirming custody.
- The service MUST return a durable submission identifier and receipt.
- The receipt MUST distinguish custody from final corpus acceptance.
- Client-provided scores MUST NOT control acceptance or credit.

Current compatibility mapping:

- `POST /v1/traces`
- Request schema: `ironclaw.trace_contribution.v1`

### SUB-003: Submission idempotency

Class: Product contract

- A submission identifier MUST be stable across transport retries.
- An identical retry MUST return the same durable submission.
- A retry MUST not consume another quota unit.
- A retry MUST not create another credit source.
- Reuse by another principal MUST return a conflict.
- Previously withdrawn content MUST NOT be restored by resubmission.

The current content-derived `submission_id` can satisfy this contract.
Its exact derivation is not required.

### SUB-004: Asynchronous processing

Class: Product contract

User need: Slow privacy and classifier work must not make uploads unreliable.

- The service MAY process required checks asynchronously.
- A custody receipt MUST use `received` or `processing` semantics.
- A trace MUST NOT be consumer-visible before all required controls pass.
- Processing MUST resume after worker or service failure.
- Exhausted retries MUST create an operator-visible blocked state.
- Exhausted retries MUST NOT silently remove the trace from all work queues.

The current use of `accepted` before all gate work is complete is a behavior to replace.

### SUB-005: Processing result

Class: Product contract

The user-visible result MUST map to one of these outcomes:

- `processing`: required work is incomplete;
- `accepted`: all required controls passed;
- `quarantined`: human action or contributor remediation is required;
- `rejected`: the current processing run produced a terminal negative decision;
- `withdrawn`: the contributor removed consent;
- `expired`: the retention period ended;
- `purged`: payload deletion completed.

Internal states MAY be more detailed.
Examples include privacy-backstop wait, effect retry, and review lease.

Each result MUST include:

- the submission identifier;
- the current outcome;
- safe reason codes;
- whether the result is final for the current processing run;
- available next actions;
- pending and final credit values when relevant.

### SUB-006: Quarantine and remediation

Class: Product contract

User need: A contributor can correct a recoverable submission without creating a competing trace.

- Quarantine MUST explain the safe reason category.
- Quarantined content MUST remain unavailable to consumers.
- The contributor MUST be able to submit a corrected revision.
- The correction MUST remain linked to the original submission.
- A correction MUST start a new processing run.
- Earlier evidence and decisions MUST remain immutable.
- Remediation MUST NOT create duplicate credit.

Compatibility:

- The current API reuses the same `submission_id`.
- A redesign MAY use explicit revision identifiers under one logical submission.

### SUB-007: Status synchronization

Class: Product contract

- A contributor MUST be able to request the current state of owned submissions.
- Batch status reads MUST have a documented maximum size.
- Unknown and unowned identifiers MUST be omitted or returned identically.
- Status MUST include safe delayed-work and credit information.
- Status MUST be derived from authoritative workflow records.

Current compatibility mapping:

- `POST /v1/contributors/me/submission-status`
- Current batch maximum: 500 identifiers.

## 10. Contributor trace management

### MAN-001: List and inspect owned traces

Class: Product contract

- An authenticated account holder MUST be able to list owned submissions.
- The list MUST use stable pagination.
- The holder MUST be able to read stored redacted content when retention permits.
- A content read MUST be audited.
- The service MUST never return the contributor's original local trace.

Current compatibility mapping:

- `GET /v1/account/traces`
- `GET /v1/account/traces/{submission_id}`
- `GET /v1/account/traces/{submission_id}/content`

### MAN-002: Withdraw consent

Class: Product contract

User need: A contributor can stop future use of a trace.

- Withdrawal MUST be available to the current owner.
- Withdrawal MUST be idempotent.
- The trace MUST become unavailable to new consumers immediately after the durable withdrawal record.
- The system MUST queue deletion and downstream invalidation.
- The contributor MUST be able to see propagation progress.
- A failed downstream action MUST remain visible to operators.
- Withdrawal MUST NOT expose whether a foreign submission exists.
- Withdrawal MUST NOT claw back already settled credit.
- Unsettled credit from the withdrawn trace MUST not settle.

Current compatibility mapping:

- `DELETE /v1/traces/{submission_id}`
- `POST /v1/traces/{submission_id}/revoke`
- `POST /v1/account/traces/{submission_id}/withdraw`

The redesign SHOULD expose one canonical external withdrawal operation.
Aliases MAY remain in a compatibility adapter.

### MAN-003: Expiration and purge

Class: Product contract

User need: Trace use ends when its allowed retention period ends.

- The server MUST derive retention from stored consent and allowed use.
- A client-provided retention class MUST NOT extend server policy.
- An expired trace MUST be unavailable to new consumers and credit workflows.
- Purge MUST remove readable payloads and invalidate derived artifacts.
- Purge MUST keep the minimum audit and tombstone data required for proof.
- A legal hold MAY delay purge.
- A legal hold MUST NOT make the trace available for an otherwise prohibited use.
- Live destructive purge MUST require an authorized purpose.

## 11. Consumer workflow

### CON-001: Authorized trace selection

Class: Product contract

User need: A consumer receives only traces approved for its declared purpose.

- Consumer access MUST require tenant, principal, role, and purpose authorization.
- Selection MUST include only currently accepted traces.
- Selection MUST intersect contributor consent with the consumer's allowed-use grant.
- Selection MUST enforce privacy, withdrawal, expiration, and legal restrictions.
- Selection MUST use server-derived policy data.
- A storage or policy uncertainty MUST exclude the trace.

### CON-002: Filtered trace projection

Class: Product contract

- A consumer MUST receive a purpose-specific projection.
- The projection MUST include only fields permitted for that purpose.
- Public attribution MUST NOT grant trace-content access.
- The projection MUST identify its schema.
- The projection MUST preserve source and consent provenance.
- A consumer MUST NOT receive raw local sessions or unredacted server inputs.

The projection implementation can change.
The allowed information boundary cannot change silently.

### CON-003: Dataset export

Class: Product contract

- An export request MUST state an allowed purpose.
- The system MUST create an immutable source snapshot for the job.
- The job MUST record the applied selection policy.
- The job MUST have queued, running, complete, failed, and cancelled or invalidated outcomes.
- Job claims MUST be exclusive.
- A retry MUST reuse the same job or create an explicitly linked attempt.
- The system MUST publish no partial result as complete.
- The completed export MUST include a manifest and source-set identity.
- The manifest MUST allow later withdrawal and provenance checks.

Current compatibility mapping:

- `GET /v1/datasets/replay`
- `GET /v1/datasets/replay/manifests`
- export worker and export job routes.

The current synchronous route is not a required architecture.

### CON-004: Revocation after export

Class: Product contract

- A withdrawal MUST invalidate future access through managed exports.
- The system MUST record which manifests and artifacts included the trace.
- Managed consumers MUST receive an invalidation or updated manifest.
- The system MUST record delivery attempts and receipts.
- The system MUST not claim deletion from an unmanaged copy without evidence.
- Contributor feedback SHOULD state the known distribution reach.

### CON-005: Benchmark and training consumers

Class: Product contract

- Benchmark generation MUST require benchmark consent and allowed use.
- Ranking training MUST require ranking or model-training consent.
- Model training MUST require model-training consent.
- Derived artifacts MUST retain source, projection, policy, and bundle provenance.
- Publication to an external registry MUST use an idempotent effect workflow.

Current benchmark and ranking route layouts are internal compatibility concerns.

## 12. Review workflow

### REV-001: Quarantine queue

Class: Product contract

- Reviewers MUST see only tenant-authorized metadata and content.
- The queue MUST prioritize safety and service-level risk.
- A privacy-backstop item MUST not become reviewer work before automated processing finishes.
- Queue reads MUST not expose raw reviewer identities.

### REV-002: Review lease

Class: Product contract

- A reviewer MUST be able to claim work exclusively.
- A lease MUST expire or be releasable.
- Another reviewer MUST not overwrite an active lease decision.
- Lease loss MUST not lose the trace or prior review work.
- Batch claim MUST use the same eligibility rules as single claim.

### REV-003: Human decision

Class: Product contract

- A reviewer MUST be able to approve or reject an eligible quarantined trace.
- A decision MUST include a non-empty reason category.
- A decision MUST identify reviewer authority, policy, input revision, and time.
- Approval MUST start or resume required downstream processing.
- Approval MUST NOT bypass any unrelated required classifier or policy.
- Rejection MUST preserve evidence and decision provenance.
- A concurrent or stale decision MUST return a conflict.

Current compatibility mapping:

- `POST /v1/review/{submission_id}/decision`
- `POST /v1/review/batch-decisions`
- review lease and rescrub routes.

### REV-004: Reviewer protection

Class: Product contract

- Review interfaces MUST show the minimum content needed for the decision.
- Every content read MUST be audited.
- The system SHOULD use frozen review snapshots.
- A reviewer MUST NOT receive object-store credentials.

## 13. Versioned classifier processing

### PROC-001: Processing run assignment

Class: Product contract

- Each processing run MUST use an immutable deployment assignment.
- The assignment MUST select classifier, projection, evidence, fact, and policy bundle versions.
- The assignment MUST be resolved before classifier work starts.
- Later deployment changes MUST NOT change an existing run.
- Active and shadow assignments MUST be distinguishable.

### PROC-002: Evidence to decision chain

Class: Product contract

Required logical order:

1. Build a versioned input projection.
2. Resolve required external evidence.
3. Execute classifiers and append observations.
4. Derive versioned facts.
5. Execute a versioned policy.
6. Append a decision.
7. Create authorized effect intents.
8. Apply effects and append receipts.

The system MAY combine steps inside one transaction.
It MUST preserve the logical provenance chain.

### PROC-003: Acceptance decision

Class: Product contract

- Trace acceptance MUST be a versioned policy decision.
- Novelty, substance, duplication, and privacy observations MUST remain facts or evidence.
- A classifier MUST NOT directly mutate final trace state.
- The decision MUST record the policy result and safe reason codes.
- The policy MUST state which observations and facts it used.

### PROC-004: Reprocessing

Class: Product contract

User need: An operator can evaluate existing traces under a new bundle without destroying history.

- Reprocessing MUST create a new processing run.
- The run MUST identify its reason and assignment.
- Earlier observations, facts, decisions, and effects MUST remain unchanged.
- The system MUST define whether the new decision can supersede the effective decision.
- Shadow reprocessing MUST NOT change production availability or credit.
- Active reprocessing effects MUST use ordinary idempotent effect handling.
- Reprocessing MUST support bounded batches and resumable progress.
- Failed items MUST remain discoverable and retryable.

### PROC-005: Mutable anti-spam state

Class: Product contract

- Online duplicate prevention MAY use mutable recent state.
- The decision MUST identify the state snapshot or sequence boundary used.
- A retry MUST not compare a trace against its own earlier partial insert.
- An anti-spam index update MUST be an explicit effect.
- A failed index update MUST have a retryable intent.
- Offline novelty evaluation MUST use a reproducible index epoch.

### PROC-006: Vector index separation

Class: Product contract

- Anti-spam, novelty evaluation, search, and training retrieval MUST use distinct logical namespaces.
- Each vector entry MUST identify model, dimensions, purpose, source projection, and lifecycle.
- A bundle change MUST not mix incompatible vectors.
- An index MUST be rebuildable from authoritative projections and receipts.
- An index rebuild MUST not create new classifier decisions or credit.

### PROC-007: Model development and certification

Class: Product contract

- Training and calibration MUST use versioned dataset snapshots.
- Training data and holdout data MUST not overlap.
- A bundle MUST have immutable artifacts and a manifest.
- Certification MUST identify evaluation data, metrics, thresholds, and policy.
- Production deployment MUST require valid certification when policy requires it.
- Deployment rollback MUST select a prior assignment.
- Rollback MUST not rewrite decisions made under the replaced assignment.

Current in-server ranking promotion routes are an implementation to replace.
The certification and deployment outcomes remain required.

## 14. Effect contracts

### EFF-001: Decision and effect separation

Class: Product contract

- A decision states what should happen.
- An effect intent requests a state change.
- An effect receipt states what happened.
- An accepted decision MUST NOT prove that an external effect completed.
- Operators MUST be able to trace a receipt back to its decision.

### EFF-002: Effect lifecycle

Class: Product contract

An effect intent MUST expose these logical states:

- pending;
- claimed;
- applied;
- retryable failure;
- terminal failure;
- cancelled or invalidated.

A lease is mutable execution state.
Effect intent and receipt history is append-only.

### EFF-003: External side-effect idempotency

Class: Product contract

- Every external request MUST carry a stable idempotency key when supported.
- The system MUST serialize effects that cannot safely run concurrently.
- A timeout MUST produce an unknown or retryable result, not assumed success.
- Confirmation MUST use external evidence.
- Recovery MUST detect an effect that completed before a local crash.

### EFF-004: Revocation propagation

Class: Product contract

Withdrawal or purge MUST create effects for all applicable targets:

- trace payload objects;
- derived artifacts;
- vector entries;
- export membership;
- benchmark artifacts;
- training queues;
- caches;
- pending credit;
- managed external publications.

Each target MUST have a receipt or a visible failure.
Terminal failures MUST block a clean operational status.

## 15. Credit and settlement

### CRD-001: Credit ledger

Class: Product contract

- Credit MUST be an append-only ledger.
- Every credit event MUST identify its source and authorizing policy.
- A retry MUST not append the same source event twice.
- Upload acceptance alone MUST NOT prove settlement eligibility.
- A contributor MUST see pending, held, settled, and reversed totals.
- A reviewer or worker MUST not issue positive credit outside its authority.

The current `novelty_utility` signal is not settlement eligible.
That special event type is not a permanent product requirement.

### CRD-002: Credit eligibility

Class: Product contract

- Settlement MUST recheck current trace eligibility.
- Withdrawn, expired, purged, or rejected traces MUST not create new settlement.
- Model-derived credit MUST reference a certified production decision.
- Credit policy MUST be versioned.
- Account holds and policy caps MUST be applied before settlement.
- Exclusions MUST have safe reason codes.

### CRD-003: Settlement preview and approval

Class: Product contract

- An issuer MUST be able to preview the exact eligible source set.
- The preview MUST identify policy version, totals, exclusions, and source-set identity.
- Required approval MUST bind to that exact source set.
- A changed source set MUST require a new approval.
- Preview MUST not create settlement or external payment effects.

### CRD-004: Settlement finalization

Class: Product contract

- Settlement MUST create one immutable batch.
- A source credit event MUST settle at most once.
- Concurrent settlement runs MUST not select the same source.
- Finalization and external effect intents MUST have a recoverable transaction boundary.
- Missing effect intents after a crash MUST be repairable.
- Settled credit MUST not be removed by ordinary trace withdrawal.
- Corrections MUST use explicit reversal events.

### CRD-005: Payout resolution

Class: Product contract

- The contributor MUST control the payout destination.
- The system MUST not guess between multiple active destinations.
- No destination or ambiguous destinations MUST place settlement on hold.
- Resolving the destination MUST release the hold idempotently.
- Cross-account destination access MUST not create an existence oracle.

### CRD-006: External settlement

Class: Product contract

- External settlement MUST use an outbox or equivalent durable effect mechanism.
- Submission and confirmation MUST be separate observable steps.
- Two workers MUST not submit the same payment twice.
- Adapter failures MUST store only safe error hashes or codes.
- A synthetic dry-run receipt MUST never be represented as a production settlement.

NEAR is the current settlement adapter.
The behavioral contract does not require the control plane to be NEAR-specific.

## 16. Internal operational workflows

Internal HTTP paths and binary names can change.
The following operator outcomes must remain available.

### OPS-001: Scoped worker execution

Class: Product contract

- Each workflow MUST support a dedicated worker identity.
- A worker MUST claim bounded work.
- A claim MUST use a lease or transaction that prevents duplicate execution.
- Failed work MUST retain attempt count, safe error code, and next retry time.
- Retry exhaustion MUST create a visible terminal failure.
- Operators MUST be able to requeue terminal work with an audited reason.

### OPS-002: Privacy backstop

Class: Product contract

- Traces selected for asynchronous privacy checks MUST stay unavailable.
- A failing safety canary MUST stop the processing batch.
- Successful processing MUST create a new redacted artifact revision.
- Release from hold MUST be atomic with the authoritative decision.
- Failure MUST keep the trace held.
- Operators MUST be able to inspect and retry blocked items safely.

### OPS-003: Review operations

Class: Product contract

- Operators MUST see queue depth, age, lease state, and blocked reasons.
- Operators MUST be able to rescrub one item or a bounded batch.
- Batch actions MUST report each item result.
- The system MUST not claim that a human review will occur without an active review workflow.

### OPS-004: Retention maintenance

Class: Product contract

- Operators MUST be able to preview expiration and purge.
- Scheduled maintenance MUST use the same policy as manual maintenance.
- Live purge MUST require explicit authority and purpose.
- Maintenance MUST produce aggregate results and per-item failure records.
- Repeated maintenance MUST converge without duplicate effects.

### OPS-005: Export job operations

Class: Product contract

- Operators MUST see queued, running, failed, stale, and complete jobs.
- Operators MUST be able to recover a stale lease.
- Operators MUST be able to retry a replayable failed job.
- Retries MUST use bounded exponential backoff.
- A permanently failed job MUST remain inspectable.

### OPS-006: Revocation operations

Class: Product contract

- Operators MUST see pending and failed propagation by target kind.
- A worker MUST retry transient failures.
- A terminal failure MUST remain visible until resolved or waived.
- A readiness gate MUST fail while required propagation has terminal failures.

### OPS-007: Classifier deployment operations

Class: Product contract

- Operators MUST inspect active assignments and certification.
- Operators MUST preview an assignment change.
- Operators MUST deploy, pause, roll back, and retire assignments.
- Operators MUST start shadow or active reprocessing.
- Operators MUST inspect progress, failures, and decision distribution.
- A new assignment MUST not silently resume credit before required approval.

### OPS-008: Settlement operations

Class: Product contract

- Operators MUST preview source selection.
- Authorized issuers MUST record source-bound approval.
- Operators MUST finalize a batch once.
- Operators MUST inspect holds and exclusion reasons.
- Operators MUST inspect, retry, and confirm external settlement effects.
- Repair MUST recreate missing outbox intents without duplicating payment.

### OPS-009: Scheduler behavior

Class: Product contract

- A scheduled workflow MUST call the same domain operation as a manual run.
- Scheduler credentials MUST have the narrow worker role.
- Startup MUST validate scheduler configuration.
- A failed tick MUST not terminate unrelated workflows.
- The next tick MAY retry due work.
- Overlapping live runs MUST be rejected or serialized.
- Dry-run scheduling MUST not create live effects.

### OPS-010: Health and readiness

Class: Product contract

- Liveness MUST show whether the process can serve a basic request.
- Readiness MUST show whether required dependencies and controls are usable.
- Readiness MUST distinguish optional disabled features from failed required features.
- Responses MUST expose safe labels only.
- Deployment promotion MUST use readiness, not liveness alone.

### OPS-011: Operational summary

Class: Product contract

Operators MUST be able to answer:

- Are submissions processing?
- Are privacy or review queues growing?
- Are any required classifiers unavailable?
- Are revocations fully propagated?
- Are retention jobs current?
- Are exports blocked or stale?
- Are effect outboxes draining?
- Is settlement blocked?
- Are any audit or tenant-isolation checks failing?

The summary MUST use bounded aggregates.
It MUST not expose trace bodies, identities, tokens, URLs, or transaction identifiers.

### OPS-012: Drills and promotion gates

Class: Product contract

- Operators MUST test critical controls without destructive production effects.
- Each drill MUST return a pass result, safe blockers, time, and evidence identity.
- Required drill results MUST expire after a defined period.
- Promotion MUST fail when required evidence is missing, failed, or stale.
- Repeating a drill MUST be safe.
- A drill MAY append a new audit or evidence record.

Required capabilities include:

- tenant isolation;
- key rotation;
- audit verification;
- database and object consistency;
- retention selection;
- vector rebuild or invalidation;
- withdrawal propagation;
- rollback;
- settlement preview;
- external effect readiness.

The current set of individual drill endpoints is not a permanent API contract.

### OPS-013: Audit and forensic review

Class: Product contract

- Security-sensitive and value-sensitive actions MUST create audit events.
- Audit ordering MUST be tamper evident.
- A verifier MUST detect missing, changed, or reordered records.
- Audit reads MUST themselves be audited when required.
- Operators MUST trace a user-visible outcome to decisions and effects.
- The audit system MUST not store raw sensitive content.

## 17. Current interface compatibility map

This section records the current interface.
It does not make every path a product requirement.

### Stable public compatibility candidates

- `GET /health`
- `GET /.well-known/trace-commons-ed25519-keyset.json`
- `GET /.well-known/trace-commons-attestation-keyset.json`
- `POST /v1/onboard`
- `POST /v1/enroll`
- `POST /v1/trace-upload-claim`
- `POST /v1/traces`
- `DELETE /v1/traces/{submission_id}`
- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `POST /v1/contributors/me/submission-status`
- `GET /v1/contributors/me/score-attestation`

### Account compatibility candidates

- login-link, browser, native, passkey, and NEAR login flows;
- account trace list, detail, content, and withdrawal;
- session logout and revoke-all;
- passkey management;
- payout identity management;
- account merge.

These interfaces SHOULD have a separate compatibility and security review.
Cookie, proof-key, and strong-auth behavior differs from upload-claim behavior.

### Optional public community interfaces

- `GET /v1/community/leaderboard`
- `GET /v1/community/contributors/{handle}`
- `GET /v1/community/analytics/summary`
- `PUT /v1/community/profile`
- `DELETE /v1/community/profile`

These interfaces require a separate product decision.
They are not necessary for trace contribution or consumption.

### Internal compatibility surfaces

The current system exposes many routes under:

- `/v1/review/*`;
- `/v1/workers/*`;
- `/v1/admin/*`;
- `/v1/datasets/*`;
- `/v1/benchmarks/*`;
- `/v1/ranker/*`;
- `/v1/audit/*`;
- `/v1/analytics/*`.

The redesign MAY replace these routes.
Operator clients can use adapters until their workflows move to the new control plane.

## 18. Behaviors to replace

The redesign MUST NOT treat these behaviors as requirements:

- file-backed state as a production source of truth;
- optional best-effort database mirrors;
- plaintext trace-body fallbacks;
- long-lived static bearer tokens;
- HS256 bridge authentication;
- a single signing key that requires a disruptive rotation;
- mixed sentence-only error responses;
- receipt fields that default to `accepted` when absent;
- use of `accepted` before all required controls finish;
- gate requests that append duplicate decisions on retry;
- retry exhaustion that silently removes work;
- mutable classifier behavior without a recorded state boundary;
- one vector namespace for unrelated purposes;
- classifier code that directly changes corpus state;
- unversioned policy decisions;
- untracked side effects;
- synthetic settlement receipts in production;
- manual credit values without a versioned policy source;
- in-process ranking promotion as the only deployment model;
- shadow deduplication that fails open into production decisions;
- mock or deterministic test scorers in production;
- current endpoint count or monolithic binary layout.

## 19. Open product decisions

These questions need product decisions before the contracts become final.

### DEC-001: Public community product

Decide whether public profiles, leaderboard, and aggregate analytics remain part of the product.

### DEC-002: Compatibility duration

Set an end date for `ironclaw.trace_contribution.v1` and current endpoint adapters.

### DEC-003: Contributor explanation depth

Define which classifier and policy reason codes contributors can see.
The answer must balance transparency, privacy, and anti-abuse controls.

### DEC-004: Export revocation guarantee

Define obligations for managed consumers after they download an export.
The system cannot prove deletion from an unmanaged copy.

### DEC-005: Credit finality

This document preserves settled credit after ordinary withdrawal.
Define the separate fraud and operator-correction reversal policy.

### DEC-006: Acceptance supersession

Define when active reprocessing can replace an earlier effective acceptance decision.
The rule must address content already exported under the earlier decision.

### DEC-007: Review service level

Define a maximum quarantine age.
Do not promise review without staffing and monitoring that can meet the promise.

### DEC-008: External consumer API

Decide whether consumers receive direct query access, asynchronous exports, or both.
The authorization and projection contracts apply to either design.

## 20. Acceptance test model

The replacement test suite SHOULD test behavior through public or operator boundaries.
It SHOULD not assert table names or internal call order.

### Test layers

#### Contract schema tests

- Validate every public request and response against its published schema.
- Validate backward-compatible reading of supported versions.
- Reject unsupported required versions.

#### Black-box API tests

- Run the same approved scenarios against old and new systems.
- Compare semantic outcomes instead of unstable text.
- Normalize timestamps, opaque identifiers, and safe correlation values.

#### Workflow tests

- Start each workflow through a public or operator boundary.
- Inject failures at every durable boundary.
- Restart workers and services.
- Verify eventual convergence and no duplicate effect.

#### Policy tests

- Use fixed evidence and facts.
- Execute named policy bundles.
- Verify decisions and safe reasons.
- Verify that a policy change does not rewrite prior decisions.

#### Adapter tests

- Simulate timeout before external acceptance.
- Simulate timeout after external acceptance.
- Simulate duplicate delivery and delayed confirmation.
- Verify idempotent recovery.

#### Security tests

- Attempt cross-tenant and cross-principal access.
- Attempt role escalation.
- Remove each required control.
- Search responses, logs, audit rows, and metrics for seeded secrets.
- Verify that withdrawal remains available after grant removal.

### Minimum end-to-end scenarios

#### SCN-001: New contributor

1. A contributor uses an invitation.
2. The device registers.
3. The device obtains a scoped upload claim.
4. The contributor submits a redacted trace.
5. The contributor sees processing and final status.

Expected result:

- No shared long-lived secret is issued.
- Scope does not exceed the invitation.
- The submission has one logical identity.

#### SCN-002: Safe retry

1. Submission storage succeeds.
2. The response is lost.
3. The contributor retries.

Expected result:

- One submission exists.
- One processing workflow exists for the request.
- No duplicate credit or effect exists.

#### SCN-003: Privacy hold

1. A submitted trace requires an asynchronous privacy check.
2. The privacy adapter is unavailable.
3. The worker retries.

Expected result:

- The trace stays unavailable.
- The contributor sees a processing or held reason.
- Operators see the retry state.

#### SCN-004: Quarantine remediation

1. A trace enters quarantine.
2. The contributor submits a corrected revision.
3. A new processing run accepts it.

Expected result:

- The logical submission remains one contribution.
- Earlier evidence remains available.
- Credit is not duplicated.

#### SCN-005: Consumer export

1. A consumer requests an evaluation export.
2. The source set contains mixed consent.
3. One object read fails.

Expected result:

- Ineligible traces are excluded.
- The failed job is not published as complete.
- The job can retry without changing its source snapshot.

#### SCN-006: Withdrawal

1. An accepted trace exists in vectors and an export manifest.
2. The contributor withdraws it.
3. One deletion adapter fails once.

Expected result:

- New reads stop after the durable withdrawal.
- All required propagation items exist.
- The failed effect retries.
- Settled credit remains unchanged.
- Unsettled credit cannot settle.

#### SCN-007: Bundle deployment and reprocessing

1. A certified bundle becomes active.
2. Existing traces are reprocessed.
3. The operation stops and resumes.

Expected result:

- Every new run uses the selected assignment.
- Earlier decisions remain unchanged.
- Resume does not duplicate effects.
- Shadow mode changes no production state.

#### SCN-008: Settlement crash recovery

1. Settlement finalization commits.
2. The service crashes before external submission.
3. Recovery runs.

Expected result:

- The missing effect intent is repaired.
- Each source settles once.
- The adapter receives one idempotent logical request.

#### SCN-009: Cross-tenant probe

1. Tenant A requests Tenant B's submission.
2. Tenant A guesses object, vector, and export identifiers.

Expected result:

- Each operation is indistinguishable from an unknown identifier.
- No audit, log, metric, or timing response reveals Tenant B's content.

#### SCN-010: Missing required control

1. A required classifier, key, or policy source is unavailable.
2. A live operation starts.

Expected result:

- The operation fails closed.
- No partial production effect occurs.
- The response names only a safe control code.

## 21. Redesign completion rule

The redesign fulfills the old system's required contracts when:

1. Every product contract has passing acceptance tests.
2. Every retained public interface has passing compatibility tests.
3. Every removed interface has an approved replacement or removal decision.
4. Every external effect has an idempotency and recovery test.
5. Every user-visible state has a provenance path to evidence, decisions, and effects.
6. Every behavior in the replace list is absent or isolated in a temporary adapter.

Passing the current unit test suite is not sufficient.
Matching the current table schema or route count is not required.

## 22. Source material

This contract was extracted from:

- `docs/trace-spec.md`;
- `docs/trace-commons.md`;
- `docs/trace-commons-storage.md`;
- `docs/upload-claim-issuer.md`;
- `docs/operator/`;
- `crates/trace-commons-protocol/`;
- the ingest and upload-claim issuer route definitions;
- current PostgreSQL migrations and storage implementations;
- current contributor and operator clients.

When these sources conflict, this document selects the behavior that serves a stated user or operator need.
It records known compatibility paths separately from target product behavior.
