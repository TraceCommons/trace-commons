# Linux pending trace map implementation plan

**Goal:** Show pending folder/tool relationships and open the existing read-only inspector by stable ID.
**Architecture:** A pure pending-only scene projects the same grouping as the queue. GTK overlays static decorative connectors behind accessible nodes; the existing queue remains the complete list and action surface.
**Constraints:** No dependencies, new product copy, transfer inference, enrollment or permission actions. Reuse monitor_screens_copy. No library edge. Honor gtk-enable-animations for disclosure only.

- [x] Add pure scene model and tests for pending-only counts, duplicate labels, multiple tools, and ID selection.
- [x] Add scrollable GTK map and shared show/hide control. Reuse inspector selection, highlight current folder/tool, clear stale selection, adapt column layout to width.
- [x] Add mounted widget test covering activation, selection, narrow layout and reduced motion; register with Weston.
- [ ] Run GTK formatting, tests and clippy once after changes; run mounted tests with native display and require hosted Weston for Linux claims. Obtain independent code review. Commit and open stacked PR, no merge.

## Validation and review

GTK unit/integration suite: 624 passed, 9 display tests ignored. GTK clippy passed with the repository allow-list; formatting, shell syntax and diff whitespace passed. Independent review found narrow connector overlap and insufficient mounted assertions; corrections route edges behind nodes through a gutter and exercise focus/activation, dimensions and scrolling. Follow-up review found no remaining blocker.

The mounted test is registered with Weston. A local macOS libtest attempt failed before the view ran because GTK initialization must happen on the OS main thread. Linux mounted rendering remains a CI validation requirement; no desktop visual or Linux-mounted pass is claimed.
