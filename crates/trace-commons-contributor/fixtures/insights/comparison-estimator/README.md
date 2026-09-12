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
