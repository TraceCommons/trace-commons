# Local Insights

Windows opens Insights before starting the contributor daemon. Select Contributions,
Queue, History, Model Calls, or Settings to start the existing contribution flow;
source declaration and enrollment gates still apply. Insights itself neither starts
that flow nor discovers files.

Choose a Codex rollout or trajectory file and press Analyze. Analysis is ephemeral.
Re-read and save explicitly reads the selected file again into the CLI's dedicated
local store. Refresh saved insights lists stored snapshots without re-reading source
files. Show evidence loads a saved snapshot; Delete removes its derived record and
references while preserving the original file. Assessments are user-reported and
remain separate from verified outcome metrics.

The view renders Rust-provided metrics, observed/total coverage, evidence digests,
provider/rubric versions and shared copy. Unknown stays unknown. No cross-model
ranking or event-level explanation is inferred by the shell. Dates and counts use
the current culture. There is no automatic source refresh.

The handle-free wrapper schedules bounded native IO off the UI thread, releases
both returned native strings in a finally block, and serializes in-process calls.
Cancel or window close suppresses late results. An already-started save or deletion
may finish; cancellation does not roll back local persistence. Mutations remain
serialized behind that operation. Hiding to the tray is not closing the window.

## Verification

The Interop test project includes the platform-independent Insights viewmodel,
source-wiring checks for first activation, and a native selected-file lifecycle test.
Rebuild contributor FFI before running it. On this macOS development host, net8.0
was compiled with zero warnings and tested on the installed .NET 10 runtime using
`DOTNET_ROLL_FORWARD=Major` and `TC_FFI_LIB_DIR` pointing at the freshly built dylib.
That verifies the managed wrapper against the macOS ABI, not Windows packaging.

Before Windows release, build the WinUI application on Windows and run these tests
against the newly built Windows DLL, then manually qualify fresh-install navigation,
file picker and keyboard focus, long text and scaling, cancellation/window close,
source replacement and CLI store interoperability, and owner-only persistence ACLs.

## Saved history summary

The summary reads the shared derived store on entry and refresh, and after save,
delete, annotation, and annotation removal. It never reopens source files. Native
observed totals retain unknown versus known zero, snapshot availability/missingness,
and recognized-record coverage with its unit. Categories and outcomes distinguish
explicit Unknown assessments from sessions that have no assessment.

Summary rows link their contributing snapshot IDs to the existing evidence view.
Analysis dates describe snapshot imports, not activity duration. The UI renders
shared limitations and does not calculate rates, rankings, savings, or cost. A
failed or canceled summary refresh clears the previous summary; re-entry retries
through the existing serialized service boundary.

Summary-specific validation includes typed decoding, partial record coverage despite
an available snapshot, known zero, explicit unknown assessments, missingness, evidence
navigation, saved mutations, failure, cancellation, and close. The updated summary
WinUI markup still requires Windows CI compilation and interactive qualification;
the earlier snapshot UI's Windows CI result does not qualify these new controls.

File selection stays above the summary. Detailed summary rows and contributing
snapshot lists start collapsed. Opening evidence clears any previous selected
snapshot before lookup and scrolls the successful result into view; a failed
lookup cannot retain an unrelated successful result. Schema-v1 summaries refuse
unknown coverage units and unknown or missing required limitation labels.

## Tabs

The Insights control has tabs: **Overview**, **Patterns**, **Sessions** and
**Analyze**, as on macOS. Analyze holds the whole screen described above.
**Spend** sits beside the tabs, disabled, with its "Later" chip and no content.
A tab reads its figures when it is shown, and the tab on screen reads again
when the saved list changes.

Overview reads `week_overview` and, for each card's drill-down, `card_inputs`;
Patterns reads `patterns` and `pattern_sessions`; Sessions reads
`session_drill` for one saved snapshot, the newest unless another is picked.
All three count only the sessions you analyzed and saved (feed S) and say so
with the core's feed lines; week-to-week change reads as the core's dash.
Claude Code and Codex lines are never summed. An unknown figure is the dash,
never zero, and a week or turn with no figure is a gap rather than a zero
bar. Bars are plain rectangles. Files are named only by a letter and an
extension. Codex sessions show that turn-by-turn usage is not recorded. No
advice, what-if estimate, project name or cost is shown.

Every word is the core's `ui_copy()` analytics table, read through the `copy`
operation and marked DRAFT pending approval; the shell authors no sentence, so
`ShellWordingTests` is unchanged. Every figure is the core's; the view model
computes only a bar's length on screen. A failed or late read clears the
tab's figures rather than keep stale ones.

The view models are tested against the responses in
`tests/fixtures/insights-analytics`, which the local service produced and the
GTK tests read too. The WinUI markup (a `Pivot`) still requires Windows CI
compilation and interactive qualification.

The tray menu is a Win32 popup menu of text rows and has no room for a glance
card, so this shell gets the news mark and tip through the nudge slice W2
only.
