# Comparison estimator experiments

This directory and the crate-private `comparison_estimator` test module record candidate methods and reproducible counterexamples. The module is compiled only under `cfg(test)`. It does not supply a saved-specification estimator, change the service/FFI, or enable model recommendations.

Run the focused experiments from the workspace root:

```sh
RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor --lib insights::comparison_estimator
```

The versioned percentile-bootstrap artifact pins an exploratory failure: 60 of 72 null contrast intervals covered their targets. Sparse/homogeneous suppression prevented observed-difference results in that small grid, but does not repair raw interval coverage. This candidate is explicitly unqualified.

The subsequent Wilson component test uses six approximate component intervals and subtracts endpoints to form three outcome contrasts. It observed joint null coverage in 360 of 360 experiments over nine settings. Those settings share neither the same sampling distribution nor a prospective qualification design. The pooled uncertainty calculation is exploratory: it does not prove worst-case coverage, exact finite-sample control, power, usefulness, or model superiority. Its variable named `promoted` counts strict interval exclusions, not the complete product decision rule.

The `exact-component-candidate-v1.json` checkpoint freezes a subsequent exact-binomial candidate and its qualification protocol. Six component intervals allocate 12 one-sided tails at 1/240 each; exact integer tail inversion rounds bounds outward to millionths. Independent small-sample arithmetic oracles and endpoint/maximum-size tests cover implementation behavior. The full frozen simulation grid has not completed, and these helpers are not connected to saved specifications or the earlier bootstrap candidate.

Before admission, freeze a versioned candidate/rule and scenario packet, use canonical arithmetic with verified outward rounding, report per-scenario null and non-null coverage, boundary and imbalance cases, interval widths and suppression, and qualify the complete decision rule on independent simulation evidence. A separate reviewed artifact must justify any supported cohort/sampling assumptions and missing-outcome limitations. Disjoint deterministic seeds alone do not establish a prospectively frozen experiment.

The exact task-count ratios and canonical assessed-task seed inputs are reusable implementation groundwork. Empty cohort estimands remain unavailable; pending, unknown, unassessed, excluded, or audit-only facts must not change the assessed-task estimation seed. User reward systems are outside this work.

## Runtime observations

`runtime-observations/apple-m4-max-2026-09-12/` is an immutable capture and is
left byte-for-byte as measured, which is why this caveat is recorded here
rather than inside it.

`measure.sh` produces only the per-run `.json`, `.stdout`, `.stderr`, and
`.time` files, plus a `BINARY_SHA256SUMS` that is not part of the capture. It
does not produce `summary.json` and it does not produce `SHA256SUMS`. Both were
written by a separate aggregation step that was not committed, so re-running
the script reproduces the raw observations but not the aggregate file or the
manifest.

The aggregate was checked against the raw files rather than trusted: for all
eight build/composition pairs, `summary.json`'s `candidate_elapsed_nanos`
values, median, minimum, and maximum are exactly what the three measured runs
contain. `SHA256SUMS` is verified on every test run by
`runtime_observation_manifest_covers_every_committed_measurement_file`, which
also requires the manifest and the directory to list the same files.

A future capture should fold the aggregation into the driver so the whole
directory comes from one committed tool.
