# Linux Traces inspector implementation plan

**Goal:** Add contextual folder/session inspection without changing contribution permissions or existing routes.

**Architecture:** A GTK side pane owns an identity selection, resolved from live pending entries. It reads scheduled preview summaries and opens the existing complete review sheet by entry ID. The sheet continues to own consent, transcript search and contribute actions.

**Approaches:** Embedding the full sheet would require replacing its window, pending snapshot and advancement lifecycle. The approved bounded approach retains it and composes a read-only inspector alongside Queue. Map and History-detail parity follow separately.

**Constraints:** Existing shared copy only; no new notices, dependencies, send controls, or withdrawal semantics. Keep the persistent undo bar outside the queue scroller. Use the donor worktree. Tests run once after both tasks, per repository multi-task guidance.

- [x] Guard preview replies and consent/search followups by request epoch; invalidate on close. Test late replies after advancement, repeated requests and close.
- [x] Add folder/session inspector with live identity reconciliation and Look inside by ID. Test disappearance, duplicate labels and reordered pending entries.
- [x] Run GTK formatting, complete GTK tests and clippy with RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0. Report native Linux visual validation separately.


## Verification

GTK all-target check, full tests (622 passed, 8 ignored), and clippy using the existing CLAUDE allow-list passed with `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0`. GTK formatting and `git diff --check` passed. Raw clippy without the repository allow-list reports existing collapsible-if, type-complexity and useless-vec findings; no allow-list was expanded.

The inspector uses generated flat typography and spacing roles, stacking below the queue below 1000 px. Native Linux visual/interaction verification remains an integration task; these checks do not claim it. History detail and Map are outside this bounded slice.
