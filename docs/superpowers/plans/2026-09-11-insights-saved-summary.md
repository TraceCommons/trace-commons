# Saved-history Insights summary

This slice follows the desktop stories and adds a shared, local answer to
“What have I saved, and how did I assess those sessions?” CLI and native
clients consume the same typed result. Native presentation can follow in a
separate stacked PR after the contract is tested.

## Scope and evidence

The population is the current saved Insights history. It is not all developer
activity, a verified task set, or an unbiased sample. Imported aliases must not
increase the population. Deleting a saved snapshot removes it from the next
summary; annotations and explicit reimports affect the next read.

Report task-category counts and user-reported outcome counts. Preserve explicit
unknown labels separately from unassessed sessions. Do not infer a model's
acceptance rate from a session assessment: sessions can involve multiple models
and rejected sessions do not measure rejected lines of code.

For each observed metric, retain available and missing snapshot counts, the
underlying record coverage, and references to contributing snapshots. A sum
over available observations is a partial observed sum whenever coverage is
incomplete. Empty or wholly missing observations must not become a measured
zero. Arithmetic overflow refuses the entire summary with a fixed error label; it never wraps, clamps, or reports a misleading partial total.

Snapshot dates describe analysis time, not when work occurred. Source format
labels describe adapters, not serving-model identity. Cost, time saved,
independently verified success, and model recommendations remain unavailable.

## Lifecycle

Summary reads only derived saved data and does not reopen original traces,
discover sessions, enroll, launch a daemon, upload, or initialize an absent
Insights directory. Existing store validation applies before aggregation.
The summary is computed on demand, so it adds no persisted schema or cache
invalidation path.

## Acceptance evidence

Use synthetic fixtures to verify empty-history behavior; alias deduplication;
annotation, clear, delete, and reimport changes; partial and missing metrics;
explicit unknown versus unassessed labels; overflow; and CLI/service agreement.
Validation must preserve the permissive license boundary and add no dependency.

The next product slice can render this contract in the three native shells.
Matched model comparisons still require task boundaries, attributed model use,
outcome evidence, and a separately reviewed comparison method.

## Implemented contract

`insights summary` computes the current history; `--json` returns
`SavedInsightsSummary`. The handle-free service accepts
`{"operation":{"type":"summary"}}` and wraps the same value in
`{"type":"summary","summary":...}`. An optional `store_dir` selects the
same dedicated store used by the other local commands.

The schema version is 1. `saved_snapshots` counts unique saved observations.
`user_reported` has assessed/unassessed counts, all six category counts and all
four outcome counts, with contributing snapshot IDs. Explicit `unknown` counts
only annotations deliberately labeled unknown. Clearing an annotation moves
that snapshot to unassessed.

Each metric supplies `observed_value_sum`, `available_snapshots`,
`missing_snapshots`, `record_coverage`, `coverage_unit`, and the snapshot IDs
contributing known values. Coverage units preserve the existing metric
semantics: normalized events, tool results, or session snapshots. A known zero
is `0`; wholly missing or empty data is `null`. There are no inferred rates.

`snapshot_analysis_range` is `null` for an empty history; otherwise it bounds
the analysis dates in the saved snapshots. Sorted `snapshots` retain the source
format, analysis timestamp, and digest evidence. The current store accepts only
the versioned first-party analyzer; its manifest remains attached. Typed
`limitations` travel with the response so future native views can preserve the
scope, partial-observation, and unsupported-comparison caveats.

No native summary screen is changed in this PR. Existing individual snapshot
screens are described in the [desktop story record](2026-09-11-insights-desktop-stories.md).

## Validation

Focused checks with `RUSTFLAGS='-D warnings'`:

- Contributor `--lib insights::`: 31 passed, including three summary tests.
- `--test local_insights_cli`: nine passed, including CLI/service equivalence.
- Contributor FFI `--lib insights`: two passed.
- Server `--test license_boundary`: four passed; expected boundaries unchanged.

Clippy for contributor/FFI all targets uses the repository allowlist and denies
other warnings. `cargo fmt --all -- --check` and C-header correspondence are
also checked. The empty, annotated, alias, reimport, deletion, missing, partial,
known-zero, and overflow cases use synthetic local data. Native summary
presentation and whole-product release qualification are not claimed by these
checks.
