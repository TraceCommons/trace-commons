# Versioned Pipeline Implementation Plan

> Build an observable end-to-end pipeline first, then strengthen recovery,
> integrate effects, and introduce production policies in reviewable steps.

- **Status:** Proposed implementation sequence
- **Date:** 2026-09-11
- **Architecture:** [proposal.md](./proposal.md)
- **Final acceptance:** [system-behavioral-contracts.md](./system-behavioral-contracts.md)

## 1. Delivery approach

The first implementation phase must run a fixed corpus through Admission,
Review, Score, and Settle using minimal policies, real persisted outcomes, and
an inspectable report. It must not depend on model integration, calibration,
production package distribution, or completion of every behavioral contract.
Every subsequent phase extends that same runnable path.

Implementation phases below are delivery milestones. They are distinct from
the four runtime phases in the proposal.

The completed redesign must satisfy every non-deferred behavioral contract,
including inherited authentication, privacy, credit, lifecycle, customer, and
operator behavior. Intermediate milestones may satisfy only a subset. Their
exit criteria prove the stated increment, not readiness for production.

Use these rules throughout implementation:

- Keep the existing production path available while the new path is developed.
  Route a submission to one processing implementation; never let both issue
  credit or index writes for the same submission.
- Run incomplete milestones in an isolated local or test deployment with
  redacted fixtures, separate database/object/index namespaces, and external
  payout disabled. Minimal policies must not be selectable in production.
- Add the two workflow tables from the proposal beside existing storage.
  Reuse registry, credit, hold, batch, outbox, audit, and lifecycle records.
  Keep bundle packages and operational policy status within the proposed
  bundle-registry boundary.
- Establish run identity, typed outcomes, integer microcredits, authenticated
  tenant scope, encrypted artifacts, and safe reporting on the first path.
  These choices let later work extend it without replacing its foundations.
- Split each milestone into the ordered review slices listed below. A slice
  should answer one main review question and include its relevant evidence.
  Keep unrelated cleanup and broad binary reorganization separate.
- Keep the corpus demo passing after Phase 1. Each change adds fixtures and
  shows the resulting semantic report diff, including intentionally changed
  behavior and the contracts still incomplete.

No step introduces durable observations, facts, deployment assignments,
generic effect records, vector epochs, a generic schema registry, or a new
lab service/database. Production reprocessing is outside this plan.

## 2. Sequence at a glance

| Milestone | Usable result | Main increment | Environment at exit |
|---|---|---|---|
| 1. Minimal complete pipeline | Submit a corpus, run all four phases, inspect outcomes and status | Static policies and a small immutable bundle | Local/test |
| 2. Durable execution | Restart and race workers without changing logical results | Transactions, fencing, idempotency, retained bundle readers | Local/test |
| 3. Index and credit operations | Complete fixed positive-credit runs with real internal effects | Sealed index commands and existing settlement integration | Local/test |
| 4. Authority and privacy policies | Quarantine, transform, withdraw, suspend, and resume safely | Real Admission/Review policies and phase guards | Local/test |
| 5. Compatibility policies | Reproduce the approved baseline through the new boundaries | Real scoring dependencies and compatibility bundle | Staging comparison |
| 6. Complete product integration | Contributor, customer, and operator paths use authoritative new records | Inherited public and lifecycle contracts | Staging |
| 7. Qualify production behavior | Demonstrate all applicable contracts and required drills | Security, recovery, packages, lab, and operational qualification | Production candidate |
| 8. Activate and finish migration | New submissions use the qualified bundle; old records remain readable | Controlled activation, rollback, and retirement of superseded writes | Qualified production |

Phases proceed in this order. Test fixtures and narrow specifications can be
prepared earlier, but later policies must not become prerequisites for the
Phase 1 demonstration. New valuation rules beyond compatibility follow stable
compatibility transitions, as required by `LAB-003`.

## 3. Repository seams to use

The repository already has most of the integration boundaries needed for this
sequence. The plan changes their responsibilities incrementally.

| Existing area | Planned use |
|---|---|
| `crates/trace-commons-gate-api/src/` | Add phase traits, inputs, decisions, evidence/evaluation families, and read/write capability boundaries. Test policies implement these production traits. |
| `crates/trace-commons-protocol/src/` | Keep public envelopes, receipt/status DTOs, and versioned compatibility representations here. |
| `crates/trace-commons-server/src/trace_corpus_storage.rs`, `src/db/trace_corpus_pg.rs`, `src/db/postgres.rs`, and `migrations/` | Add atomic pipeline operations, forced RLS, and narrow worker claiming; reuse existing registry and financial records. Assign migration numbers when implementing. |
| `crates/trace-commons-server/src/trace_artifact_store.rs` and `src/trace_artifact_kek.rs` | Reuse encrypted, tenant-scoped storage for submitted/reviewed content, evidence, and sealed commands. |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` and the review/worker binaries | Wire the new receipt and worker path through small server modules. Share domain operations between manual and scheduled execution. |
| `crates/trace-commons-server/src/trace_gate_service.rs` and `crates/trace-commons-gate-enclave/src/orchestrator.rs` | Extract compatibility scoring from the current combined scoring/index-insertion behavior. |
| `crates/trace-commons-gate-api/src/vector_index.rs` and gate-enclave index implementations | Separate reader/writer capabilities and add deterministic upsert, conflict, snapshot-evidence, and self-exclusion semantics. |
| Existing credit storage, settlement operations, and `crates/trace-commons-server/src/near_credit.rs` | Adapt Score eligibility and Settle completion to existing account-level approval, batching, holds, caps, and payout. |
| `crates/trace-commons-server/src/bin/pilot_bootstrap/` and `src/bin/gate_calibrate/` | Extend existing submission/corpus/reporting tooling; retain local development records outside ingest. |
| `crates/trace-commons-server/tests/` and `.github/workflows/ci.yml` | Add PostgreSQL, corpus, adapter, API, and crash qualification alongside current suites. |

Two existing details require explicit changes: the current vector trait exposes
both reads and writes, and the orchestrator inserts vectors while scoring.
Wrapping that orchestrator unchanged as a Score policy would violate the
target design. Also, the pilot submitter currently reports the receipt result;
the new corpus path must wait for and inspect pipeline completion.

## 4. Phase 1 — Minimal complete pipeline

**Outcome:** A developer can submit a local corpus to the service, advance a
persisted run through all four phases, and explain every result without a
model, vector service, or credit settlement dependency.

### Ordered review slices

1. **Small types and one bundle.** Define the initial typed phase contracts,
   outcome schema/version, safe reason codes, checked microcredit conversions,
   and canonical manifest encoding. Define precisely which submitted artifact
   bytes `request_content_hash` identifies; preserve that identity across
   Review transformations. Build one local package selecting all four policies
   and validate its manifest and included artifacts against its bundle ID.
   Keep the package format small; production signing/distribution follows in
   Phase 7. Runners and dependencies use trait objects.
2. **One persisted path.** Add `pipeline_runs` and `phase_outcomes`, their
   uniqueness constraints and forced tenant RLS, and the receipt/worker
   operations. Authenticate with existing claim validation and derive tenant
   scope from it. Store encrypted content, bind the bundle before Admission,
   and atomically commit the run, Admission outcome, and next phase. A bounded
   manually invoked worker executes Review, Score, and Settle asynchronously
   from the request. Persist each outcome and transition atomically; Review
   also commits its approved registry revision. Initially operate one worker
   per test tenant; Phase 2 qualifies concurrent and interrupted execution.
3. **Corpus and inspection.** Extend the existing bootstrap tooling with local
   versioned fixtures, exact request replay, bounded completion polling, a
   local machine-readable report, and a short Markdown comparison. Add the
   minimal contributor-status projection and scoped operator outcome
   inspection using existing records. Missing or malformed receipts and
   incomplete runs must be explicit failures, never defaulted to acceptance.
   Capture an initial legacy baseline and its input/configuration identities
   before changing existing policy behavior; the minimal-bundle demonstration
   itself does not need to match that baseline.

### Minimal bundle

| Runtime phase | Initial policy | Observable outcome |
|---|---|---|
| Admission | Admit a valid authenticated fixture after basic schema/authority validation | `Admit`, validation evidence, fixed rule identifier |
| Review | Pass through the stored redacted test artifact | `Approved`, exact source hash, approved registry revision, identity-transformation evidence |
| Score | Fixed zero microcredits | Completed zero-credit decision with fixed-amount evaluation |
| Settle | Exclude index membership and require no credit operation | Explicit exclusion reason, zero finalized microcredits, absent batch reference |

Zero credit is an intentional first slice: it permits a truthful complete
Settle outcome before financial integration exists. It must remain distinct
from unscored credit. Phase 3 adds fixed positive credit and index inclusion;
do not simulate either effect as completed in this phase.

### Exit demonstration and tests

- Submit a clean fixture corpus through HTTP, process the real PostgreSQL-backed
  worker path, and read status. Each successful trace has one run, one bound
  bundle, one approved revision, and four typed immutable outcomes.
- Replay the exact submission: return the same run. Reuse its key with changed
  content: refuse it. Fail a policy: show an operational error without a
  fabricated decision.
- Pause before Review and Score to demonstrate pending versus completed zero
  states. Query as another test tenant and confirm no outcome visibility.
- Assert decision, evidence, evaluation, source/bundle hashes, and final status
  in the report. Seed a secret to prove it does not appear in report or logs.
- Provide one documented command or script that starts the test environment,
  runs the corpus, and writes the report. It must need no network corpus fetch,
  model download, or external payout. Proposed new CLI options are documented
  with their implementation, not assumed to exist today.

**Deliberately incomplete:** concurrency/crash qualification, positive credit,
index writes, production privacy/rate policies, intervention ordering, complete
public compatibility, production bundles, and operational drills. This exit
establishes the reusable test path, not full compliance with any contract group.

## 5. Phase 2 — Durable execution and bundle recovery

**Outcome:** The minimal corpus retains the same logical results across
retries, concurrent workers, activation changes, and process restarts.

### Ordered review slices

1. **Atomic storage boundaries.** Complete receipt replay/conflict handling,
   immutable run identity and outcome enforcement, and Review's atomic
   artifact-reference/registry/outcome/transition commit. Protect against
   concurrent first submissions of the same key. Add immutable schema readers
   and reject unknown required versions. Encrypted orphan objects remain
   distinguishable from accepted submissions and eligible for safe cleanup.
2. **Queue recovery.** Add fenced leases, bounded claims, backoff, attempt
   limits, safe terminal errors, and stale-worker rejection. Give the
   cross-tenant claimer only the metadata access necessary to return trusted
   tenant/run/lease identity and only lease-column mutation authority. It
   cannot read artifacts or execute policy work. Perform work and commits
   with tenant-scoped capabilities and transactions.
3. **Bundle registry behavior.** Retain validated packages, select the active
   bundle explicitly per tenant, and load the bound package on every retry.
   Test code/configuration/data/projection/format/policy identity changes and
   missing/tampered artifacts. Add operational runnable status outside the
   immutable manifest. Qualify its concurrency semantics in Phase 4.

### Exit demonstration and tests

- Race duplicate receipts and workers; expire a lease while a worker is
  executing. Only the current lease can commit, with one outcome per phase.
- Inject the receipt, Admission, Review, and pre-Score-commit crashes from
  `RUN-004` (boundaries 1–5). Restart with the same database and artifact store
  and demonstrate convergence. Add the remaining boundaries when effects
  exist in Phase 3.
- Activate bundle B between phases of a run bound to A, including after
  resolution but before Admission commit. Existing runs remain on A; a new
  run uses B. Rollback changes selection for subsequent runs only.
- Run actual PostgreSQL tests using two tenants with overlapping identifiers,
  including direct role/column privilege checks. Unit-store tests alone do
  not establish transaction or RLS behavior.
- Extend the report with attempts, pending/retry/failed states, outcome
  identity, and phase age. Assert that committed outcomes are byte-identical
  after retries and bundle changes.

**Deliberately incomplete:** real side-effect recovery, full guard races,
production policy behavior, and product-wide isolation. Continue to use the
minimal bundle and isolated environment.

## 6. Phase 3 — Real index and credit operations with fixed policies

**Outcome:** The corpus covers zero and positive credit, index inclusion and
exclusion, holds, and recovery using simple decisions and real internal
operation records.

### Ordered review slices

1. **Positive Score and credit eligibility.** Add a fixed positive Score
   variant. Atomically commit its outcome, Settle transition, and one eligible
   event keyed by tenant/run/Score outcome. Zero creates no event. Adapt the
   existing event linkage and checked storage conversions without replacing
   the ledger or weakening settlement approval. Exercise maximum, negative
   source, overflow, and excess-precision inputs.
2. **Sealed index commands.** Specify and implement the vector reader/writer
   adapter boundary. Use deterministic test embeddings stored as encrypted
   Score evidence and an isolated implementation of the actual index adapter.
   Decide membership from the bound bundle and committed Review/Score
   outcomes, persist the decision and encrypted command reference/hash on the
   run, then write. A retry after sealing bypasses membership evaluation and
   reuses the stored bytes. Keys cover tenant, index, revision, projection,
   model, and chunk; equal content is a no-op and conflicting content refuses.
   Implement query self-exclusion and snapshot-evidence semantics now so the
   later real Score policy only receives a reader.
3. **Governed internal completion.** Connect eligible events to existing
   previews, source-list and issuer approvals, account batches, holds, caps,
   duplicate-source protection, and payout-identity holds. Record index and
   credit progress independently. Emit Settle only when every required
   internal operation finishes, with the per-run finalized amount and actual
   batch hash, or no batch reference when no batch finalized credit. Reuse
   existing NEAR outbox records and its separate submission/confirmation
   operations.

### Exit demonstration and tests

- Complete all four index include/exclude and Score zero/positive combinations
  permitted by the fixed bundle configurations. Inclusion need not imply
  positive credit. Several compatible runs can finalize in one approved batch.
- Hold credit while completing the index operation; fail the index while
  retaining an eligible event. Recover either without repeating the other.
  A held or unfinished operation must not produce a premature Settle outcome.
- Implement the remaining `RUN-004` crash boundaries 6–11. A counting policy
  and writer prove one membership evaluation before sealing, command reuse,
  one logical index entry, one credit event, and one logical payout request.
  Pre-seal failures may reevaluate; post-seal retries may not.
- Test lost adapter responses and stale leases before dispatch and commit.
  An uncertain index response remains recoverable using the sealed command.
  Equal-key/different-content conflicts fail closed.
- Exercise NEAR recovery with a test adapter that records logical requests;
  external payout stays disabled. Disabled/pending/submitted/confirmed/failed
  payout states never rewrite a completed Settle outcome.
- Show independent index, internal credit, and payout progress in status and
  reports. Outcomes and operational surfaces contain no raw account reference
  or transaction hash.

**Deliberately incomplete:** production authorization for these operations,
intervention ordering, real valuation, and production adapter qualification.
The fixed bundle remains test-only even though the internal integrations are real.

## 7. Phase 4 — Authority, privacy, and intervention policies

**Outcome:** Real Admission and Review behavior protects the pipeline while
Score remains fixed, making privacy and authority behavior easy to review.

### Ordered review slices

1. **Phase guards and lifecycle ordering.** Check authority before content
   access/policy work and again inside outcome/effect commits. Lock applicable
   submission and policy-status rows at the final boundary. For an index
   write, retain these locks until command-result commit, with bounded
   adapter timeouts and recoverable uncertain results. Check operability,
   consent, allowed uses, withdrawal, revocation, and expiry. Integrate the
   existing invalidation path for completed index writes.
2. **Suspension and credit after withdrawal.** Implement audited suspension,
   resumption, and terminal-stop controls with operational status outside the
   bundle hash. Suspension retains the same bundle and safe retryable state;
   it must not silently exhaust into failure solely because it remains
   suspended. Withdrawal before membership forces an evidenced exclusion;
   after sealing it stops pending index work; after writing it queues
   invalidation. Committed Score credit can settle without a trace read.
   Repeat applicable bound-policy guards before NEAR dispatch, including for
   batches covering multiple runs. Resolve those dependencies from existing
   batch/event/outcome links.
3. **Admission validation and limits.** Introduce real schema/path/grant/
   consent/allowed-use checks, tenant/principal quotas and rate limits,
   tombstone rejection, and bounded synchronous privacy-risk handling.
   Persist required evidence and stable reasons. Admission itself makes no
   external writes; transactional quota accounting must count a logical
   receipt once, including across instances and concurrent retries. Keep
   model-based substance and novelty out of Admission.
4. **Review transformation and human evidence.** Add server-side privacy
   transformation, source/result evidence, and Review rejection. Connect
   scoped quarantine reads, audit, leases, and reasoned human assessments to
   the bound Review policy as server-generated evidence. The policy decides;
   a reviewer cannot bypass it or any subsequent phase. Approval must resolve
   every Admission quarantine reason. Store transformed encrypted content
   before the atomic registry commit.

### Exit demonstration and tests

- Add real Admit/Quarantine/Reject and Review Approved/Rejected/transformed
  fixtures. A terminal Admission rejection has one outcome; a Review rejection
  has two. Neither fabricates outcomes for skipped phases.
- Race every applicable lifecycle and consent/use change at pre-work and
  pre-commit boundaries. Include withdrawal after index dispatch but before
  local result commit, and verify which operation committed first.
- Withdraw before and after positive Score; only previously committed credit
  remains eligible. Use a content-reader spy to prove subsequent credit
  finalization does not read the trace.
- Suspend each bound policy during work and before commit; resume the same
  run under the same bundle. Suspend after Settle and before outbox dispatch.
  Outcomes stay immutable and interventions append safe audit evidence.
- Race two reviewers and two submitting service instances. Stale assessments,
  unresolved quarantine reasons, foreign content, and limit conflicts cannot
  become favorable decisions or leak sensitive data.

**Deliberately incomplete:** real scoring/compatibility parity and the full
inherited product surface. All content/effect paths implemented so far must
now obey their guards before expanding policy complexity.

## 8. Phase 5 — Production compatibility policies

**Outcome:** A bundle reproduces the approved current review, scoring, index,
and settlement results through the new phase boundaries.

### Ordered review slices

1. **Pin the compatibility comparison.** Finalize the baseline first captured
   with the Phase 1 corpus tooling: current code/configuration/model identities,
   redacted corpus digest and order, initial index contents, measured external
   inputs, and approved expected results. Define the mapping from legacy
   results to new decisions before implementing the adapter. Include privacy,
   rejection, zero credit, chunking, novelty, quality/dedup/cap rules where
   active, index membership, and governed settlement behavior.
2. **Extract real Score.** Adapt the current substance/novelty algorithms and
   dependencies behind `ScorePolicy` without index insertion. Preserve the
   current deployed rules in immutable bundle inputs. Record model and
   projection identity, measured values, index/snapshot identity, neighbors,
   cardinality and coverage whenever they affect the decision. Store sensitive
   neighbor material and embeddings encrypted; expose hashes and bounded
   measurements. Dependency/evidence failure remains operational failure.
3. **Complete the compatibility bundle.** Pair the real Admission/Review
   policies with extracted Score and Settle membership rules that consume
   committed evidence only. Ensure all effective deployed inputs participate
   in bundle identity. Adapt legacy public representations at their boundary
   without restoring classifier-specific persistence or Score index writes.

### Exit demonstration and tests

- Run legacy and new behavior against equivalent isolated initial indexes,
  controlled corpus ordering, and the same pinned dependency responses where
  required. Never run a shadow comparison against an active write namespace
  or create live credit from comparison runs.
- Compare reviewed artifacts, rejection behavior, awarded/finalized amounts,
  index content, and settlement eligibility/holds. Compare semantic content
  where legacy entry keys differ from the required new deterministic keys.
  Normalize only timestamps and opaque IDs that are not part of identity.
- Identify proposal-required semantic changes explicitly. A mismatch is either
  an implementation defect or a separately reviewed baseline/spec decision;
  do not silently regenerate goldens or preserve a forbidden old behavior to
  make the comparison pass. Review approval remains before Score, and Score
  failure cannot retroactively rewrite Review.
- Prove with capability tests and a write-detecting adapter that Score cannot
  write an index. Change the mutable index after Score and prove Settle neither
  queries it for membership nor repeats valuation.
- Run real-dependency staging smoke tests in addition to pinned-response
  tests. Historical production behavior need not be deterministically
  replayable; evidence must still explain each recorded decision.

**Deliberately incomplete:** full product integration, production package
operations, and final qualification. Do not activate the compatibility bundle
for production merely because corpus parity passes. Alternative valuation
rules remain a follow-up after compatibility transitions are stable.

## 9. Phase 6 — Complete inherited product behavior

**Outcome:** All supported user and worker surfaces operate correctly with
new runs and retained legacy records, without requiring legacy policy columns
as the new source of truth.

### Ordered review slices

1. **Contributor and public compatibility.** Preserve all public surfaces in
   `CMP-001` and their error shapes, or deliver a versioned replacement with
   its client. Finish bounded submission-status batches, processing versus
   credit versus payout states, owned submission pagination, audited retained
   redacted-content reads, credit/event views, and signed score attestations
   identifying schema and bundle. Retain legacy readers without inventing
   historical bundles or phase outcomes.
2. **Authentication and worker authority.** Verify onboarding, invitations,
   upload claims, grant intersections, account sessions, strong-auth gates,
   merge history, and self-withdrawal after ordinary access removal. Complete
   scoped service-credential issuance, rotation, revocation, and separate
   worker/reviewer roles. Remove static production bearer and HS256 bridge
   dependencies from the target deployment. Client redaction and opt-in
   contracts remain covered by protocol/contributor tests.
3. **Customer, export, and derived artifacts.** Select only authorized,
   approved, operable revisions; preserve consent, view-schema, source, and
   bundle provenance. Complete immutable export source snapshots, manifests,
   recoverable claims, partial-output handling, and benchmark/ranking/training
   authority. Connect managed derived-artifact invalidation to the existing
   lifecycle work. Public attribution never authorizes content access.
4. **Lifecycle and optional community.** Complete hash-only tombstones,
   retention, legal holds, purge, bounded revocation retries, visible failed
   targets, and managed-distribution reporting across objects, index, caches,
   and exports. If community remains enabled, qualify attribution consent,
   snapshot withdrawal, and aggregate privacy; otherwise verify its disabled
   not-found behavior. Do not infer deletion from unmanaged customer copies.

### Exit demonstration and tests

- Complete black-box contributor onboarding/submission/status/withdrawal and
  customer export journeys, including legacy/new mixed account histories.
- Run role-by-route and tenant-by-resource tests; unknown and inaccessible
  identifiers are indistinguishable. Every content read is authorized before
  decryption and audited without raw content.
- Test every valid combination of credit, hold, and payout states. In
  particular, `finalized` requires an approved internal batch and cannot be
  inferred from a positive Score or submitted NEAR request.
- Remove a source present in registry/index/cache/export/derived artifacts,
  fail each invalidation adapter, retry, and verify immediate read exclusion
  plus accurate remaining-work/readiness reporting.
- Snapshot published response/error compatibility and verify signed score
  statements offline through missing/rotated/wrong-key cases.

**Deliberately incomplete:** exhaustive contract evidence, production
restore/drill qualification, and cutover. Existing tests count as evidence only
if they exercise the relevant new path and contractual boundary.

## 10. Phase 7 — Lab, security, and operational qualification

**Outcome:** A deployable compatibility bundle and the whole target deployment
have current, automated evidence for all applicable contracts.

### Ordered review slices

1. **Production packages and policy lab.** Finish the package signature/trust,
   canonical integrity, registration, activation, and retention specification.
   Validate all artifacts before activation, reject unknown implementations
   and missing controls, retain readers/packages through outcome retention,
   and prove earlier bound packages still execute after a deploy. Extend the
   existing calibration tool with versioned corpus/input digests, separate
   bootstrap/holdout sets, local reports, and a local bundle-to-development
   catalog. Ingest consumes only the package. Development-only keys, scorers,
   stores, and synthetic settlement backends cannot satisfy production
   readiness.
2. **Operations and complete coverage.** Finish safe readiness, bounded worker
   scheduling, operational summaries, and forensic traversal from run to
   evidence, command, Score event, batch, and interventions. Cover retry
   exhaustion, suspended-policy backlogs, credit holds, outbox draining, and
   invalidation/export blockers. Extend the surface inventory and contract
   map to every enabled route, operation, adapter, table, namespace, telemetry
   sink, and role. Fail CI on an unclassified addition.
3. **Drills and migration qualification.** Adopt the narrow qualification
   specification, run the full crash matrix, and qualify PostgreSQL/object
   restore plus index rebuild from authoritative revisions/commands. Restore
   must preserve IDs, hashes, audit order, and pending operation identity;
   index rebuild creates no new outcomes or credit. Add all `OPS-004` drills,
   including key rotation, audit verification, tenant isolation, package
   integrity, activation/rollback, outcome atomicity, leases, index/Settle,
   settlement approvals, NEAR, withdrawal, and backup/restore. Promotion
   rejects missing, failed, or stale evidence.

### Exit demonstration and tests

- Pass all ten acceptance layers in contracts §21.1, all applicable fixtures
  in §21.2, all 15 scenarios, and every required current drill. Run database
  tests against PostgreSQL with the actual runtime/claimer/worker roles.
- Run seeded-secret scanning over success and failure logs, errors, audits,
  metrics, reports, and operational responses. Probe every inventory surface
  with unrelated roles and tenants, including object/index/cache/export and
  financial adapters. Verify audit tamper detection and staged key rotation.
- Inject each missing required control. The affected operation must not
  commit a partial result or side effect; previously committed phase history
  remains intact. Encrypted receipt orphans are permissible only as described
  by `SYS-003`. Partial Settle progress from an interrupted valid operation
  remains governed by `STL-003` and crash recovery.
- Verify production has no file-backed metadata authority, best-effort DB
  mirror, plaintext fallback, static/HS256 bridge auth, synthetic receipt,
  mock scorer, or unversioned policy dependency. Inspect both configuration
  refusal tests and the deployable composition.
- Produce a qualification report keyed by code revision, bundle, corpus,
  configuration, contract/test identifiers, and evidence hashes. Keep test
  and drill evidence in the test/operations tooling, outside pipeline history.
- Add required CI jobs for the new PostgreSQL, corpus, adapter, and black-box
  suites alongside the existing workspace tests, formatting/lint checks, and
  default/NEAR AI/local-model build checks. A required database or adapter test
  must fail or report an explicit blocker if its environment is missing; it
  must not silently skip and count as qualification. Keep live external payout
  disabled in every policy, runner, bundle, and integration test.

**Deliberately incomplete:** production activation and migration observation.
The candidate is eligible for Phase 8 only when its applicable contracts pass;
a list of tests that have not run is not passing evidence.

## 11. Phase 8 — Activation, rollback, and completion

**Outcome:** New production submissions use the qualified pipeline, retained
history is readable, and the terminal state satisfies contracts §22.

### Ordered review slices

1. **Cutover rehearsal.** Rehearse additive migration and rollback on a restored
   deployment with legacy records, pending legacy work, and new pipeline runs.
   Keep authoritative legacy idempotency lookups so a pre-cutover receipt
   retry cannot start a new pipeline run. Drain legacy work or route it to its
   existing executor with an explicit ownership boundary; never rescore it
   automatically or fabricate phase outcomes. Preserve ledger source
   uniqueness across both processing paths.
2. **Tenant activation.** Activate the qualified compatibility bundle for new
   submissions in a limited tenant cohort, then expand after current drills,
   readiness, corpus evidence, error/age metrics, credit reconciliation, and
   index/invalidation checks pass the qualification thresholds. Record the
   operational activation explicitly; timestamps do not select a bundle.
3. **Rollback and retirement.** Demonstrate selecting an earlier qualified
   bundle for subsequent runs. Existing runs retain their bound package;
   suspend an unsafe bound policy instead of silently switching it. Before
   an earlier pipeline bundle exists, contain an initial rollout by stopping
   new pipeline routing while retaining its workers/readers/packages; this is
   separate from routine bundle rollback. Disable superseded legacy writers
   only after their owned work is drained. Keep legacy reads and required
   package/schema support through retention; destructive schema cleanup is a
   separate later change.

### Terminal acceptance gate

The implementation is complete only when all of the following are true:

- Every non-deferred contract has an automated passing test at its specified
  boundary. Conditional community contracts have a recorded applicability
  decision and the disabled behavior is tested when appropriate.
- Every completed runtime phase has exactly one immutable typed outcome;
  phases skipped after rejection have none. Every visible state is derived
  from authoritative outcomes and operational records.
- All retryable effects pass idempotency, concurrency, and crash recovery,
  including a single logical external payout with separate confirmation.
- All tenant boundaries pass PostgreSQL and black-box isolation tests, and
  every required drill has current passing evidence.
- The compatibility corpus matches its approved baseline, and activation/
  rollback changes only new runs. Legacy requests and historical reads retain
  their correct identity and meaning.
- Score performs no index writes; Settle uses sealed commands and immutable
  inputs; existing governed settlement and no-clawback behavior remain intact.
- The target deployment does not depend on the excluded implementation or
  domain concepts in contracts §20.

Realistic Admission/Review and compatibility Score/Settle policies are already
implemented by this point. Subsequent calibrated valuation changes ship as
new bundles through the same lab, comparison, qualification, and activation
path. Multi-party valuations require their separate protocol and tests first.

## 12. Corpus and review evidence

The Phase 1 harness is a delivery requirement, not a final-phase test project.
Use it continuously with two corpus layers:

- A small, checked-in redacted fixture corpus for fast local and CI runs.
  Start with the complete minimal path; add rejection, quarantine,
  transformation, failures, positive credit, index operations, interventions,
  and product cases as their implementations arrive.
- A versioned compatibility corpus with a digest, approved baseline, pinned
  configurations, explicit ordering, and isolated initial index state. Capture
  the baseline early so subsequent changes cannot redefine current behavior
  unnoticed. Large or sensitive evidence stays encrypted and outside reports.

The harness must exercise HTTP receipt, actual asynchronous runner operations,
PostgreSQL persistence, artifact storage, and status/inspection. Direct policy
tests are useful but do not replace that path. It must support pausing at a
phase, injecting a failure, restarting, replaying exact requests, changing the
active bundle, and comparing final semantic results as those features arrive.
Every wait is bounded; timeouts report incomplete work and fail the requested
completion check instead of treating a receipt as success.

Each local report contains:

| Scope | Required report information |
|---|---|
| Execution | Corpus/input digest, code revision, bundle ID, isolated dependency configuration identities, expected fixture count |
| Per fixture | Safe fixture label or hash, scoped run/outcome references, phase/state, decision, bounded evidence and artifact hashes, evaluation rule, retry/error labels |
| Operations | Index decision/command hash/progress, Score microcredits, credit state and finalized amount, batch hash when present, separate payout state |
| Comparison | Expected/actual semantic differences, unexplained failures, missing outcomes, completed versus incomplete contract/scenario coverage |
| Aggregate | Counts by phase/decision/state, pending-work age, failures, and completion duration, with no trace text, secrets, raw account IDs, or transaction hashes |

Do not normalize content hashes, bundle identity, stable idempotency keys, or
identity-bearing fields out of comparisons. The report may link to authorized
encrypted evidence through scoped tooling; it must not embed that evidence.

For each review slice, include the new observable behavior, relevant report
diff, tests executed, current limitations, and the next increment. A reviewer
should not need to understand a new model algorithm to review lease recovery,
or a new settlement system to review a scoring rule.

## 13. Contract completion ownership

Maintain a machine-checkable test manifest beginning in Phase 1. Each contract
ID maps to applicable boundaries, test IDs, implementation phase, and evidence
status (`planned`, `partial`, `passing`, or an explicitly specified deferral).
An existing test can be reused after checking that it proves the target
behavior. Every normative bullet needs coverage; one token test per contract
is insufficient. Do not persist this manifest in the ingest database.

The table assigns completion responsibility. Cross-cutting contracts expand
with every phase and are finally qualified in Phase 7; an earlier owner does
not exempt later code from their requirements.

| Contract IDs | Completion responsibility and evidence |
|---|---|
| SYS-001, SYS-002 | Phases 2/4/6; Phase 7 inventory-wide tenant, PostgreSQL role, and pre-content authority tests |
| SYS-003, SYS-004, SYS-009 | Incremental from Phase 1; Phase 7 dependency-removal, privacy-sink, error-compatibility, and audit-tamper tests |
| SYS-005, SYS-010 | Phases 2/3; Phase 7 every-mutation replay, fencing, concurrency, and visible failure tests |
| SYS-006, SYS-007, SYS-008 | Phases 1/2/3/6; Phase 7 provenance traversal, immutable-byte comparison, and retained schema readers |
| AUTH-001, AUTH-002, AUTH-003, AUTH-004, AUTH-005 | Phase 6; onboarding/claims/grants/session/credential matrices, with Phase 7 rotation and least-privilege qualification |
| SUB-001, SUB-002 | Phase 6 full protocol/contributor suites, building on Phase 1 receipt validation |
| SUB-003, SUB-004 | Phase 2 atomic receipt, exact replay/conflicts; Phase 4 quota replay; Phase 8 legacy retry qualification |
| SUB-005, SUB-006, SUB-007 | Phase 4 Admission decisions, distributed limits, and tombstones; Phase 6 all lifecycle/submission/export paths |
| BND-001, BND-002, BND-003, BND-004 | Phases 1/2 identity, traits, binding and integrity; Phase 7 production package retention/loading; Phase 8 activation |
| REV-001, REV-002, REV-003, REV-004 | Phases 1/2/4 source binding, atomic revisions, review leases/evidence, and transformations |
| SCR-001, SCR-002, SCR-003, SCR-004 | Phases 1/3/5 fixed/real policies, read-only index evidence, checked units, and atomic event creation |
| SCR-005 | Explicitly deferred until external valuation protocol adoption; activation of a dependent bundle is refused meanwhile |
| STL-001, STL-002, STL-003, STL-004, STL-005 | Phase 3 effect and settlement tests; Phase 4 guards; Phase 7 real adapter/outbox qualification |
| RUN-001, RUN-002, RUN-003, RUN-004 | Phases 2/3 state, uniqueness, leases, and all eleven crash boundaries; full repetition in Phase 7 qualification |
| RUN-005 | Phase 7 inherited backup/restore behavior and adopted migration qualification; index rebuild never creates awards |
| GRD-001, GRD-002, GRD-003, GRD-004 | Phase 4 intervention/race tests, repeated with real policies and batched payout in Phase 7 |
| STA-001, STA-002, STA-003, STA-004, STA-005 | Phases 1/3 status increments; Phase 6 complete status, ownership, and signed-attestation matrices |
| CRD-001, CRD-002, CRD-003, CRD-004, CRD-005 | Phases 3/4 ledger, approvals, caps/holds, and no-clawback; Phase 6 all public credit paths |
| LIF-001, LIF-002, LIF-003 | Phases 4/6 withdrawal, retention, purge, and every managed invalidation target; Phase 7 drills |
| EXP-001, EXP-002, EXP-003, EXP-004 | Phase 6 exact authorized selections/views, source snapshots, partial export recovery, and derived provenance/invalidation |
| COM-001, COM-002 | Phase 6 if enabled; otherwise disabled-surface tests and explicit applicability record |
| OPS-001, OPS-002, OPS-003, OPS-004, OPS-005, OPS-006 | Incremental diagnostics from Phase 1; Phase 7 full readiness, bounds, summaries, drills, traceability, and keys |
| LAB-001, LAB-002, LAB-003 | Phases 1/5/7 corpus, all test levels, lab/package separation; Phase 8 activation/rollback evidence |
| CMP-001, CMP-002 | Phase 6 public compatibility; Phases 7/8 forbidden production dependencies and legacy write retirement |
| CMP-003, CMP-004 | Record scope/applicability in Phase 1; resolve production-use specifications and required product decisions by their owning phase below |

Track the minimum scenarios explicitly as well:

| Scenarios | First complete implementation; final qualification is Phase 7 |
|---|---|
| SCN-001 | Phase 6, extending the Phase 1 submit-to-status path with full onboarding |
| SCN-002 | Phase 3, extending Phase 2 receipt recovery with credit and sealed commands |
| SCN-003, SCN-004 | Phase 4 quarantine and Review rejection |
| SCN-005 | Phase 5 real Score dependency failure |
| SCN-006 | Phase 3 Settle index crash |
| SCN-007 | Phase 2 bundle change during a run |
| SCN-008, SCN-009 | Phase 4 suspension and withdrawal before Settle |
| SCN-010 | Phase 6 complete withdrawal propagation |
| SCN-011 | Phase 3 internal settlement and test-adapter payout recovery |
| SCN-012, SCN-013 | Phase 7 complete surface inventory, expanded from Phase 1 onward |
| SCN-014 | Phase 6 customer export |
| SCN-015 | Phase 7 staging activation rehearsal of Phase 5 compatibility results; Phase 8 production activation |

## 14. Narrow specifications and open decisions

Write specifications when their implementation boundary is reached, so they
do not postpone the first runnable pipeline. The following are deliverables
within this plan unless explicitly deferred here:

| Specification | Needed by |
|---|---|
| Initial phase payloads, schema versions, reason codes, and exact source-hash boundary | Phase 1 small fixed family; extend/version with Phases 3–5 policies before production use |
| Vector adapter idempotency, deterministic keys, conflict behavior, self-exclusion, and snapshot evidence | Phase 3 |
| Score event/batch linkage, checked storage amounts, Settle completion, and NEAR integration | Phase 3; suspension behavior completed in Phase 4 |
| Suspension/resumption/termination authority, ordering, and audited operator controls | Phase 4 |
| Compatibility semantic mapping and baseline approval | Capture inputs in Phase 1; finalize before Phase 5 policy adaptation |
| Bundle package signatures, trust, executable-version retention, and package retention rules | Phase 7, before any production activation |
| Migration, backup/restore, recovery, and promotion qualification | Phase 7, before Phase 8 cutover |
| External valuation trust/attestation/aggregation protocol | Deferred; required before any dependent policy can activate |

`CMP-003` permits deferring detailed specifications for initial architecture
work; it does not justify shipping an affected production feature without its
specification and tests. In particular, this plan completes inherited restore
behavior and adopts migration qualification before cutover despite the current
document's detailed-qualification deferral.

Open product decisions from `CMP-004` remain visible. Preserve published
interfaces while their support period is undecided; keep contributor reasons
safe and bounded; report only known managed distribution. Resolve quarantine
remediation and maximum age before Phase 4 production-policy qualification,
and choose enabled customer/community surfaces before Phase 6 qualification.
Do not introduce an automatic fraud clawback while correction policy is open;
preserve governed reversal events and ordinary no-clawback behavior. Unresolved
choices block only the affected production surface, not local pipeline work,
and cannot silently remove a non-deferred acceptance requirement.
