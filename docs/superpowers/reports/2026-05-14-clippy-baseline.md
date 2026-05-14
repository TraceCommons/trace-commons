# Clippy Baseline Audit

Date: 2026-05-14
Audit base commit: `1face90762f7ca236834ba10ce5818f0eacf6849`
Toolchain: `cargo clippy` (rustc 1.94.0 stable channel as bundled).
Invocations:

- `cargo clippy --workspace --all-targets -- -D warnings` (gating run)
- `cargo clippy --workspace --all-targets`             (full inventory)

Goal: enumerate every clippy / rustc lint that currently fires so a
follow-up PR can turn clippy on in CI with `RUSTFLAGS=-D warnings`
without surprise.

## Totals

| Bucket    | Count |
|-----------|-------|
| Critical  | 2     |
| Important | 5     |
| Style     | 21    |
| Pedantic  | 0     |
| (rustc `dead_code`, separate) | 24 |

Total lint instances: 52. Workspace-wide, no `error:` lines under
default warning levels — the `-D warnings` gating run only fails because
warnings are promoted to errors.

The two Critical instances are fixed in this PR (`neg_cmp_op_on_partial_ord`
in `embedder_fastembed.rs`, `cloned_ref_to_slice_refs` in
`tests/bakeoff_report.rs`). All remaining counts below are pre-fix.

## Critical

Semantic-correctness lints. Real bugs or latent foot-guns.

| Lint                              | Count |
|-----------------------------------|-------|
| `neg_cmp_op_on_partial_ord`       | 1     |
| `cloned_ref_to_slice_refs`        | 1     |

Notes:

- `neg_cmp_op_on_partial_ord` fires in
  `crates/tracedao-gate-enclave/src/embedder_fastembed.rs:36` on the
  degenerate-vector guard `if !(norm > epsilon)`. Inputs are already
  required to be finite (NaN/Inf bail above), so the equivalent
  `if norm <= epsilon` form is semantically identical. Fixed here.
- `cloned_ref_to_slice_refs` fires in
  `crates/tracedao-server/tests/bakeoff_report.rs:159` — test-only,
  swapped to `std::slice::from_ref`.

## Important

Style/perf lints with semantic impact. Worth fixing before enabling
clippy in CI.

| Lint                       | Count |
|----------------------------|-------|
| `err_expect`               | 4     |
| `manual_is_multiple_of`    | 1     |

`err_expect` (`.err().expect(...)` on a `Result`) is a latent panic
shape that silently treats `Ok(_)` as failure. All four sites are in
`bin/tracedao-ingest.rs` — switching to `expect_err` preserves intent
and is a one-line change each.

`manual_is_multiple_of` is a single mechanical rewrite (`x % y == 0` →
`x.is_multiple_of(y)`).

## Style

Pure style / readability. Low priority; the recommendation below is to
allow these globally and revisit opportunistically.

| Lint                          | Count |
|-------------------------------|-------|
| `type_complexity`             | 6     |
| `collapsible_if`              | 6     |
| `manual_option_as_slice`      | 6     |
| `useless_vec`                 | 2     |
| `redundant_pattern_matching`  | 1     |

`type_complexity` clusters in `bin/tracedao-ingest.rs` and the calibrate
report code, all on ad-hoc `[(&str, fn(...) -> u64); N]` tables used for
audit projection. They could be cleaned up but inventing a `type` alias
for each table is not obviously better; leaving the lint allowed is
reasonable.

`manual_option_as_slice` clusters in
`crates/tracedao-server/src/bin/gate_calibrate/bakeoff_report.rs` and
`src/bin/pilot_bootstrap/translators.rs`. Mechanical fix, but not
load-bearing.

## Pedantic

No pedantic-tier lints are currently emitted under default clippy
warning levels.

## rustc `dead_code` (separate from clippy buckets)

24 instances across the bake-off and pilot-bootstrap binaries when
compiled with `--all-targets` (i.e., dragged in by integration tests
via `tests/../src/bin/...`). These do not fail CI today because the CI
matrix builds `--bins` and `--tests` separately, not `--all-targets`,
and the dead code is reachable in the bin target. There is a known
branch (`gate-test-target-dead-code`) staged to address this; merging
it would clear all 24.

Top contributors:

- `tests/../src/bin/gate_calibrate/bakeoff_manifest.rs` — 7
- `tests/../src/bin/gate_calibrate/bakeoff_corpus.rs` — 6
- `tests/../src/bin/pilot_bootstrap/translators.rs` — 5
- `tests/../src/bin/gate_calibrate/bakeoff_report.rs` — 5
- `tests/../src/bin/gate_calibrate/run_candidate_eval.rs` — 2

## Top 10 files by lint count

| Rank | File                                                                                       | Count |
|------|--------------------------------------------------------------------------------------------|-------|
| 1    | `crates/tracedao-server/src/bin/tracedao-ingest.rs`                                        | 8     |
| 2    | `crates/tracedao-server/tests/../src/bin/gate_calibrate/bakeoff_manifest.rs`               | 7     |
| 3    | `crates/tracedao-server/tests/../src/bin/gate_calibrate/bakeoff_corpus.rs`                 | 6     |
| 4    | `crates/tracedao-server/tests/../src/bin/pilot_bootstrap/translators.rs`                   | 5     |
| 5    | `crates/tracedao-server/tests/../src/bin/gate_calibrate/bakeoff_report.rs`                 | 5     |
| 6    | `crates/tracedao-server/tests/../src/bin/pilot_bootstrap/hf_dataset.rs`                    | 2     |
| 7    | `crates/tracedao-server/tests/../src/bin/gate_calibrate/run_candidate_eval.rs`             | 2     |
| 8    | `crates/tracedao-server/src/trace_artifact_gcs.rs`                                         | 2     |
| 9    | `crates/tracedao-server/src/bin/pilot_bootstrap/translators.rs`                            | 2     |
| 10   | `crates/tracedao-gate-enclave/src/vector_index.rs`                                         | 2     |

`bin/tracedao-ingest.rs` is the known mega-file; per repo convention it
is not to be split unilaterally, so its 8 instances should be addressed
in place.

## Recommendation

To enable clippy in CI without immediate code work, the workflow should
pass:

```
-A clippy::type_complexity
-A clippy::collapsible_if
-A clippy::manual_option_as_slice
-A clippy::useless_vec
-A clippy::redundant_pattern_matching
```

That suppresses the 21 Style instances. The 2 Critical lints are fixed
in this PR; the 5 Important lints (`err_expect` x4 and
`manual_is_multiple_of` x1) should be cleared before the CI flip so the
gating run is green on first attempt.

Estimated effort to reach CI-ready: **Quick** (under one focused
session). Five line edits in `bin/tracedao-ingest.rs` plus one
`is_multiple_of` rewrite. The `dead_code` queue is independent and is
already tracked on the `gate-test-target-dead-code` branch; landing
that branch first is the cleanest order.
