# Skill installation confirmation: P13

The owner selected this existing promise on 2026-11-06:

> A tested skill is installed only after I confirm the matching installation plan.

The candidate also excludes presenting daemon success as a validated installation
without independent status reconciliation. This change selects the promise in
`client_learning.cfg` and observes the existing transitions. It changes no
production behavior, transition guard, fairness assumption or exploration bound.

## Current source correspondence

The reviewed baseline was `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`;
these skill installation source seams were unchanged at current main
`a2e0aa566` when inspected.

- `macos/Sources/TraceCommonsApp/SkillLearningStore.swift:286-336` admits planning
  only after successful evaluation, binds the returned plan to the evaluation and
  skill digest, and requires a held installable plan before beginning install.
- `macos/Sources/TraceCommonsApp/AppModel+SkillLearning.swift:193-260` submits
  the confirmed plan ID and previewed skill/marker digests. It then reads install
  status independently; daemon commit success alone does not finish installation.
  `SkillLearningStore.swift:338-376` validates the returned installed lineage.
- `crates/trace-commons-contributor/src/daemon/skill_loop/handlers.rs:510-599`
  binds the plan to the owned passing evaluation and reviewed package.
  `:602-687` checks the owner lease, consumes the matching plan, commits the
  previewed digests, and rechecks the owner after publication, compensating an
  owner change when possible. `:690-735` reads authenticated owned install status.
- `crates/trace-commons-contributor/src/daemon/skill_loop.rs:464` consumes the
  exact held plan and rejects expiry at admission.
- `crates/trace-commons-contributor/src/skill_loop/install.rs:295-370` builds the
  plan from the reviewed bytes, evaluation, owner scope and signed marker.
  `:397-488` rejects changed preview/review/marker digests, checks actual bytes
  and authenticated marker lineage, and publishes exclusively.

## Observer and original-behavior projection

`YouInstallTheReviewedSkill` captures the confirmed plan identity, package,
marker, account, owner generation and tested lineage. At
`TheFilesystemPublishesTheSkill`, `installViolation` becomes permanently true
if those observations do not match the held plan and current request lineage.
This historical check avoids judging a past valid publication against a later
account change, package edit or rollback. It requires confirmation of the
matching plan, rather than merely some earlier confirmation.

`reconciledPublication` becomes true only in an independent successful status
read. It resets on new filesystem publication or removal. `P13` also requires
this observation whenever the model claims installed status. An external edit
or account change does not rewrite a prior status observation; this is a claim
about the last validated observation, not continuous filesystem surveillance.

All observers live inside the existing state record and skill records; original
actions still assign the complete state. Dropping these observer fields produces
the original transitions and control states. The planning action additionally
chooses one of three symbolic plan names distinct from the held and last
confirmed names. At most two names are excluded, so an original transition is
never disabled. Names represent equality of retained plan IDs, not real UUID
values. No original action has a stronger guard and no original action was removed.

## Bounds and limits

The model retains two submission records, two owners with one owner-generation
change, two package revisions, one held evaluation and one in-flight request
index, and two marker identities. Three reusable plan names distinguish the
retained plan and last confirmation. These are finite reference projections,
not a proof over unbounded concurrent requests, real hashes, signatures, paths,
filesystem atomicity or the operating system. Recovery begins with no externally
installed skill and can recover only publication modeled in this run; importing
an arbitrary pre-existing marker is outside the initial-state bound.

The inherited model checks `planLive` at its abstract publication boundary;
production consumes and validates TTL at commit admission. Admission and
publication are coarsened together here, so this is not a wall-clock scheduling
proof. The model preserves that existing abstraction rather than adding a
stricter TTL policy. Changed-file refusal and owner-change compensation remain
separate existing actions. No liveness, deadlock, model-quality or public run
publication promise is selected by P13.

## Focused validation

`quint typecheck docs/flows/trace-commons/models/client_learning.qnt` passed.
`quint test docs/flows/trace-commons/proofs/client_learning_p13_tests.qnt
--backend typescript --seed 20261106 --max-samples 5` passed five runs with
five samples each: a valid confirmed publication; later account and external
edits preserving history; and two test-only faults detecting a missing
confirmation or a success claim without independent status reconciliation.
The fault actions exist only in the test module and are not reachable product
transitions or production counterexamples.

The selected invariant's formal results belong to the shared proof report. These
focused tests establish observer discrimination, not exhaustive state checking
or a production correctness proof. No production counterexample was established
by this source review and these focused tests.

## Inductive proof harness

`client_learning_p13_induction.qnt` imports the original model and its full
`step`; it does not reimplement or select a subset of transitions. Its symbolic
initializer enumerates every scalar field independently, both complete skill
records, and exactly the original map keys `{0, 1}`. Text domains include every
literal in the original model (including extra values for fields where they are
unreachable); integer domains preserve every original identity and sentinel.
The domain is included in the strengthened invariant and checked for closure.
The initializer's only additional predicate is the proposed strengthening.

The strengthening records the existing review/evaluation/plan lineage, current
row owner, past-or-current request lease, confirmation lineage of any live
install, and independently reconciled installed status. The account/generation
and request lease relations rule out arbitrary future leases, rather than
removing any original account transition. It also includes the P13 historical
violation flag. These are proof obligations, not additional product guards.

The certificate requires four queries on the same model and harness hashes:
original `init` implies `strengthened`; every original `step` from arbitrary
`strengthened` state preserves `strengthened` (including the domain); every such
state implies `P13`; and an invariant `false` check finds a state from the
symbolic initializer. The last query rules out an empty initializer. A one-step
check here is the complete consecution obligation over all symbolic predecessor
states, not a depth-one reachability claim. Together with initiation and
implication it supports induction over all paths of the finite reference model.

`client_learning_p13_negative.qnt` adds only a test fault that erases the
confirmation snapshot before the original publication action. It is separate
from the product model and positive proof harness. A detected violation there
is an expected negative control, not a production counterexample.

## Completed obligations

On 2026-11-06, all four certificate phases completed on model SHA-256
`dadc2e60fca0973717bf66e9c5e596b3e1912573febf4965b6eb7774f792b2ad`
and harness SHA-256
`a10417d273b6b2969b8b788792a518601712670c939f2cf4cf74b4d78ffd9d43`.
Initiation, consecution and implication ended with exit zero, `NoError` and
`[ok]`; nonvacuity produced the expected false-invariant counterexample.
Consecution checked all 53 original transitions and 72 verification conditions,
including domain closure. The shared classifier reports this four-phase
certificate as proved. This establishes P13 over all reachable states of the
finite reference model, subject to the source projection and bounds above.

With `H` denoting `docs/flows/trace-commons/proofs/client_learning_p13_induction.qnt`,
the exact logical queries were:

```sh
quint verify "$H" --server-endpoint localhost:8854 --init init --invariant strengthened --max-steps 0
quint verify "$H" --server-endpoint localhost:8854 --init arbitrary --invariant strengthened --max-steps 1
quint verify "$H" --server-endpoint localhost:8854 --init arbitrary --invariant P13 --max-steps 0
quint verify "$H" --server-endpoint localhost:8854 --init arbitrary --invariant false --max-steps 0
```

They ran from separate temporary working directories with Java 21. Exact
commands, logs and artifact hashes are retained in the shared proof evidence.
The earlier bare-induction assignment error and depth-ten timeout are
superseded by this complete induction certificate; they were never passing
results or production counterexamples.

The separate formal negative control completed with the expected two-step
counterexample: confirmation erasure followed by the original publication action
sets `installViolation`. Its command used `client_learning_p13_negative.qnt`
with `--init arbitrary --step negativeStep --invariant P13 --max-steps 2`.
The negative module SHA-256 is
`29a68215e17661d48d892c009264547584349bb015c654086f4f2591673ebedf`.
The positive induction harness and model hashes remained unchanged.
