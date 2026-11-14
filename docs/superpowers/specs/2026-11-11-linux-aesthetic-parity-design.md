# Linux aesthetic parity with native macOS

Status: approved for implementation. Source audit only; no new
runtime screenshots or visual qualification have been completed.

## Intent and approach

Bring the GTK application into the same visual family as the native macOS app,
using GTK and the existing Rust services. The user's instruction to get started
follows the proposed sequence: audit, shared primitives and Insights, remaining
screens, Linux qualification.

Use shared design tokens and native GTK components with opaque, layered surfaces.
This is the recommended implementation choice. A CSS-only pass is smaller but
cannot fix hierarchy and widget composition. A full glass/window rewrite would
expand platform risk before establishing the basic visual language. Desktop blur
and macOS-specific window behavior are deferred.

## Findings

- `scripts/design-tokens/generate.py` already generates GTK brand/status colors
  from `design-tokens/glass.tokens.json`.
- GTK `ui/style.rs` and `ui/style.css` retain independent grounds, typography,
  geometry and the older paper-like presentation.
- macOS `TCDesign` supplies surfaces, controls, headings, cards and pane layouts;
  `Views/InsightsGlass.swift` applies those primitives to Insights.
- GTK `ui/insights_analytics.rs` uses generic GTK heading/card classes and saved
  snapshot analytics. Its Overview includes token/session cards, daily bars and
  breakdowns.
- macOS `InsightsOverviewTab.swift` now leads with week tiles, usage, spend and
  trends. It also supports data unavailable in GTK's current saved-feed screen.
  Therefore matching the current Overview fully is not a styling-only task.
- Existing Weston scripts capture composed frames and check text. Their Overview
  assertions include Tokens by day; reorganizing that content later requires
  intentionally updating those assertions.

## First deliverable

Implement a reusable GTK Insights visual vocabulary and apply it to the existing
saved-data Overview. Preserve current navigation and analytics operations.

Visual arrangement:

1. Existing navigation remains reachable, styled consistently.
2. A quiet header groups week selection and coverage, with secondary evidence
   visually subordinate and still reachable.
3. Summary figures use consistent title, value and supporting-text treatments.
   Replace the rigid three-card row with a layout that wraps at narrow widths.
4. Content cards share padding, surface, border and corner roles.
5. Charts and breakdowns retain their data and drill-down actions, with aligned
   labels and consistent spacing.

The screen should read as related to macOS through hierarchy, spacing, current macOS
selection, typography roles and surfaces. It must remain honest about its saved
data; missing live values must not be imitated with fabricated figures.

## Implementation boundaries

- Extend token generation for the subset of spacing, radius and type roles used
  by the pilot. Keep generated output deterministic and checked by the generator.
- Add scoped Insights CSS/component helpers before changing global defaults;
  avoid accidentally restyling every existing screen in the first slice.
- Map the active macOS flat theme's roles to solid Linux surfaces with readable
  light/dark contrast. Do not copy transparency values onto opaque backgrounds
  without checking their resulting appearance.
- Retain shared service copy, counts, coverage, unknown semantics and selection.
- Preserve asynchronous cancellation, stale-result rejection and account-free
  local analysis. No new notices, dependencies, enrollment or upload behavior.
- Defer live Overview data, navigation migration, maps/inspectors and animation
  parity to separately scoped follow-ups.

## Validation and acceptance

Capture a baseline and changed GTK frame with identical fixtures. Compare the
matching saved-feed macOS state where available; document content differences.
Use empty, populated, unavailable and busy states, both appearances, narrow and
wide windows, and enlarged text. Check keyboard focus and drill-down reachability.

Run the token generator check, the GTK workspace's relevant tests and CSS
contracts, and the existing Linux Weston display/lifecycle scenarios. Use
`RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0` consistently for Cargo commands.
Only run the full relevant suite after implementation tasks are complete.

Acceptance requires actual Linux rendered frames without clipped controls,
legible secondary text, distinguishable focus/selection and unchanged data
semantics. Compilation or OCR alone does not establish aesthetic parity.

## Environment and workflow

The primary checkout contains unrelated untracked work; leave it intact. Product
implementation must use `.claude/bin/tc-worktree <branch> [base]` to take over a
qualified finished donor. Do not copy build directories.

No Linux host was returned by `rex hosts check` during this audit. Docker and
GTK tools are installed locally; availability of a working Linux rendering lane
must be established before claiming visual qualification.
The initial Docker probe failed because the Colima Docker socket was absent.
Starting the existing Colima instance recovered Docker during implementation;
a synthetic GTK component harness is being used for Linux CSS validation.
That harness does not establish full-application parity.

## Subsequent slices

1. Bring Overview data and destination structure to the current macOS contract.
2. Apply the validated components to contributions, previews, Private AI,
   onboarding and Settings, with state-by-state comparisons.
3. Evaluate pane interactions and restrained motion, then qualify Linux desktop
   variations and packaging.

The first slice establishes a reusable visual system, not full application parity.
