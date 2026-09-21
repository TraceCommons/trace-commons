# Trace Commons System Behavioral Contracts

> Acceptance specification for the target architecture in
> [the versioned pipeline design](./2026-09-09-versioned-pipeline-design.md).

- **Status:** Proposed
- **Date:** 2026-09-11
- **Scope:** System behavior, pipeline behavior, recovery, security, and
  acceptance tests

## 1. Purpose

This document defines the behavior that the completed redesign must provide.
It is the acceptance specification for system tests of the target architecture.

[the versioned pipeline design](./2026-09-09-versioned-pipeline-design.md) is authoritative for architecture. The public
protocol and storage documents remain authoritative for their existing
contracts:

- [`../../trace-commons.md`](../../trace-commons.md)
- [`../../trace-commons-storage.md`](../../trace-commons-storage.md)
- [`../../trace-spec.md`](../../trace-spec.md)
- [`../../upload-claim-issuer.md`](../../upload-claim-issuer.md)

If those documents conflict with the pipeline model, `2026-09-09-versioned-pipeline-design.md` controls
the pipeline design. A compatibility adapter can preserve an existing public
interface without changing the pipeline model.

This specification has two sources:

- Pipeline contracts come from `2026-09-09-versioned-pipeline-design.md`.
- Inherited product contracts come from the public protocol, storage, and
  operator documents.

The proposal does not originate the inherited contracts. The completed system
must satisfy both contract groups.

The system tests must prove outcomes and failure behavior. They must not
require the old table layout, route layout, or classifier implementation.

## 2. Normative language

`MUST` identifies a required acceptance condition. `MUST NOT` identifies a
prohibited result. `CAN` identifies permitted behavior.

Each contract has a stable identifier. An automated test must prove every
completed contract at the stated boundary.

Test tooling CAN map contract identifiers to test identifiers and passing
evidence. This mapping is not a pipeline domain record. Ingest does not need
to persist it.

## 3. System model

### 3.1 Actors

- A **contributor** submits a locally redacted trace and manages its consent.
- A **reviewer** examines traces that Admission quarantines.
- A **customer** reads approved views for an authorized use.
- A **tenant administrator** manages grants and tenant policy.
- A **platform operator** deploys bundles and operates the service.
- A **central issuer** approves internal credit settlement.
- A **worker** performs one scoped class of background work.
- An **external adapter** provides storage, scoring, indexing, or payout
  integration.

### 3.2 Pipeline terms

- A **run** processes one trace under one immutable bundle.
- A **phase** is Admission, Review, Score, or Settle.
- A **policy** is the versioned implementation of one phase.
- A **bundle** selects all four policies and their immutable deployed inputs.
- A **projection** is versioned input that a policy gives to a dependency.
- **Evidence** records the state that a policy used.
- An **evaluation** explains how evidence produced a decision.
- A **decision** is the typed result of one phase.
- An **outcome** stores one phase decision, its evidence, and its evaluation.

The system does not require durable observations, facts, deployment
assignments, generic effect records, vector epochs, or a generic schema
registry.

### 3.3 Pipeline order

The pipeline follows these branches:

1. Admission `Reject` ends the run.
2. Admission `Admit` or `Quarantine` continues to Review.
3. Review `Rejected` ends the run.
4. Review `Approved` continues to Score.
5. Score completion continues to Settle.
6. Settle completion ends the run.

One run uses the same bundle for all four phases. A phase cannot select a
different bundle.

## 4. Cross-cutting contracts

### SYS-001: Tenant isolation

- Every read and write MUST use the tenant from authenticated context.
- Envelope tenant fields MUST provide attribution only.
- A caller MUST NOT detect another tenant's resources.
- Database, object, vector, cache, credit, and export access MUST use the same
  tenant boundary.
- Every tenant table MUST use forced PostgreSQL row-level security.
- Tenant predicates MUST use `trace_current_tenant_id()`.
- Cross-tenant job claims MUST use a narrow role.
- The claimer role MUST update lease columns only.
- All policy work MUST use a tenant-scoped transaction.

**Acceptance:** Seed equal identifiers in two tenants. Exercise every read,
write, claim, object, index, credit, and export path. No path can cross tenants.

### SYS-002: Least privilege

- Contributor, reviewer, administrator, and worker permissions MUST remain
  separate.
- Each worker credential MUST authorize one class of work.
- Worker roles MUST NOT inherit reviewer visibility.
- Missing authority MUST fail before the service reads sensitive content.
- Access grants CAN narrow token authority.
- Access grants MUST NOT increase token authority.
- Self-withdrawal MUST remain available after ordinary access is removed.

**Acceptance:** Call each protected route with every unrelated role. Each
request must fail without a content read or state change.

### SYS-003: Fail-closed behavior

- A missing required policy, key, store, scorer, index, or authority source
  MUST stop the affected operation.
- The system MUST NOT use plaintext or a weaker backend as a fallback.
- A refusal MUST use a stable, safe control label.
- Missing required evidence MUST NOT become favorable evidence.
- An operational error MUST NOT become a phase decision.
- A refusal MUST NOT commit a run, outcome, registry mutation, credit event,
  index write, or external payout.
- An encrypted content-addressed object CAN remain after a later receipt
  failure.
- Orphan cleanup MUST NOT treat that object as a completed submission.
- Live mutation CAN be blocked while dry-run diagnostics remain available.

**Acceptance:** Remove each required dependency in turn. The affected path
must stop without a partial outcome or side effect.

### SYS-004: Privacy

- Trace contribution MUST remain off by default.
- Raw local sessions MUST NOT leave the contributor device.
- The client MUST redact a trace before upload.
- The server MUST treat every upload as untrusted.
- The server MUST apply its own validation and privacy controls.
- Unresolved privacy risk MUST prevent customer access.
- Logs, errors, audits, metrics, and reports MUST NOT contain trace bodies or
  secrets.

**Acceptance:** Seed unique secrets, URLs, account references, and trace text.
Exercise success and failure paths. Search all operational surfaces for the
seeded values.

### SYS-005: Idempotency

- Every retryable mutation MUST have a stable idempotency key.
- A successful retry MUST return or continue the existing logical operation.
- A retry MUST NOT duplicate outcomes, credit, index entries, exports,
  deletion, or external submission.
- The system MUST detect conflicting key reuse.
- A conflict MUST NOT reveal the existing owner.

**Acceptance:** Lose each successful response and retry the request. Then
compare all authoritative records and external adapter calls.

### SYS-006: Provenance

- Each run MUST identify its tenant, trace, request key, content hash, and
  bundle.
- Each completed phase MUST store one outcome.
- Each outcome MUST identify its run, trace, tenant, phase, bundle, and schema.
- Each outcome MUST contain its decision, evidence, and evaluation.
- Evidence MUST record mutable external state that affected the decision.
- Large or sensitive evidence MUST remain in encrypted object storage.
- Stored outcomes MUST use bounded values and content hashes.
- A user-visible result MUST trace to its run and relevant outcomes.
- Each instrument operation MUST identify its Score outcome and instrument.
- The Settle result MUST identify every operation and result by hash.
- A runner MUST persist and honor those references instead of recomputing
  operations from mutable run state.

**Acceptance:** Start from each user-visible state. Walk to the run, bundle,
phase outcomes, evidence, and relevant settlement records.

### SYS-007: Immutable history

- A completed phase outcome MUST be immutable.
- A later policy change MUST NOT rewrite an earlier outcome.
- A retry MUST NOT replace a committed outcome.
- An intervention MUST append a hash-only audit event.

**Acceptance:** Complete a run, change the active bundle, suspend a policy,
and withdraw the trace. The original outcomes must remain byte-identical.

### SYS-008: Schema evolution

- Each outcome MUST contain a stable schema identifier and version.
- The schema MUST identify the decision, evidence, and evaluation payload
  family.
- An incompatible payload change MUST increment the schema version.
- Readers MUST retain support for stored versions during their retention
  period.
- A reader MUST reject an unknown required schema.
- The implementation MUST NOT require a generic schema registry.

**Acceptance:** Read fixtures for all supported versions. Reject an unknown
required version. Confirm that old records stay readable.

### SYS-009: Safe errors and audit

- Existing public APIs MUST keep their published error shape until versioned
  replacements exist.
- Internal errors MUST use stable labels or salted hashes.
- Unknown and inaccessible resources MUST produce indistinguishable results.
- Privileged mutations MUST include a reason and append an audit row.
- Audit rows MUST be hash-only or label-only.
- Audit ordering MUST be tamper-evident.

**Acceptance:** Snapshot public error responses. Change, remove, and reorder
audit fixtures. The verifier must detect each mutation.

### SYS-010: Time and concurrency

- Durable records MUST use server-assigned timestamps.
- Runs, outcomes, leases, commands, and credit events MUST have stable
  identifiers.
- A fenced lease MUST prevent a stale worker from committing work.
- Concurrent workers MUST produce at most one outcome for each run and phase.
- Terminal failures MUST remain visible to operators.

**Acceptance:** Race two workers at every phase. Expire one lease during work.
Only the valid worker can commit the transition. Confirm server timestamps,
stable retry identifiers, and visible terminal failures.

## 5. Identity and authentication contracts

### AUTH-001: Contributor onboarding

- Onboarding MUST bind a generated Ed25519 device key to a tenant and
  principal.
- The device private key MUST remain on the contributor device.
- An invitation MUST have a bounded use count, expiry, or both.
- The server MUST store only the invitation hash.
- The response MUST provide service endpoints and the granted scope ceiling.
- Invalid, expired, and revoked invitations MUST NOT create an existence
  oracle.

Compatibility routes include:

- `POST /v1/onboard`
- `POST /v1/enroll`

**Acceptance:** Onboard with valid, expired, revoked, malformed, and exhausted
credentials. Confirm key ownership and scope bounds.

### AUTH-002: Upload claims

- The issuer MUST authenticate a registered device or workload.
- An upload claim MUST identify issuer, audience, principal, tenant, role,
  issue time, expiry, and token identifier.
- A claim MUST carry consent and allowed-use ceilings.
- The claim lifetime MUST have a server-controlled maximum.
- Ingest MUST validate the signature, algorithm, key identifier, issuer,
  audience, and expiry.
- Ingest MUST NOT hold the upload-claim signing key.
- The issuer MUST publish verification keys.

Compatibility routes include:

- `POST /v1/trace-upload-claim`
- `GET /.well-known/trace-commons-ed25519-keyset.json`

**Acceptance:** Mint through every supported authentication path. Exercise
expiry, key rotation, stale keysets, wrong algorithms, and wrong audiences.

### AUTH-003: Consent and grant intersection

- A contributor MUST choose the requested consent scopes.
- Tenant policy MUST define the maximum scopes and allowed uses.
- The issued authority MUST equal the intersection of request and ceiling.
- An empty permitted scope set MUST be refused.
- `public_attribution` MUST NOT grant trace-content access.
- Each submission MUST retain the granted scopes used at receipt.

**Acceptance:** Request scopes below, equal to, and above the ceiling. Confirm
that no response increases authority.

### AUTH-004: Account sessions

- Human account sessions MUST remain separate from upload claims.
- Login codes MUST be single-use and short-lived.
- Native login MUST bind authorization to proof-key material.
- Sensitive changes MUST require recent strong authentication.
- Logout MUST end the current session.
- Revoke-all MUST invalidate all account sessions.
- Account merge MUST require control of both identities.
- Account merge MUST be irreversible and retain credit history.

**Acceptance:** Exercise session creation, replay, rotation, strong-auth
gates, logout, revoke-all, and account merge.

### AUTH-005: Service credentials

- A service credential MUST identify its tenant, principal, role, issuer,
  audience, and expiry.
- Its authority MUST be no wider than its worker task.
- Operators MUST be able to issue, rotate, and revoke service access.
- Raw credentials MUST NOT appear in reports, logs, commands, or audits.
- Long-lived static production tokens MUST NOT be required by the target
  design.

**Acceptance:** Rotate a credential while work is in flight. New claims must
use the new key. Bounded in-flight work can finish.

## 6. Submission and Admission contracts

### SUB-001: Envelope boundary

- The public submission format MUST remain versioned.
- The compatibility format is `ironclaw.trace_contribution.v1`.
- The server MUST reject unsupported required versions.
- Public DTOs MUST remain in `trace-commons-protocol`.
- Client-provided scores, retention classes, and tenant fields MUST NOT grant
  authority.

**Acceptance:** Process golden valid envelopes and one invalid fixture for
each schema rule.

### SUB-002: Local privacy

- Contribution MUST require explicit opt-in.
- Message text and tool payload inclusion MUST be explicit.
- The client MUST run the configured local redaction pipeline.
- Residual key-shaped secrets MUST stop the upload.
- An optional remote PII filter MUST receive redacted prose only.
- A required unavailable remote filter MUST stop the batch.
- A redaction canary failure MUST stop the batch.
- Full local paths MUST NOT appear in the envelope.

**Acceptance:** Use golden source sessions with secrets, paths, tool payloads,
and canary values. No prohibited value can cross the wire.

### SUB-003: Durable receipt

- Submission MUST require an authenticated contributor claim.
- The service MUST enforce claim, grant, consent, and tenant policy.
- The service MUST store the encrypted trace before it confirms custody.
- The service MUST resolve and validate the active bundle.
- Admission MUST execute before the receipt transaction commits.
- The transaction MUST commit the run, Admission outcome, and next phase
  together.
- The receipt MUST identify the logical submission.
- The receipt MUST distinguish custody and pipeline progress from final credit.

**Acceptance:** Inject a failure before and after each receipt transaction
step. A success must always resolve to a run and Admission outcome.

### SUB-004: Request idempotency

- The request key MUST be unique within a tenant.
- The request key MUST bind to the request-content hash.
- An identical replay MUST return the existing run.
- Reuse with different content MUST fail.
- A retry MUST NOT consume another quota unit.
- A retry MUST NOT create another credit event.

**Acceptance:** Replay the same bytes, changed bytes, and the same key in a
different tenant. Confirm the exact run counts.

### SUB-005: Admission behavior

- Admission MUST use bounded local work.
- Admission MUST produce `Admit`, `Quarantine`, or `Reject`.
- Admission MUST validate schema, contribution path, tenant grant, consent,
  allowed uses, quotas, and rate limits.
- Admission MUST own synchronous privacy-risk handling.
- Model-based substance and novelty valuation MUST NOT run in Admission.
- Admission CAN read an index.
- Admission MUST NOT modify an index or make another external write.
- Admission evidence MUST identify each detector, index, and projection that
  affected its decision.
- `Admit` and `Quarantine` MUST both continue to Review.
- `Reject` MUST complete without registry admission.

**Acceptance:** Use one fixture for each decision and validation branch.
Confirm the next phase, evidence, and absence of external writes.

### SUB-006: Rate and quota limits

- Submit limits MUST apply per authenticated principal and tenant policy.
- An idempotent retry MUST NOT count as a new request for quota purposes.
- A limit refusal MUST use a stable safe label.
- A limit implementation MUST enforce the configured system-wide bound.

**Acceptance:** Exercise each limit across concurrent service instances.
Confirm that retries do not increase the accepted logical count.

### SUB-007: Revoked-content tombstones

- Withdrawal, revocation, expiry, and purge MUST leave hash-only tombstones.
- A tombstone MUST outlive readable content.
- Ingest MUST refuse previously withdrawn content for the tenant.
- Export MUST exclude tombstoned content.
- The refusal MUST NOT reveal contributor identity.

**Acceptance:** Remove a trace, then resubmit its identifiers and content
hashes through every supported submission path.

## 7. Bundle and policy contracts

### BND-001: Bundle identity

- The manifest MUST include its format version.
- A bundle manifest MUST select one policy for each phase.
- Each policy reference MUST identify its policy, code, configuration, data
  artifacts, and projection identifiers.
- The bundle identifier MUST be the canonical hash of the manifest.
- The bundle hash MUST exclude mutable external state.
- A change to a listed immutable input MUST change the bundle identifier.

**Acceptance:** Use golden manifests. Change each identity input separately.
Change the format version and each policy identifier. Confirm the expected
bundle identifiers.

### BND-002: Package integrity

- The bundle package MUST contain its manifest and deployable artifacts.
- The ingest service MUST validate the package against its bundle identifier.
- A missing or mismatched artifact MUST prevent activation.
- The bundle registry MUST retain packages needed by stored outcomes.
- The ingest database MUST NOT require lab reports or lab run identifiers.

**Acceptance:** Tamper with each package component. Registration or loading
must fail before a run uses the package.

### BND-003: Run binding

- The server MUST bind the active tenant bundle during run creation.
- The binding MUST occur before Admission executes.
- All phases MUST use the run's bound bundle.
- A retry MUST retain the same bundle.
- An activation change MUST affect new runs only.
- A rollback MUST NOT rewrite outcomes or move an existing run.

**Acceptance:** Change the active bundle between every pair of phases. The
existing run must retain its original bundle. Also race activation after
bundle resolution and before Admission commits. Admission and the run must use
the resolved bundle.

### BND-004: Policy interfaces

- Phase traits, result types, and decision types MUST live in
  `trace-commons-gate-api`.
- Client DTOs MUST live in `trace-commons-protocol`.
- Runners MUST hold policies as trait objects.
- Policies MUST hold scorers, embedders, indexes, and credit adapters as
  trait objects.
- A test policy MUST implement the same production trait.
- The implementation MUST NOT require a mock-only policy hierarchy.

**Acceptance:** Run phase and bundle tests with small test implementations
through the production traits.

## 8. Review contracts

### REV-001: Review input boundary

- Review MUST run asynchronously after Admission.
- Review MUST use the stored contribution and server-generated evidence only.
- Review MUST NOT collect more contributor-side data.
- Review MUST identify the exact source artifact with
  `request_content_hash`.
- Review CAN query an index.
- Review MUST NOT modify an index.

**Acceptance:** Attempt to supply local source data or a mismatched artifact.
Review must fail before transformation or registry mutation.

### REV-002: Review decision and registry commit

- Review MUST produce `Approved` or `Rejected`.
- An approved result MUST identify the approved registry revision.
- The outcome MUST bind the source hash to the approved registry revision.
- Review MUST store transformed encrypted content by content hash first.
- One transaction MUST commit its reference, registry revision, Review
  outcome, and Score transition.
- A rejected trace MUST NOT enter the registry.
- A rejected run MUST NOT continue to Score.

**Acceptance:** Inject failures around object storage and the transaction.
No state can show an approved revision without its Review outcome.

### REV-003: Quarantine review operations

- Reviewers MUST see tenant-authorized quarantine items only.
- Content reads MUST use scoped object references and append audits.
- Review claims MUST use exclusive leases.
- A lease MUST expire or support release.
- A stale reviewer MUST NOT overwrite a committed decision.
- Approve and reject actions MUST include a non-empty reason.
- A decision MUST apply to an operable, quarantined trace only.
- Human action MUST become server-generated evidence for the bound Review
  policy.
- A reviewer MUST NOT commit a bundle-independent `ReviewDecision`.
- Approval MUST show how each Admission quarantine reason was resolved.
- The bound Review policy MUST produce the `ReviewDecision`.
- Review approval MUST NOT bypass a later required phase.

**Acceptance:** Race two reviewers. Revoke and expire items during review.
Only one valid assessment can become policy evidence. If the assessment does
not resolve every quarantine reason, approval must fail.

### REV-004: Review transformation

- A production Review policy CAN apply deterministic PII transformation.
- A test Review policy CAN pass content through.
- Transformation evidence MUST identify the source and produced artifact.
- A transformation MUST NOT hide missing evidence with a favorable default.

**Acceptance:** Compare pass-through, transformed, rejected, missing-evidence,
and retry fixtures.

## 9. Score contracts

### SCR-001: Score input

- Score MUST run only after Review approves a registry revision.
- Score MUST use the committed Review outcome and the bound bundle.
- Score CAN use a fixed amount, model measurements, or index queries.
- Score MUST NOT define one objective value for a trace.
- Score MUST record every mutable input that affected the decision as
  evidence.

**Acceptance:** Run fixed and model-backed test policies. Each outcome must
explain its own credit decision.

### SCR-002: Index access

- Score CAN query an active index through a read-only capability.
- Score MUST NOT modify an active index.
- When Score queries an index, evidence MUST identify the index and snapshot
  state used.
- If cardinality, coverage, or neighbors affect the decision, evidence MUST
  record their bounded values.
- Evidence MUST identify every projection and measured value used.
- A query MUST exclude the same reviewed revision.
- A shadow comparison MUST use an isolated index namespace.
- A shadow comparison MUST NOT affect an active decision or credit event.

**Acceptance:** Give Score a test index that detects writes. Attempt an active
and shadow comparison. No active index mutation can occur.

### SCR-003: Instrument units and decision

- Score MUST return an ordered collection keyed by bounded `instrument_id`.
- A decision MUST reject duplicate instrument identifiers.
- Each award MUST use checked integer atomic units.
- An empty collection MUST remain distinct from an incomplete Score phase.
- Trace Credit MUST use the `trace_credit` instrument.
- One Trace Credit MUST equal 1,000,000 microcredits.
- Its adapter conversion MUST be exact and MUST reject overflow and excess
  decimal precision.

**Acceptance:** Exercise an empty award set, two simultaneous instruments,
deterministic ordering, duplicate identifiers, maximum values, overflow,
negative source input, and excess-precision Trace Credit conversion.

### SCR-004: Score persistence and instrument operations

- The Score outcome and next phase MUST commit atomically.
- Each positive award MUST create one eligible instrument operation.
- The operation key MUST identify the tenant, run, Score outcome, and
  instrument.
- A retry MUST NOT create another operation.
- An embedding needed by Settle MUST remain encrypted evidence.
- The Score outcome MUST store only its artifact hash.

**Acceptance:** Crash before and after the Score transaction commits. An empty
award set creates no operation. A committed two-instrument Score creates
exactly one stable operation for each instrument.

### SCR-005: External valuations

**Status:** Deferred until the external valuation protocol is specified.

- A signature MUST prove attribution and integrity only.
- A multi-party policy MUST define signer eligibility, quorum, expiry,
  ranges, and aggregation before production use.
- If the policy requires trust evidence, missing evidence MUST fail closed.
- The outcome MUST retain bounded attestation evidence or its content hash.
- A bundle that needs external valuations MUST NOT activate before this
  contract has tests.

**Acceptance:** The follow-up protocol must define these tests.

## 10. Settle contracts

### STL-001: Settle input and decision

- Settle MUST run after every completed Score phase.
- Settle MUST use the committed Review and Score outcomes.
- Settle MUST use the same bound bundle.
- Settle MUST decide index membership from those immutable inputs only.
- Settle MUST NOT query mutable index state to make the membership decision.
- Settle MUST NOT repeat Score valuation work.
- The submission guard CAN force `Exclude` without a content or index read.
- The guard result MUST appear in Settle evidence and evaluation.

**Acceptance:** Change the live index after Score. Settle must not read that
state while it makes the membership decision. Make the submission inoperable
and confirm guard-driven exclusion.

### STL-002: Deterministic index command

- An included revision MUST use deterministic entry keys.
- Each key MUST cover tenant, index, revision, projection, model, and chunk.
- Settle MUST store the encrypted command and command hash before the write.
- A retry MUST use the stored command.
- A retry MUST NOT repeat the membership decision.
- An equal key and equal content MUST be a successful no-op.
- An equal key and different content MUST fail closed.

**Acceptance:** Vary tenant, index, revision, projection, model, and chunk.
Each change must change the entry key. Confirm that encrypted command storage
precedes the first write. A retry must reuse the sealed command bytes.

### STL-003: Independent operation progress

- Index application and every instrument settlement MUST record independent
  progress.
- A failure for one instrument MUST NOT corrupt or repeat another instrument.
- A Trace Credit hold MUST NOT alter the index decision or another instrument.
- Settle MUST wait for all required internal operations.
- Settle MUST preserve retry information for each incomplete operation.

**Acceptance:** Fail the index and two instrument paths independently. Recover
each path without repeating completed operations. No Settle outcome can exist
until all required operations complete.

### STL-004: Instrument settlement integration

- Settlement MUST use the eligible operations created from the Score outcome.
- Settle MUST process every awarded instrument, not only the first one.
- The Settle decision MUST return every instrument identifier, atomic amount,
  operation reference, and result reference in deterministic order.
- The persisted operations MUST exactly match the committed Score awards.
- The Trace Credit adapter MUST preserve account-level batching, holds, caps,
  issuer approval, source-list approval, and duplicate-credit protection.
- Concurrent Trace Credit batches MUST NOT select the same event.
- Raw account references MUST NOT appear in outcomes.

**Acceptance:** Settle two simultaneous instruments and fail each one once.
Verify independent retry, exact amounts, deterministic ordering, and stable
operation and result references. Run Trace Credit previews, approvals, holds,
caps, concurrent settlement, and duplicate-source fixtures.

### STL-005: Settle completion and NEAR

- Settle completion MUST mean that required internal index and credit
  operations are complete.
- A disabled or pending NEAR outbox item MUST NOT delay the Settle outcome.
- External NEAR state MUST NOT change a committed Settle outcome.
- Outcomes, audits, logs, reports, and operational responses MUST remain
  hash-only or label-only.
- External submission and confirmation MUST remain separate.
- Two workers MUST NOT submit the same logical payout twice.

**Acceptance:** Complete internal settlement with each NEAR state. Then retry
submission and confirmation around injected crashes.

## 11. Persistence and recovery contracts

### RUN-001: Run state

- `pipeline_runs` MUST contain immutable run identity and mutable recovery
  state.
- The run MUST record the next phase, state, lease, attempts, and retry time.
- The run MUST retain safe error labels only.
- The run MUST record index and credit progress needed for recovery.
- Immutable phase history MUST remain in `phase_outcomes`.
- A terminal infrastructure error MUST NOT create a phase outcome.

**Acceptance:** Force retryable and terminal errors in each phase. Inspect the
run and confirm the absence of fabricated outcomes.

### RUN-002: Outcome uniqueness

- `phase_outcomes` MUST contain immutable history.
- The database MUST enforce one outcome for each tenant, run, and phase.
- A phase transition and its outcome MUST commit in one transaction.
- A concurrent retry MUST resolve to the committed outcome.

**Acceptance:** Commit the same phase through concurrent connections. Exactly
one outcome can exist.

### RUN-003: Fenced leases

- A worker MUST claim a run with a fenced lease.
- A lease MUST have a bounded lifetime.
- A stale lease token MUST NOT commit an outcome or side effect.
- A worker MUST load the bundle already bound to the run.
- Retry exhaustion MUST leave visible terminal state.

**Acceptance:** Pause a worker past lease expiry. Let another worker finish.
The first worker must fail at commit. Expire leases before adapter calls.
Stable commands and event keys must prevent a second logical operation.

### RUN-004: Crash matrix

System tests MUST inject a crash at these boundaries:

1. After encrypted body storage and before the receipt transaction.
2. After Admission work and before the Admission outcome commit.
3. After Review transformation storage and before registry commit.
4. After Review commit and before the response to the worker.
5. After Score work and before its outcome commit.
6. After the Score outcome and credit event commit.
7. After Settle command storage and before index application.
8. After index application and before local completion storage.
9. After internal settlement and before Settle outcome commit.
10. After Settle completion and before NEAR outbox submission.
11. After external submission and before confirmation storage.

Every case MUST converge without duplicate outcomes, credit, or index content.
Repeated payout attempts MUST use one idempotency key. The adapter must accept
one logical payout.

### RUN-005: Backup and restore

**Status:** Inherited operational contract. Detailed migration qualification
remains deferred.

- PostgreSQL recovery MUST retain runs, outcomes, credit events, batches, and
  audit order.
- Object recovery MUST retain encrypted artifacts and content-hash integrity.
- The index MUST be rebuildable from authoritative revisions and stored
  commands.
- Index rebuild MUST NOT create new phase outcomes or credit events.
- Restore qualification MUST include the crash matrix.

**Acceptance:** Restore a qualified snapshot and replay pending work. Compare
all authoritative identifiers and hashes.

## 12. Phase guard contracts

### GRD-001: Guard boundaries

- Each phase MUST run its guard before it reads content or starts policy work.
- Each phase MUST run its guard inside the commit transaction.
- The final guard MUST lock applicable submission and policy-status rows.
- The operation that commits first MUST define the ordering.
- For an index write, Settle MUST hold these locks until command-result commit.

**Acceptance:** Race withdrawal, revocation, retention expiry, consent change,
allowed-use change, and suspension against each applicable phase boundary.
Race withdrawal after index dispatch and before command-result commit.

### GRD-002: Submission operability

- Review and Score MUST require an operable submission.
- Consent and allowed uses MUST authorize the phase.
- Withdrawal, revocation, and retention expiry MUST make content inoperable.
- Settle index work MUST require an operable submission.
- Credit settlement CAN use a committed Score outcome without reading content.

**Acceptance:** Apply every lifecycle event before work and before commit.
Confirm the permitted content and credit behavior.

### GRD-003: Withdrawal ordering

- Withdrawal before the Settle decision MUST cause index exclusion.
- Withdrawal after command storage MUST stop a pending index command.
- Withdrawal after index completion MUST invoke the existing invalidation
  path.
- Withdrawal after Score commits MUST NOT remove awarded credit.
- Existing settlement CAN finalize that committed credit.
- Ordinary withdrawal MUST NOT claw back settled credit.

**Acceptance:** Withdraw at every boundary from Admission through external
payout. Confirm index and credit results.

### GRD-004: Policy suspension

- Every phase MUST require a runnable bound policy.
- Suspension MUST NOT change the bundle identifier.
- Suspension MUST NOT move the run to another bundle.
- A suspended policy MUST leave the run retryable with a safe label.
- The run CAN resume only under the same bundle after policy resumption.
- The NEAR worker MUST repeat its policy guard before dispatch.
- A guard MUST NOT claim to retract an accepted external operation.

**Acceptance:** Suspend each policy during work and before commit. Resume it
and confirm that the same run completes under the same bundle. Suspend the
policy after Settle and before NEAR dispatch. Dispatch must wait for resumption.

## 13. Contributor status contracts

### STA-001: Submission status

- `POST /v1/contributors/me/submission-status` MUST derive state from runs,
  outcomes, credit records, and the existing outbox.
- The response MUST distinguish processing, credit, and payout state.
- Unknown and unowned identifiers MUST be omitted or returned identically.
- Batch requests MUST have a documented maximum.
- Status MUST use stable safe reason labels.
- Status MUST NOT require a new durable read-model table.

**Acceptance:** Create one fixture for every phase and terminal state. Compare
the response with authoritative records.

### STA-002: Processing state

- A pending Review state MUST mean that Review has no outcome.
- A pending Score state MUST mean that Review approved and Score is due.
- A retry state MUST remain distinguishable from terminal failure.
- A policy suspension MUST show a safe blocked reason.
- A rejected phase MUST show the responsible phase and reason.

**Acceptance:** Pause and fail each phase. Confirm contributor and operator
views.

### STA-003: Credit and payout state

- Zero credit MUST mean that Score completed with zero credit.
- Unscored MUST mean that Score has no outcome.
- Held credit MUST identify an existing account hold by a safe label.
- Finalized credit MUST mean that an approved internal batch finalized it.
- Payout MUST expose `disabled`, `pending`, `submitted`, `confirmed`, or
  `failed`.
- A payout hold MUST remain separate from internal credit state.
- A payout hold MUST use the existing safe payout-hold reason.

**Acceptance:** Combine each credit state with each valid payout state. No
state can imply an operation that did not complete.

### STA-004: Account trace access

- An account holder MUST be able to list owned submissions with stable
  pagination.
- An owner CAN read retained, redacted content.
- The service MUST never return the original local trace.
- Every content read MUST append an audit event.
- Unknown and unowned trace identifiers MUST produce the same response.
- Decryption or integrity errors MUST fail closed without raw detail.

**Acceptance:** Read owned, foreign, removed, corrupt, and empty result
fixtures.

### STA-005: Signed score attestation

- A signed score statement MUST identify its schema and bundle.
- The principal MUST come from authenticated context.
- Signing MUST be all-or-nothing.
- An unavailable signing key MUST return a safe missing-control label.
- Verification keys MUST be public and selectable by key identifier.

**Acceptance:** Verify a valid statement offline. Exercise missing, stale,
rotated, and wrong-key cases.

## 14. Credit contracts

### CRD-001: Credit meaning

- Credit MUST remain a non-transferable record.
- Upload custody MUST NOT mean payment.
- Score MUST assign credit through a versioned policy.
- Settlement MUST remain a separate governed operation.
- No route MUST transfer, sell, or withdraw credit as currency.

**Acceptance:** Inspect every positive credit path. Each entry must identify
an authorized Score outcome or existing governed event.

### CRD-002: Ledger integrity

- The credit ledger MUST be append-only.
- Every event MUST identify its source and authority.
- A source event MUST settle at most once.
- Reviewer and worker roles MUST NOT create unauthorized positive credit.
- Corrections MUST use explicit reversal events.

**Acceptance:** Retry, race, reverse, and cross-role credit writes. Compare
the event ledger and account totals.

### CRD-003: Preview and approval

- A dry run MUST produce a canonical source-list hash.
- Approval MUST bind to that exact list, policy, and evidence.
- A changed source list MUST require a new approval.
- An unlisted principal CAN run a dry run.
- An unlisted principal MUST NOT approve or finalize positive credit.
- A dry run MUST NOT change credit or create an external outbox item.

**Acceptance:** Change each approved input after preview. Live settlement must
refuse the stale approval.

### CRD-004: Holds, caps, and payout identity

- Settlement MUST exclude held accounts.
- Settlement MUST apply configured per-account caps.
- No payout identity or ambiguous identities MUST create a hold.
- The system MUST NOT guess a payout destination.
- Hold release MUST be idempotent.
- Cross-account payout operations MUST NOT create an identity oracle.

**Acceptance:** Exercise zero, one, multiple, and designated payout
identities with held and capped accounts.

### CRD-005: No-clawback rule

- Ordinary withdrawal MUST leave committed Score credit unchanged.
- Existing settlement CAN finalize credit committed before withdrawal.
- Withdrawal MUST prevent new content reads and index membership.

**Acceptance:** Before Score, withdrawal prevents a Score outcome and credit
event. After Score, credit can finalize without a content read. Later
withdrawal does not claw back finalized credit.

## 15. Lifecycle contracts

### LIF-001: Withdrawal and revocation

- An owner MUST be able to stop future use of a trace.
- Withdrawal and revocation MUST be idempotent.
- New customer reads MUST stop after the durable lifecycle record.
- Downstream invalidation MUST remain visible until complete.
- A failed invalidation MUST remain visible to operators.
- The response MUST NOT reveal a foreign submission.
- Distribution reach MUST report known managed distribution only.

**Acceptance:** Remove a trace that exists in the registry, index, cache, and
export manifest. Fail each invalidation target once.

### LIF-002: Retention

- The server MUST derive retention from stored consent and allowed use.
- Client fields MUST NOT extend retention.
- Expired content MUST become inoperable before later phases read it.
- Purge MUST remove readable payloads and invalidate derived artifacts.
- Purge MUST keep minimum hash-only audit and tombstone data.
- A legal hold CAN delay purge.
- A legal hold MUST NOT authorize prohibited use.
- Live purge MUST require an authorized purpose.

**Acceptance:** Process each retention class through expiry, legal hold,
release, and purge.

### LIF-003: Revocation propagation

- Each propagation target MUST have bounded retries.
- An exhausted target MUST enter a visible terminal state.
- Required terminal failures MUST block a clean readiness result.
- A synthetic no-op MUST NOT appear as successful invalidation.
- Managed export manifests MUST record invalidation progress.
- The system MUST NOT claim deletion from an unmanaged copy.

**Acceptance:** Break each adapter, exhaust retries, and recover the target.
Confirm readiness and contributor reach reporting.

## 16. Customer and export contracts

### EXP-001: Authorized selection

- Customer access MUST require tenant, principal, role, and declared use.
- Selection MUST include an approved and operable registry revision only.
- Selection MUST intersect contributor consent with customer authority.
- Selection MUST enforce privacy, withdrawal, expiry, and legal restrictions.
- Missing policy or storage evidence MUST exclude the trace.
- Public attribution MUST NOT grant content access.

**Acceptance:** Use a matrix of consent, use, risk, lifecycle, grant, and
storage states. Compare the exact selected identifier set.

### EXP-002: Authorized export view

- A customer MUST receive an authorized view for the requested use.
- The view MUST include permitted fields only.
- The view MUST identify its schema and source revision.
- The view MUST preserve consent and bundle provenance.
- A customer MUST NOT receive raw local or unredacted content.

**Acceptance:** Compare each use-specific response with a field allowlist.
Seed forbidden values in every omitted field.

### EXP-003: Export jobs

- An export request MUST state an allowed use and purpose.
- The job MUST use an immutable source snapshot.
- The job MUST record the selection policy.
- Claims MUST be exclusive and recoverable.
- A retry MUST retain or explicitly link the source snapshot.
- A partial output MUST NOT appear as complete.
- A completed export MUST include a manifest and source-list identity.
- Revocation and expiry MUST invalidate managed membership.

**Acceptance:** Fail one object read, restart the worker, and revoke one
source. The published manifest must remain complete and current.

### EXP-004: Derived artifacts

- Derived artifacts MUST retain source revision, authorized-view schema,
  consent, policy, and bundle provenance.
- Benchmark creation MUST require benchmark authority.
- Ranking and training creation MUST require their corresponding authority.
- External publication MUST use an idempotent outbox or equivalent mechanism.
- Revocation MUST identify every managed derived artifact.

**Acceptance:** Create each artifact type, revoke a source, and confirm every
managed invalidation.

## 17. Optional community contracts

These contracts apply only to an enabled community surface.

### COM-001: Public attribution

- Public attribution MUST require explicit consent.
- A profile write MUST require `public_attribution`.
- Handles MUST remain bounded, unique, and validated.
- Profile withdrawal MUST remove the handle from served snapshots.
- A stale withdrawn handle MUST NOT be served.

**Acceptance:** Create, replace, collide, and withdraw profiles across
snapshot refresh boundaries.

### COM-002: Public aggregate privacy

- Public aggregate cells MUST enforce the configured minimum count.
- Broad release MUST require the configured privacy controls.
- Missing controls MUST withhold data and return safe labels.
- A disabled community surface MUST return not-found responses.
- Public responses MUST NOT reveal tenant or contributor secrets.

**Acceptance:** Query below and above each privacy bound with controls present
and absent.

## 18. Operator contracts

### OPS-001: Health and readiness

- Liveness MUST report whether the process can serve a basic request.
- Readiness MUST report required dependency and control state.
- Readiness MUST distinguish disabled optional features from failed required
  features.
- Responses MUST expose safe labels, booleans, counts, and hashes only.
- Promotion MUST use readiness, not liveness alone.

**Acceptance:** Exercise the complete startup matrix and snapshot all safe
response fields.

### OPS-002: Bounded worker operation

- Every background worker MUST claim bounded work.
- Claims MUST use leases or equivalent transaction fencing.
- Retries MUST use bounded backoff and a maximum attempt count.
- Stale work MUST support safe recovery.
- A manual and scheduled invocation MUST call the same domain operation.
- Overlapping work MUST serialize or refuse the second invocation.
- Dry-run work MUST NOT create live mutations.

**Acceptance:** Exercise invalid limits, overlap, stale leases, retry
exhaustion, manual runs, and scheduled runs.

### OPS-003: Operational summary

Operators MUST be able to determine:

- The count and age of runs in each phase and state.
- Which bound policies are suspended.
- Which phase errors are retryable or terminal.
- Whether index commands are pending or failed.
- Whether internal credit settlement is held or delayed.
- Whether the NEAR outbox is draining.
- Whether revocation, retention, and export work is blocked.
- Whether audit and tenant-isolation controls pass.

The summary MUST use bounded safe aggregates.

**Acceptance:** Seed each blocked state. Confirm that the summary identifies
the class without sensitive data.

### OPS-004: Drills and promotion

- Each critical control MUST have an idempotent drill.
- A drill MUST return pass, fail, safe blockers, time, and evidence hash.
- Required evidence MUST have a maximum age.
- Promotion MUST fail for missing, failed, or stale required evidence.
- Repeating a drill MUST NOT create a production side effect.

Required drills include:

- Tenant isolation.
- Bundle package integrity.
- Bundle activation and rollback.
- Phase outcome atomicity.
- Fenced lease recovery.
- Settle command recovery.
- Index idempotency and conflict handling.
- Settlement preview and approval.
- NEAR outbox recovery.
- Withdrawal propagation.
- Key rotation.
- Audit-chain verification.
- Backup and restore.

### OPS-005: Forensic traceability

An operator MUST be able to answer:

- Which bundle processed a run?
- Why did each phase produce its decision?
- Which mutable external state affected Score?
- Which index command did Settle seal?
- Which Score outcome authorized a credit event?
- Which batch finalized the credit?
- Which interventions stopped or resumed work?
- Which invalidation targets remain incomplete?

**Acceptance:** Build each answer from stored hashes, labels, outcomes, and
authoritative operational records.

### OPS-006: Key management

- Per-object data keys MUST use a key wrapper with context binding.
- A local wrapper MUST remain development-only.
- A production deployment MUST require a production trust boundary.
- Key rotation MUST support staged decrypt and encrypt versions.
- The rotation drill MUST prove wrap and unwrap.
- Loss of required key material MUST fail closed.

**Acceptance:** Rotate, disable, restore, and mismatch key versions across
stored artifacts.

## 19. Policy development contracts

### LAB-001: Separation from ingest

- Policy development MUST occur outside the ingest path.
- If a policy uses calibration, the lab MUST use a versioned corpus and
  immutable input digest.
- For calibrated policies, bootstrap and holdout data MUST remain separate.
- Each calibration MUST produce a local outcome report.
- The lab MUST build a deployable bundle package.
- The ingest database MUST NOT store lab runs or calibration reports.
- A lab catalog MUST map bundle identifiers to development records.

**Acceptance:** Build a bundle from a fixed corpus. Confirm that ingest needs
the package only, not the lab database.

### LAB-002: Test levels

The implementation MUST provide:

1. Policy tests with typed fixtures.
2. Phase-runner tests through production policy traits.
3. Bundle tests across all four phases.
4. Integration tests with isolated indexes and settlement adapters.
5. Golden bundle-identity tests.
6. Compatibility-corpus tests against current behavior.

Bundle tests MUST assert all four decisions, evidence shapes, evaluation
shapes, and the bundle identifier. External payout MUST remain disabled in
all policy, runner, bundle, and integration tests.

### LAB-003: Activation

- A compatibility bundle MUST reproduce current results on a fixed corpus.
- A compatibility bundle MUST pass policy, phase, bundle, and integration
  tests before activation.
- Activation MUST affect new runs only.
- Rollback MUST select an earlier active bundle for new runs.
- Old outcomes and packages MUST remain readable through retention.
- New valuation rules MUST follow stable compatibility transitions.

**Acceptance:** Promote and roll back two bundles while runs are active.
Compare new and existing run bindings.

## 20. Compatibility and replacement contracts

### CMP-001: Public compatibility

The compatibility release MUST preserve these public surfaces unless a
versioned replacement ships with its client:

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

Internal worker, review, administration, export, benchmark, and ranking routes
CAN change after equivalent operator behavior exists.

### CMP-002: Behaviors not preserved

The target system MUST NOT preserve these implementation requirements:

- File-backed production metadata.
- Best-effort database mirrors.
- Plaintext artifact fallbacks.
- Long-lived static production bearer tokens.
- HS256 bridge authentication in production.
- A single disruptive key-rotation model.
- Classifier-specific decision columns.
- `gate_version_hash` as the complete policy identity.
- Column-specific rescore routes as the target reprocessing model.
- Index insertion during Score.
- Unversioned policy decisions.
- Generic durable observations or facts.
- Durable deployment assignments as a pipeline domain concept.
- Generic effect-intent and effect-receipt tables.
- Vector epochs as a required domain concept.
- Timestamp order as bundle activation.
- Synthetic settlement receipts in production.
- Mock scorers in production.
- The current endpoint count or monolithic binary layout.

### CMP-003: Deferred work

**Status:** Deferred contracts do not block initial architecture completion.
They become required only after their named follow-up specification is
adopted.

These items are not acceptance conditions for the initial architecture:

- Production reprocessing.
- Supersession of an earlier effective decision.
- Deterministic replay of unavailable external inputs.
- A generic schema registry.
- A new lab service or lab database.
- An objective value for a trace.

These items require follow-up specifications before production use:

- Exact phase payloads and reason codes.
- Bundle package signature and retention rules.
- External valuation and attestation protocols.
- Vector adapter idempotency and self-exclusion.
- Settlement batch and NEAR integration details.
- Policy suspension, resumption, and termination controls.
- Migration qualification.

### CMP-004: Open product decisions

**Status:** Open product decisions do not change the pipeline model.

The product must decide:

- The support period for existing public interfaces.
- The reason-code detail visible to contributors.
- The guarantee for deletion from unmanaged customer copies.
- The correction policy for fraud or operator error.
- The remediation flow for a quarantined submission.
- The maximum age of a quarantine item.
- Whether customers use direct queries, exports, or both.
- Whether the optional community surface remains a product feature.

## 21. Acceptance test suite

### 21.1 Test layers

The completed system MUST pass all applicable layers:

1. Protocol schema tests.
2. Policy contract tests.
3. Phase-runner tests.
4. Bundle identity and golden-corpus tests.
5. PostgreSQL transaction and RLS tests.
6. Adapter idempotency tests.
7. Crash-recovery tests.
8. Black-box API tests.
9. Operator drill tests.
10. Full end-to-end scenarios.

Tests MUST compare semantic outcomes. Tests CAN normalize timestamps and
opaque identifiers only for values that are not part of identity.

The harness MUST maintain an inventory of routes, worker operations, adapters,
tables, object namespaces, telemetry sinks, and roles. Cross-cutting tests
MUST cover this inventory. An unclassified new path MUST fail the test suite.

### 21.2 Required fixtures

The suite MUST include:

- Two tenants with overlapping identifiers.
- Two principals in one tenant.
- Every supported consent combination.
- Every Admission decision.
- Review approval, rejection, and transformation.
- Zero and positive Score decisions.
- Index exclusion and inclusion.
- Credit holds, caps, and approval states.
- Every payout state.
- Revoked, withdrawn, expired, purged, and held traces.
- Corrupt and missing encrypted objects.
- Suspended and resumed policies.
- Equal and conflicting deterministic index entries.
- Seeded secrets for leak detection.

### 21.3 Minimum end-to-end scenarios

#### SCN-001: New contributor

1. Onboard a new device.
2. Mint a scoped upload claim.
3. Submit a redacted trace.
4. Process Admission, Review, Score, and Settle.
5. Read contributor status.

Expected results:

- No shared long-lived secret exists.
- The granted scope does not exceed the request or tenant ceiling.
- One run uses one bundle.
- Each completed phase has one outcome.
- Status distinguishes processing, credit, and payout.

#### SCN-002: Safe submit retry

1. Commit the receipt transaction.
2. Lose the response.
3. Repeat the submission.
4. Complete the recovered run through Settle.
5. Repeat the original submission again.

Expected results:

- One logical submission exists.
- One pipeline run exists.
- Each completed phase has one outcome.
- Each positive Score award has one eligible instrument operation.
- An included revision has one sealed index command.

#### SCN-003: Admission quarantine

1. Submit a trace with synchronous privacy risk.
2. Confirm an Admission quarantine outcome.
3. Process Review approval.
4. Complete the remaining phases.

Expected results:

- The trace is not customer-visible before Review approval.
- The Review outcome addresses the Admission reason.
- The approved revision binds to the submitted content hash.

#### SCN-004: Review rejection

1. Admit a trace.
2. Reject it during Review.

Expected results:

- No registry revision becomes active.
- No Score or Settle outcome exists.
- Contributor status identifies Review as the rejecting phase.

#### SCN-005: Score dependency failure

1. Bind a bundle whose Score policy requires a scorer.
2. Approve a trace in Review.
3. Make that scorer unavailable.
4. Retry the phase.

Expected results:

- No favorable Score outcome appears.
- No credit event appears.
- The run retains a safe retry label.
- A later successful retry uses the same bundle.

#### SCN-006: Settle index crash

1. Complete Score with positive credit.
2. Seal the Settle index command.
3. Crash before the index response.
4. Retry Settle.

Expected results:

- The retry uses the sealed command.
- A counting Settle policy records one membership evaluation before sealing.
- A counting writer confirms reuse of the same sealed command.
- The index contains one logical entry per deterministic key.
- Credit remains idempotent.

#### SCN-007: Bundle change during a run

1. Start a run under bundle A.
2. Activate bundle B.
3. Complete the first run.
4. Start another run.

Expected results:

- The first run uses bundle A for every phase.
- The second run uses bundle B.
- No existing outcome changes.

#### SCN-008: Policy suspension

1. Complete Review.
2. Suspend the bound Score policy.
3. Attempt Score.
4. Resume the policy.
5. Retry the run.

Expected results:

- The suspended run remains retryable.
- The refusal uses a safe label.
- The run does not move to another bundle.
- The retry completes under the original bundle.

#### SCN-009: Withdrawal before Settle

1. Complete a positive Score.
2. Withdraw before Settle decides index membership.
3. Complete permitted settlement work.

Expected results:

- Settle excludes index membership.
- The committed Score credit remains.
- Existing governed settlement can finalize the credit.
- No content read occurs during credit finalization.

#### SCN-010: Withdrawal after index write

1. Complete the full pipeline with index inclusion.
2. Withdraw the trace.
3. Fail one invalidation attempt.
4. Retry invalidation.

Expected results:

- New customer reads stop immediately after durable withdrawal.
- Invalidation remains visible until complete.
- The index entry becomes invalid.
- Settled credit remains unchanged.

#### SCN-011: Settlement and NEAR crash recovery

1. Finalize an internal credit batch.
2. Commit the Settle outcome.
3. Crash before NEAR submission.
4. Recover the outbox.
5. Confirm the external receipt.

Expected results:

- Settle remains complete before NEAR confirmation.
- The external adapter receives one logical request.
- The outbox records submission and confirmation separately.
- Outcomes, audits, logs, reports, and operational responses remain hash-only.

#### SCN-012: Cross-tenant probe

1. Authenticate as tenant A.
2. Guess tenant B run, trace, outcome, object, index, and export identifiers.

Expected results:

- Each lookup is indistinguishable from an unknown identifier.
- No response, stored result, audit, log, or metric reveals tenant B.

#### SCN-013: Missing required control

1. Remove one required policy, key, store, or authority source.
2. Start the affected operation.

Expected results:

- The operation fails closed.
- No run, outcome, registry mutation, credit event, index write, or payout
  appears.
- An orphaned encrypted object can remain for safe cleanup.
- The result names a safe control label only.

#### SCN-014: Customer export

1. Create approved revisions with mixed consent and lifecycle state.
2. Request an export for one allowed use.
3. Fail one object read.
4. Recover the export.

Expected results:

- Only authorized and operable revisions enter the source snapshot.
- No partial export appears as complete.
- Retry retains the source identity.
- The manifest supports later invalidation.

#### SCN-015: Compatibility rollout

1. Process a fixed corpus through current behavior.
2. Process it through the compatibility bundle.
3. Compare the defined outcomes.
4. Activate the compatibility bundle.

Expected results:

- Defined review, scoring, index, and settlement behavior matches.
- Each new result has complete bundle and outcome provenance.
- Activation changes new runs only.

## 22. Completion rule

The redesign is complete after all of these conditions hold:

1. Every non-deferred contract has an automated passing test.
2. Every completed phase stores exactly one immutable outcome.
3. A phase skipped after terminal rejection stores no outcome.
4. Every side effect passes idempotency and crash-recovery tests.
5. Every user-visible state traces to authoritative outcomes and operations.
6. Every tenant boundary passes PostgreSQL and black-box isolation tests.
7. Every required drill has current passing evidence.
8. The compatibility corpus matches the approved baseline.
9. Score performs no index writes.
10. The redesign does not depend on the excluded domain concepts in section
   20.

Passing the current unit tests is not sufficient. Matching the old schema,
route count, or binary layout is not required.
