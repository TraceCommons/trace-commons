# Insights desktop user stories and verification

This work implements packet 5 of the [program](2026-09-11-trace-insights-program.md), using the [local service](2026-09-11-local-insights-next-wave.md). The wider program remains active: native screens do not establish outcome linkage, calibrated comparisons, coaching, hosted consent, or qualified mission publication.

## Shared behavior

A user opens Insights before enrolling or declaring watched sources, selects a Codex rollout or trajectory file, and receives local descriptive observations. Analysis is ephemeral. Saving explicitly reads the selected file again, so the button says **Re-read and save**. Saved history uses the same store as the CLI. Loading an absent history creates no directory; existing stores still undergo permission and symlink checks.

Every platform uses shared service results and the Rust `copy` operation for wording. Missing counts remain unknown; user-reported assessments remain separate from independently verified outcomes. Evidence identifies the source snapshot digest and analyzer/rubric. Refreshing saved history does not read original files again. Explicit reimport refreshes source evidence; deletion removes the saved result and references while preserving the original file.

Local I/O runs off the UI thread. Cancelling or closing suppresses late screen updates; a mutation already started may finish. Refresh after re-entry reconciles the actual store. Shells must not imply that cancelling a screen update cancels disk writes.

## Launch behavior

Desktop launch opens Insights. Contribution-related daemon startup, source discovery, and account work are deferred until the user explicitly enters a contribution destination. That destination retains the existing roots and enrollment gates. This changes the previous automatic contribution startup behavior, including for an existing installation; the new launch route must be visible in release notes and qualified before release. Entering contribution mode must not repeatedly start background services when navigating between destinations.

## Acceptance evidence to collect

| User story | Required evidence |
| --- | --- |
| Open private Insights on a fresh installation | Native startup/navigation test proves no contribution startup; runtime checks show no enrollment or Insights directory from empty history and no-save analysis |
| Analyze only a chosen file | Native picker/explicit path plus shared bounded-reader tests; unknown and partial evidence shown without invented values |
| Save and revisit a snapshot | Real native/service round trip saves, lists, explains, and shares the CLI store; re-read-on-save wording visible |
| Understand an observation | Visible analyzer, rubric, date, coverage, missingness, source digest, and calculation inputs; long identifiers remain usable |
| Correct a task assessment | User-reported category/outcome controls round-trip through the shared store, with clear/remove behavior and digest provenance |
| Remove a saved observation | Native/service deletion test removes history and annotations, preserves the original file, and clears stale selection |
| Leave during local I/O | Controlled late completion test proves no stale screen mutation; started-write caveat remains visible |
| Continue contributing | Existing roots/onboarding gates and explicit entry path still pass their native tests |
| Use each supported desktop | macOS build/tests and visual inspection; Windows WinUI/native DLL checks on Windows; GTK separate workspace and Linux Weston/portal smoke |

A managed test linked to the macOS dylib is useful interop evidence, but does not qualify the Windows DLL or WinUI runtime. GTK tests on macOS do not qualify Linux portal, tray, or Weston behavior. Rust workspace tests do not substitute for native checks. Record those platform results in the corresponding PRs and keep unresolved release gates explicit.
