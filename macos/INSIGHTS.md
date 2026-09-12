# Local Insights on macOS

The app opens on Insights. Watcher startup, compute monitoring, update checks,
and notification setup begin only after selecting another destination, once
per app launch. Their existing roots, onboarding, and activation checks still
apply. Returning to Insights does not stop services already activated.

Choose a Codex rollout or trajectory file to analyze it locally. Analysis is
temporary; **Re-read and save** reads the selected file again and saves derived
observations to the same local store as the CLI. Saved snapshots update only
on explicit import. Deleting a snapshot keeps the original file. Assessments
are user-reported and remain separate from verified outcomes.

The saved-history summary loads on entry and refreshes after saving, deleting,
annotating, or clearing an assessment. It shows saved-session counts, reported
categories/outcomes, and observed metric totals with both snapshot availability
and original evidence coverage. Unknown assessments and unassessed sessions are
separate; a measured zero remains distinct from an unavailable value. Dates are
analysis dates, not work dates. Summary evidence buttons open the corresponding
saved snapshot and scroll its detail into view. Separate summary and detail
reads can observe different store versions; a missing snapshot clears the old
detail and reports failure. A failed summary refresh removes the stale summary.

The screen displays Rust-provided observations, coverage, provider/rubric
attribution, and source-digest evidence. It does not calculate independent
metrics, discover files, enroll contributors, or upload traces. Native usage
accounting, model comparisons, coaching, and event-level explanations are not
part of this screen yet.

Local operations run off the UI thread. Closing or leaving the screen discards
late results; a save or deletion already started can still finish. Reopening
loads saved results again. Selection uses the native macOS file picker.

Verify with a freshly built contributor FFI library:

```sh
TC_FFI_LIB_DIR=/path/to/target/debug swift test
```

Bridge tests exercise the real ABI with a temporary store; model tests cover
first-run navigation, explicit persistence, summary refresh/failure, missing
evidence navigation, unknown/zero/partial coverage, and completion after close.
The automated suite does not qualify VoiceOver, the native file picker,
Gatekeeper/notarization, or all display sizes on a packaged release.
