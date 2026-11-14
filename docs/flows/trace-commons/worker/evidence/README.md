# Witness worker validation results

Recorded for the source projection at Rust baseline `fb16129e4`. The exact
model, harness, checker and Rust source hashes are in [manifest.json](manifest.json).

| Check | Result |
| --- | --- |
| Model and test typechecking | Passed with Quint 0.33.0 |
| Deterministic model regressions | 35 passed, including six negative controls |
| Full-step sampled safety | No violation in 1,000 traces of up to 100 steps; seed 20261114 |
| Evidence-classifier tests | 6 passed |
| Existing model-tool tests | 13 passed |
| Pre-review eight-step bounded search, 240-second budget | Incomplete: timed out during state 6; no proof result |
| Pre-review five-step bounded search, 240-second budget | Incomplete: timed out during state 5; no proof result |
| Induction initiation | Proved |
| Complete original-step preservation | Proved |
| Strengthened invariant implies original safety | Proved |
| Arbitrary initializer checked against false | Counterexample, as required for nonvacuity |

The [induction certificate](induction/certificate.json) establishes safety for
arbitrary-length executions of this four-identity, three-entry-queue Quint
model. Its [domain coverage](induction/domain-coverage.json) accounts for all
151 scalar leaves and four exact-key maps. No numeric clipping, exploration
constraint or subset of the model's transitions establishes the result. The
full-step preservation log records 23 extracted transitions. This is not a
production-size, liveness, cryptographic or Rust refinement theorem.

The [final validation report](validation/report.json) records the separate
regression/sampling commands. Earlier bounded attempts are retained under
`bounded-8-incomplete/` and `bounded-5-incomplete/`; their failures to complete
are not promoted to proofs by successful later checks. Those searches used the
earlier pre-review model hash and are not certificates for the final model.

Each raw `check.log` is stored as deterministic gzip beside its result. The
JSON retains the original command, local execution path and SHA256 of the
**uncompressed** log. Decompress `check.log.gz` to verify that hash. Solver
intermediate SMT files are not part of this compact evidence bundle. The
original work directories were `/private/tmp/tc-worker-induction-reviewed`,
`/private/tmp/tc-witness-quint-reviewed`, `/private/tmp/tc-witness-quint-first` and
`/private/tmp/tc-witness-quint-bound5`.

The existing watchdog and automatic-approval identity behaviors are tested
as source facts. Their presence does not contradict the stated safety
properties; stronger unconditional deadline or generation-isolation claims
would be false. See [model limits](../README.md).
