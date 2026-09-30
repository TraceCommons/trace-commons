# Native saved-history summary

This slice presents the [saved-history summary](2026-09-11-insights-saved-summary.md)
in the existing macOS, Windows, and GTK Insights screens. The shared Rust
service owns the calculation and wording; native views own presentation,
selection, and asynchronous lifecycle.

## User stories

- Open Insights and see the population of saved session snapshots without
  enrolling, starting contribution services, or creating an absent store.
- See categories and outcomes from personal assessments, with an explicit
  distinction between unknown and not assessed.
- Read available metric totals alongside missing snapshot counts and the
  original evidence coverage, including its unit. A missing value stays unknown;
  a measured zero stays zero.
- Open a contributing saved snapshot to inspect its evidence. Deletion by another
  window or process must produce absence or refreshed state, not a fabricated
  detail or an error hidden behind an old result.
- Refresh, save, delete, annotate, or clear an assessment and see a freshly
  computed summary. A failed refresh must not continue presenting a stale summary
  as current. Unsaved previews never enter the saved summary.
- Leave while local work is running without receiving stale screen updates.
  A mutation already started can still complete; re-entry reconciles the store.

## Shared interpretation

All platforms use `ui_copy()` keys prefixed `summary_`, including unit and
limitation labels. Existing metric, category, outcome, and source labels remain
shared. Snapshot analysis dates are not activity dates; source formats are not
model identities. User-reported outcomes do not establish verified task success,
rejected lines of code, model rankings, cost, or time savings.

The summary response defines its own population and evidence IDs. Separate
history/detail reads can observe a later store version. The UI must preserve
that distinction and handle missing detail gracefully; it must not imply an
atomic multi-call snapshot. No native layer independently recomputes totals or
turns observed counts into comparative rates.

## Verification

Each platform needs typed decoding and rendering checks for empty data, known
zero, unknown values, partial coverage, and all supported limitation/unit labels.
Lifecycle checks cover refresh after mutations, failure clearing stale summary,
late completion suppression, and evidence navigation. Native bridge checks use
a freshly built FFI library containing the summary and common wording.

Windows DLL/WinUI checks and Linux Weston/display checks remain platform-specific
CI evidence; local macOS tests cannot substitute for them. Existing contribution
startup gates and license boundaries remain applicable. No dependency or store
schema change is required.
