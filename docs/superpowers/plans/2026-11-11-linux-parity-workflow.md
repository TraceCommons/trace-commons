# Dynamic Linux parity workflow

Status: executing. User authorized workflow and execution across all six Linux priorities.

## Operating rules

Current native macOS is the parity reference. Deliver independently reviewable increments, preserve existing Linux behavior, and distinguish source/unit evidence from rendered real-app behavior. No new dependencies, unsolicited notices, credential collection, uploads or activation defaults. Shared service copy/data remain authoritative. New notices or dependency exceptions require their existing explicit approval gates. No automatic merge/release authorization is inferred from this workflow.

Use `.claude/bin/tc-worktree <branch> [base]` for every implementation checkout, never a fresh worktree or copied target. Preserve primary checkout. Every Cargo command uses `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0`. Finish task edits before final suite per repository rules. Use bounded parallel lanes with disjoint ownership; one integration owner for shell wiring. Independent review and exact-head validation precede each PR.

## Dispatch and adaptation

1. Validate current main and inspect exact CI failures. Reproduce causal failures; repair regressions before integrating dependent features. Unrelated baseline advisories are tracked separately, not silently fixed by widening scope.
2. Run Insights and Home design independently while the Linux gate is repaired. Implementation can proceed on isolated branches; integration waits on regression review.
3. Promote a slice only after tests, independent review, and explicit limitations recorded. A new critical finding returns it to repair, irrespective of earlier green checks.
4. When data/API needed by a screen is absent, use a separate service-contract slice; never invent totals, placeholder success, or unsupported actions.
5. After each PR, reassess remaining gaps against current macOS and select the next smallest complete slice. Avoid stacking unrelated UI work into one PR.

## Work queue

| Lane | Deliverable | Dependencies | Acceptance | Status |
|---|---|---|---|---|
| A Linux quality | Actual app navigation, layout, CSS, lifecycle checks; resolve Weston failure | Existing merged #2083/#2091 | Direct widget tests plus real app rendered verification; preserve OCR gate | Active |
| B Insights usage | Subscription limits, NEAR AI credit and daily usage | Existing live Overview worker | Read-only entry, no account refresh prompting, unknown/partial handling, stale results guarded | Active |
| B2 Insights depth | Recaps and Benchmarks with real destinations | B + shared existing APIs | Correct populations, evidence/actions and empty states | Queued |
| C Home | Unified status/balance/activity and existing-action shortcuts | A gate, existing daemon model | No fabricated activity, shared labels, accessible responsive layout | Design active |
| D Traces | Unified Queue/History navigation with contextual inspector | C shell design | Preserve consent/send/delete boundaries, selection and preview lifecycle | Queued |
| D2 Map | Useful trace map and restrained motion | D | Same underlying state as list; reduced motion and accessible alternative | Queued |
| E Onboarding/Settings | macOS-aligned guided setup and section navigation | C + platform rules | Preserve Linux folder/portal semantics, existing grants and auth flows | Queued |
| F Desktop delivery | GNOME/KDE, Wayland/X11, Flatpak, update/autostart/tray qualification | A; final repeat after C-E | Record observed environment/device matrix, actual tests, unresolved gaps | Audit queued |

## Review matrix

For every changed screen cover populated/empty/unavailable/loading, light/dark, narrow/wide, large text, keyboard navigation and canceled operations. Live values never open saved-data evidence. Linux tray is optional and never the only route to an action. Counter-pass/local analytics stays separate from contribution enrollment and upload authority.

Full device matrix work may need a Linux desktop beyond local Colima; missing environments remain explicit blockers, not claimed successes. CI screenshot/log artifact upload was previously rejected by automatic approval review, so this workflow does not add such uploads. Local synthetic captures are marked as component evidence.

## Deliveries already completed

- #2083: shared GTK Insights visual system, merged.
- #2091: live weekly spend and model overview, merged.
- Their Linux CI still fails; inspect current logs before deciding cause.

## Session ledger

- Created isolated workflow checkout from current origin/main.
- Dispatched Linux gate diagnosis, Insights usage implementation and read-only Home design.
