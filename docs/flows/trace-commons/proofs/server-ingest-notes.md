# Receipt and leased processing proof notes

The owner selected all thirteen candidate promises. These two models encode the
following safety projections; this note records encoding and typechecking,
not a completed TLC result. The runner's evidence report owns check outcomes.

| Model | Configured invariant | Observable safety projection |
| --- | --- | --- |
| `server_receipt` | P5 | Committed receipts are immutable. A completed-admission retry returns the expected replay/conflict/owner-refusal result; authorized same-body replay selects the exact saved structured receipt, while differing content is refused. Replay preserves receipts, owners and quota. |
| `server_receipt` | P8 | A successful precheck had authority/privacy controls at that boundary; new durable receipt acceptance has that request's captured successful validation. |
| `server_receipt` | P11 | Existing authenticated tenant/principal/role context stays fixed; each durable receipt write belongs to the authenticated tenant/key of the committing request. |
| `server_pipeline` | P6 | Once the receipt creates a run, its accepting bundle never changes. |
| `server_pipeline` | P7 | New privacy, revision, phase, award or settlement progress requires a current worker claim; changed human assessment requires a matching live human claim. |
| `server_pipeline` | P8 | Privacy commits have the privacy control and operability; favorable review commits have recorded privacy and required review evidence plus the bound runnable policy; score and settlement commits use their bound runnable policy. |

## Observation and time boundaries

Every original action computes the same `nextState`, then independently assigns
`s` and `proof`. Each proof field is a monotonic conjunction of transition
observations. Failing observations remain reachable and latch false; they are
never guards, constraints, or assumptions used to remove a behavior. `vars`
includes both variables and all named actions assign both, including direct
operator interventions. Request `validatedControls` is observation-only evidence
set at successful precheck, and never consulted by an action guard.

P5 now captures the actual completed-retry result and selected structured saved
receipt, and checks both directions against the authenticated retry input and
pre-existing durable record. This strengthens the original result-implies-input
monitor: arbitrary refusal or silent content conflict cannot pass. The added
request result/returned-receipt fields are passive observations and never guards.
It does not model serialization or prove exact HTTP response bytes.
Ordinary routing, source-session, quota, staging and owner refusals retain their
source ordering; the promise does not imply every in-flight matching request
must complete or that a final-transaction source withdrawal must be ignored.
Same-key/different-body creation cannot overwrite an existing receipt. HTTP
retries, lost replies and failures remain separate from durable commit.

Authority/privacy controls can change after receipt precheck or a privacy pass.
The properties deliberately inspect the decision boundary or previously recorded
evidence, never demand a current world flag for a historical acceptance. Existing
account-admitted submissions do not acquire an inference-receipt requirement.
P7 permits stale-worker expiry bookkeeping and attempt refunds; it fences new
phase decisions, not every possible write. Human rejection is fenced too.
P6 checks preservation of the receipt-input bundle; this separate model does not
prove correspondence with another model's concrete receipt row.

## Source correspondence and limits

The source baseline is main `a2e0aa566`. Comparing the seed's source baseline
`8218cc8` to that main commit found no changes in the selected
`versioned_pipeline.rs`, `versioned_pipeline_authority.rs`,
`versioned_pipeline_activation.rs`, `versioned_pipeline_bundle.rs`,
`payout_pass.rs`, or `payout_worker.rs` seams. Relevant server changes concerned
witness routes/metrics and witness-blocked registry/migration work, not these
transaction or lease boundaries. Existing model notes retain the detailed Rust
anchors; large ingest-file line numbers may have shifted.

The receipt model begins after HTTP authentication and upload-admission
authorization. P11 establishes modeled tenant-key isolation, not middleware,
PostgreSQL/RLS, signature, encryption, storage-key derivation, or arbitrary
principal correctness. Privacy/provider verification is an explicit external
predicate; these invariants do not prove cryptography or execute Rust.

Domains and guards remain the seed's four request attempts, two tenants,
three keys, two runs, two worker/human slots, sixteen worker token identities,
eight artifact slots and reduced attempt cap. No exploration constraints,
fairness, or reduced identities were introduced to obtain a result. Full finite
state enumeration can still be large. A timeout, sampled trace or bounded search
must not be reported as an exhaustive proof.

## Local checks

Both models pass `quint typecheck`; the existing `validate_frames` control
projection checks pass for all five receipt and nine pipeline frames. No TLC
run was performed by the model author. Check outcomes and limits must come from
the runner's recorded evidence.

P5 focused observer tests in `server_receipt_p5_tests.qnt` pass five deterministic
runs: valid replay, legitimate content conflict, incorrect refusal, incorrect
saved result, and silent content conflict. The latter three are test-only faults
expected to fail P5; no fault action is a reachable product transition. These
tests demonstrate monitor discrimination, not exhaustive proof.
