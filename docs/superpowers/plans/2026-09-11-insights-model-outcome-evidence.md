# Model declarations and explicit outcome evidence

This packet advances the [Insights program](2026-09-11-trace-insights-program.md)
from descriptive summaries toward model attribution and outcome linkage.
It does not turn trace claims, commit existence, or imported reports into
verified task success.

## User stories

A user analyzing a selected trace can see which model names its supported
metadata declares, whether multiple names occur, and exactly which bounded
record references support those observations. Missing, invalid, and omitted
declarations remain visible. Metadata declarations do not allocate work or
cumulative usage to a model; absent declarations cannot establish a single-model
session.

A user can explicitly link an existing saved snapshot to a commit in a chosen
local repository. Inspection reads that exact full object ID and retains only
repository digest, object/tree/parent identifiers, and observation time. It does
not inspect branch proximity, infer merge or revert status, execute hooks, fetch,
or associate another trace automatically. A selected subdirectory cannot trigger
parent-repository discovery. Git must support disabling lazy object fetch; an
unsupported installation refuses inspection rather than falling back.

A user can import a bounded structured test report and link it to a saved
snapshot. Its counts and claimed test time retain imported-report provenance.
A supplied commit ID is a report claim, not a verified association to a Git
object or executed revision. The importer runs no test command and retains no
raw test names, output, or shell commands.

## Lifecycle and authority

The local store moves to schema v3. Legacy v1/v2 records remain readable without
inventing model observations or outcome evidence. A mutation writes v3; explicit
reimport is required to populate model declarations for old source snapshots.
All new evidence validates against the saved source digest before presentation.
Older clients that support only v1/v2 refuse the upgraded store; automatic
downgrade is not implemented. Clients sharing this store must support v3.

Each evidence association is explicitly user-linked. Repeating a link is
idempotent; one commit or report may be linked to several snapshots. A snapshot
may carry at most 128 links. Identical-content reimports retain links, while
changed-content imports do not inherit them. Removing a snapshot or its last
source alias removes its links. Unlinking preserves the original trace,
repository, and test-report file.

No evidence in this packet increments independently verified outcomes or changes
the user's manual assessment. Later episode boundaries, test execution authority,
review/revert evidence, and matched-context attribution are required before
comparative rankings. Native snapshot and summary presentation of these new
fields follows through the same shared service; the initial entry points are
CLI and structured responses.

## Commands

`insights link-git SNAPSHOT_ID --repository /chosen/repo --commit FULL_OBJECT_ID`
inspects and links one explicit commit.

`insights link-test-report SNAPSHOT_ID --file /chosen/report.json` imports and
links one strict report. Schema v1 contains `schema_version`, `runner`, `passed`,
`failed`, `skipped`, `observed_at`, and optional `commit_id`.

`insights unlink-evidence SNAPSHOT_ID EVIDENCE_ID` removes one association.
`insights explain SNAPSHOT_ID` includes declared model observations and links.

## Verification

Synthetic fixtures must cover mixed/missing/invalid model declarations and
ignored model-like prose; exact-object Git validation, bounded process behavior,
and rejection of revision/option injection; strict test reports and overflow;
legacy migration, exact-content link retention, changed-source invalidation,
deduplication, deletion/unlinking, and unchanged verified-outcome metrics.
No user repositories or traces are used for qualification without selection.
The permissive license boundary and existing native compatibility checks remain
required. This packet adds no dependency or hosted authority.

Validation on the integrated tree: 45 focused Insights tests and five GTK
compatibility tests passed. The full warning-denied workspace suite passed
5,495 tests with zero failures and 21 ignored tests. The first sandboxed run was
blocked by an existing test's temporary-directory write under the home directory;
rerunning with that permission passed. Contributor/FFI all-target Clippy passed
with the repository allowlist. Native platform CI and any real-provider or
independent test-execution qualification remain separate.
