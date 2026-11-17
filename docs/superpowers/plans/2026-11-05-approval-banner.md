# Approval banner repair

User requested a PR and a search for related contradictions. Fix existing result notices; no new notices or permission changes.

- Pin zero approvals versus positive approvals in macOS inspector/popover regression tests.
- Gate success badges, success color and Undo visuals on actual positive approval. Remove upload-started fallback when nothing was approved.
- Include the daemon's separate held-for-review count in group outcome text using shared core copy.
- Audit sibling paths; fix Windows preview results that turn zero or malformed approval responses into success.
- Run focused Swift, Rust and Windows interop tests; inspect rendered banner, review, and open a ready PR.

Keep skipped, held and ineligible counts distinct. Preserve existing positive approval and Undo behavior. Do not send or approve any live traces while testing.

## Validation and audit results

75 Swift tests passed, including a group-result integration test and rendered notice export. 18 shared Rust copy tests passed. Windows interop: 1375 passed, 1 Windows-only skip. FFI rebuilt from this branch. Formatting and diff checks passed. Independent review found no remaining blockers after fixes.

The search covered macOS inspector/popover, preview and group submission paths, plus Windows/GTK submission and Undo. Fixed the Windows preview false-success path and the unknown-response no-send claim. GTK/Windows one-click Undo already requires a positive approval and live hold. Held-count detail in GTK/Windows group outcomes remains a parity follow-up; this branch adds the explanation to the native macOS lead client.
