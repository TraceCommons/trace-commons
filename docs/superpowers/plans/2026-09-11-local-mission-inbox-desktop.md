# Local mission inbox in the unified desktop product

Date: 2026-09-11
Status: Implementation follow-on to the private mission draft inbox. Part of the existing authorized Insights and missions program; this is not publication or execution approval.

## User outcome

A user can import an explicitly selected mission proposal, see locally saved drafts, inspect the claim and experimental task, and delete a draft without enrollment. The same backend, statuses, and copy drive macOS, Windows, GTK, and the CLI. This exposes the existing local inbox as a useful curation surface while published mission discovery and participation continue as separate work.

## Shared contract

Add a bounded local mission service, separate from the Insights operation enum, with list, import-file, show-ID, delete-ID, and shared-copy operations. Reuse MissionDraftInbox validation, canonical digest deduplication, private atomic storage, and fixed errors. Return typed operation-tagged results. Import/list return digest, counts, and review state; only explicit show returns the full validated proposal. Opening an absent inbox is read-only and never creates enrollment state.

Add a contributor FFI dispatch operation with the existing pointer ownership, input/output bounds, panic boundary, and error conventions. All shells consume the same schema. Unknown operations, fields, versions, IDs, oversized input, and malformed stored drafts refuse with fixed errors. Paths are explicit local input only and never appear in shared status messages or logs. Do not introduce any network access, plugin execution, source URL opening, or mission publication authority.

## Desktop behavior

Place a Mission drafts surface within the existing Trace Commons product navigation. It is a local curator inbox, not the published Discover feed. Use shared labels for title, empty state, import, refresh, inspect, delete, and required review. Keep import available without enrollment. A selected file is the sole import input; do not scan or watch directories.

A list row identifies the proposal digest and source count with the common needs-curator-review status. Inspect loads the selected digest explicitly and displays title, source claim, task, starting artifact digest, proposed success criteria, allowed tools/models, declared budgets, and author/evaluator labels as unverified proposal data. Render all proposal strings as plain text, including URLs; never execute, automatically follow, or interpret them as instructions or trusted markup.

Keep source claims, proposed methods, and eventual findings distinct. A successful structural review means the draft has the required shape; it does not verify sources or identities, approve a task, grant a budget, or publish it. No run/publish/fund buttons appear until those actual workflows exist.

Deletion uses the existing platform confirmation convention, removes only the local draft, and clears its detail selection. Imports refresh the list and distinguish inserted from duplicate. In-flight requests use cancellation or generation checks so stale show/import responses cannot resurrect a deleted draft, change selection after navigation, or overwrite a newer operation. Errors preserve the last valid view while making failure visible through shared copy.

## Stack and verification

1. Shared service, fixed copy, CLI reuse, and FFI transport with lifecycle and bounded-input tests.
2. macOS surface and decoder/view-model tests plus native FFI-backed import/list/show/delete lifecycle.
3. Windows surface and decoder/view-model tests plus actual Windows build and runtime CI.
4. GTK surface with binding tests and actual display-backed CI.

Sol implements each slice; Astra reviews each implementation and rechecks fixes. Do not infer platform readiness from Rust-only tests or a Windows cross-compile. Include exact PR heads and test limitations in release evidence. Tests cover duplicate imports, unchanged source files, absent-store reads, invalid IDs/data, no enrollment mutation, deletion and stale completion, and hostile proposal strings rendered as data.

Live source adapters, scout generation, curator authorization/publication, unified published discovery, mission attempts/evaluation, challenges, funding, and mission-to-Insights findings remain required later work in the full program.
