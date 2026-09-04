# Review of the Versioned Classifier Processing Design

Date: 2026-09-04
Subject: `[proposal.md](proposal.md)`
Related: `[classifier-issues.md](classifier-issues.md)`,
`[project-review.md](project-review.md)`,
`[system-behavioral-contracts-fable.md](system-behavioral-contracts-fable.md)`,
`[system-behavioral-contracts-sol.md](system-behavioral-contracts-sol.md)`

Code checked: `crates/trace-commons-gate-enclave/src/orchestrator.rs`,
`crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (gate worker,
novelty-utility credit, residual-risk derivation),
`crates/trace-commons-protocol/src/trace_contribution.rs`,
`migrations/V23` through `V45`.

## 1. Summary

The problem definition is well grounded. Each of the eight problems maps to
code and to a filed issue. Two problems are missing: the privacy classifier
and the credit scorecard. Both are classifiers with the same defect class.

The direction is correct. Observation before policy, intent before effect,
immutable evaluation, and explicit deployment selection are the right four
ideas. They match the processing and effect contracts in both contract
documents.

The proposal has more parts than the properties need. It has four JSONB
documents, six hash kinds, five policies, and three fact families. Two
documents, three hashes, and one action policy give the same properties.

Remark: I aggree with this. We should simplify

The proposal has one design gap that blocks implementation. It does not say
when the privacy decision happens or how a human review enters the
evaluation. Today the privacy decision is immediate and the gates run later.
The proposal inverts this order without saying so.

Remark: I aggree with this. We should specify how privacy should be handled

Section 5 gives twelve recommendations in priority order.

## 2. Problem definition



### 2.1 Grounded claims


| Claim in the proposal            | Evidence                                                                                                                                                                                                                                                                                                                    |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| No complete classifier identity  | `gate_version_hash` covers floors and model ids. It does not cover the renderer. Issue #211.                                                                                                                                                                                                                                |
| Schema and classifier coupling   | Migrations V23, V24, V25, V37, V39, V40, V41 each add classifier-specific columns to `trace_gate_decisions`.                                                                                                                                                                                                                |
| Mixed domain concepts            | The decision row holds `credit_withheld_reason`, `vector_entry_id`, and the shadow scores `credit_quality_micros`, `dedup_simhash`, `contributor_factor_micros`.                                                                                                                                                            |
| Mutable audit records            | The four admin routes `rescore-perplexity`, `score-credit-quality`, `recluster-dedup`, `recompute-contributor-caps` update decision rows in place.                                                                                                                                                                          |
| Incomplete provenance            | No decision row links to a calibration corpus, a report, or the index state at scoring time. Issue #199.                                                                                                                                                                                                                    |
| Effects occur inside classifiers | `EnclaveGateOrchestrator::evaluate` calls `self.index.insert` before it returns. The handler writes the decision row after the return. The usearch index flushes every 32 inserts or 60 seconds. A crash between the two leaves either an index entry with no decision row or a decision row with an unflushed index entry. |
| Several vector planes            | The per-tenant novelty usearch index, the `trace_vector_entries` worker with external adapters, and the global dedup usearch index are three paths with no shared consistency rule.                                                                                                                                         |
| No controlled reprocessing unit  | The gate driver enumerates through `trace_gate_evaluation_attempts` and skips submissions that have a decision row.                                                                                                                                                                                                         |


The root problem statement is correct. The system cannot always name the
producer of a result and cannot always reproduce it.

### 2.2 Missing problems

`Classifier` is the wrong word for that. A classifier’s product is a class. This thing’s product is typed measurement data: novelty micros, neighbor lists, substance scores, later maybe a privacy rubric. None of those have to be labels.

`Observer` also sits cleanly in the existing chain:

```
Projection prepares input.
Evidence supplies comparison state.
Observer produces an Observation.
Policy produces a Decision.
```

**The privacy classifier is not in the model.** The residual-risk rule in
`residual_risk()` in the protocol crate is a classifier. Its rubric changed
five times behind unchanged wire values (#325). It decides the submission
status today. It is the reason zero of ninety-nine real sessions reached
acceptance (#219, #373). The proposal moves it out of scope as "a persisted
privacy-review fact" that "the review workflow supplies". This is the
classifier with the most policy drift and the largest user impact. It has the
same defect class as novelty and substance. It belongs inside the versioned
model.

Remark: Exactly, We might want a ResidualRisk classifer

**The credit scorecard is not in the model.** `compute_value_scorecard` is a
policy with hand-set weights. It charged privacy risk twice for months, and
every medium-risk trace scored exactly zero (#298 and the double-penalty
design). It is versioned by a constant in the wire crate. The proposal has a
`CreditDecision` with `Issue` or `Withhold` and says nothing about the
amount. The current credit event at evaluation time is `novelty_utility`
with a fixed delta from an environment variable. The pending estimate is a
different number from a different function. The proposal does not say which
of these the credit policy replaces.

Remark: Yeah why don't we simplify the model and just have a Scorer?

**The cost of measurement is not named.** The issues report says one fact
prices out every fix: a real evaluation needs a 27B model on a GPU, and
nobody re-measures. The proposal solves provenance and durability. It does
not solve cheap repeatable evaluation. That is acceptable scope, but the
document should say so and should point to the three-tier harness in the
issues report as a required companion.

Remark: Right. This issue should be handled by the "lab" crate. the ingest server should just attribute the result to whatever training run the lab produces. However it's treated the proposal should address this.

**The enclave boundary is not named.** The current `EnclaveGateOrchestrator`
trait is the seam the Phase B migration depends on. The proposal does not say
where classifiers run or how a remote scorer fits the job and lease model.

Remark: Right, this should be addressed. Actually we aare missing details about the crate architecture in general, including the introduction of the "lab" crate

## 3. Direction



### 3.1 What is right

- **Observation without policy.** A classifier returns measurements. A policy
returns a disposition. This is the fact-versus-label split the issues report
asks for. It makes a threshold change a policy re-run over stored data.
- **Intent before effect, receipt after.** The outbox pattern closes the
crash window in `evaluate`. Idempotency keys make retries safe.
- **Immutable evaluation, mutable job.** One row for coordination, one
aggregate for audit. Reprocessing appends. Nothing rewrites history.
- **Explicit active selection.** A deployment assignment, not a timestamp,
selects production behaviour. Shadow runs persist with no effects.
- **Missing is missing.** `Fact::Unavailable` with a reason is the #206 fix as
a type.
- **Typed JSON with a schema identifier.** This ends the column-per-metric
pattern.

These match `PROC-001` through `PROC-006` and `EFF-001` through `EFF-003` in
the sol contracts, and `EVAL-1` through `EVAL-7` in the fable contracts.

### 3.2 Where the direction needs a decision

**Privacy timing.** Today, status comes from residual risk at submit time,
and the gates run minutes to hours later. A quarantined trace waits for a
human. The queue sat at 48 for 71 days. The proposal's registration policy
consumes `privacy_review: PrivacyReviewFact`, with no unavailable variant.
Either the evaluation cannot commit until a human reviews, or it commits with
a missing fact and the document does not say what happens next. This must be
decided before anything else. The recommended shape is in R1.

**Recompute versus checkpoint.** The default workflow recomputes all
observations after a retry. Substance scoring is sequential per chunk against
a remote 27B model. A crash after chunk 30 of 40 repeats 30 remote calls. The
proposal allows a "content-addressed observation cache" but gives it no table
and no place in the crash table. A cache with no durable home is a design
note, not a design. See R8.

Remark: This can probably be added later. It's important that the proposed architecture supports this evolution

**Assignment binding time.** Step 1 resolves the deployment at receipt and
pins it in the job. With a backlog, a rollback does not reach pending jobs.
That may be intended. The document should say which behaviour it wants. See
R7.

Remark: Right. Ideally we need to detail to migration process. Do we stop accepting new traces while we clear the queue, probably not. This also raises an interesting point, at what point to we specify how a trace should be processed? We need to projection at ingestion time but maybe the rest can be late binding, where policies are determine when a work item is dequeued. We would then just need projection mapping, but wait it's not that simple. What if the projection has different fields/semantics and can't be mapped. Yeah i think we need a migration plan that clears the queue before depricating the old pipeline.

Numbered for reference in the recommendations.

1. **Bundle naming drifts.** "Classifier bundle" in section 1, "gate bundle"
  in sections 3 through 6, "bundle set" in the HTML version, and
   `executor_bundle_id` on receipts. `executor_bundle_id` is never defined.
  1. Decision: make this consistent
2. **Effect is listed as an immutable reasoning concept.** Section 3 lists
  five immutable concepts and includes Effect, which "records a requested
   state change and the result". Section 5 splits it into a mutable intent
   and an immutable receipt. The list should hold four concepts.
  1. Decision: Agreed. This should be consistent. Seciont 5 is correct (i think)
3. **Facts violate their own invariant.** Invariant 6 says each fact set
  identifies its source. `RegistrationFacts.envelope_valid`,
   `CreditFacts.consent_allows_credit`, and `CreditFacts.issuer_authorized`
   are bare booleans with no source and no unavailable variant.
   `PrivacyReviewFact` has no unavailable variant.
4. `Review` **dispositions imply an effect.** `NoveltyDisposition::Review` and
  `SubstanceDisposition::Review` ask for a human. Invariant 9 says classifier
   decisions create no effects. Nothing maps `Review` to a registration
   disposition or to a `ScheduleReview` effect. The HTML version had
   `ScheduleReview` in `EffectKind`; the markdown dropped it.
  1. Remark: Yes review should be an effect
5. **One index decision, several namespaces.** `IndexDisposition` is
  `AddToFutureSnapshot` or `DoNotAdd`. Section 7 defines an immutable novelty
   epoch, where a future snapshot makes sense, and an online tenant dedup
   namespace that "changes after accepted submissions", where insertion is
   immediate. One disposition cannot express both. `VectorIndexMembership`
   keys on `epoch_id` only, so online members have no record shape.
6. **Schema granularity differs between the table and the document.** The
  `evaluations` table has one `observations_schema` and one
   `evidence_schema`. The JSON example puts a `schema` on each observation.
   Per-observation is correct. The singular columns are wrong.
7. **An author remark is left in the text.** "It isn't clear that we need
  output_schema here." The answer is no. The per-observation `schema` field
   already carries it.
8. **The two provenance paths differ.** Section 3 ends at
  `TraceRevision + GateBundle + IndexEpoch`. Section 5 ends at
   `trace revision and gate bundle` and drops the index epoch.
9. **The hash policy contradicts the HTML version.** The HTML says one hash
  over the evaluation with optional component hashes. The markdown says
   every observation, fact set, and decision gets a hash, plus `payload_hash`
   and `result_hash` on effects. That is six hash kinds.
10. `required` **on effects is undefined.** The job completes "when required
  effects have receipts". Nothing says what an optional effect is or what
    happens to it.
11. **Shadow completion is undefined.** A shadow evaluation inserts no effects.
  The state machine has no path from `DecisionsCommitted` to `Complete`
    with zero effects.
12. **Reprocess has no reference column.** Section 5 says a reprocess
  evaluation "references an earlier evaluation". No column holds that
    reference. Nothing defines which evaluation is effective for a trace
    revision after a reprocess. `PROC-004` in the sol contracts requires that
    rule.
13. **Reprocess always re-runs classifiers.** The issues report's main payoff
  is "run the new policy over stored facts, in SQL, with no GPU". The
    proposal's reprocess mode goes through step 2, "produce observations",
    every time. There is no policy-only reprocess.
14. **Runtime identity leaks into facts.** `CreditFacts.issuer_authorized`
  depends on which principal runs the worker. Two workers evaluating the
    same revision under the same bundle would produce different decisions.
    An evaluation must be a function of the trace, the bundle, and the
    evidence only.
15. **Job uniqueness includes the deployment.** The unique key is
  `(tenant, revision, deployment, bundle, mode)`. Once the bundle is pinned,
    `deployment_id` adds nothing to logical uniqueness. It also means a
    terminal reprocess failure cannot be retried under the same key.
16. **Cross-tenant claim under forced RLS is not addressed.** The job claim
  selects across tenants with `FOR UPDATE SKIP LOCKED`. Every tenant table
    has forced RLS. The existing driver role has `SELECT` with a
    `USING (true)` policy and no write. The claim needs an `UPDATE`. Section
    6 says "worker claims use narrow roles" and stops there.
17. **No contributor-visible projection.** Job states and effect states are
  internal. Nothing says how `TerminalFailure` or a pending job appears in
    submission status. The current system shows fail-closed as "not
    credited". A contract for the projection is missing.



## 5. Simplification

The properties to keep are: immutability of results, a complete provenance
chain, intents before effects, explicit active selection, missing-is-missing,
and schema-tagged documents. The shape below keeps all six with fewer parts.

### 5.1 Two documents, not four

Evidence descriptors already live inside each observation in the pseudocode
(`NoveltyObservation.evidence_id`). Facts are a typed view over observations
plus availability. Store:

- `observations`: a list of observation records, each with `role`,
`classifier_id`, `evidence` descriptor, `schema`, `payload` or
`unavailable { reason }`, and `observation_hash`.
- `decisions`: a list of decision records, each with `policy_id`, `schema`,
`input_refs` (observation and decision hashes), `disposition`,
`reason_codes`, and `decision_hash`.

Drop the separate `evidence` and `facts` documents. Keep `Fact<T>` and the
typed fact structs as in-memory types built at decode time. `facts_hash`
becomes the hash of the sorted `input_refs`, which is derivable and need not
be stored.

### 5.2 External inputs are observations too

Consent, envelope validity, the privacy assessment, and a human review are
all inputs to a policy. Record each as an observation with a role and a
producer:

- `role: privacy`, producer: the residual-risk classifier, payload: scrub
outcome, finding counts, confidence.
- `role: review`, producer: the review workflow, payload: reviewer decision
and reason label, with the review id as evidence.
- `role: consent`, producer: the envelope validator.

This removes the bare booleans (inconsistency 3), gives the privacy classifier
a version (missing problem 1), and gives a review a place to enter the chain
(section 3.2). It also matches `PROC-003`: privacy observations remain facts
or evidence.

### 5.3 One action policy, not five

Novelty and substance policies are thresholds over one observation each.
Registration, credit, and index policies are small pure functions over the
same inputs. Five policy refs, five fact types, and five decision records buy
independent versioning of five tiny functions. One versioned action policy
that returns one decision document with four dispositions gives the same
audit trail:

```text
decision {
  policy_id
  input_refs
  privacy:      accept | quarantine | reject
  commons:      novel_and_substantive | duplicate | insufficient | review
  credit:       eligible { basis } | withheld { reason }
  index:        [ { namespace, action } ]
  reason_codes
}
```

Thresholds move into the policy's configuration document, which is versioned
with it. A threshold change is a policy version and a policy-only reprocess.
This resolves inconsistencies 4, 5, and 10. If the team wants per-role
thresholds versioned with the classifier, put them in `ClassifierRef` as
calibration parameters and let the action policy read them as inputs.

Remark: This is a big change and i'm not sure if it's an improvement.  My current thinking distinct delicions which require provenance:
Ingest (Ingest, Quarentine), based on privacy
Review (accept, reject)
Credit (eligibe(reason, amount), withheld(reason))

### 5.4 Three hashes, not six

Hash at reference boundaries only: `observation_hash` because decisions
reference observations, `decision_hash` because effects reference decisions,
and `evaluation_hash` because attestation references the whole. The effect
payload is derived from the decision and needs no separate hash. The receipt
result can carry a hash when an external system signs it, not by default.

### 5.5 Two index namespace kinds, not four

Define `epoch` (immutable, sealed, for novelty reference) and `online`
(mutable, with a generation counter, for dedup). Membership carries either
`epoch_id` or `generation`. The index disposition is a list of
`(namespace, action)`. Remove ranking-features from this document. It is a
downstream consumer with its own design.

### 5.6 Smaller job model

- Unique key `(tenant, trace_revision, bundle, mode)`. Store `deployment_id`
as an attribute.
- Drop `required` on effects. Every effect from an active evaluation is
required. A shadow evaluation has none, and the job completes at commit.
- Add `based_on_evaluation_id` to jobs and evaluations for reprocess.
- Add a job mode `policy_reprocess` that reads the observations of
`based_on_evaluation_id` and runs only the policy.



### 5.7 What not to simplify

- Keep the outbox and receipts as separate tables. They cross a failure
boundary and the separation is the point.
- Keep the observation cache if the substance scorer stays remote. Give it a
table keyed by `(classifier_id, input_hash, evidence_id)`. It is a
checkpoint, not a cache, and the crash table should list it.
- Keep deployment assignments as a table with history. Do not replace them
with "latest bundle wins".



## 6. Recommendations

In priority order.

**R1. Bring privacy into the model and fix the timing.** Make residual risk a
classifier role with a versioned rubric. Run the privacy observation and the
privacy disposition at receipt, in the same transaction that stores the trace
revision, so quarantine stays immediate. Run the commons classifiers in the
job. Make a human review an appended `review` observation that triggers a
policy-only reprocess. Define which evaluation is effective for a revision:
the newest committed active evaluation, and say so in the read model.

Remark: Agreed. It feels like this is part of the ingest decision pipeline

**R2. Merge facts into observations.** Store observations with availability
and decisions with input references. Drop the `evidence` and `facts` columns
and `facts_hash`. Keep the typed fact structs in code.

Remark: I think this makes sense, i would need to see it to confirm

**R3. Use one action policy per bundle.** One decision document with privacy,
commons, credit, and index dispositions. Thresholds live in the policy
configuration or in the classifier's calibration parameters, both versioned.

Remark: I'm ensure about this. At a high level it makes sense, i'm just a bit stuck in my thinking that we are making granual decisions but i think simplfiication would be helpful

**R4. Add policy-only reprocess.** A job mode that reuses stored observations
and re-runs the policy. Record `based_on_evaluation_id`. This is the cheap
path the issues report exists to enable.

Remark: But how do we guarentee that different policies will not require different observations?
The current schema where we model that observations can be missing (or i guess they used to be caleld facts). supports this, but in practice THe re-evaluation may be limited to the availability of re-usable facts between policy changes.

**R5. Keep governance out of the evaluation.** The credit disposition says
eligible or withheld with a basis, computed from the trace, the bundle, and
the evidence. The issuer allowlist, caps, holds, and settlement stay in the
ledger path, where they are today. Define what `IssueCredit` carries: the
event type, the basis, and the decision hash. Say whether the pending
estimate is produced by this policy or stays in the wire crate. Recommended:
move it here.

Remark: Yeah we didn't specify this very clearly. Probably because i don't think credit issuing is imeplemented completelty.  Directionally, issuing credit is a decision which should be the function of some policy and some state.

**R6. Split the index model into epoch and online.** One membership record
shape with `epoch_id` or `generation`. One index disposition list. Drop
ranking features from this document.

Remark: the distinction makes sense. I don't understand what ranking features are here (nth closest neighbour maybe?) Yeah i would say storing closest neightbours in the database is probably not that helpful

**R7. Decide assignment binding.** Recommended: resolve the assignment at
claim time and pin it on the job then, so a rollback reaches the backlog. If
receipt-time binding is wanted, say why.

Remark: Yes i had the same intuition, late binding makes sense

**R8. Give the checkpoint a table.** `observation_checkpoints` keyed by
`(classifier_id, input_hash, evidence_id)`, written after each expensive
observation, read before recompute. Add a row to the crash table for "after
a checkpoint, before evaluation commit".

Remark: not sure about this. I suspect this about vector indexing and i'm not sure how we should model this. My intuition wrapping this in evidence abstraction directionally makes sense, but how we handle evolving evidence i'm not sure.

**R9. Define the contributor projection.** A table from job state and effect
state to submission status and explanation label. `TerminalFailure` is
visible, fail-closed, and not credited. A pending job shows as pending.

Remark: Agreed this makes sense

**R10. Address the cross-tenant claim.** Name the role and the policy that
lets a worker `UPDATE` across tenants under forced RLS, with bounded columns.
The existing `trace_gate_driver` role is read-only and is the template.

Remark: Not sure i understand this

**R11. Clean the document.** One bundle name. Define or remove
`executor_bundle_id`. Remove the author remark. Make the two provenance paths
equal. Reconcile the hash policy with the HTML version. Move `schema` to
per-item. Add `ScheduleReview` back to the effect kinds or explain why review
is not an effect. State the scope boundary with the evaluation harness and
link to it.

**R12. Keep the incremental path.** The six steps in the issues report still
apply. Steps 1 and 2 (stamp what exists, golden traces in CI) cost nothing
and protect the migration. Step 4 (registry table, bundle id on every
decision) is where this proposal starts.

## 7. What the proposal gets right and should not lose

- The durability principle: persist results and irreversible boundaries,
recompute pure work, send execution detail to telemetry.
- The classifier restriction: a classifier can query an index but cannot
insert into one.
- Invariant 16: missing required evidence causes a closed decision or a
processing error, never a favorable default.
- Invariant 17: current state is a projection, not an overwritten record.
- The crash table. It is the clearest statement of the durability model in
the document. Extend it; do not remove it.



## 8. Issues worklist

This section collects the remarks in sections 1 through 6 into issues. Each
issue has a stable id, a status, the remarks it came from, what is at stake,
the options, and a decision line to fill in. Work through them in order. The
order follows dependency, not importance.

Status values: `agreed` means the remark and the review say the same thing
and only the proposal text needs the change. `open` means a decision is
needed. `deferred` means the decision can wait but the design must not block
it.


| Id   | Title                                                | Status                  | Depends on |
| ---- | ---------------------------------------------------- | ----------------------- | ---------- |
| I-1  | Names: observer, classifier, scorer                  | open                    |            |
| I-2  | Privacy in the ingest decision                       | agreed, needs spec      | I-1        |
| I-3  | Decision structure: granular decisions or one policy | agreed                  | I-2        |
| I-4  | Credit as a scorer plus a policy                     | open                    | I-3        |
| I-5  | Policy-only reprocess and observation compatibility  | open                    | I-3        |
| I-6  | Late binding and the migration plan                  | open                    | I-3        |
| I-7  | Checkpoints and evolving evidence                    | deferred, split         | I-6        |
| I-8  | Vector namespaces and ranking features               | agreed, one question    |            |
| I-9  | Lab crate and crate architecture                     | open                    |            |
| I-10 | Cross-tenant job claim under forced RLS              | open, needs explanation | I-6        |
| I-11 | Review is an effect                                  | agreed                  | I-3        |
| I-12 | Small consistency fixes                              | agreed                  | I-1, I-3   |
| I-13 | Contributor-visible projection                       | agreed                  | I-3        |




### I-1. Names: observer, classifier, scorer

Status: open.

Source: the inserted paragraph in section 2.2, and the remark on the credit
scorecard ("just have a Scorer").

What the remarks say. "Classifier" is the wrong word. The unit produces typed
measurements, not classes. "Observer" fits the chain: projection, evidence,
observer, observation, policy, decision. For credit, "just have a Scorer".

What is at stake. The name decides how the reader thinks about the unit. If
it is called a classifier, people will put thresholds inside it. If it is
called an observer, thresholds stay in the policy. That is the fact-versus-
label split the issues report asks for.

Options.

1. `Observer` produces an `Observation`. `Policy` produces a `Decision`. No
  `Classifier` type anywhere. The residual-risk rule is a `PrivacyObserver`.
2. Keep `Classifier` for the model-backed units and add `Observer` as the
  trait name. Two names for one thing.
3. Add `Scorer` as a third kind for credit. A scorer is an observer whose
  observation is a number and a breakdown. It needs no third kind.

Review position. Option 1. One noun for the measuring unit. A scorer is an
observer with role `credit_score`. The credit amount then comes from a policy
that reads that observation and the state it needs. See I-4.

Decision: Option 1

### I-2. Privacy in the ingest decision

Status: agreed, needs a specification.

Source: remarks at "We should specify how privacy should be handled",
"We might want a ResidualRisk classifier", and on R1 ("part of the ingest
decision pipeline").

What the remarks say. Residual risk is an observer with a versioned rubric.
The privacy decision belongs in the ingest pipeline, not in the later job.

What is at stake. Quarantine must stay immediate. The rubric changed five
times with no version. Zero of ninety-nine real sessions reached acceptance
because of it. A human review must have a place to enter the chain.

Proposed shape.

1. At receipt, in one transaction: store the trace revision, run the
  `PrivacyObserver`, run the ingest policy, write the ingest evaluation,
   write the effect intents (`RegisterTrace` or `QuarantineTrace`, and
   `ScheduleReview` when quarantined), and create the commons job.
2. The ingest evaluation is an evaluation like any other: immutable, hashed,
  with the observer version and the policy version.
3. A human review is an appended observation with role `review`, producer
  `review_workflow`, evidence the review id. It triggers a policy-only
   reprocess of the ingest decision. See I-5.
4. The privacy observation payload records scrub outcome, finding counts by
  detector, and assessment confidence as separate fields. The rubric that
   maps them to a tier is the policy, not the observer. This is the #325
   split.

Open question. Does the PII backstop (the remote prose-PII pass) run as a
second privacy observer inside the same ingest evaluation, or as a later
job? Today it is a later hold state. Recommended: a second observer role
`privacy_backstop` in a later job, with the ingest policy treating it as
`Unavailable` until it exists, and a reprocess when it lands.

Decision:

Recommendation. A second observer role.

### I-3. Decision structure: granular decisions or one policy

Status: agreed.

Source: remarks on section 5.3 and on R3.

What the remarks say. One action policy is a big change and may not be an
improvement. The current thinking is a small set of distinct decisions that
each need provenance:

- Ingest: `ingest | quarantine`, based on privacy.
- Review: `accept | reject`.
- Credit: `eligible(reason, amount) | withheld(reason)`.

What is at stake. The number of policy artifacts, fact types, and decision
records in the bundle. Also the audit question: can a reader see one
decision per disposition that matters, with its own inputs.

Analysis of the three-decision shape. It is close to right, and it maps to
real workflow boundaries. Two things are missing from it:

1. The novelty and substance outcome has no home. Today both gates must pass
  for `novelty_utility` credit and for index insertion. In the three-decision
   shape they can be inputs to Credit with a reason code. But index
   insertion also depends on them, and "review" as a commons outcome (a
   borderline trace) has no place.
2. "Review: accept | reject" is a human act today, not a policy. In the
  model the human produces an observation. The policy that reads it is the
   ingest decision run again. So Review is not a separate policy. It is a
   reprocess of Ingest with one more observation.

Options.

1. Three decisions as remarked: Ingest, Review, Credit. Novelty and substance
  are inputs to Credit. Index insertion is a side output of Credit or of
   Ingest. Weak point: index insertion has no decision of its own, and a
   human review is modelled as a policy.
2. Three decisions by workflow boundary: Ingest (privacy, at receipt),
  Commons (novelty and substance, in the job: `register | duplicate |  insufficient | review`, plus index actions), Credit (eligibility and
   amount, from Commons and state). Human review is an observation that
   re-runs Ingest. Each decision has its own policy version and its own
   provenance.
3. One action policy with four dispositions (the review's 5.3).

Decision. Use one primary action policy for each independently triggered
workflow. A workflow coordinates actors, observations, retries, decisions,
and effects. Its policy maps recorded inputs to a decision.

The five workflows are:

- Admission: evaluate privacy and accept, quarantine, or reject the trace.
- Review: record an authorized assessment and decide its system consequence.
- Commons qualification: evaluate novelty, substance, and duplication, then
request indexing when appropriate.
- Credit: determine credit eligibility, amount, and basis.
- Settlement: apply governance to eligible credit and request ledger effects.

Each decision records its policy version and exact inputs. A workflow can
consume observations and decisions from earlier workflows.

### I-4. Credit as a scorer plus a policy

Status: open.

Source: remarks on the credit scorecard ("just have a Scorer") and on R5
("issuing credit is a decision which should be the function of some policy
and some state").

What the remarks say. Credit is not fully implemented. Directionally, credit
is a decision from a policy and some state. Simplify.

What is at stake. Today there are three credit numbers from three places:
the pending estimate from `compute_value_scorecard` in the wire crate, the
fixed `novelty_utility` delta from an environment variable, and the shadow
multipliers on the decision row. None is versioned with the others.

Proposed shape.

- One observer with role `credit_score` produces a breakdown: quality,
novelty basis, duplicate penalty, coverage, and so on. It reads the trace
and the commons observations. It has a version.
- One credit policy reads that observation, the commons decision, and the
state it needs (consent, retention class, contributor cap epoch), and
returns `eligible { event_type, amount, basis } | withheld { reason }`.
- The `IssueCredit` effect carries the event type, the amount, the basis,
and the decision hash. The ledger path applies governance as today:
issuer allowlist, caps, holds, settlement. Governance is not an input to
the evaluation. See inconsistency 14.
- The pending estimate on the submit receipt is the credit observer's
number. It moves out of the wire crate.

Open question. Which "state" may the credit policy read? Recommended rule: only
state that is recorded as an observation with a source and a snapshot
reference, so two runs with the same inputs give the same decision.

Decision:

Agreed: only state that is recorded as an observation with a source and a snapshot reference, so two runs with the same inputs give the same decision.

### I-5. Policy-only reprocess and observation compatibility

Status: open.

Source: remark on R4 ("how do we guarantee that different policies will not
require different observations?").

What the remark says. A new policy may need observations the stored
evaluation does not have. The missing-fact model supports this, but in
practice reprocess is limited by which observations are reusable.

What is at stake. Whether "run the new policy over stored data with no GPU"
is a real path or a hope.

Proposed rule.

1. Each policy version declares its input manifest: the observation roles
  and schema versions it reads, and for each whether it is required or
   optional.
2. A policy-only reprocess is admissible when every required input exists in
  the base evaluation with a compatible schema. The job planner checks this
   before it creates the job.
3. When a required input is missing, the planner creates a partial reprocess:
  run only the observers whose roles are missing, reuse the rest, then run
   the policy. This is the checkpoint idea from R8 in another form.
4. When a required input exists but with an older schema, the schema
  registry says whether an upcast exists. No upcast means the observer
   runs again.
5. A policy run with an `Unavailable` required input returns a closed
  decision with the reason. It never returns a favorable default.

This makes the answer to the remark: the guarantee is a declared manifest
plus a planner that reads it. Reprocess cost is then known before the job
runs.

Decision:

Agreed

### I-6. Late binding and the migration plan

Status: open, revised for I-3.

Source: remarks on assignment binding time and on R7.

What the remarks say. Each workflow must bind its bundle when execution
starts. A later workflow must not inherit the bundle of an earlier workflow.
A migration plan must drain each workflow queue before its old worker retires.

What is at stake. Whether a rollback reaches queued work, whether workflows
can evolve independently, and whether migration can continue while admission
stays open.

Proposed rules.

1. Admission binds its active assignment when the trace arrives because it
  runs immediately.
2. Review binds its assignment when the authorized assessment is processed.
3. Commons Qualification and Credit bind their active assignments when their
  jobs are claimed.
4. Settlement binds its assignment before approval of the source list. The
  approval covers that policy version and source-list hash.
5. Each workflow run records its workflow kind, bundle id, deployment id,
  mode, and upstream decision references.
6. A queued workflow remains unbound until its binding point. A rollback can
  therefore reach work that has not started.
7. Each observer names its projection. The workflow projects from the stored
  trace revision when it runs.
8. Migration occurs per workflow. The old worker stops new claims at its
  cutover time, and the new worker claims unfinished work.
9. Old decisions remain immutable. An operator can request reprocessing under
  a new workflow bundle.

Open question. Does anything still need a projection at receipt? The
canonical summary hash for exact-duplicate detection does. Recommended:
treat that hash as an observation with its own role and version, computed at
receipt.

Decision:

I don't thinkk we need to add the trace as an observation

### I-7. Checkpoints and evolving evidence

Status: deferred, split into two sub-issues.

Source: remarks on recompute versus checkpoint ("can probably be added
later; the architecture must support it") and on R8 ("I suspect this is
about vector indexing").

Clarification. R8 and the vector index are two different things.

- I-7a. Checkpoint of an expensive observation. The substance observer calls
a remote 27B model once per chunk. A crash mid-trace repeats the calls. A
checkpoint stores a finished observation keyed by observer id, input hash,
and evidence id, so a retry reuses it. This is about cost, not about
vectors. Deferred is acceptable. The architecture supports it if
observations are individually addressable, which I-3 option 2 and I-5
give.
- I-7b. Evolving evidence. The novelty observer compares against an index
that changes. The observation must record what it compared against. The
proposal's answer is an index epoch for the reference index and a
generation watermark for the online index. That is the evidence
abstraction the remark points at. What remains undecided is who seals an
epoch, how often, and whether a trace inserted after epoch N is visible to
a trace evaluated under epoch N. Recommended: not visible; online dedup is
the only path that sees recent inserts; novelty is measured against sealed
epochs only.

Decision on I-7a:

Decision on I-7b:

### I-8. Vector namespaces and ranking features

Status: agreed, one question.

Source: remark on R6.

What the remark says. The epoch and online split makes sense. What are
ranking features? Storing nearest neighbours in the database is probably not
helpful.

Answers.

- Ranking features in the proposal are embedding vectors kept for a
downstream ranking model to read as inputs. They are not nearest
neighbours. They are out of scope for this design. Drop the namespace.
- Nearest neighbours are not stored in the database today. The decision row
stores `nearest_neighbor_hash`, a hash over the neighbour list, so a replay
can prove it saw the same list. The proposal keeps that as
`neighbor_evidence_hash` on the observation. That is correct and small.

Decision:

leave this out of scope for now

### I-9. Lab crate and crate architecture

Status: open.

Source: remarks on the cost of measurement ("handled by the lab crate; the
ingest server attributes the result to whatever training run the lab
produces") and on the enclave boundary ("we are missing details about the
crate architecture").

What the remarks say. Calibration, bake-off, and evaluation belong in a
separate `lab` crate. The ingest server consumes bundles and attributes each
result to the lab run that produced the bundle. The proposal must describe
the crate layout, where observers run, and the enclave seam.

Proposed shape.

- `trace-commons-gate-api`: the traits `Observer`, `Policy`, `Evidence`, the
observation and decision document types, and the schema registry. No
implementations.
- `trace-commons-gate-enclave`: observer implementations that run inside the
scoring boundary: substance, novelty embedding, dedup. Exposes one remote-
capable interface so Phase B can move it out of process.
- `trace-commons-lab`: calibration corpora, bake-off, operating-point
reports, golden traces, and bundle assembly. Produces a bundle manifest
with a lab run id and a report digest. Never runs in the ingest process.
- `trace-commons-server`: the job runner, the effect runner, the policies,
the registry tables, and the read models. Consumes bundles by id.

Bundle provenance. The bundle manifest carries `lab_run_id` and
`report_digest`. Every evaluation carries the bundle id. That is the
attribution the remark asks for. The ingest server does not judge whether a
bundle is fit for production. The operator's assignment does.

Decision:

Good enough

### I-10. Cross-tenant job claim under forced RLS

Status: open, needs an explanation first.

Source: remark on R10 ("not sure I understand this").

Explanation. Every tenant table in PostgreSQL has forced row-level security.
A connection sees only the rows of the tenant set in its session. The job
queue is a tenant table. A worker that claims "the next job from any tenant"
needs to read and update rows across all tenants in one statement. The
runtime role cannot do that. Today the gate driver uses a separate role,
`trace_gate_driver`, that has a `SELECT` policy with `USING (true)` and no
`INSERT` or `UPDATE`. It enumerates work across tenants, then switches to a
tenant-scoped connection to do the work. The proposal's claim statement is a
cross-tenant `UPDATE`, which no existing role can run.

Options.

1. A `trace_job_claimer` role with `SELECT` and `UPDATE` policies limited to
  `processing_jobs` and to the lease columns. The worker claims with that
   role, then does all other work with a tenant-scoped connection.
2. Per-tenant claiming: the worker iterates tenants and claims within each
  tenant's session. Simpler roles, more round trips, and starvation risk
   for small tenants.
3. A queue table outside the tenant schema, holding only job ids and tenant
  ids, with no RLS. The evaluation tables stay tenant-scoped.

Review position. Option 1. It matches the existing pattern and keeps one
queue. The policy must bound the columns the role can write.

Decision:

Yup that makes sense

### I-11. Review is an effect

Status: agreed.

Source: remark on inconsistency 4 ("yes review should be an effect").

Change to the proposal. Add `ScheduleReview` to the effect kinds. The ingest
decision requests it when the disposition is `quarantine`. The review
workflow produces a `review` observation when a human decides. See I-2 and
I-3.

Decision:

### I-12. Small consistency fixes

Status: agreed.

Source: decisions on inconsistencies 1 and 2, and R11.

Checklist for the proposal text.

- One bundle name throughout.
- Define or remove `executor_bundle_id` on receipts.
- Section 3 lists four immutable concepts, not five. Effect is an intent and
a receipt, as in section 5.
- Remove the author remark on `output_schema`. Per-observation `schema`
carries it.
- Make the two provenance paths equal, including the index epoch.
- Reconcile the hash policy with the HTML version. Three hashes.
- Move `schema` from the table columns to each observation and decision.
- Remove `required` from the effect outbox, or define it.
- Define shadow completion: a shadow job completes at commit.
- Add `based_on_evaluation_id` to jobs and evaluations.
- Remove `deployment_id` from the job unique key; keep it as an attribute.
- State the scope boundary with the lab crate and link to it.

Decision:

### I-13. Contributor-visible projection

Status: agreed.

Source: remark on R9.

Change to the proposal. Add a read-model table that maps job state and effect
state to the submission status and an explanation label. A pending job shows
as pending. A terminal failure shows as not credited with a stable reason
label. The ingest decision sets `accepted` or `quarantined` at receipt, as
today.

Decision: