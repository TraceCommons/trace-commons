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

Saved details show bounded declared-model metadata with physical source
coordinates, missing/invalid/omitted counts, and its source digest. Older
snapshots remain unknown until explicitly reimported. Declarations do not
verify serving identity or allocate work to a model.

Saved snapshots can link an explicitly selected local repository and full
commit object ID, or import a selected structured test-report JSON file. The
import control explains the accepted fields. Git inspection and imported
report assertions have separate authority labels; neither establishes task
acceptance or model attribution. Removing a link preserves original files.
Each picker freezes its snapshot selection (and Git commit); refresh, selection
changes, and closing invalidate pending selections. Successful link changes
refresh saved detail, history, and summary; failures clear prior detail.

Episodes group one or more whole saved snapshots selected by the user. The
native screen creates and lists groups, resolves current member evidence, shows
snapshot overlap with other groups, and supports complete membership replacement,
an independent category/outcome assessment, clearing that assessment, and group
deletion. These groups do not establish task boundaries, independent tasks,
model attribution, rankings, time saved, or cost.

An open group freezes its ID and revision. Member edits submit the complete
selection against that revision; assessment and deletion use the same conflict
check. A conflict discards the draft, refreshes the group, and requires review
instead of retrying the write. Leaving the group or Insights invalidates pending
callbacks. Episode IO has separate busy state, so the compact screen keeps its
snapshot analysis, save, detail, evidence, and assessment controls available.

Snapshot replacement or deletion can remove episode groups. The screen shows
the committed group IDs and explains the loss of grouping and assessments. This
notice survives automatic episode/history refresh, including a refresh failure,
and clears on the next user action or closing Insights. A failed or missing
episode refresh clears editable detail; a successful mutation is reconciled by
reading the episode list and current detail from the local service.
