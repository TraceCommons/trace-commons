# Receipt and pipeline induction obligations

This proof work imports the selected models without changing any original
initializer, transition, identity domain, guard or observer. The proof-only
`server_receipt_induction.qnt` and `server_pipeline_induction.qnt` each expose
`originalInit`, `originalStep`, `Inductive`, `arbitraryInit` and `Conclusion`.
Results are recorded only after the individual solver obligations terminate;
a successful initialization is not a successful induction proof.

## Logical lifting

For each model, let I be `Inductive` and P be `Conclusion`:

1. Check `originalInit => I` (`--init originalInit --max-steps 0`).
2. Check `I(s) /\ originalStep(s,s') => I(s')` by choosing arbitrary states
   satisfying I and checking every original step (`--init arbitraryInit
   --step originalStep --max-steps 1`).
3. Check `I => P`; the `Inductive` definition contains the exact selected
   predicates as conjuncts, and the arbitrary-state zero-step implication
   check verifies the conclusion on that same domain.

These obligations imply P for every finite-length trace of the original
model. One symbolic step from **every** I-state is an induction obligation,
not a one-step bounded search from the original initial state. No fairness or
liveness claim follows. No exploration constraint is put on the original model.

Every original map is constructed with its original exact key set and an
independent arbitrary value per key. All set fields choose their complete
original finite powersets. `apalache::generate(1)` is used only for primitive
records/scalars with no nested collections; thus its collection capacity does
not limit an original map or set. Unrestricted integer/string fields range
over a superset of reachable scalar values. The invariant explicitly checks
collection key domains and the numeric domains needed by indexed lookups.
Consequently every reachable state satisfying I is represented; nothing is
silently excluded by a reduced arbitrary-state arena. The original-init
obligation supplies a nonempty witness for the assumed induction domain.

Receipt strengthening includes immutable request identity domains, saved
submission IDs in 0..1, exact unused-request initialization, and successful
precheck evidence throughout preparation/acceptance phases. Pipeline
strengthening includes the original map domains, tenant/bundle/token/attempt
ranges, exact absent-run initialization, and at least one charged attempt for
leased runs. These are hypotheses to be proved initially and preserved by
every original action, rather than new source guards. The original proof flags
are also conjuncts of I and are assigned only by the original observers.

The selected conclusion is P5/P8/P11 for receipts and P6/P7/P8 for processing.
Existing source-boundary limits remain: structured saved-result selection is
modeled, HTTP serialization is not; cryptographic validation and the composed
receipt-to-run interface are external correspondence assumptions.

## Exploratory attempts

The first whole-State generated receipt check found a missing domain clause:
a symbolic saved receipt could have an out-of-domain submission ID, then the
original retry copied it into the request. The final harness includes the
saved-ID domain and proves its preservation. This was a strengthening
counterexample, not a reachable production counterexample. A subsequent
whole-State attempt shared endpoint 8852 with another agent and was stopped;
its interleaved/unfinished log is not proof evidence. Final isolated runs use
8858 and exact-key maps instead of allocating unused symbolic map slots.

## Reproduction

With Java 21+ on PATH and Quint 0.33.0 / Apalache 0.62.1, run these four
commands from the repository root. Substitute `server_pipeline_induction.qnt`
for `server_receipt_induction.qnt` to check the other model. Use an isolated
Apalache endpoint; do not run concurrent clients on the same endpoint.

```sh
quint verify docs/flows/trace-commons/proofs/server_receipt_induction.qnt --init originalInit --step originalStep --invariant Inductive --max-steps 0 --server-endpoint localhost:8858 --verbosity 2
quint verify docs/flows/trace-commons/proofs/server_receipt_induction.qnt --init arbitraryInit --step originalStep --invariant Inductive --max-steps 1 --server-endpoint localhost:8858 --verbosity 2
quint verify docs/flows/trace-commons/proofs/server_receipt_induction.qnt --init arbitraryInit --step originalStep --invariant Conclusion --max-steps 0 --server-endpoint localhost:8858 --verbosity 2
quint verify docs/flows/trace-commons/proofs/server_receipt_induction.qnt --init arbitraryInit --step originalStep --invariant false --max-steps 0 --server-endpoint localhost:8858 --verbosity 2
```

The first three must terminate with `NoError`, `[ok] No violation found` and
exit zero, without unsupported-expression errors. The last must return an
actual counterexample and exit one: falsifying `false` demonstrates a nonempty
arbitrary-state domain, not a product defect. Record original-model and harness
hashes before/after every phase; a timeout, tool error, missing terminal verdict,
or changed artifact does not yield a complete certificate.

## Completed obligations

Both certificates completed on 2026-11-06 with Quint 0.33.0, Apalache 0.62.1
and Homebrew OpenJDK 21.0.13.0.3. Initiation, consecution and implication
terminated with `NoError`, `[ok] No violation found`, exit zero and no solver
error. Nonvacuity produced the required counterexample to `false`, exit one.
The strict shared certificate classifier accepts both, with identical original
model/harness hashes across all four phases. An independent reviewer checked
domain coverage, original-step linkage, current hashes and raw terminal outputs.

| Model | Selected predicates | Original transitions | Consecution time |
| --- | --- | ---: | ---: |
| server_receipt | P5, P8, P11 | 22 | 387.430 s |
| server_pipeline | P6, P7, P8 | 34 | 103.644 s |

The certificates establish safety in these source-corresponded abstractions
for arbitrary trace lengths through induction. They do not prove HTTP
serialization, production cryptography, middleware/RLS internals, cross-model
composition, liveness, or deployment state. Full commands, per-phase logs and
verdicts belong to the accompanying evidence manifest. Input snapshots:

- `server_receipt.qnt`: `0bceeb8153c5347626dc3676da4cdebb4942fc6301fe9c09a796f76a719cf69e`;
  `server_receipt_induction.qnt`: `96ffd090665627a552d2b29900e33ba0ed9e56527859376fb2b3c8b4ef361c3c`.
- `server_pipeline.qnt`: `b637efc035cf624c3c73f5bf574a860935e0386ffd0317369a5b89d5c669117f`;
  `server_pipeline_induction.qnt`: `e1c4fb0435a272007a69e042b83446b1a7dbcc36b5da2543f024650e0f6b04df`.
