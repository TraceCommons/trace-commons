# Native model declarations and outcome links

This slice presents the [local evidence contracts and lifecycle](2026-09-11-insights-model-outcome-evidence.md)
in the shared Insights experience on macOS, Windows, and GTK.

## User stories

- Inspect supported model declarations for a selected snapshot, including mixed
  names, missing/invalid/omitted metadata, source digest, and record coordinates.
  A legacy snapshot shows unavailable data; the absence of mixed declarations
  never establishes exclusive use of one model.
- On a saved snapshot, explicitly choose a repository and full commit ID to
  inspect and link. No branch/revision guessing or automatic repository discovery.
- Explicitly choose a structured test report to import and link. The UI explains
  the accepted report format and preserves imported-assertion provenance.
- Inspect each link's kind, authority, observation/link times, digest references,
  Git identifiers or reported test counts, and optional unverified revision.
- Remove an association while preserving all original files and other snapshots'
  independent links. Refresh after mutations reconciles detail, history, and
  summary without inventing accepted/verified outcomes.

## Selection and async lifecycle

Picker completion must carry the original saved snapshot ID and a presentation/
selection-generation token. Switching snapshots, replacing selection through a
refresh, closing, canceling, or leaving/re-entering invalidates the token. A
commit field is captured with the picker request rather than read later from
mutable UI state. Late callbacks must not attach evidence to another snapshot.

The shared store rejects a snapshot removed externally while the picker was
open. Errors clear stale success/detail as appropriate and remain visible. Local
I/O runs off the UI thread. Closing suppresses presentation but cannot cancel a
mutation already started; refresh reconciles its eventual result.

Evidence sections should be compact or collapsible. The normal analyze/save
controls stay accessible, and evidence navigation reveals its result. Native
layers use shared Rust wording and typed values; they do not recalculate or
upgrade evidence authority.

## Verification

Use synthetic sources, repositories, and reports for typed/native round trips.
Cover legacy unknown, multiple declarations, incomplete metadata, provenance,
link/unlink refresh, stale picker tokens, source removal/replacement, and close/
re-entry. Verify each shell against a newly built FFI/shared service. Platform
CI remains necessary for Windows and Linux display behavior, independently of
local tests. No new dependency or store migration is introduced by these views.
