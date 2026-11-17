# Linux first-run parity

The authorized migration follows the current native macOS sequence: Welcome,
Tools, Private AI, Traces, Uses, Terms, Join. Linux keeps its explicit source
answers and portal-backed folder chooser. Discovery is not authorization.
Standalone Insights continues to use the existing local startup path.

## State and commit contract

- Pure `first_run::State` stores answers separately from confirmed daemon facts.
  Its ordered planner and ticketed serial runner reject duplicate, cancelled and
  stale results. Failed required actions stop before completion.
- Welcome and Tools run before daemon startup for undeclared roots. Every tool
  remains unanswered until chosen. Only successful source declaration starts
  the daemon; a one-use handoff resumes at Private AI. Already running daemons
  receive changed declarations through `set_settings`, never direct file writes.
- Private AI is explanatory, matching the current native screen without a
  switch. Configuration remains in Settings. No first-run action enables it
  merely by visiting the page.
- Traces reads discovered watched projects through `list_projects` with
  `include_found`. Folder rules and selected historical sessions are draft
  answers until Start. `Never` excludes history from the plan. Missing or failed
  reads remain unavailable rather than fabricated empty results.
- Uses reads the daemon's scope titles and descriptions. Required use is visibly
  checked and locked, as in modern native first run; optional uses start off.
  Sharing starts Ask. Automatic is an explicit answer.
- Terms fetches the published document anonymously on a worker. The full body
  is shown, including tables. Continue requires reaching the end and explicitly
  accepting the exact loaded version and content hash. Back preserves the same
  document's receipt. Acceptance is handed to the daemon before sign-in/Start;
  a failed handoff retains the daemon's terms hold, as on native macOS.
- Join reuses Linux browser/provider and wallet ceremonies. No sign-in bypass
  or watch-only completion surface is introduced. Invite credentials remain
  only in the existing entry/IPC path. A fresh status confirms enrollment.
- Start uses the pure ordered plan: withdraw Automatic when choosing Ask; save
  scopes; save Never/Automatic folder rules and their selected history; optional
  explicit Private AI write; automatic grant; save explicit Ask exceptions and
  their history; finally shared setup completion. Nothing later runs when an
  earlier required action fails.
- Automatic requires the core-selected scrub disclosure, then the current route
  and witness disclosure, then `flow1::grant_request` against fresh enrollment
  and selected scopes. The requested witness is the one shown. The daemon
  independently enforces current consent and witness identity.
- A refused grant must successfully restore Ask before the runner can finish.
  Completion uses fresh `get_setup_status.scope` and `complete_setup` and checks
  the response. The new flow never writes the legacy global completion marker.
- Closing invalidates tickets and future callbacks. Successful writes already
  dispatched remain authoritative; the next run rereads daemon facts.

## Implementation and verification

1. Fix legacy Consent advancing after a failed scope save; retain retry and
   reject duplicate submissions.
2. Add pure state/planner/runner and tests for every failing action, grant
   fallback failure, cancellation and duplicate/stale completions.
3. Add published Terms reader and pure exact-document/end-of-scroll tests.
4. Wire actual GTK coordinator, source-startup handoff and existing auth.
5. Verify strict mutation acknowledgements and delayed-read lifecycle guards.
6. Run formatting, GTK compilation/tests/clippy and the license-boundary test
   after all tasks, then independent root review before any merge.

No dependency, backend permission or license-boundary change is required.
Native Linux presentation and portal/auth ceremonies need retained runtime
review; host compilation and pure tests alone do not prove visual parity.
