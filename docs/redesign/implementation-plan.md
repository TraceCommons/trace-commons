# Versioned Pipeline Implementation Plan

> First, build a complete pipeline with results that developers can inspect.
> Then add recovery, index and credit operations, and production policies in
> small steps that reviewers can check.

- **Status:** Phase 8 implemented
- **Date:** 2026-09-14
- **Architecture:** [proposal.md](./proposal.md)
- **Final acceptance:** [system-behavioral-contracts.md](./system-behavioral-contracts.md)

## 1. Delivery approach

The first implementation phase must run a fixed set of test traces, called a
corpus, through Admission, Review, Score, and Settle. Use minimal policies,
store the actual outcomes, and produce a report that developers can inspect.
This phase must not require models, calibration, production package
distribution, or completion of every behavioral contract. Each later phase
extends the same working pipeline.

The implementation phases below are delivery milestones. The four runtime
phases in the proposal are steps that the service executes for each run.

The completed redesign must satisfy every behavioral contract that is not
explicitly deferred. This includes existing authentication, privacy, credit,
lifecycle, customer, and operator requirements. An intermediate milestone can
satisfy only some contracts. Its exit criteria prove that milestone's changes.
They do not prove that the system is ready for production.

Use these rules throughout implementation:

- Keep the existing production path available while you develop the new path.
  Send each submission to one implementation. Never let both implementations
  issue credit or write to the index for the same submission.
- Run incomplete milestones in an isolated local or test deployment. Use
  redacted test fixtures and separate database, object, and index namespaces.
  Disable external payout. Production must not permit selection of minimal
  test policies.
- Add the two workflow tables from the proposal alongside existing storage.
  Reuse registry, credit, hold, batch, outbox, audit, and lifecycle records.
  Keep bundle packages and operational policy status in the bundle registry
  defined by the proposal.
- The first working path must use run identity, typed outcomes, integer
  microcredits, encrypted artifacts, and safe reports. Derive tenant scope
  from authentication. Later work can then extend these foundations.
- Split each milestone into the ordered review steps below. Each step should
  answer one main review question and include evidence for its changes.
  Keep unrelated cleanup and major changes to binary organization separate.
- Keep the corpus demonstration passing after Phase 1. Add fixtures for each
  change and show the differences in reported behavior. Identify intentional
  behavior changes and contracts that are still incomplete.

No step adds durable observations, facts, deployment assignments,
generic effect records, vector epochs, a generic schema registry, or a new
lab service or database. Production reprocessing is outside this plan.

## 2. Sequence at a glance

| Milestone | Usable result | Main change | Environment at exit |
|---|---|---|---|
| 1. Minimal complete pipeline | Submit a corpus, run all four phases, inspect outcomes and status | Static policies and a small bundle that cannot change | Local/test |
| 2. Durable execution | Restart workers and run them concurrently without changing results | Transactions, current-lease checks, safe retries, and readers for retained bundles | Local/test |
| 3. Index and credit operations | Complete runs with fixed positive credit and actual internal operations | Stored index commands and connection to existing settlement | Local/test |
| 4. Authority and privacy policies | Quarantine, transform, withdraw, suspend, and resume safely | Real Admission and Review policies, with phase authority checks | Local/test |
| 5. Compatibility policies | Match the approved reference results through the new phase boundaries | Real scoring dependencies and a compatibility bundle | Staging comparison |
| 6. Complete product integration | Contributor, customer, and operator paths use the new authoritative records | Existing public and lifecycle contracts | Staging |
| 7. Qualify production behavior | Prove all applicable contracts and complete required drills | Production checks for security, recovery, packages, lab, and operations | Production candidate |
| 8. Activate and finish migration | New submissions use the qualified bundle; old records remain readable | Controlled activation, rollback, and removal of superseded write paths | Qualified production |

Complete the phases in this order. You can prepare test fixtures and focused
specifications earlier. The Phase 1 demonstration must not require policies
from later phases. As `LAB-003` requires, stabilize compatibility transitions
before you introduce new valuation rules.

## 3. Repository areas to use

The repository already has most of the interfaces needed for this sequence.
The plan changes their responsibilities one step at a time.

| Existing area | Planned use |
|---|---|
| `crates/trace-commons-gate-api/src/` | Add phase traits, inputs, decisions, and related evidence and evaluation types. Separate read and write capabilities. Test policies implement these production traits. |
| `crates/trace-commons-protocol/src/` | Keep public envelopes, receipt and status DTOs (data transfer objects), and versioned compatibility formats here. |
| `crates/trace-commons-server/src/trace_corpus_storage.rs`, `src/db/trace_corpus_pg.rs`, `src/db/postgres.rs`, and `migrations/` | Add pipeline transactions that commit all required records together. Force row-level security (RLS) and limit worker claim permissions. Reuse existing registry and financial records. Assign migration numbers during implementation. |
| `crates/trace-commons-server/src/trace_artifact_store.rs` and `src/trace_artifact_kek.rs` | Reuse encrypted storage scoped to the tenant. Store submitted and reviewed content, evidence, and sealed commands there. |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` and the review/worker binaries | Connect receipt and worker processing through small server modules. Manual and scheduled execution use the same domain operations. |
| `crates/trace-commons-server/src/trace_gate_service.rs` and `crates/trace-commons-gate-enclave/src/orchestrator.rs` | Separate compatibility scoring from the current behavior that combines scoring and index insertion. |
| `crates/trace-commons-gate-api/src/vector_index.rs` and gate-enclave index implementations | Separate reader and writer capabilities. Define deterministic upserts, conflict handling, index snapshot evidence, and exclusion of the queried revision from its own results. |
| Existing credit storage, settlement operations, and `crates/trace-commons-server/src/near_credit.rs` | Connect Score eligibility and Settle completion to existing account-level approvals, batches, holds, caps, and payout. |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/` and `src/bin/gate_calibrate/` | Extend existing submission, corpus, and reporting tools. Keep local development records outside ingest. |
| `crates/trace-commons-server/tests/` and `.github/workflows/ci.yml` | Add PostgreSQL, corpus, adapter, API, and crash tests alongside current test suites. |

Two existing details require changes. The vector trait exposes both reads and
writes. The orchestrator inserts vectors while it scores a trace. Using that
orchestrator unchanged as a Score policy would violate the target design.
The pilot submitter currently reports only the receipt result. The new corpus
path must wait for pipeline completion and inspect the results.

## 4. Phase 1 — Minimal complete pipeline

**Outcome:** A developer can submit a local corpus, store a run, and process it
through all four phases. The results can be explained without a model, vector
service, or credit settlement dependency.

**Implementation:** The isolated local/test path is documented in
[phase-1.md](./phase-1.md). Its contract evidence manifest is
[contract-test-manifest.json](./contract-test-manifest.json).

### Ordered review steps

1. **Small types and one bundle.** Define the initial typed phase contracts,
   outcome schema and version, safe reason codes, and checked microcredit
   conversions. Define one fixed encoding, called the canonical encoding, for
   the manifest. Specify the exact submitted artifact bytes that
   `request_content_hash` identifies. Preserve that source identity when
   Review transforms the content. Build one local package that selects all
   four policies.

   Validate its manifest and included artifacts against its bundle ID. Keep
   the package format small. Phase 7 adds production signing and
   distribution. Runners and dependencies use Rust trait objects.
2. **One stored processing path.** Add `pipeline_runs`, `phase_outcomes`, and
   the receipt and worker operations. Add uniqueness constraints and force
   tenant RLS on the tables. Use existing claim validation to authenticate
   requests and determine the tenant. Store encrypted content and bind the
   bundle before Admission. Commit the run, Admission outcome, and next phase
   together in one transaction.

   A manually started worker processes Review, Score, and Settle outside the
   request. Limit the work it can claim. Commit each outcome and transition
   together. Review also commits its approved registry revision in that
   transaction. Start with one worker per test tenant. Phase 2 tests
   concurrent execution and recovery after interruption.
3. **Corpus and inspection.** Extend the existing bootstrap tools with local,
   versioned fixtures and exact request replay. Add completion polling with a
   time limit, a local machine-readable report, and a short Markdown
   comparison. Use existing records to add basic contributor status and
   operator outcome inspection with scoped access. Report missing or
   malformed receipts and incomplete runs as failures. Never treat them as
   accepted by default.

   Before changing existing policies, capture their reference results and
   input and configuration identities. These form the legacy baseline. The
   minimal bundle demonstration does not need to match that baseline.

### Minimal bundle

| Runtime phase | Initial policy | Observable outcome |
|---|---|---|
| Admission | Admit a valid, authenticated fixture after basic schema and authority checks | `Admit`, validation evidence, and a fixed rule identifier |
| Review | Pass through the stored, redacted test artifact | `Approved`, the exact source hash, an approved registry revision, and evidence that the content did not change |
| Score | Assign fixed zero microcredits | A completed zero-credit decision with an evaluation that identifies the fixed amount |
| Settle | Exclude the revision from the index and require no credit operation | An explicit exclusion reason, zero finalized microcredits, and no batch reference |

Start with zero credit so Settle can record an accurate completed outcome
before financial operations are connected. Keep completed zero credit distinct
from unscored credit. Phase 3 adds fixed positive credit and index inclusion.
Do not report either operation as complete in this phase.

### Exit demonstration and tests

- Submit a clean fixture corpus through HTTP. Process it with the actual
  PostgreSQL worker path and read status. Each successful trace must have
  one run, one bound bundle, one approved revision, and four typed outcomes.
  These outcomes are immutable: their stored content cannot change.
- Repeat the exact submission and return the same run. Refuse a request that
  reuses its key with changed content. Make a policy fail and report an
  operational error without inventing a decision.
- Pause before Review and Score to show pending states and completed zero
  credit as separate states. Query as another test tenant and confirm that
  it cannot see the outcomes.
- Check the decision, evidence, evaluation, source and bundle hashes, and
  final status in the report. Insert a known secret into a test fixture.
  Confirm that the secret does not appear in reports or logs.
- Document one command or script that starts the test environment, runs the
  corpus, and writes the report. It must not require a network corpus fetch,
  model download, or external payout. Document new CLI options when they are
  implemented. Do not assume that proposed options already exist.

**Not yet complete:** concurrency and crash qualification, positive credit,
index writes, production privacy and rate policies, and intervention ordering.
Full public compatibility, production bundles, and operational drills also
remain incomplete. This milestone provides the reusable test path. It does
not prove full compliance with any contract group.

## 5. Phase 2 — Durable execution and bundle recovery

**Outcome:** The minimal corpus produces the same logical results after
retries, concurrent work, activation changes, and process restarts.

**Implementation:** The local and test path is documented in
[phase-2.md](./phase-2.md).

### Ordered review steps

1. **Atomic storage operations.** Complete receipt replay and conflict
   handling. Enforce immutable run identity and outcomes. Review must commit
   the artifact reference, registry revision, outcome, and transition in one
   transaction. Protect against simultaneous first submissions with the same
   key. Add readers for the immutable outcome schemas. Reject unknown
   required versions. Identify encrypted objects left without a committed
   submission as orphans, not accepted submissions.

   Keep them eligible for safe cleanup.
2. **Queue recovery.** Add fenced leases: a worker can commit only with a
   current lease token. Limit claims and attempts. Add backoff (retry
   delays), safe terminal error labels, and rejection of workers with stale
   leases. The cross-tenant claimer can read only the metadata needed to
   return trusted tenant, run, and lease identity. It can update only lease
   columns. It cannot read artifacts or execute policies.

   Scope policy work, capabilities, and commit transactions to the tenant.
3. **Bundle registry behavior.** Retain validated packages. Explicitly select
   the active bundle for each tenant and load the bound package on each
   retry. Test changes to code, configuration, data, projection, format, and
   policy identity. Test missing and altered artifacts. Store whether a
   policy can run outside the immutable manifest. Phase 4 tests concurrent
   changes to that operational status.

### Exit demonstration and tests

- Submit duplicate receipts and run workers concurrently. Expire a lease
  during execution. Only the current lease can commit, with one outcome
  per phase.
- Inject the receipt, Admission, Review, and pre-Score-commit crashes from
  `RUN-004`, boundaries 1–5. Restart with the same database and artifact store.
  Confirm that all retries reach the same final result. Phase 3 adds the
  remaining crash boundaries when index and credit operations exist.
- Activate bundle B during a run bound to A. Include activation after bundle
  resolution but before Admission commits. Existing runs must keep A, and a
  new run must use B. Rollback changes the selected bundle for later runs only.
- Run PostgreSQL tests with two tenants that use overlapping identifiers.
  Check database role and column privileges directly. Unit tests of the store
  alone do not prove transaction or RLS behavior.
- Add attempts, pending/retry/failed states, outcome identity, and time in
  phase to the report. Compare committed outcome bytes after retries and
  bundle changes. They must remain identical.

**Not yet complete:** recovery of actual index and credit operations, all
concurrent guard tests, production policies, and isolation across the full
product. Continue to use the minimal bundle in an isolated environment.

## 6. Phase 3 — Real index and credit operations with fixed policies

**Outcome:** The corpus tests zero and positive credit, index inclusion and
exclusion, holds, and recovery. It uses simple decisions and actual internal
operation records.

### Ordered review steps

1. **Positive Score and credit eligibility.** Add a fixed positive Score
   variant. In one transaction, commit its outcome, Settle transition, and
   one eligible credit event. Key the event by tenant, run, and Score
   outcome. A zero Score creates no event. Adapt existing event links and
   checked conversions at storage boundaries. Keep the ledger and settlement
   approval requirements.

   Test maximum values, negative source inputs, overflow, and excess
   precision.
2. **Sealed index commands.** Specify and implement separate vector reader
   and writer adapter interfaces. Use deterministic test embeddings and store
   them as encrypted Score evidence. Use the actual index adapter in an
   isolated environment. Decide membership from the bound bundle and
   committed Review and Score outcomes. Store the decision and encrypted
   command reference and hash on the run before writing to the index.

   This seals the command. After sealing, a retry must reuse the stored bytes
   without evaluating membership again. Keys must include tenant, index,
   revision, projection, model, and chunk. An equal key with equal content
   succeeds without a change. Refuse an equal key with different content.
   Implement query self-exclusion and evidence of the index snapshot used.
   The later real Score policy must receive only a reader.
3. **Internal completion under existing controls.** Connect eligible events
   to existing previews, source-list approvals, issuer approvals, and account
   batches. Preserve holds, caps, duplicate-source protection, and holds
   caused by payout identity. Track index and credit progress separately.
   Record the Settle outcome only after every required internal operation
   finishes. Include the finalized amount for that run and the actual batch
   hash.

   Omit the batch reference if no batch finalized credit. Reuse existing NEAR
   outbox records and keep submission and confirmation as separate
   operations.

### Exit demonstration and tests

- Test all four permitted combinations of index inclusion or exclusion and
  zero or positive Score. Inclusion does not require positive credit.
  Finalize several compatible runs in one approved batch.
- Hold credit while the index operation completes. Fail the index while the
  eligible event remains stored. Recover either operation without repeating
  the other. A held or unfinished operation must prevent a Settle outcome
  until all required operations complete.
- Implement `RUN-004` crash boundaries 6–11. Count policy evaluations and
  writer calls. Prove one membership evaluation before sealing and reuse of
  the command on retry. Prove one logical index entry, one credit event, and
  one logical payout request. A failure before sealing can cause another
  evaluation. A retry after sealing cannot.
- Test lost adapter responses and stale leases before dispatch and commit.
  Use the sealed command to recover when the index response is uncertain.
  Refuse equal-key/different-content conflicts without applying the change.
- Test NEAR recovery with an adapter that records logical requests. Keep
  external payout disabled. Test disabled, pending, submitted, confirmed, and
  failed payout states. None can change a completed Settle outcome.
- Show index, internal credit, and payout progress separately in status and
  reports. Outcomes and operational outputs must not contain raw account
  references or transaction hashes.

**Not yet complete:** production authorization for these operations,
intervention ordering, real valuation, and production adapter qualification.
The internal connections are real, but the fixed bundle remains test-only.

## 7. Phase 4 — Authority, privacy, and intervention policies

**Outcome:** Real Admission and Review policies protect the pipeline. Score
remains fixed so reviewers can focus on privacy and authority behavior.

### Ordered review steps

1. **Phase guards and lifecycle ordering.** Check authority before reading
   content or starting policy work. Check it again inside transactions that
   commit outcomes or operations. Lock the applicable submission and
   policy-status rows at that final check. For an index write, keep the locks
   until the command result commits. Set adapter timeouts and preserve
   recovery information for uncertain results.

   Check whether the submission is operable: its consent, allowed uses,
   withdrawal, revocation, and expiry must permit the operation. Connect
   completed index writes to the existing invalidation process.
2. **Suspension and credit after withdrawal.** Add controls to suspend,
   resume, and permanently stop work. Audit each action. Keep operational
   status outside the bundle hash. Suspension must preserve the bundle and a
   safe, retryable state. Do not exhaust retries into failure only because
   the policy remains suspended. Withdrawal before the membership decision
   must force exclusion and record evidence.

   Withdrawal after sealing must stop pending index work. Withdrawal after
   the write must queue invalidation. Credit from a committed Score can
   settle without reading the trace. Repeat applicable bound-policy guards
   before NEAR dispatch, including batches with multiple runs. Find the
   required policies through existing batch, event, and outcome links.
3. **Admission validation and limits.** Add real checks for schema,
   contribution path, grants, consent, and allowed uses. Add tenant and
   principal quotas and rate limits, tombstone rejection, and synchronous
   privacy-risk handling with bounded work. Store required evidence and
   stable reasons. Admission itself makes no external writes. Count quota use
   once per logical receipt in a transaction. Enforce this across service
   instances and concurrent retries.

   Model-based substance and novelty work must remain outside Admission.
4. **Review transformation and human evidence.** Add server-side privacy
   transformation, source and result evidence, and Review rejection. Connect
   scoped quarantine reads, audits, leases, and reasoned human assessments to
   the bound Review policy. Treat assessments as evidence generated by the
   server. The policy must make the decision. A reviewer cannot bypass the
   policy or any later phase. Approval must resolve every Admission
   quarantine reason.

   Store transformed encrypted content before the registry transaction
   commits all required records together.

### Exit demonstration and tests

- Add fixtures for real Admit, Quarantine, and Reject decisions. Add Review
  approval, rejection, and transformation fixtures. A terminal Admission
  rejection has one outcome. A Review rejection has two. Do not create
  outcomes for skipped phases.
- Test concurrent lifecycle, consent, and allowed-use changes before work and
  before commit at every applicable boundary. Include withdrawal after index
  dispatch but before the local result commits. Check which operation
  committed first.
- Withdraw before and after positive Score. Only previously committed credit
  remains eligible. Use a test reader that records content reads. Prove that
  later credit finalization does not read the trace.
- Suspend each bound policy during work and before commit. Resume the same
  run with the same bundle. Also suspend after Settle but before outbox
  dispatch. Outcomes must remain unchanged. Each intervention must append
  safe audit evidence.
- Test concurrent work by two reviewers and two submitting service instances.
  Stale assessments, unresolved quarantine reasons, foreign content, and limit
  conflicts must not produce favorable decisions or expose sensitive data.

**Not yet complete:** real scoring, a match with the compatibility baseline,
and the full existing product interfaces. Before adding more complex policies,
all implemented content and operation paths must obey their guards.

## 8. Phase 5 — Production compatibility policies

**Outcome:** A bundle matches the approved current review, scoring, index,
and settlement results through the new phase boundaries.

**Implementation:** The isolated local/test path is documented in
[phase-5.md](./phase-5.md). The comparison mapping is
[compatibility-mapping.md](./compatibility-mapping.md).

### Ordered review steps

1. **Fix the compatibility comparison inputs.** Finalize the baseline
   captured with the Phase 1 corpus tools. Record the current code,
   configuration, and model identities. Record the redacted corpus digest and
   order, initial index contents, measured external inputs, and approved
   expected results. Before implementing the adapter, define how legacy
   results map to new decisions.

   Include privacy, rejection, zero credit, chunking, novelty, and active
   quality, deduplication, and cap rules. Include index membership and
   settlement under existing controls.
2. **Separate real Score from index writes.** Put the current substance and
   novelty algorithms and dependencies behind `ScorePolicy`. Remove index
   insertion from this work. Keep current deployed rules in immutable bundle
   inputs. Record each input that affects the decision: model and projection
   identity, measured values, index and snapshot identity, neighbors, index
   cardinality, and coverage. Cardinality is the number of index entries.

   Encrypt sensitive neighbor data and embeddings. Expose only hashes and
   bounded measurements. A dependency or evidence failure remains an
   operational failure.
3. **Complete the compatibility bundle.** Combine the real Admission and
   Review policies with the separated Score and Settle membership rules.
   These rules use only committed evidence. Include every effective deployed
   input in bundle identity. Adapt legacy public formats at their interface.
   Do not restore classifier-specific storage or index writes during Score.

### Exit demonstration and tests

- Run legacy and new behavior with equivalent, isolated initial indexes and
  a controlled corpus order. Where required, supply the same fixed dependency
  responses. A shadow comparison tests an alternative without changing active
  results. It must not use an active write namespace or create live credit.
- Compare reviewed artifacts, rejection behavior, awarded and finalized
  amounts, index content, settlement eligibility, and holds. Where legacy
  keys differ from the required deterministic keys, compare the associated
  content. Normalize only timestamps and opaque IDs that are not part of
  identity. Do not normalize fields that determine identity.
- Identify each behavior change required by the proposal. Treat a mismatch
  as a defect or a separate baseline or specification decision for review.
  Do not silently regenerate golden expected results. Do not preserve
  prohibited behavior to make a test pass. Review approval must remain before
  Score. A Score failure cannot change the earlier Review outcome.
- Use capability tests and an adapter that detects writes to prove that
  Score cannot write to an index. Change the live index after Score.
  Prove that Settle does not query it for membership or repeat valuation.
- In staging, run basic tests with real dependencies as well as tests with
  fixed responses. Historical production behavior does not need to be
  reproducible through deterministic replay. Stored evidence must still
  explain each decision.

**Not yet complete:** full product integration, production package operations,
and final qualification. A passing corpus comparison alone does not permit
production activation. Add alternative valuation rules only after the
compatibility transitions are stable.

## 9. Phase 6 — Complete inherited product behavior

**Outcome:** All supported user and worker interfaces work with new runs and
retained legacy records. The new implementation does not require legacy
policy columns as its authoritative records.

**Implementation:** The staging product path is documented in
[phase-6.md](./phase-6.md).

### Ordered review steps

1. **Contributor and public compatibility.** Preserve all public interfaces
   in `CMP-001` and their error formats. Alternatively, deliver a versioned
   replacement with its client. Complete submission-status batches with
   explicit size limits. Keep processing, credit, and payout states separate.
   Complete pagination for owned submissions, audited reads of retained
   redacted content, and credit and event views. Complete signed score
   attestations that identify the schema and bundle.

   Keep legacy readers. Do not invent historical bundles or phase outcomes.
2. **Authentication and worker authority.** Verify onboarding, invitations,
   upload claims, grant intersections, account sessions,
   strong-authentication checks, and account merge history. Verify
   self-withdrawal after ordinary access is removed. Complete scoped
   service-credential issuance, rotation, and revocation. Keep worker and
   reviewer roles separate. Remove static production bearer tokens and HS256
   bridge dependencies from the target deployment. Keep protocol and
   contributor tests for client redaction and explicit opt-in.
3. **Customer, export, and derived artifacts.** Select only authorized,
   approved revisions that can still be used. Preserve consent and view
   schema. Keep links to the source and bundle as provenance. Complete
   immutable snapshots of export sources, manifests, recoverable claims, and
   handling of partial outputs. Complete authority checks for benchmark,
   ranking, and training operations. Connect invalidation of managed derived
   artifacts to existing lifecycle work.

   Public attribution never grants content access.
4. **Lifecycle and optional community.** Complete hash-only tombstones,
   retention, legal holds, and purge. Limit revocation retries and make
   failed targets visible. Report managed distribution across objects, index,
   caches, and exports. If community remains enabled, test attribution
   consent, withdrawal from snapshots, and aggregate privacy. Otherwise,
   verify that the disabled interfaces return not-found responses. Do not
   claim deletion from customer copies that the system does not manage.

### Exit demonstration and tests

- Test contributor onboarding, submission, status, and withdrawal through
  the public interfaces. Test customer export the same way. Include accounts
  with both legacy and new histories.
- Test each route with each role and each resource with each tenant.
  Unknown and inaccessible identifiers must produce indistinguishable
  responses. Authorize every content read before decryption and audit it
  without recording raw content.
- Test every valid combination of credit, hold, and payout states.
  `finalized` requires an approved internal batch. Neither a positive Score
  nor a submitted NEAR request is sufficient.
- Remove a source that appears in the registry, index, cache, exports, and
  derived artifacts. Fail each invalidation adapter, then retry. Verify
  immediate exclusion from reads. Verify accurate reports of remaining work
  and readiness.
- Compare published responses and errors with saved compatibility fixtures.
  Verify signed score statements offline. Test missing, rotated, and
  incorrect keys.

**Not yet complete:** evidence for every contract, production restore and
operational drill qualification, and the production switch. Existing tests
count only if they exercise the relevant new path at the required boundary.

## 10. Phase 7 — Lab, security, and operational qualification

**Outcome:** The compatibility bundle can be deployed. Current automated
checks prove that the full target deployment meets every applicable contract.

### Ordered review steps

1. **Production packages and policy lab.** Finish the specification for
   package signatures, trust, canonical integrity checks, registration,
   activation, and retention. Validate all artifacts before activation.
   Reject unknown implementations and missing controls. Keep readers and
   packages for as long as their outcomes must be retained. Prove that
   previously bound packages still execute after a deployment. Extend the
   existing calibration tool with versioned corpus and input digests.

   Keep bootstrap and holdout data separate. Produce local reports and a
   local catalog that links bundles to development records. Ingest uses only
   the package. Development-only keys, scorers, and stores cannot satisfy
   production readiness. Neither can synthetic settlement backends.
2. **Operations and complete coverage.** Complete safe readiness responses,
   worker scheduling with explicit limits, and operational summaries. Let
   operators follow a run to its evidence, command, Score event, batch, and
   interventions. Cover exhausted retries, suspended-policy backlogs, credit
   holds, outbox progress, and blocked invalidation and export work. List
   every enabled route, operation, adapter, table, namespace, telemetry
   destination, and role in the inventory.

   Map each to its contracts. Fail CI if an addition is not classified.
3. **Drills and migration qualification.** Adopt the focused qualification
   specification and run every test in the crash matrix. Test PostgreSQL and
   object restore. Test index rebuild from authoritative revisions and
   commands. Restore must preserve IDs, hashes, audit order, and the identity
   of pending operations. Index rebuild must not create outcomes or credit.
   Add every `OPS-004` drill.

   Include key rotation, audit verification, tenant isolation, package
   integrity, activation, and rollback. Also include outcome atomicity,
   leases, index and Settle recovery, settlement approvals, NEAR, withdrawal,
   and backup and restore. Block promotion if required evidence is missing,
   failed, or stale.

### Exit demonstration and tests

- Pass all ten acceptance layers in contracts §21.1, all applicable fixtures
  in §21.2, all 15 scenarios, and every required current drill. Run database
  tests against PostgreSQL with the actual runtime, claimer, and worker roles.
- Insert known secrets into fixtures and search all success and failure
  outputs for them. Include logs, errors, audits, metrics, reports, and
  operational responses. Probe every listed interface with unrelated roles
  and tenants. Include object, index, cache, export, and financial adapters.
  Verify audit tamper detection and staged key rotation.
- Remove each required control in turn. The affected operation must not
  commit a partial result or side effect. Earlier committed phase history
  must remain unchanged. Encrypted receipt orphans are permitted only under
  `SYS-003`. Recover partial Settle progress from an interrupted valid
  operation under `STL-003` and the crash-recovery rules.
- Verify that production has none of these dependencies: file-backed
  authoritative metadata, best-effort database mirrors, plaintext fallbacks,
  static bearer authentication, or HS256 bridge authentication. Also exclude
  synthetic receipts, mock scorers, and unversioned policy dependencies.
  Check configuration refusal tests and the actual deployable components.
- Produce a qualification report linked to the code revision, bundle, corpus,
  configuration, contract and test identifiers, and evidence hashes. Keep
  test and drill evidence in test and operations tools, outside pipeline
  history.
- Add required CI jobs for PostgreSQL, corpus, adapter, and black-box suites.
  Keep existing workspace tests, formatting and lint checks, and default,
  NEAR AI, and local-model build checks. A required database or adapter test
  must fail or report a blocker when its environment is missing. A skipped
  test must not count as qualification. Disable live external payout in
  every policy, runner, bundle, and integration test.

**Not yet complete:** production activation and observation of the migration.
The candidate can enter Phase 8 only after its applicable contracts pass.
Tests that have not run do not provide passing evidence.

## 11. Phase 8 — Activation, rollback, and completion

**Outcome:** New production submissions use the qualified pipeline. Retained
history remains readable. The final state satisfies contracts §22.

### Ordered review steps

1. **Rehearse the production switch.** Test additive migration and rollback
   on a restored deployment. Include legacy records, pending legacy work, and
   new pipeline runs. Keep authoritative legacy idempotency lookups so
   retries find the original operation. A retry of a receipt from before the
   switch must not start a new run. Finish legacy work or send it to its
   existing executor.

   Define which implementation owns each item. Never rescore legacy work
   automatically or invent phase outcomes. Enforce unique ledger sources
   across both paths.
2. **Tenant activation.** Start with a small group of tenants. Activate the
   qualified compatibility bundle for their new submissions. Expand only
   after current drills, readiness checks, corpus evidence, and error and
   work-age metrics meet the qualification thresholds. Credit reconciliation,
   index checks, and invalidation checks must also pass. Record activation
   explicitly. Timestamps must not determine which bundle is active.
3. **Rollback and retirement.** Demonstrate selection of an earlier qualified
   bundle for later runs. Existing runs must retain their bound package.
   Suspend an unsafe bound policy instead of silently switching the bundle.
   During the first rollout, an earlier pipeline bundle may not exist. In
   that case, stop sending new submissions to the pipeline. Keep its workers,
   readers, and packages available.

   This containment step is separate from routine bundle rollback. Disable
   superseded legacy writers only after they finish their owned work. Retain
   legacy reads and required package and schema support through retention.
   Make destructive schema cleanup a separate later change.

### Terminal acceptance gate

The implementation is complete only when all of these conditions are met:

- Every contract that is not deferred has an automated passing test at its
  specified boundary. Record whether each conditional community contract
  applies. Test disabled behavior where required.
- Each completed runtime phase has exactly one immutable typed outcome.
  Phases skipped after rejection have no outcome. Derive every visible state
  from authoritative outcomes and operation records.
- Every retryable operation passes idempotency, concurrency, and crash tests.
  Idempotency means that retries do not duplicate the logical operation.
  External payout must remain one logical operation with separate confirmation.
- Every tenant boundary passes PostgreSQL and black-box isolation tests.
  Every required drill has current passing evidence.
- The compatibility corpus matches its approved baseline. Activation and
  rollback affect only new runs. Legacy requests and historical reads
  preserve their correct identity and meaning.
- Score does not write to an index. Settle uses sealed commands and immutable
  inputs. Preserve existing settlement controls and the no-clawback rule:
  ordinary withdrawal must not remove awarded credit.
- The target deployment does not depend on the implementation or domain
  concepts excluded by contracts §20.

Realistic Admission and Review policies, plus compatibility Score and Settle
policies, are complete by this point. Deliver later calibrated valuation
changes as new bundles. Use the same lab, comparison, qualification, and
activation process. Multi-party valuations first require their separate
protocol and tests.

## 12. Corpus and review evidence

Deliver the test harness in Phase 1 and use it throughout implementation.
Do not leave it until the final phase. Use two corpus layers:

- Keep a small redacted fixture corpus in the repository for fast local and
  CI tests. Start with the complete minimal path. As features are added,
  include rejection, quarantine, transformation, failures, positive credit,
  index operations, interventions, and product cases.
- Keep a versioned compatibility corpus with a digest, approved baseline,
  fixed configurations, explicit order, and isolated initial index state.
  Capture the baseline early so later changes cannot silently redefine
  current behavior. Keep large or sensitive evidence encrypted and outside
  reports.

The harness must test HTTP receipt, actual asynchronous runners, PostgreSQL
storage, artifact storage, status, and inspection. Direct policy tests do
not replace this path. As features are added, support pausing at a phase,
causing a failure, restarting, and replaying exact requests. Also support
changing the active bundle and comparing final behavior. Set a time limit
for every wait. On timeout, report incomplete work and fail the requested
completion check. A receipt alone does not prove completion.

Each local report contains:

| Scope | Required report information |
|---|---|
| Execution | Corpus and input digest, code revision, bundle ID, configuration identities for isolated dependencies, and expected fixture count |
| Per fixture | Safe fixture label or hash; scoped run and outcome references; phase and state; decision; bounded evidence and artifact hashes; evaluation rule; retry and error labels |
| Operations | Index decision, command hash, and progress; Score microcredits; credit state and finalized amount; batch hash when present; separate payout state |
| Comparison | Differences between expected and actual behavior, unexplained failures, missing outcomes, and complete or incomplete contract and scenario coverage |
| Aggregate | Counts by phase, decision, and state; age of pending work; failures; completion duration. Exclude trace text, secrets, raw account IDs, and transaction hashes. |

Keep content hashes, bundle identity, stable idempotency keys, and all other
identity fields in comparisons. Do not normalize them away. A report can
link to authorized encrypted evidence through tools with scoped access.
It must not include the evidence itself.

For each review step, show the new behavior, report differences, tests run,
current limits, and next change. A reviewer should be able to check lease
recovery without learning a new model algorithm. A reviewer should be able
to check a scoring rule without learning a new settlement system.

## 13. Contract completion ownership

Start a machine-checkable test manifest in Phase 1. Map each contract ID to
its applicable boundaries, test IDs, implementation phase, and evidence
status. Use `planned`, `partial`, `passing`, or an explicitly specified
deferral. Reuse an existing test only after confirming that it proves the
target behavior. Cover every required bullet in the contracts. A test that
covers only part of a contract is not sufficient. Keep this manifest outside the ingest
database.

The table assigns responsibility for completing each contract. Contracts
that apply across the system must cover each new phase's code. Phase 7
provides their final qualification. An earlier completion phase does not
exempt later code from these requirements.

| Contract IDs | Completion responsibility and evidence |
|---|---|
| SYS-001, SYS-002 | Phases 2, 4, and 6. In Phase 7, test tenant boundaries, PostgreSQL roles, and authority before content access across the full inventory. |
| SYS-003, SYS-004, SYS-009 | Start in Phase 1. In Phase 7, remove dependencies, scan every reporting destination for private data, check error compatibility, and test audit tampering. |
| SYS-005, SYS-010 | Phases 2 and 3. In Phase 7, retry every mutation. Test lease fencing, concurrency, and visible failures. |
| SYS-006, SYS-007, SYS-008 | Phases 1, 2, 3, and 6. In Phase 7, follow provenance links, compare immutable bytes, and test readers for retained schemas. |
| AUTH-001, AUTH-002, AUTH-003, AUTH-004, AUTH-005 | Phase 6: test combinations of onboarding, claims, grants, sessions, and credentials. Phase 7: qualify rotation and least privilege. |
| SUB-001, SUB-002 | Phase 6: complete protocol and contributor suites, extending Phase 1 receipt validation. |
| SUB-003, SUB-004 | Phase 2: atomic receipt, exact replay, and conflicts. Phase 4: quota behavior on replay. Phase 8: qualify legacy retries. |
| SUB-005, SUB-006, SUB-007 | Phase 4: Admission decisions, limits across instances, and tombstones. Phase 6: all lifecycle, submission, and export paths. |
| BND-001, BND-002, BND-003, BND-004 | Phases 1 and 2: identity, traits, binding, and integrity. Phase 7: production package retention and loading. Phase 8: activation. |
| REV-001, REV-002, REV-003, REV-004 | Phases 1, 2, and 4: source binding, atomic revisions, review leases and evidence, and transformations. |
| SCR-001, SCR-002, SCR-003, SCR-004 | Phases 1, 3, and 5: fixed and real policies, read-only index evidence, checked units, and atomic event creation. |
| SCR-005 | Deferred until the external valuation protocol is adopted. Refuse activation of any bundle that requires it until then. |
| STL-001, STL-002, STL-003, STL-004, STL-005 | Phase 3: operation and settlement tests. Phase 4: guards. Phase 7: real adapter and outbox qualification. |
| RUN-001, RUN-002, RUN-003, RUN-004 | Phases 2 and 3: state, uniqueness, leases, and all eleven crash boundaries. Repeat all tests during Phase 7 qualification. |
| RUN-005 | Phase 7: existing backup and restore behavior, with the adopted migration qualification. Index rebuild must never create awards. |
| GRD-001, GRD-002, GRD-003, GRD-004 | Phase 4: interventions and concurrent operation tests. Repeat with real policies and batched payout in Phase 7. |
| STA-001, STA-002, STA-003, STA-004, STA-005 | Phases 1 and 3: add status behavior. Phase 6: test all status, ownership, and signed-attestation combinations. |
| CRD-001, CRD-002, CRD-003, CRD-004, CRD-005 | Phases 3 and 4: ledger, approvals, caps, holds, and no clawback. Phase 6: every public credit path. |
| LIF-001, LIF-002, LIF-003 | Phases 4 and 6: withdrawal, retention, purge, and every managed invalidation target. Phase 7: drills. |
| EXP-001, EXP-002, EXP-003, EXP-004 | Phase 6: exact authorized selections and views, source snapshots, partial export recovery, and provenance and invalidation of derived artifacts. |
| COM-001, COM-002 | Phase 6 if enabled. Otherwise, test the disabled interfaces and explicitly record which requirements apply. |
| OPS-001, OPS-002, OPS-003, OPS-004, OPS-005, OPS-006 | Add diagnostics from Phase 1 onward. Phase 7: complete readiness, limits, summaries, drills, traceability, and key checks. |
| LAB-001, LAB-002, LAB-003 | Phases 1, 5, and 7: corpus, all test levels, and separation of lab records from packages. Phase 8: activation and rollback evidence. |
| CMP-001, CMP-002 | Phase 6: public compatibility. Phases 7 and 8: exclude prohibited production dependencies and retire legacy writes. |
| CMP-003, CMP-004 | Record scope and applicability in Phase 1. Complete production-use specifications and required product decisions by the phases assigned below. |

Also track every minimum scenario:

| Scenarios | First complete implementation; final qualification is Phase 7 |
|---|---|
| SCN-001 | Phase 6: add full onboarding to the Phase 1 submission and status path. |
| SCN-002 | Phase 3: add credit and sealed commands to Phase 2 receipt recovery. |
| SCN-003, SCN-004 | Phase 4: quarantine and Review rejection. |
| SCN-005 | Phase 5: failure of a real Score dependency. |
| SCN-006 | Phase 3: Settle index crash. |
| SCN-007 | Phase 2: bundle change during a run. |
| SCN-008, SCN-009 | Phase 4: suspension and withdrawal before Settle. |
| SCN-010 | Phase 6: complete withdrawal propagation. |
| SCN-011 | Phase 3: internal settlement and payout recovery with a test adapter. |
| SCN-012, SCN-013 | Phase 7: complete interface inventory, expanded from Phase 1 onward. |
| SCN-014 | Phase 6: customer export. |
| SCN-015 | Phase 7: rehearse activation in staging with Phase 5 compatibility results. Phase 8: production activation. |

## 14. Focused specifications and open decisions

Write each specification when its implementation needs it. Specifications
must not delay the first working pipeline. Complete the following items
within this plan unless they are explicitly deferred here:

| Specification | Needed by |
|---|---|
| Initial phase payloads, schema versions, reason codes, and the exact source-hash boundary | Phase 1: a small fixed set of related types. Extend and version them with Phase 3–5 policies before production use. |
| Vector adapter idempotency, deterministic keys, conflict behavior, self-exclusion, and snapshot evidence | Phase 3. |
| Score event and batch links, checked storage amounts, Settle completion, and NEAR integration | Phase 3. Complete suspension behavior in Phase 4. |
| Authority to suspend, resume, and terminate; operation ordering; audited operator controls | Phase 4. |
| Mapping of compatibility behavior and baseline approval | Capture inputs in Phase 1. Finalize the mapping before Phase 5 policy adaptation. |
| Bundle package signatures, trust, executable-version retention, and package retention rules | Phase 7, before any production activation. |
| Migration, backup and restore, recovery, and promotion qualification | Phase 7, before the Phase 8 production switch. |
| External valuation trust, attestation, and aggregation protocol | Deferred. Required before any dependent policy can activate. |

`CMP-003` permits deferral of detailed specifications during initial
architecture work. Before shipping an affected production feature, complete
its specification and tests. The current document defers detailed migration
qualification. This plan still completes existing restore requirements and
adopts migration qualification before the production switch.

Keep the open product decisions from `CMP-004` visible. Preserve published
interfaces while their support period is undecided. Keep contributor reasons
safe and bounded. Report only known managed distribution. Resolve quarantine
remediation and maximum age before Phase 4 production-policy qualification.
Choose enabled customer and community interfaces before Phase 6 qualification.

Do not add automatic fraud clawback while the correction policy remains
open. Preserve reversal events under existing controls. Ordinary withdrawal
must not remove awarded credit. Unresolved choices can block the affected
production interface, but must not block local pipeline work. They must not
silently remove an acceptance requirement that is not deferred.
