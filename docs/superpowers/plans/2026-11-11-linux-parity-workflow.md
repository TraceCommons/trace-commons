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
| A Linux quality | Actual app navigation, layout, CSS, lifecycle checks; resolve Weston failure | Existing merged #2083/#2091 | Direct widget tests plus real app rendered verification; preserve OCR gate | Passed at dc522585 and 073789798 via #2098; feature PR checks pending |
| B Insights usage | Subscription limits, NEAR AI credit and daily usage | Existing live Overview worker | Read-only entry, no account refresh prompting, unknown/partial handling, stale results guarded | Draft PR #2101; reviewed and locally tested |
| B2 Insights depth | Recaps and Benchmarks with real destinations | B + shared existing APIs | Correct populations, evidence/actions and empty states | Benchmarks draft PR #2104; recap draft PR #2106 |
| C Home | Unified status/balance/activity and existing-action shortcuts | A gate, existing daemon model | No fabricated activity, shared labels, accessible responsive layout | Draft PR #2102; reviewed and locally tested |
| D Traces | Unified Queue/History navigation with contextual inspector | C shell design | Preserve consent/send/delete boundaries, selection and preview lifecycle | Draft PR #2105; reviewed and locally tested |
| D2 Map | Useful trace map and restrained motion | D | Same underlying state as list; reduced motion and accessible alternative | Draft PR #2107; reviewed and locally tested |
| E Onboarding/Settings | macOS-aligned guided setup and section navigation | C + platform rules | Preserve Linux folder/portal semantics, existing grants and auth flows | Settings draft PR #2108; first-run draft PR #2111 |
| F Desktop delivery | GNOME/KDE, Wayland/X11, Flatpak, update/autostart/tray qualification | A; final repeat after C-E | Record observed environment/device matrix, actual tests, unresolved gaps | Draft PR #2103; container checks pass, desktop matrix unqualified |

## Review matrix

For every changed screen cover populated/empty/unavailable/loading, light/dark, narrow/wide, large text, keyboard navigation and canceled operations. Live values never open saved-data evidence. Linux tray is optional and never the only route to an action. Counter-pass/local analytics stays separate from contribution enrollment and upload authority.

Full device matrix work may need a Linux desktop beyond local Colima; missing environments remain explicit blockers, not claimed successes. CI screenshot/log artifact upload was previously rejected by automatic approval review, so this workflow does not add such uploads. Local synthetic captures are marked as component evidence.

## Deliveries already completed

- #2083: shared GTK Insights visual system, merged.
- #2091: live weekly spend and model overview, merged.
- The earlier Linux CI failure was fixed by #2098; baseline Linux qualification now passes.

## Session ledger

- Created isolated workflow checkout from current origin/main.
- Dispatched Linux gate diagnosis, Insights usage implementation and read-only Home design.

### Reassessment 1

- Linux root cause already repaired externally by merged #2098. Hosted Weston at dc522585 passed 604 unit tests, Clippy, mounted navigation, wrapping/focus, CSS and sparse OCR. Evidence: actions/runs/43152689075/job/130965363543. Main 073789798 job130977821782 also passed; do not conflate baseline success with later feature heads.
- Usage lane owns insights usage module and optional worker reads in tc-wt-linux-insights-usage.
- Home lane owns additive Home/model, App wiring, shared balance renderer in tc-wt-linux-home-parity.
- Benchmarks lane owns separate module/tab in tc-wt-linux-insights-benchmarks; integration with usage reviewed later.
- Traces lane owns preview stale-response guard plus additive contextual inspector in tc-wt-linux-traces-inspector; avoid reparenting full preview sheet.
- Settings lane owns grouping/navigation in tc-wt-linux-settings-parity; existing GTK first-run commits differ from macOS and require explicit state-machine work before reordering.
- Desktop lane uses tc-wt-linux-parity-validation for qualification report. No GNOME/KDE real desktop available yet; Flatpak network and broad session-bus permissions require installed-package evidence before altering them.

### Reassessment 2

- Draft PRs: [usage #2101](https://github.com/TraceCommons/trace-commons/pull/2101), [Home #2102](https://github.com/TraceCommons/trace-commons/pull/2102), [portal qualification #2103](https://github.com/TraceCommons/trace-commons/pull/2103), [Benchmarks #2104](https://github.com/TraceCommons/trace-commons/pull/2104), [Traces #2105](https://github.com/TraceCommons/trace-commons/pull/2105). None merged or released.
- A local integration branch combines those five slices. Cargo check passes; registration and lifecycle hooks were independently reviewed. Final combined tests follow remaining integration.
- Review repaired Home missing-value handling and sign-in polling, weekly usage pace fallback, and map connector/layout-test issues. Settings compact navigation reactivation is being repaired before publication.
- User chose container qualification with explicit desktop gaps. Actual GNOME/KDE login, tray, permission behavior and installed Flatpak remain unqualified.
- The worktree helper exhausted eligible cached donors. Map and recap reuse separately verified clean merged donors without active processes; no open-PR checkout or target copy is used.
- Next bounded Traces increment is read-only History detail using existing history_detail and shared copy, with owner/request invalidation. It remains queued behind the current validation and integration.

### Reassessment 3

- [Recap #2106](https://github.com/TraceCommons/trace-commons/pull/2106), [Map #2107](https://github.com/TraceCommons/trace-commons/pull/2107) and [Settings #2108](https://github.com/TraceCommons/trace-commons/pull/2108) are draft PRs. Recap stacks on Usage; Map stacks on Traces. Final review cleared their source changes.
- Combined Cargo check passes with all eight published slices; Map and Settings mounted regressions are both retained in the shared Weston list. The required license-boundary suite passed all four tests. No dependencies changed.
- Hosted Linux desktop jobs passed on Usage, portal response handling, Benchmarks and Traces. Captured frames show empty Overview; they do not establish populated Usage/Benchmarks/inspector visual correctness. Benchmarks navigation is visible and mounted tab switching passes.
- Home exposed a fixture route assumption after the default screen changed, plus Mac-specific copy. Both are under repair; the OCR gate remains required. A pre-existing unreachable portal probe was also discovered and is being repaired in the qualification lane.
- First-run review found wallet and browser enrollment lifecycle hazards when navigating Back. Repairs and regression checks precede publication.

### Reassessment 4

- [First run #2111](https://github.com/TraceCommons/trace-commons/pull/2111) implements Welcome → Tools → Private AI → Traces → Uses → Terms → Join, with shared consent planning, exact-document acceptance and ordered confirmed writes. Its final suite passed 636 tests, with eight display tests ignored locally; configured Clippy and independent auth/consent review passed.
- Home repair `5182bb2a9` explicitly selects Insights in the connected Weston driver and uses device-neutral shared copy. Its 619 regular tests passed; final source review cleared it.
- Portal repair `166e4429b` runs the formerly unreachable independent live probe. Six production-tail regressions failed before the repair and pass afterward; historical green jobs do not establish that probe ran.
- All nine implementation slices now combine at integration commit `8a5fd9cdb`. Review verified the Home/first-run conflict resolution preserves duplicate-sign-in checks, Join lifecycle guards and exact-attempt cancellation. Final combined GTK suite passed 669 tests with 11 display-dependent tests ignored locally; all four license-boundary tests passed again on this integration. Repository-configured all-target Clippy, formatting and shell syntax checks passed; the six portal control-flow tests passed as well.
- No new dependencies or manifest changes. Nothing in this workflow has been merged or released.

### Hosted follow-up

- Home `5182bb2a9`, Usage `66a61307a`, Benchmarks `a4a39bd36`, Traces `c27b09a4e`, Recap `975b5148b`, and Map `0f6e9bef1` passed their Linux desktop jobs. Home and Map mounted tests actually ran under Weston. Empty Overview captures remain limited visual evidence.
- Portal `166e4429b` passed the corrected [live probe job](https://github.com/TraceCommons/trace-commons/actions/runs/43159789245/job/130988129138): the daemon replied with UnknownMethod for the unavailable Background interface. This proves a bounded live service reply, not a user permission grant.
- Settings first hosted run failed its initial wide-layout assertion after a fixed 180ms delay. Follow-up `2ad9e0cca` waits for actual mapped allocation, retains every wide/narrow assertion and adds diagnostic timeout state. Regular suite passed 616 tests; combined test compilation passes. Its [hosted rerun](https://github.com/TraceCommons/trace-commons/actions/runs/43161134691/job/130992455864) passed the actual mounted resize/navigation test in 1.94 seconds, with the original wide/narrow and repeat-selection assertions intact.
- First-run `befaa7cd8` passed its [hosted Linux job](https://github.com/TraceCommons/trace-commons/actions/runs/43160040468/job/130988771554), including new pure planning/Terms tests and existing account-free startup and baseline mounted Insights/OCR checks. There is no mounted seven-step first-run interaction test or first-run screenshot evidence; that qualification remains open.
- Other current checks are not fully green: Usage has an [unchanged macOS layout test failure](https://github.com/TraceCommons/trace-commons/actions/runs/43157845135/job/130981834216); Map has an [unchanged Windows account-admission test failure](https://github.com/TraceCommons/trace-commons/actions/runs/43159159551/job/130986002900) and a downstream missing artifact. These have not been reproduced or established as flakes.

## Execution checkpoint

All nine implementation PRs have passing Linux desktop jobs on their current heads. The workflow and implementation remain draft and unmerged. Combined source passed 669 regular GTK tests, four license tests, six portal script regressions and configured lint/format checks; the later Settings test-only follow-up passed combined test compilation and its hosted mounted test.

This completes the implemented increment across all six lanes, not exhaustive macOS parity. Remaining qualification includes populated screen states, the seven-step onboarding interaction, real GNOME/KDE login/tray/permission behavior and installed Flatpak. Read-only History detail is the next scoped implementation increment. Unchanged-source macOS/Windows failures are tracked above and prevent an all-CI-green claim.
