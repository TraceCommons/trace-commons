# Background witness worker model

This companion models the queue/worker policy at origin/main
`f785452ae54d2732ac623f4f9be65c4984b9fc74` (#2139). The Rust files it projects
are unchanged since; the evidence was recorded on branch
`witness-model-retry-order`, which changes only the model. Earlier evidence
used baselines `fb16129e4` (#2138) and `7be2c37ba` (#2139). It supplements the
[two-trace contribution lifecycle](../models/client_contribution.md); it does
not change that model, its selected P2/P3/P4/P12 properties, or their existing
induction certificates. There is no composed refinement proof between the two
models or automatic equivalence proof to Rust.

## State and correspondence

Four identified traces are sufficient to distinguish three failures from a
fourth healthy trace. Initialization starts after legitimate approval of four
automatic traces; discovery, enrollment and grant issuance belong to the
existing lifecycle projection. Each durable entry has approval terms, manual/automatic
origin, a manual-review hold, attempts, a saturated transient-failure count, an effective retry
marker/deadline, and a certified pin. Queued snapshots and in-flight requests
are separate maps; an in-flight request records the time it was dispatched. Dispatch and actual request start are separate transitions,
so cancellation, pause and reapproval can intervene. Completion must match the
current terms before it can pin a review. Upload is a later, separately gated
transition.

| Model behavior | Rust anchor |
| --- | --- |
| Latest queued snapshot; in-flight dedup; full-queue displacement | `daemon/witness_worker.rs:246-285` |
| Probe selection and available slots; dispatch time recorded on the job | `daemon/witness_worker.rs:186-190,344-378` |
| Manual/fresh/retry ordering; a retry is an attempted entry holding a deadline | `daemon/mod.rs:2589-2625` |
| Six transient labels share a per-entry retry path | `submit.rs:684-699`, `daemon/mod.rs:2099-2134` |
| Jittered nominal 60/120/300/900/3600-second delays | `daemon/witness_retry.rs:16-25` |
| Failure run reset by a quiet gap (latest failure end to next failing dispatch); shared pause; probe ownership | `daemon/witness_retry.rs:7-14,37-112` |
| `witness_redaction_failed` retries per trace but ends the job as `Done` for the circuit | `daemon/witness_retry.rs:27-35`, `daemon/witness_worker.rs:404-408,734-738,776-781` |
| Successful completion and saturation overlap | `daemon/witness_worker.rs:390-427` |
| Saturation drains hints, retains approvals and spreads due times | `daemon/witness_worker.rs:754-780`, `daemon/mod.rs:2148-2165,2280-2291` |
| Recheck captured approval; persist before pinning | `daemon/witness_worker.rs:539-567,688-730` |
| Readiness depends on the retry reason, not just a timestamp | `daemon/queue.rs:808-833,2722-2725` |

Anchors are relative to `crates/trace-commons-contributor/src/` at commit
`f785452ae`. The validation report records hashes of these source files as well
as the Quint model and tests.

`checked` is an observation-only record. It records illegal dispatch order or
capacity, requests under invalid approvals/holds, uncertified pinning, upload
without a matching pin, lost retry approval, automatic approval through a
manual-review hold, queue-wide effects of isolated
failure, late circuit mutations, early capacity reopening, and stale queued
snapshots. It does not restrict `step`. `safety` additionally checks queue and
in-flight bounds, disjoint queue/running IDs, valid probe ownership, the shared
pause ladder, and pin/approval identity.

## Explicit abstractions and omissions

- The waiting queue has **three** slots instead of production's 64. With four
  IDs, this exercises full-queue displacement. No result establishes every
  possible 64-entry scheduling history.
- There are two background slots. The independent interactive review lane,
  its UI, and concurrent interactive pin replacement are outside this model.
- Source age/known-age/UUID ordering is represented by a fixed ID order;
  manual ties retain queued insertion order. The producer's ordered durable
  scan is represented by independently eligible enqueue actions.
- Approval terms are represented by a revision, manual flag and approval
  timestamp. A changed revision retracts approval atomically; filesystem
  discovery, superseding allocation, individual scope grants and terms-sweep
  lock ordering remain in the lifecycle projection or outside both models.
- A review success is an external **verified and persistable certificate**
  outcome. Successful pinning clears the witness retry marker/deadline and
  failure run while retaining attempts. Cryptography, quote/collateral verification, classifier correctness,
  raw bytes, filesystem transactions and claim minting are not proved here.
  `drop` includes discarded results, interrupted jobs and persistence failures.
- Failure counts saturate at five, because later counts select the same
  one-hour nominal delay. Attempts retain their distinction from failures and
  are not reset by reapproval. Transient jitter ranges over every integer in
  the half-to-full interval. Capacity jitter uses the real ID-modulo formula
  over the four modeled integer IDs.
- Random/full-step symbolic exploration chooses capacity hints from
  `{1,45,120,300,3600}` and positive time increments from
  `{1,29,30,59,60,120,240,300,301,3600}` seconds. Deterministic tests also use
  other explicit positive increments. System-clock rollback is not modeled.
- Enrollment is the current send/record authority input, not a full account
  identity, unenroll transaction or key-erasure model. Pause after a request
  allows pinning, while uploads remain held. Restart clears volatile worker
  state and preserves durable entries/deadlines/pins.
- Trusted-routing/accepted-measurement configuration stays fixed. Signed-list
  updates that release measurement holds are not modeled; under the modeled
  configuration a trust hold requires manual reapproval.
- Poisoned locks, panic backoff, interactive slots and operating-system
  execution failures are not represented as separate transitions.
- The six transient labels are represented by two transitions: `transient`
  (the five outage labels, which feed the circuit) and `redactionFailed`
  (`witness_redaction_failed`, which takes the same per-trace retry but
  releases a probe slot like a cancellation and records nothing in the
  failure run). Both close the second slot.

Two limitations are modeled deliberately rather than silently repaired:

1. **Approval equality is not a generation token.** Cancel then automatic
   reapproval with identical terms can recreate the same `JobKey`; the old
   matching result may pin. Manual reapproval changes origin/timestamp and
   rejects an old automatic result. The tests assert both behaviors.
2. **The existing outer watchdog can write an ineffective deadline.** On a
   fresh trace it sets `retry_after` without a retry reason, so readiness still
   returns true. On a trace already marked for retry the new deadline applies.
   The model records both. This is an executable counterexample to claiming
   every timestamp always paces a trace, not a repair or endorsement of the bug.
   The 15-minute watchdog also cannot preempt synchronous setup/load work until
   it yields; no absolute wall-clock job bound is claimed.

Late non-probe circuit completion guards are defensive in the source. With
normal worker transitions, the first transient failure closes the second slot;
by the third distinct failure no older job remains in flight. Late completion
across a **saturation** hold is reachable and explicitly tested both before and
after expiry. The model keeps the circuit's ownership guards without inventing
an otherwise unreachable background interleaving to demonstrate them.

## Checked behavior and progress limits

The regression suite includes failure-run gap boundaries (a quiet gap of
exactly 300 seconds keeps a run, 301 seconds starts a new one, with 100-second
requests so end-to-end distances exceed 300 seconds), back-to-back 570-second
timeouts opening the circuit, a quiet gap after long timeouts starting a new
run, a success ending a run, redaction failures of three distinct traces not
opening the circuit and neither counting toward nor breaking an outage run,
a redaction-failed probe releasing its slot without extending the pause, a
repeated single failure, the three-ID threshold, single/different recovery probes, capped
shared pauses, retained approvals, late capacity completions, manual reapproval,
queue displacement, fresh/retry order, restart, pause, enrollment and permanent
refusals. Six test-only mutations reproduce the old global pause, late-success
capacity reopening, stale queued reapproval, priority bypass, automatic trust-hold bypass and uncertified
pinning. Each must make `safety` false. These mutations are excluded from the
production `step` and the sampled/symbolic checks.

The induction theorem is for the stated `safety` predicate. The three-distinct
threshold, the failure-run rule (a run resets only when the failing request was
dispatched more than 300 seconds after the latest recorded failure ended), the
exclusion of redaction failures from the circuit and the exact backoff
progression have explicit regression coverage; they are not separate conjuncts of that theorem.

**No unconditional fairness or eventual-upload theorem is claimed.** Infinite
manual arrivals can starve automatic work; infinite fresh work can starve
retries. Time/pump execution, finite overtaking work, continued consent and
an eventually successful review are necessary environmental assumptions for
any stronger progress theorem. Those temporal assumptions are not encoded or
verified by this update. Priority is checked as a dispatch safety property.

## Reproduce validation

Installed tools: Quint 0.33.0, Apalache 0.62.1 and Java 21. No package changes
are required. Use a fresh output directory outside the checkout:

```sh
python3 docs/flows/trace-commons/worker/check.py \
  --out /private/tmp/tc-witness-worker-validation --samples 1000 --steps 100 \
  --seconds 240
python3 docs/flows/trace-commons/worker/induction.py \
  --out /private/tmp/tc-witness-worker-induction --seconds 240 --port 8899
python3 -m unittest discover -s docs/flows/trace-commons/worker -p 'test_*.py'
python3 -m unittest discover -s docs/flows/trace-commons/scripts -p 'test_*.py'
```

The first runner typechecks, runs deterministic positive/negative tests and
samples the complete `step`. Optional `--bound N` adds a bounded symbolic
search; the default skips it in favor of the full-step induction command.
It records tool version, exact commands, source/model/test hashes, exit status,
resource timeout, log hashes and explicit verdicts. A timeout, source mutation,
frontend error or missing terminal solver verdict is **incomplete**, never a
proof. A clean bounded result means no counterexample through that bound;
sampling and regression tests remain distinct evidence.

## Full-step safety induction

`witness_worker_induction.qnt` imports the unchanged original `init` and the
complete original `step`. Its generated arbitrary initializer independently
covers every field of both `State` and the observation record `Checks`: 159
scalar leaves (including each job's `dispatched` time) and four exact four-key maps. Integers and strings are not
clipped to a finite numeric/string list. The domain requires the original map
keys; that domain's closure is part of the proof, not an exploration filter.

The strengthening adds four reachable facts to `safety`: pinned entries are
approved/uploaded, queued jobs have not started, absent running jobs have not
started, and an open recovery circuit has delay at least 30. Initialization
establishes these facts and the full original step preserves them. The final
implication checks the original model's `safety`, and the expected
counterexample to `false` checks that the arbitrary initializer admits a
state. No production transition is removed or replaced.

All four obligations completed successfully for the current model; the full
step has 25 extracted transitions. This is safety for arbitrary-length
executions of the stated four-identity, queue-bound-three model; it remains
separate from fairness, production-size generalization and source equivalence.
See [retained validation evidence](evidence/README.md) for hashes, commands,
terminal results and the earlier bounded searches that timed out. Those
bounded searches ran on a pre-review model and do not cover this one.
