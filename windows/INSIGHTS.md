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
