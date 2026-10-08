# macOS Glass Rebuild and R15 Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the native macOS app's remaining legacy screens (Settings
internals, onboarding internals, Skills, Compute) on the TCDesign glass
system, then make the glass windows the release default (R15) and delete the
legacy main window, menu bar, `CommunityBrand` and `DesignSystem.swift`.

**Architecture:** Four phases, each a PR of its own that leaves the app
shippable. Phases 1-3 replace the *content* of the glass frames that already
exist on `main` (the glass Settings window, the glass first-run pane) with
TCDesign components, keeping every daemon binding and every core-copy source
exactly as the legacy views have them. Phase 4 switches the Monitor stores
to the live daemon client, moves what the legacy main window still owns
(notices, navigation hooks, deep links, undo, quit refusal, Insights, mission
drafts, History withdraw, the Private AI account/tools/switch, the full
preview sheet) into the Monitor, removes the `#if DEBUG` gates, and deletes
the legacy shell. Nothing in any phase authors wording in Swift: the copy
ratchets fix the per-file sentence counts, so every new file is wording-free
and every rebuilt legacy file keeps its path and shrinks to a words table.

**Tech Stack:** Swift 6 package in `macos/` (SwiftUI, AppKit), the `TCDesign`
target (components over `GlassTokens.swift`, generated from
`design-tokens/glass.tokens.json`), `TCShellCore` (pure logic, no FFI),
`TCBridge` (the C ABI), XCTest. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-09-30-macos-glass-design.md` on PR
#1152's branch `macos-glass-spec` (read with
`git show origin/macos-glass-spec:docs/superpowers/specs/2026-09-30-macos-glass-design.md`);
the first-run concept in `docs/superpowers/specs/2026-09-25-ftux-glass-flows-design.md`
on PR #1030's branch `ftux`; the R-numbered roadmap in issue #1173; the
owner decisions of 2026-09-28 (#1030 review), 2026-09-30 (macOS 14 floor)
and 2026-10-02 (native macOS is the lead client) recorded in "Global
Constraints" below. This plan argues from those documents and from the tree
at `origin/main` `f7269c856` (2026-10-02), which it verified directly; where
the task description and the tree disagreed, the tree is reported.

## Owner and design decisions

Nothing below is decided by this plan. Each item names the phase and task it
blocks; a task that is blocked records the sentences or choices it needs and
builds nothing that depends on them. "Default" is what the plan assumes if
the owner says nothing, and is always the choice that claims less.

| # | Decision | Who | Blocks | Default assumed |
|---|---|---|---|---|
| D-1 | **R15 ships with three surfaces hosted as legacy content inside glass frames, or waits for their TCDesign rebuilds.** On `main` the glass Traces tab has no Search/Transcript/Permissions preview, no witness-review consent, no verdict/correction, no surviving-secret line, no arming offer card and no Private AI offer card (`PreviewSheet.swift`, `QueueView.swift`); glass History has no Withdraw, no session detail, no public-run editor (`HomeViews.swift:140`: "Withdrawing a contribution stays in the shipping window until C1 carries the withdrawal call"); the glass Inference tab has no switch, no sign-in and no tool connect (`PrivateInferenceView.swift`, `CredentialSection.swift`, `HarnessListView.swift`; `LiveDaemonClient.setPrivateAI` throws `notAvailableYet`). Phase 4 hosts those legacy views inside glass surfaces so nothing becomes unreachable; they keep `TC.` tokens, so `DesignSystem.swift` cannot be deleted until they are rebuilt (R7, R8, R9 completions, Ron's lane). | Owner, Ron | Phase 4 Task 14 (delete `DesignSystem.swift`) | Host, keep `DesignSystem.swift` as a palette (its `accent` and `primaryFill` already read `GlassTokens.Color.purple`, `DesignSystem.swift:537,594`), delete it in a follow-up PR when the zero-reference test passes. |
| D-2 | **Appearance.** #1173 D2 and the #1152 spec say dark only; `TCDesign` on `main` carries light values and `LightAppearanceTests` asserts both appearances and the HIG rule of following the system. The task description says "light and dark". | Owner, Ron | Every phase's contrast test (which appearances it checks); Phase 4 Task 11 (`TRACE_COMMONS_APPEARANCE`, `TC.forcedColorScheme`). | Follow the system, both appearances, as `TCDesign` does today; tests check both. |
| D-3 | **Folders before Join on a fresh install.** The 2026-09-28 concept orders Join, Folders, Uses. On macOS the daemon refuses to start until the session roots are declared (`AppModel.Startup.needsRoots`), and Join's `enroll` needs the daemon, so `OnboardingCoordinatorView` puts the roots step before Connect by necessity. | Owner | Phase 2 Task 1 (step order and progress labels) | Keep Folders before Join on the fresh-install path; the progress bar shows the real order. |
| D-4 | **The always-on data use.** Today the always-on scope is locked and included (`ConsentScopesContent` RULE 2, "always on" tag). The concept makes it "required but unticked" (rule 3). That is a consent-semantics change. | Owner | Phase 2 Task 5 (Uses step) | Keep locked-and-included, as the daemon's `always_on` flag says. |
| D-5 | **Sharing as a step of onboarding.** The concept adds a Sharing moment (Ask me each time / Share automatically, then the scrub and witness disclosures, then the grant). Today onboarding deliberately withholds arming (`OnboardingProjectsView` doc comment) and the arming offer lives in the Queue. The core already exports the sharing card's words (`tc_automatic_contribution_copy_json`, `BRIDGE_ONLY` in the copy ratchet) but no macOS screen renders them, and the grant call path on macOS (`acceptArmingOffer`) is the Queue's. Also: the Quick/Custom tiers, the Tools step ("add your tool", needs daemon source detection, not built) and the Rules step (per-repo rule plus past-session selection, no backend). | Owner, Kristi (K-lane wiring), Ron | Phase 2 Task 10 | No Sharing step, no tiers, no Tools or Rules steps in Phase 2; Task 10 records the DRAFT sentences and the wiring needed. |
| D-6 | **Private AI in onboarding.** The concept shows the Private AI switch on Custom setup's Uses step as a consent event. Today the first-run Private AI offer is a card on the Queue (`PrivateInferenceOfferCard`), not an onboarding step. | Owner | Phase 2 Task 10; Phase 4 Task 7 (where the offer card lives in glass) | Offer card stays where the Queue put it, hosted in the Traces tab. |
| D-7 | **Where Skills lives.** No glass design exists. `SkillLearningView` is per-history-record and lives inside `SessionDetailView`. | Ron, owner | Phase 3 Task 2 | The History page's inspector, for the selected row. **Settled 2026-10-06 (Ron): as built in #1258.** |
| D-8 | **Compute controls' look.** No glass design exists; the spec only says "a Settings section, including pause, resume and withdraw". | Ron | Phase 3 Task 1 (component choices only; bindings are fixed) | `GlassCard` with the core's sentences, `GlassTextField` for the allowance, `GlassButtonStyle(.glass)` for the four controls. **Settled 2026-10-06 (Ron): four glass buttons, as built in #1258.** |
| D-9 | **Missions catalogue in release.** `LiveDaemonClient.missionCatalogue` throws `notAvailableYet`; `MissionsPage` says it "stays in the debug window until" the core's disclosure exists (M4). | Owner | Phase 4 Task 8 | Ship the page; it draws the core's `requestFailed` line for `notAvailableYet`, which is the fail-closed state. |
| D-10 | **Menu-panel override rows.** `MenuBarGlassPanel.modeOptions` draws the three overrides disabled ("setting an override needs its confirmation ... and a client write the data contract does not carry yet"). Ship disabled, or hide until the write exists? | Owner, Kristi | Phase 4 Task 9 | Ship disabled, as built. |
| D-11 | **Insights without the daemon.** `MainWindowNavigation.activateServicesIfNeeded` never starts services while the main window rests on Insights or Mission drafts (local, account-free); the Monitor starts services on appear ("reads the daemon whatever the main window shows"). With the Monitor as the main window, an Insights-only launch starts the watcher. | Owner | Phase 4 Task 3 | Services start when the Monitor opens; the Insights-only mode is lost and said so in the PR. |
| D-12 | **Pause-duration words.** The glass panel reads `MenuBarContent.pauseHourLabel`, `pauseMorningLabel`, `pauseIndefiniteLabel`, `resumeLabel`, `pauseLabel` from `MenuBarView.swift` (Swift-authored; three count as sentences in `ShellWordingTests`). `MonitorScreensCopy` has `watching` and `paused` but no durations. A core export (`tc_monitor_screens_copy_json` gaining `pauseHour`, `pauseMorning`, `pauseIndefinite`, `resume`, `pause`) is Kristi's lane. | Kristi | Phase 4 Task 9 (deleting the last of `MenuBarView.swift`) | `MenuBarView.swift` shrinks to a `MenuBarWords` table holding exactly those words (baseline lowered from 11 to 3) until the export lands. **Done 2026-10-06:** the words are the core's shell table (`MonitorShellCopy`, #1146's `tray.rs` words) and the baseline entry is deleted. |
| D-13 | **Who lands the live-client switch.** `MonitorWindowView` builds its three stores on `DaemonDataWiring.sample(...)`; only `MenuBarStripLabel` attaches `model.daemonData`. Switching the call site is the one-line change `DaemonDataWiring`'s doc describes, in Kristi's K1 lane. | Owner, Kristi | Phase 4 Task 1 | This plan's Task 1 does it; tell Kristi before merging. |
| D-14 | **An invite link while already onboarded.** Today `AppDelegate.application(_:open:)` parks the invite in `PendingInvite` and opens the main window; only `OnboardingConnectContent` consumes it, so an onboarded person's link is never consumed. | Owner | Phase 4 Task 3 | Same behaviour: open the Monitor; the invite stays parked. Said in the PR. |
| D-15 | **The public-profile panel in glass.** `SettingsView.profilePanel` and `GoPublicDialog` are drawn in `CommunityBrand` on purpose: "the black frame is the exact boundary of what becomes public" (DESIGN-SPEC §7.3). TCDesign has no brand-seam surface. | Ron | Phase 1 Task 4 | A `GlassCard` with a `GlassTag(_, tone: .accent)` eyebrow reading the core's `PublicProfileCopy.heading`; the go-public dialog as a `GlassSheet`. |
| D-16 | **The Welcome hero in glass.** `OnboardingWelcomeContent` is the community-brand hero (ink frame, Helvetica display ladder, wireframe globe). | Ron | Phase 2 Task 2 | A `GlassPane` with the `display` type step for the headline and no globe. |
| D-17 | **Settings window chrome.** #1173 D8 says the stock Settings window, "theming may follow". Phase 1 keeps `MonitorSettingsWindow`'s `NavigationSplitView` list and only rebuilds the detail content. | Ron | Phase 1 (all) | Stock list, glass content. |
| D-18 | **Development hooks kept under `#if DEBUG`.** `TRACE_COMMONS_MENU_PREVIEW`, `TRACE_COMMONS_SAMPLE`, `SampleDaemonClient`, `GlassGallery`, `TRACE_COMMONS_SCREENSHOT_DIR`, `TRACE_COMMONS_APPEARANCE`. They are not release switches. | Owner | Phase 4 Task 11 | Keep them, debug builds only. |
| D-19 | **Terminal status wording.** "Status unavailable" for an unknown status is the owner's rule; the word must come from the core. `MonitorScreensCopy.unknown` exists ("unavailable" for the badge). Whether it is the sentence for a History row's status, or a new export is needed, is the core's call. | Owner, Kristi | Phase 4 Task 6 | Reuse `MonitorScreensCopy.unknown`; no Withdraw on any status `ContributionStatusPresentation.offersWithdraw` refuses. |
| D-20 | **Button size in an action bar.** `GlassButtonStyle` had one size per kind: the glass and destructive pills were `size.controlLarge` (28pt, label type, 12pt padding) wherever they sat, so the first-run footer's Back and Customize stood shorter than Continue (`size.cta`, 34pt, bodyStrong bold, 16pt padding). | Owner | Every action bar | **Settled 2026-10-08 (owner ruling): in a window's action bar (the footer row that holds the primary CTA, in a window, a sheet or a modal) every button is the primary CTA's size; only inline buttons (inside cards, rows, fields, popovers, toolbars) use the smaller size.** Built as `GlassButtonStyle(_:size:small:selected:)` with `GlassButtonSize.inline` (the default) and `.bar`: a `.bar` button takes the CTA's height, type size and horizontal padding and keeps its kind's fill, edge, ink, weight, hover, pressed and disabled look. With `small` it matches the small CTA (30pt), which is what `GlassModal`'s action row uses for every action. Applied to the first-run footer (Back, Customize, Cancel), `GlassModal`'s action row and the preview sheet's Close. `ButtonSizeTests` and `FirstRunFrameTests.test_everyFooterButtonIsTheCTAsSize` hold it; the gallery's Controls section shows a bar beside inline buttons. |

## Global Constraints

Owner decisions, verbatim, and the repo's rules. Every task's requirements
include this section.

- Native macOS is the lead client and the parity target (2026-10-02).
- Purple everywhere: on macOS this comes from moving to TCDesign, so do not
  recolour legacy screens that are about to be retired.
- Folder modes are "Ask me / Automatic / Never". The words come from the
  core's `tc_contribution_mode_copy_json` (`ContributionModeCopy.choices`),
  never from `ProjectCopy.modeChoiceLabel`.
- An unknown status reads "Status unavailable", and is terminal with no
  Withdraw.
- Never is an absolute exclusion. An Auto override is a grant.
- Fail closed: an absent signal never renders as healthy. A dot, a tag or a
  count with no core answer is absent or "—", never "on", never zero.
- The core owns copy. New consent wording is DRAFT, NEEDS APPROVAL, listed
  as sentences and never built.
- "Private AI" is the destination's name; it is read from
  `PrivateInferenceCopy.destination`, never typed. No Swift literal says
  "private inference" (`swift_never_says_private_inference_to_a_contributor`).
- Minimum macOS is 14 (`.macOS(.v14)` in `macos/Package.swift`), with the
  Liquid Glass fallback before macOS 26. Screens never branch on the OS; only
  `TCDesign`'s tier code does (`glassSurface`, one call site).
- Follow Apple HIG: system focus ring; Reduce Transparency, Increase
  Contrast and Reduce Motion left to the system where it adapts
  (`GlassMotion.systemReducesMotion`, `GlassRGBA.adaptive(highContrast:)`,
  `paneOpaque` under Reduce Transparency).
- Contrast is 4.5:1 for text and 3:1 for UI.
- **Wording ratchet.** `macos/Tests/TCShellCoreTests/ShellWordingTests.swift`
  records, per file path, exactly how many Swift-authored sentences a file
  holds; the count is a ceiling and a floor, no number may be raised, and no
  file may be added. Therefore: every new glass file authors no sentence; a
  rebuilt legacy file keeps its path and holds its sentences verbatim in a
  words table, with its count unchanged; a count is lowered only when the
  sentence left the shell for the core. Measure with
  `TC_WORDING_DUMP=1 swift test --filter ShellWordingTests` before and after
  every task.
- **Copy ratchet.** `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs`
  pins core sentences (none may be a Swift literal) and pins, by file path,
  which screen calls which bridge function (`SURFACES`). Re-pointing a path
  when a screen moves is correct; changing a sentence or dropping a row is
  not. Run `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central`
  after every task that moves a file.
- No new dependencies. No emojis. Commit subjects are short imperatives
  without prefixes. Every commit ends with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Sample data (`SampleDaemonClient`, `DaemonDataWiring.sample`,
  `TRACE_COMMONS_SAMPLE`) is `#if DEBUG` and never reaches a release build.
- Tests are the house pattern: pure functions and source-scan XCTests over
  `macos/Sources`. Agents cannot drive the UI; the owner's manual checklist
  in Phase 4 covers what only a person can see.
- Build and test from the repo root first: `cargo build -p trace-commons-contributor-ffi`
  (the Swift package links the dylib from `../target/debug`), then from
  `macos/`: `swift test --filter <Suite>`. Full gates before each PR:
  `cd macos && swift test`, `python3 scripts/design-tokens/generate.py --check`,
  and the copy ratchet above.

## Review Focus

Inputs the spec implies and no task's tests exercise on their own. Each line
has its test pinned to the task named.

1. **Settings opened before the daemon answers** (a fresh launch, ⌘, within
   the first second): every section must draw its own unavailable or loading
   state and no control may read as working. Pinned in Phase 1 Task 10
   (`test_everySectionHasAnUnavailableBranch`).
2. **A consent write the daemon refuses mid-flight** (socket drops between
   press and reply): the ticks must follow the daemon's `status.consent_scopes`,
   never the press. Pinned in Phase 1 Task 3
   (`test_theTickReadsTheDaemonNeverTheDraft`).
3. **Reduce Transparency on with hosted legacy content** (D-1): `tcScreen()`
   paints an opaque `TC.ground` inside a glass pane, which reads as a hole.
   Pinned in Phase 4 Task 5 (`test_hostedLegacyContentPaintsNoGround`).
4. **A notification, Dock click or invite link arriving before the Monitor's
   open handler is installed** (cold start): the request must be held and
   replayed, as `OpenMainWindow` does today. Pinned in Phase 4 Task 3
   (`test_aRequestBeforeTheHandlerIsReplayedOnce`).
5. **A History row whose status this build cannot name**: reads "Status
   unavailable" from the core, offers no Withdraw, and is not drawn as
   rejected. Pinned in Phase 4 Task 6
   (`test_anUnknownStatusIsTerminalAndOffersNoWithdraw`).

---

## What `origin/main` holds (verified 2026-10-02 at `f7269c856`)

Read before planning; the task description's situation summary was checked
against the tree and corrected where it differed.

- **Glass is debug-only.** `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift`
  gates with `#if DEBUG`: `MenuPanelStore`, the glass `MenuBarExtra`
  (`TRACE_COMMONS_GLASS_MENU=1`), `MonitorWindowCommands`, the Monitor window
  (`TRACE_COMMONS_MONITOR=1`), the menu-bar preview window
  (`TRACE_COMMONS_MENU_PREVIEW=1`), the first-run window
  (`TRACE_COMMONS_FIRST_RUN=1`) and the `Settings` scene
  (`MonitorSettingsWindow`). Whole files are `#if DEBUG`:
  `Views/MonitorWindowView.swift`, every file under `Views/Monitor/`,
  `TCShellCore/DataContract/SampleDaemonClient.swift`,
  `SampleDaemonData.swift`, `TCDesign/Gallery/GlassGallery.swift`.
- **The Monitor runs on sample data.** `MonitorWindowView.tracesStore()`,
  `dataClient()` build `TracesStore`, `InferenceStore` and `HomeStore` on
  `DaemonDataWiring.sample(set)`. Only `MenuBarStripLabel` attaches the live
  client (`store.attach(model.daemonData)`), which
  `MenuBarGlassPanelTests.test_thePanelUsesTheLiveClient` pins.
- **The live client answers `notAvailableYet`** for `inference_summary`,
  `inference_call_proof`, `model_spend`, `private_ai`, `set_private_ai`,
  `mission_catalogue`, `invite_lookup`, `passkey_state`,
  `account_session_status` (`TCShellCore/DataContract/LiveDaemonClient.swift:218-251`).
- **Legacy-brand references per file** (`git grep -c 'CommunityBrand\|\bTC\.' origin/main -- macos/Sources/TraceCommonsApp`):
  `SettingsView.swift` 227, `PreviewSheet.swift` 204, `MainWindowView.swift`
  150, `QueueView.swift` 133, `SkillLearningView.swift` 101,
  `HistoryView.swift` 98, `SessionDetailView.swift` 78,
  `SessionContributionOverview.swift` 69, `OnboardingWelcomeView.swift` 55,
  `HarnessListView.swift` 49, `DesignSystem.swift` 48,
  `CreditRecordView.swift` 41, `PrivateInferenceView.swift` 21,
  `QueueFolderRow.swift` 19, `CommunityBrand.swift` 18,
  `ConsentScopesView.swift` 17, `RouteDisclosureView.swift` 15,
  `SourceRootRow.swift` 13, `ComputeView.swift` 13, `MenuBarView.swift` 12,
  `OnboardingConnectView.swift` 11, `OnboardingPrivacyScanView.swift` 10,
  `OnboardingDoneView.swift` 10, `CertificateSection.swift` 9,
  `ScrubbingCaveat.swift` 9, `OnboardingProjectsView.swift` 9,
  `BalanceRow.swift` 9, `OnboardingRootsView.swift` 8,
  `CredentialSection.swift` 8, `BrandMark.swift` 8,
  `TraceCommonsAppMain.swift` 7, `WhatGetsRemovedSheet.swift` 6,
  `ActionMessageBanner.swift` 5, `NativeFlowNotice.swift` 3,
  `FundingRow.swift` 3, `NearAiJoinView.swift` 2,
  `NearAccountConnectView.swift` 2, `AdmissionPreparationView.swift` 2.
  Zero in every `Views/Monitor/*` file and in `MonitorWindowView.swift`.
  `InsightsView.swift`, `MissionDraftsView.swift` and their sub-views use
  plain SwiftUI with no `TC.` at all.
- **What the glass windows replace today, and what they do not.**
  - Traces tab (`TracesViews.swift`, `TracesStore.swift`): tree, folder
    modes with the core's arming and ignore confirmations, per-session
    Contribute / Keep / Dismiss with Undo (`ReviewAction`), a preview
    summary with redaction categories, eligibility. Not there: `PreviewSheet`
    (Search, What's in it, Transcript with `transcript-copy-all`, Permissions
    tabs; witness review consent; verdict and correction on approve;
    `AdmissionPreparationView`; `SessionSendDisclosureView`;
    `CertificateSection`), `QueueView`'s `UndoBar` outside the inspector,
    `PrivateInferenceOfferCard`, `ArmingOfferCard`, Submit all (and as a
    verdict), `NotOfferedDisclosure`, `WeekBand`, the surviving-secret line
    (`TCCoreCopy.residualSecretLine`, only in `QueueView.swift:434`).
  - Home and History (`HomeViews.swift`, `HomeStore.swift`): overview, credit
    with the commons' condition, history rows, summary inspector, Missions
    catalogue page. Not there: Withdraw and its confirmation
    (`HistoryRow`, `SessionWithdrawalAction`, `WithdrawalConfirmationView`,
    `WithdrawalCopy` 48 sentences), `SessionDetailView` (public-run editor,
    `SkillLearningView`), `CommunitySection` (roster), `CreditRecordView`,
    quarantine explanations (`HistoryView.contributorFacingExplanations`).
  - Inference tab (`InferenceViews.swift`): the call ledger and the tools
    inspector with the core's sentences. Not there: the switch with
    `offerWhat`/`offerExposure`, `CredentialSection` (sign-in, provider
    picker, balance, funding), `HarnessListSection` (connect/disconnect,
    exposure sheet, preview sheet).
  - Menu bar (`MenuBarGlassPanel.swift`): pills, sub-lists, legend, graph,
    recent activity, menu items; writes only pause/resume/Private AI off; it
    still reads `MenuBarContent.pause*Label` and navigates with
    `MainWindowView.Section`.
  - First run (`FirstRunViews.swift`): the glass pane and `GlassStepProgress`
    around the unchanged `OnboardingCoordinatorView`.
  - Settings (`MonitorSettingsWindow`): the section list around the
    unchanged `SettingsContent(section:)` and `ComputeView`.
- **The legacy main window's duties** (`MainWindowView.swift`, `AppDelegate.swift`,
  `TraceCommonsAppMain.swift`): `ShellNotices` (attached-daemon, grant void,
  arming rewording, gate held, legacy migration cards); `Section`
  navigation and ⌘1-7 / ⌘⇧M `MainWindowCommands`; the watch chip and pause
  menu; `OpenMainWindow` (pending-until-handler) used by notifications'
  Review, Dock reopen, invite deep links (`PendingInvite`), quit refusal
  (`navigation.section = .compute`) and `TRACE_COMMONS_SHOW_WINDOW`; the
  onboarding gate (`model.requiresOnboarding` → `OnboardingCoordinatorView`);
  Insights (`InsightsView` with `InsightsStoreSelection` from
  `CommandLine.arguments`) and Mission drafts (`MissionDraftsView`);
  `DebugScreenshot` renders eleven legacy views.
- **Ratchets that name legacy files.** `ShellWordingTests.wordingBaseline`:
  `SettingsView.swift` 40, `MainWindowView.swift` 14, `MenuBarView.swift` 11,
  `ConsentScopesView.swift` 7, `OnboardingWelcomeView.swift` 8,
  `OnboardingConnectView.swift` 7, `OnboardingCoordinatorView.swift` 5,
  `OnboardingDoneView.swift` 8, `OnboardingProjectsView.swift` 4,
  `OnboardingRootsView.swift` 5, `QueueView.swift` 25, `QueueFolderRow.swift`
  3, `HistoryView.swift` 26, `PreviewSheet.swift` 38, `CreditRecordView.swift`
  9, `ProjectRow.swift` 3 (`ProjectCopy.modeChoiceLabel`), and others.
  `swift_copy_surface_is_central.rs` `SURFACES` pins `MainWindowView.swift`
  (grant void, arming rewording), `SettingsView.swift` (arming confirmation),
  `QueueView.swift` (arming offer, surviving secret), `PreviewSheet.swift`
  (consent gate, gate help, scrubbing panel), `OnboardingPrivacyScanView.swift`,
  `QueueFolderRow.swift`, `WithdrawalCopy.swift`, `MonitorWindowView.swift`,
  `Monitor/TracesStore.swift`, `Monitor/InferenceViews.swift`.
- **Tests that name legacy views.** `SettingsSectionsTests` and
  `SettingsPointerTests` read `SettingsView.swift`;
  `ShellNoticesPlacementTests` reads `MainWindowView.swift`;
  `ComputeNavigationTests` uses `MainWindowView.Section` and renders
  `ComputeContent(model:allowance:)`; `PrivateInferenceMenuBarTests` calls
  `MenuBarContent.privateInference*`; `QuarantineExplanationTests` calls
  `HistoryView.contributorFacingExplanations` and `HistoryView.heldReviewBody`;
  `AccentContrastTests` and `LineHeightScalingTests` read `TC.*`;
  `MenuBarGlyphRenderTests` renders `MenuBarGlyph`; `ActionNoticeDismissTests`
  names `SettingsView.loginItemActionError`, `SettingsView.consentSaveError`,
  `OnboardingRootsView.failure`; `NativeOnboardingRenderTests` renders
  `NearAccountConnectView`, `AdmissionPreparationView`; `OnboardingNavigationTests`
  uses `OnboardingCoordinatorView.Step`; `FirstRunProgressTests` reads
  `FirstRunViews.swift`; `MenuBarGlassPanelTests.test_thePanelUsesTheLiveClient`
  reads `TraceCommonsAppMain.swift`.
- **CI.** `.github/workflows/clients.yml`: `macos-app-tests` on `macos-26`
  runs the whole Swift suite after `cargo build -p trace-commons-contributor-ffi`;
  `macos-15-fallback-tests` on `macos-15` runs `swift test --filter TCDesignTests`
  only. `release-apps.yml` builds the DMG with `macos/scripts/make-release-dmg.sh`
  → `make-app-bundle.sh release`, which runs `swift build --configuration
  release`; no env flag reaches the shipped app. `macos/scripts/run-demo.sh`
  passes `TRACE_COMMONS_SHOW_WINDOW`, `TRACE_COMMONS_MONITOR`,
  `TRACE_COMMONS_SCREENSHOT_DIR`, `TRACE_COMMONS_SELFTEST_OUT`,
  `TRACE_COMMONS_APPEARANCE`, `TRACE_COMMONS_QUIT_AFTER_SHOT`,
  `TRACE_COMMONS_DEMO_PREVIEW`, `TRACE_COMMONS_CONTRIBUTOR_DIR`.
- **Accessibility identifiers in the sources.** Exactly one:
  `"transcript-copy-all"` in `PreviewSheet.swift`. No test reads it.

## Phase overview

Each phase is one PR, shippable on its own, leaving `main` with the legacy
shell still the release default until Phase 4.

| Phase | Delivers | Tasks |
|---|---|---|
| 1 | Settings sections rebuilt in TCDesign under `Views/Settings/`, drawn by `MonitorSettingsWindow`; `SettingsView.swift` reduced to its words table; parity test | 10 |
| 2 | Onboarding steps rebuilt in place in TCDesign, under the existing glass first-run pane; the concept's deltas recorded as decisions and DRAFT sentences | 10 |
| 3 | Compute section and the Skills panel on TCDesign surfaces in the Monitor; design questions recorded | 3 |
| 4 | R15: live client, notices, navigation hooks, gap closures, release default, deletion of the legacy shell and `CommunityBrand`; `DesignSystem.swift` deletion gated on D-1; rollback and the owner's manual checklist | 15 |

Phases 1-3 can be built in parallel (disjoint files) but merge in order,
because Phase 2 reuses Phase 1's `GlassSourceRow` and Phase 4 reuses both.

## File structure

New files are wording-free. Rebuilt legacy files keep their paths.

```
macos/Sources/TraceCommonsApp/
  Views/Settings/                       (Phase 1, new)
    GlassSettingsContent.swift          the section switch MonitorSettingsWindow draws
    ConnectionSection.swift
    StartupSection.swift                login item, notifications, updates
    ConsentSection.swift
    PublicProfileSection.swift          + GoPublicSheet
    WatchingSection.swift
    WatchedFoldersSection.swift
    GlassSourceRow.swift                shared with the Folders step (Phase 2)
    ToolsSection.swift                  IronWire routing
    PrivateAISection.swift              pointer + RouteDisclosureGlassBody
    WitnessSection.swift
    ProjectsSection.swift               folder modes from ContributionModeCopy
    ChangesSection.swift                the audit log
  Views/SettingsView.swift              (Phase 1) shrinks to SettingsLegacyWords, count 40
  Views/Onboarding*.swift, ConsentScopesView.swift, NearAiJoinView.swift,
    NearAccountConnectView.swift, AdmissionPreparationView.swift,
    WhatGetsRemovedSheet.swift          (Phase 2) rebuilt in place on TCDesign
  Views/Monitor/FirstRunViews.swift     (Phase 2) progress labels follow the step order
  Views/ComputeView.swift               (Phase 3) rebuilt in place; ComputeContent(model:allowance:) kept
  Views/SkillLearningView.swift         (Phase 3) rebuilt in place; SkillLearningView(record:copy:) kept
  Views/Monitor/HistoryInspector.swift  (Phase 3, new) the selected history row's inspector
  Views/ShellNotices.swift              (Phase 4, new) ShellNotices and the notice cards on TCDesign
  MonitorNavigation.swift               (Phase 4, new) MonitorDestination, OpenMonitor, LaunchRouting
  Views/Monitor/TracesOffers.swift      (Phase 4, new) hosts the offer cards and the surviving-secret line
  Views/Monitor/InferenceAccount.swift  (Phase 4, new) hosts CredentialSection, HarnessListSection, the switch
  TraceCommonsAppMain.swift             (Phase 4) release default, no window gates
  MainWindowView.swift, MenuBarView.swift (Phase 4) deleted; MenuBarView.swift keeps MenuBarWords until D-12
  Views/CommunityBrand.swift, Views/BrandMark.swift (Phase 4) deleted
  Views/DesignSystem.swift              (Phase 4, Task 14) deleted when D-1 allows
macos/Sources/TCShellCore/
  Format.swift                          (Phase 4, new) bytes/when/tomorrowMorning, moved from MenuBarView.swift
  PrivateInferenceTray.swift            (Phase 4, new) the tray action rule, moved from MenuBarContent
macos/Tests/TraceCommonsAppTests/
  GlassSurfaceRulesTests.swift          (Phase 1, new; grows each phase) the cross-cutting scan
  SettingsParityTests.swift             (Phase 1, new)
  OnboardingParityTests.swift           (Phase 2, new)
  ComputeSkillsParityTests.swift        (Phase 3, new)
  MonitorNavigationTests.swift          (Phase 4, new)
  LegacyShellRetiredTests.swift         (Phase 4, new)
```

### The cross-cutting scan (every phase adds its files)

`GlassSurfaceRulesTests` reads each glass file and asserts the rules the
owner set that a scan can check. It is the one place the Reduce
Transparency, contrast and Reduce Motion rules are tested for app files
(the tokens themselves are tested in `TCDesignTests`). Phase 1 Task 1
creates it; later tasks append paths to `files`.

```swift
import XCTest

/// Rules every glass surface in the app target obeys, checked by reading the
/// source. The list grows as screens move to TCDesign; a file on it may not
/// paint its own colours, its own materials or its own motion, because those
/// are what `TCDesign` adapts for Reduce Transparency, Increase Contrast and
/// Reduce Motion, and a screen that paints its own bypasses all three.
final class GlassSurfaceRulesTests: XCTestCase {
    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    /// Glass files, relative to `Sources/TraceCommonsApp`. Append, never remove.
    static let files: [String] = [
        "Views/Settings/GlassSettingsContent.swift",
    ]

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// No legacy palette: the glass tokens carry the contrast floors and the
    /// Increase Contrast values; `TC.` and `CommunityBrand` carry neither.
    func test_noLegacyPalette() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(rel) reads TC.")
            XCTAssertFalse(source.contains("CommunityBrand"), "\(rel) reads CommunityBrand")
        }
    }

    /// No colour of its own: a hex literal or a system colour name bypasses
    /// the 4.5:1 and 3:1 floors `TextContrastTests` and
    /// `SelectionContrastTests` check on the tokens.
    func test_noColourLiterals() throws {
        let banned = [#"0x[0-9A-Fa-f]{6}"#, #"Color\(red:"#, #"\.foregroundStyle\(\.red\)"#,
                      #"\.foregroundStyle\(\.green\)"#, #"\.foregroundStyle\(Color\.(red|green|orange|yellow)"#]
        for rel in Self.files {
            let source = try Self.text(rel)
            for pattern in banned {
                XCTAssertNil(source.range(of: pattern, options: .regularExpression), "\(rel) paints \(pattern)")
            }
        }
    }

    /// No material of its own: Reduce Transparency is honoured inside
    /// `glassSurface` and the pane backdrop, nowhere else.
    func test_noMaterialOfItsOwn() throws {
        let banned = [".ultraThinMaterial", ".thinMaterial", ".regularMaterial", ".thickMaterial",
                      "NSVisualEffectView", ".glassEffect("]
        for rel in Self.files {
            let source = try Self.text(rel)
            for token in banned { XCTAssertFalse(source.contains(token), "\(rel) uses \(token)") }
        }
    }

    /// Every animation on a glass file is gated on Reduce Motion, the way
    /// `MenuBarGlassPanel` gates its slide.
    func test_everyAnimationHonoursReduceMotion() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            for line in source.split(separator: "\n") where line.contains(".animation(") || line.contains("withAnimation(") {
                XCTAssertTrue(line.contains("reduceMotion") || line.contains("GlassMotion"),
                              "\(rel) animates without Reduce Motion: \(line.trimmingCharacters(in: .whitespaces))")
            }
        }
    }

    /// No fixed point size: type follows the macOS text-size setting through
    /// `glassType`; a literal size does not.
    func test_noFixedPointType() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            XCTAssertNil(source.range(of: #"\.font\(\.system\(size:"#, options: .regularExpression), "\(rel) fixes a point size")
        }
    }

    /// Every status dot and status label carries words: a colour alone is
    /// not a state (HIG, and the owner's fail-closed rule).
    func test_everyDotHasWords() throws {
        for rel in Self.files {
            let source = try Self.text(rel)
            XCTAssertFalse(source.contains("GlassStatusDot("),
                           "\(rel) draws a bare dot; use GlassStatusLabel or pair the dot with its sentence")
        }
    }
}
```

VoiceOver and keyboard are covered per task by the rule that every control
is a TCDesign component (which carry their accessibility obligations, spec
"Components") or a system control, plus `accessibilityElement(children: .combine)`
on composed rows; the parity tests check those calls are present where the
legacy view had them.

---

## Phase 1: Settings internals in TCDesign

**Spec:** #1152 "Settings navigation" (the section list and grouped,
independently scrolling content; Connection, Startup & notifications,
Watching, How traces may be used, Public profile, Watched folders, Tools,
Private AI, Redaction witness, Projects, Changes on this machine, plus
Compute); #1173 R11; D-15, D-17.

**Inventory of the legacy sections** (`Views/SettingsView.swift`,
`SettingsContent.body`, in the order drawn):

| Order | `SettingsSection` | Legacy body | Controls | Bindings read | Writes | Confirmations | Notices | Copy sources |
|---|---|---|---|---|---|---|---|---|
| 1 | `.connection` | `connection` | none | `model.status.loggedIn`, `model.daemonSettings?.routingSourceModes.{claude,codex,gemini,cline}`, `nearAIConfigured` | — | — | — | `TCSourceChecks.checkLine(tool:sourceMode:)`, `TCSourceChecks.{claude,codex,gemini,cline}`; Swift: "Connected", "Not connected", "Sessions are being queued, but nothing can be sent.", "Extra privacy scan configured" |
| 2 | `.startup` | `loginItem` | `Toggle(.switch)` | `LoginItemManager.currentState` (read on appear) | `LoginItemManager.register()`, `.unregister()` | — | `loginItemActionError` | Swift: "Start Trace Commons when you log in", "Waiting on approval in System Settings.", "Turn it on in System Settings -> General -> Login Items ...", "Couldn't turn this on/off: ..." |
| 2 | `.startup` | `notifications` | `Button` (allow), `Link` (System Settings) | `Notifier.shared.authorizationStatus()` on `.task` and on `didBecomeActiveNotification` | `Notifier.shared.requestAuthorization()` | — | — | `Notifier.copy?.{notificationHeading, notificationAllowed, notificationDenied, notificationNotAsked, notificationAllow, notificationUnknown, systemSettings}`, `Notifier.purpose`, `Notifier.systemSettingsURL` |
| 2 | `.startup` | `updatesSection` | `Button("Check Now")`, `Button("Copy")` | `UpdateController.shared.{currentVersion, mode, lastCheckDate, canCheckNow}` | `updates.checkNow()`, `NSPasteboard.general.setString` | — | — | `UpdatePolicy.{noFeedReason, insecureFeedReason}`; Swift: "Checks daily", "Trace Commons checks for updates automatically and asks before installing.", "Updates managed by Homebrew", "Homebrew installed this copy, so Homebrew replaces it. Run this in a terminal:", "Updates unavailable", "Not checked yet on this machine.", "Last checked ...", three disabled-feed sentences, "Updates are turned off for this build." |
| 3 | `.consent` | `consent` | one `Button` per scope with `TCReadGateCheckbox` | `model.consentScopes` (`alwaysOn`, `grantsDataUse`, `description`), `model.status.consentScopes`, `model.status.loggedIn` | `model.setConsentScopes([String])` (always-on ∪ daemon-reported ± this scope) | — | `consentSaveError` (`TCSourceChecks.settingsCopy()?.consentSaveFailed`), `consentBusy` | `ScopeCopy.title(for:options:)`; Swift: heading "How may your traces be used?", "Applies to traces you send from now on.", "Always included", "Optional — each one lets your traces do more", "Credit", "always on", "Nothing here is pre-selected on your behalf.", a11y "Granted"/"Not granted" |
| 4 | `.publicProfile` | `publicProfile` | `TextField` (handle), `TextEditor` (bio), Save, Leave, opt-in button, `GoPublicDialog` sheet with acknowledgement | `model.publicProfile?.{handle, bio, publicSince}`, `model.profileBusy`, `model.profileOutcome` | `model.claimHandle(_:bio:)`, `model.leaveRoster()`, `model.clearProfileOutcome()` | go-public sheet gated on `acknowledged && handle non-empty && !profileBusy` | `profileOutcomeSentence`, `PublicProfileCopyCheck.failures()` | `PublicProfileCopy.*`; byte counter `utf8.count/280`; Swift: the seven consent-column lines, "Do not trust the public-profile wording on this screen." |
| 5 | `.watching` | `watching` | none | `settings.quiescenceSecs`, `digestIntervalSecs`, `queueTtlDays`, `localNotifications`, `model.status.paused` | — | — | — | Swift: four sentences with figures, "Paused. Nothing is being queued or sent." |
| 6 | `.watchedFolders` | `watchedFolders` | `SourceRootRow` per `SourceKind.allCases`, Retry | `TCSourceChecks.settingsCopy()`, `model.daemonSettings` modes incl. `opencodeSourceMode`, `TCDiscovery.sourcesJSON()` → `SourceCandidate.decodeList` | `model.setSourceRoot(kind, choice)`, `model.refreshSettings()` | folder panel (`NSOpenPanel`) | `sourceSaveFailed` (`copy.saveFailed`), `copy.unavailable` | all from `SourceSettingsCopy` |
| 7 | `.tools` | `routing` | `Toggle(.switch)`, Connect, Look again, `DisclosureGroup` with port `TextField` and folder chooser, Apply | `model.routingCopy`, `model.routingForm`, `model.routingDiscovery`, `model.routingChecking`, `model.routingProbeLine`, `model.status.routing.{state, derived, lastRefreshAt}`, `model.routingEvidence`, `model.routingCalls`, `model.daemonSettings?.routingSourceModes` | `model.applyIronWire(form)`, `model.discoverRouting()`, `model.refreshRoutedTools()` | port guard `UInt16(exactly:) > 0`; `NSOpenPanel` | `routingProbeLine` | `RoutingSurface.{toolRows, discoveryLine, connecting, overrideIsCollapsed, stateLine, tone(forState:), showsLastChecked}`, `TCRoutingCopy.lastChecked(when:)`, every string on `RoutingCopy` |
| 8 | `.privateAI` | `privateInference` + `RouteDisclosureSection` | pointer `Button(copy.destination)` | `model.privateInferenceCopy?.{settingsTitle, settingsMoved, destination}`, `model.routeDisclosureState`, `model.routeDisclosureUnreadableCopy` | `navigation?.section = .privateInference` (Phase 4 retargets), `model.refreshRouteDisclosure()` | — | unreadable line with glyph | `RouteDisclosure.copy.*`, facts |
| 9 | `.witness` | `witness` | three fields (`url`, `signingAddress`, `measurements` editor), Configure, Clear, inference Enable/Disable, token Enable/Disable, capture, cleanup, discard | `model.witnessCopy`, `model.witnessState`, `model.witnessStateCode`, `model.witnessStatus?.{refusal, pinnedMeasurementLine}`, `model.witnessLabel`, `model.witnessBusy`, `model.witnessCalls`, `model.daemonSettings?.{ironwireAttestedBodies, tokenDistributionsContribution, tokenStorage}`, `model.inferenceEvidenceBusy`, `model.tokenContributionBusy`, `model.tokenStorageNotice` | `model.configureWitness(form)`, `model.clearWitness()`, `model.setInferenceEvidence(_:disclosureConfirmed:)`, `model.setTokenContribution(_:disclosureConfirmed:)`, `model.setLocalTokenCapture(_)`, `model.cleanTokenStorage(discard:)`, `model.refreshWitness()` | four `confirmationDialog`s: inference disclosure, token disclosure, token capture, token discard | `inferenceEvidenceSaveFailed`, `tokenContributionSaveFailed` (`NativeFlowNotice`), `tokenStorageNotice` | `WitnessSurface.{stateLine, tone(forState:), lastResultLine, lastResultTone, offersConfigure, offersClear}`, `WitnessForm.fromStatus`, `form.canConfigure`, every string on `WitnessCopy` |
| 10 | `.projects` | `projects` | `Picker` per project | `model.projects` (`displayLabel`, `isUnresolvedBucket`, `offerableModes`, `mode`), `model.lastActionError` | `model.setProjectMode(project, mode:)` | arming `confirmationDialog` on `ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(project:count:))` | `ActionMessageBanner` clearing `lastActionError` | `ProjectCopy.unresolvedBucketNote`; `ProjectCopy.modeChoiceLabel` (replaced by `ContributionModeCopy.choices`); Swift: "No projects seen yet." |
| 11 | `.changes` | `audit` | none | `model.audit` (`at`, `action`, `projectLabel`), `model.refreshAudit()` on appear | — | — | — | Swift: heading, "Nothing has been changed.", five action sentences |
| 12 | `.compute` | `ComputeView` | Phase 3 | | | | | |

### Task 1: The parity harness, the scan, and the three read-only sections

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/GlassSettingsContent.swift`
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/ConnectionSection.swift`
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/WatchingSection.swift`
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/ChangesSection.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` (add `SettingsLegacyWords`; the legacy bodies stay until Task 10)
- Test: `macos/Tests/TraceCommonsAppTests/GlassSurfaceRulesTests.swift` (new, the file in "The cross-cutting scan")
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift` (new)

**Interfaces:**
- Consumes: `SettingsSection` (`Views/SettingsSections.swift`), `AppModel`, `TCSourceChecks`, `MonitorWords` (`Views/Monitor/InferenceViews.swift`, debug-only until Phase 4; Phase 1 therefore does not read it).
- Produces: `struct GlassSettingsContent: View { var navigation: MainWindowNavigation?; let section: SettingsSection }` drawing one section; `enum SettingsLegacyWords` in `SettingsView.swift` holding every Swift-authored Settings sentence as `static let` / `static func`; the test helpers `SettingsParityTests.Section` and `GlassSurfaceRulesTests.files`.

- [ ] **Step 1: Record the wording baseline**

Run (from `macos/`, after `cargo build -p trace-commons-contributor-ffi` at the root):
`TC_WORDING_DUMP=1 swift test --filter ShellWordingTests 2>&1 | grep 'SettingsView.swift'`
Expected: `"TraceCommonsApp/Views/SettingsView.swift": 40,`. Keep the full dump; every later step compares against it.

- [ ] **Step 2: Write the failing parity test**

```swift
import XCTest

/// Each legacy Settings section's bindings, copy sources and confirmations
/// have a glass home. The table is the inventory in the plan; a row is
/// removed only when the owner retires the control it names.
final class SettingsParityTests: XCTestCase {
    struct Section {
        let glass: String
        let bindings: [String]
        let copySources: [String]
        let confirmations: [String]
        let accessibility: [String]
    }

    static let root = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp")

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let sections: [Section] = [
        Section(glass: "Views/Settings/ConnectionSection.swift",
                bindings: ["model.status.loggedIn", "routingSourceModes.claude", "routingSourceModes.codex",
                           "routingSourceModes.gemini", "routingSourceModes.cline", "nearAIConfigured"],
                copySources: ["TCSourceChecks.checkLine(", "TCSourceChecks.claude", "TCSourceChecks.codex",
                              "TCSourceChecks.gemini", "TCSourceChecks.cline",
                              "SettingsLegacyWords.queuedNothingSent", "SettingsLegacyWords.connected",
                              "SettingsLegacyWords.notConnected", "SettingsLegacyWords.extraScanConfigured"],
                confirmations: [],
                accessibility: [".accessibilityElement(children: .combine)"]),
        Section(glass: "Views/Settings/WatchingSection.swift",
                bindings: ["quiescenceSecs", "digestIntervalSecs", "queueTtlDays", "localNotifications",
                           "model.status.paused"],
                copySources: ["SettingsLegacyWords.sessionFinishedAfter(", "SettingsLegacyWords.atMostOneNotification(",
                              "SettingsLegacyWords.undecidedDropped(", "SettingsLegacyWords.notificationsRenderedHere",
                              "SettingsLegacyWords.pausedNothingSent"],
                confirmations: [],
                accessibility: []),
        Section(glass: "Views/Settings/ChangesSection.swift",
                bindings: ["model.audit", "model.refreshAudit()"],
                copySources: ["SettingsLegacyWords.auditHeading", "SettingsLegacyWords.nothingChanged",
                              "SettingsLegacyWords.auditSentence("],
                confirmations: [],
                accessibility: [".accessibilityElement(children: .combine)"]),
    ]

    func test_everyLegacyBindingAndCopySourceHasAGlassHome() throws {
        for section in Self.sections {
            let source = try Self.text(section.glass)
            for needle in section.bindings + section.copySources + section.confirmations + section.accessibility {
                XCTAssertTrue(source.contains(needle), "\(section.glass) lacks \(needle)")
            }
        }
    }

    /// The switch draws every section the list offers, by its own case.
    func test_theGlassContentDrawsEverySection() throws {
        let source = try Self.text("Views/Settings/GlassSettingsContent.swift")
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("case .\(section.rawValue):"), "GlassSettingsContent lacks .\(section.rawValue)")
        }
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `swift test --filter 'SettingsParityTests|GlassSurfaceRulesTests'`
Expected: FAIL, "No such file" on `Views/Settings/...`.

- [ ] **Step 4: Add the words table to `SettingsView.swift`**

Append the table and, in the same edit, replace each moved literal in the legacy bodies (`connection`, `watching`, `audit`) with the table's member, so the file still holds each sentence exactly once and the count stays 40. Later tasks do the same for their sections; Task 10 removes the bodies, leaving only the table.

```swift
/// Every sentence this file authors, in one place, so the glass sections
/// can read them without authoring any of their own. The ratchet
/// (`ShellWordingTests`) keys on this file's path: these sentences stay here
/// until the core exports them, and then this table shrinks.
enum SettingsLegacyWords {
    static let connected = "Connected"
    static let notConnected = "Not connected"
    static let queuedNothingSent = "Sessions are being queued, but nothing can be sent."
    static let extraScanConfigured = "Extra privacy scan configured"
    static func sessionFinishedAfter(_ secs: Int) -> String {
        "A session counts as finished after \(secs) seconds of quiet."
    }
    static func atMostOneNotification(_ hours: Int) -> String {
        "At most one notification every \(hours) hours, and none when nothing is waiting."
    }
    static func undecidedDropped(_ days: Int) -> String {
        "Undecided sessions are dropped after \(days) days. Dropped means never sent."
    }
    static let notificationsRenderedHere = "Notifications rendered by this app"
    static let pausedNothingSent = "Paused. Nothing is being queued or sent."
    static let auditHeading = SettingsContent.auditHeading
    static let nothingChanged = "Nothing has been changed."
    static func auditSentence(_ action: String, project: String?) -> String {
        SettingsContent.auditSentence(action, project: project)
    }
}
```

Make `SettingsContent.auditSentence` `static` (not `private`) so the table can forward to it. In Task 10 the bodies move out and the literals move in; the count never changes.

- [ ] **Step 5: Write `GlassSettingsContent`**

```swift
import SwiftUI
import TCDesign

/// One Settings section, on the glass system (R11 of #1173). The window's
/// list picks the section; this draws it. Compute has its own view.
struct GlassSettingsContent: View {
    var navigation: MainWindowNavigation?
    let section: SettingsSection

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            switch section {
            case .connection: ConnectionSection()
            case .startup: StartupSection()
            case .consent: ConsentSection()
            case .publicProfile: PublicProfileSection()
            case .watching: WatchingSection()
            case .watchedFolders: WatchedFoldersSection()
            case .tools: ToolsSection()
            case .privateAI: PrivateAISection(navigation: navigation)
            case .witness: WitnessSection()
            case .projects: ProjectsSection()
            case .changes: ChangesSection()
            case .compute: EmptyView()
            }
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: 560, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}
```

Until Tasks 2-9 exist, declare each missing section as `struct StartupSection: View { var body: some View { SettingsContent(section: .startup) } }` in `GlassSettingsContent.swift` so the file compiles; each later task replaces its stub with the real file and deletes the stub.

- [ ] **Step 6: Write the three read-only sections**

```swift
import SwiftUI
import TCBridge
import TCDesign

struct ConnectionSection: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GlassEyebrowCard(SettingsWords.connection) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if model.status.loggedIn {
                    GlassStatusLabel(SettingsLegacyWords.connected, status: .on)
                } else {
                    GlassStatusLabel(SettingsLegacyWords.notConnected, status: .ask)
                    Text(SettingsLegacyWords.queuedNothingSent)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                if let settings = model.daemonSettings {
                    sourceLine(TCSourceChecks.claude, settings.routingSourceModes.claude)
                    sourceLine(TCSourceChecks.codex, settings.routingSourceModes.codex)
                    sourceLine(TCSourceChecks.gemini, settings.routingSourceModes.gemini)
                    sourceLine(TCSourceChecks.cline, settings.routingSourceModes.cline)
                    GlassStatusLabel(SettingsLegacyWords.extraScanConfigured,
                                     status: settings.nearAIConfigured ? .on : .off)
                }
            }
        }
    }

    /// The core's sentence for one source's MODE; nothing when the ABI
    /// refused, as the legacy row does.
    @ViewBuilder
    private func sourceLine(_ tool: String, _ mode: String) -> some View {
        if let line = TCSourceChecks.checkLine(tool: tool, sourceMode: mode) {
            GlassStatusLabel(line, status: mode == "watch" ? .on : .off)
                .accessibilityElement(children: .combine)
        }
    }
}
```

`WatchingSection` draws the four figure sentences and the paused line with `Text(...).glassType(GlassTokens.TypeScale.body)`, keyed on `model.daemonSettings` and `model.status.paused`. `ChangesSection` draws `GlassEyebrowCard(SettingsLegacyWords.auditHeading)`, the empty line, then one `GlassTableRow` per `Array(model.audit.enumerated())` with the instant in `GlassTokens.TypeScale.mono` and `SettingsLegacyWords.auditSentence(entry.action, project: entry.projectLabel)`, `.accessibilityElement(children: .combine)`, and `.onAppear { model.refreshAudit() }`. Move `SettingsContent.instant(_:)` to `ChangesSection` unchanged.

- [ ] **Step 7: Register the files in the scan and run both suites**

Append to `GlassSurfaceRulesTests.files`: `"Views/Settings/ConnectionSection.swift"`, `"Views/Settings/WatchingSection.swift"`, `"Views/Settings/ChangesSection.swift"`.
Run: `swift test --filter 'SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|SettingsSectionsTests'`
Expected: PASS; the wording dump still says `SettingsView.swift: 40` and lists no `Views/Settings/` file.

- [ ] **Step 8: Commit**

```bash
git add macos/Sources/TraceCommonsApp/Views/Settings macos/Sources/TraceCommonsApp/Views/SettingsView.swift macos/Tests/TraceCommonsAppTests/GlassSurfaceRulesTests.swift macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift
git commit -m "Draw Connection, Watching and Changes on glass in Settings"
```

### Task 2: Startup (login item, notifications, updates)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/StartupSection.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` (words table gains the Startup sentences)
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`

**Interfaces:**
- Consumes: `LoginItemManager` (`currentState`, `register()`, `unregister()`, `State`, `RegisterOutcome`), `Notifier` (`shared.authorizationStatus()`, `shared.requestAuthorization()`, `copy`, `purpose`, `systemSettingsURL`), `UpdateController.shared`, `UpdatePolicy.noFeedReason`, `UpdatePolicy.insecureFeedReason`, `GlassToggleStyle(.settings)`, `GlassButtonStyle(.glass)`.
- Produces: `struct StartupSection: View`; `SettingsLegacyWords.{startAtLogin, waitingOnApproval, turnOnInSystemSettings, couldNotTurnOn(_:), couldNotTurnOff(_:), checksDaily, checksAutomatically, managedByHomebrew, homebrewReplaces, updatesUnavailable, notCheckedYet, lastChecked(_:), noFeed, insecureFeed, updatesOff}`.

- [ ] **Step 1: Extend the parity table**

```swift
        Section(glass: "Views/Settings/StartupSection.swift",
                bindings: ["LoginItemManager.currentState", "LoginItemManager.register()", "LoginItemManager.unregister()",
                           "Notifier.shared.authorizationStatus()", "Notifier.shared.requestAuthorization()",
                           "NSApplication.didBecomeActiveNotification",
                           "updates.currentVersion", "updates.mode", "updates.lastCheckDate", "updates.canCheckNow",
                           "updates.checkNow()", "NSPasteboard.general.setString"],
                copySources: ["Notifier.copy?.notificationHeading", "Notifier.copy?.notificationAllowed",
                              "Notifier.copy?.notificationDenied", "Notifier.copy?.notificationNotAsked",
                              "Notifier.copy?.notificationAllow", "Notifier.copy?.notificationUnknown",
                              "Notifier.copy?.systemSettings", "Notifier.purpose", "Notifier.systemSettingsURL",
                              "UpdatePolicy.noFeedReason", "UpdatePolicy.insecureFeedReason",
                              "SettingsLegacyWords.startAtLogin", "SettingsLegacyWords.waitingOnApproval",
                              "SettingsLegacyWords.checksAutomatically", "SettingsLegacyWords.homebrewReplaces",
                              "SettingsLegacyWords.notCheckedYet", "SettingsLegacyWords.lastChecked(",
                              "SettingsLegacyWords.updatesOff"],
                confirmations: [],
                accessibility: ["GlassToggleStyle(.settings)"]),
```

- [ ] **Step 2: Run it to verify it fails**

Run: `swift test --filter SettingsParityTests`
Expected: FAIL on `Views/Settings/StartupSection.swift`.

- [ ] **Step 3: Write the section**

Three `GlassEyebrowCard`s. The login-item switch:

```swift
    @State private var loginItemState: LoginItemManager.State = LoginItemManager.currentState
    @State private var loginItemActionError: String?

    private var loginItemCard: some View {
        GlassEyebrowCard(SettingsWords.startup) {
            switch loginItemState {
            case .enabled:
                Toggle(SettingsLegacyWords.startAtLogin, isOn: Binding(
                    get: { true }, set: { if !$0 { setLoginItem(enabled: false) } }))
                    .toggleStyle(GlassToggleStyle(.settings))
            case .notRegistered, .notFound:
                Toggle(SettingsLegacyWords.startAtLogin, isOn: Binding(
                    get: { false }, set: { if $0 { setLoginItem(enabled: true) } }))
                    .toggleStyle(GlassToggleStyle(.settings))
            case .requiresApproval:
                Text(SettingsLegacyWords.waitingOnApproval).glassType(GlassTokens.TypeScale.body)
                Text(SettingsLegacyWords.turnOnInSystemSettings)
                    .glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
            }
            if let loginItemActionError {
                GlassNotice(tone: .outside) { Text(loginItemActionError) }
            }
        }
        .onAppear { loginItemState = LoginItemManager.currentState }
    }
```

`setLoginItem(enabled:)` is the legacy function verbatim with `SettingsLegacyWords.couldNotTurnOn(message)` / `couldNotTurnOff(message)`. The notifications card keeps the four `UNAuthorizationStatus` branches, `Link(Notifier.copy?.systemSettings ?? "", destination: Notifier.systemSettingsURL)`, the allow `Button` in `GlassButtonStyle(.glass)` disabled while `notificationRequestPending`, the `.task` and `.onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification))` refreshes. The updates card keeps the three `UpdateMode` branches; the Homebrew command in `GlassWell` with `GlassTokens.TypeScale.mono` and `.textSelection(.enabled)`; "Check Now" and "Copy" in `GlassButtonStyle(.glass)`; `.disabled(!updates.canCheckNow)`. `ActionNoticeDismissTests` names `SettingsView.loginItemActionError`: update that test's comment to `StartupSection.loginItemActionError`, and keep the property name.

- [ ] **Step 4: Move the Startup sentences into `SettingsLegacyWords`** verbatim, replacing the literals in `SettingsContent.loginItem`, `notifications`, `updatesSection`, `lastCheckSentence`, `disabledSentence` with the table's members. Run the dump: `SettingsView.swift` must still read 40.

- [ ] **Step 5: Register `"Views/Settings/StartupSection.swift"` in `GlassSurfaceRulesTests.files`; run** `swift test --filter 'SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|ActionNoticeDismissTests|NotificationAuthorizationTests|UpdatePolicyTests'`. Expected: PASS.

- [ ] **Step 6: Commit** `git commit -m "Draw Startup on glass in Settings"`.

### Task 3: How traces may be used (consent scopes)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/ConsentSection.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/ConsentScopesSectionTests.swift` (new)

**Interfaces:**
- Consumes: `model.consentScopes: [ConsentScope]`, `model.status.consentScopes: [String]`, `model.setConsentScopes(_:) async -> ...` (`.succeeded` / `.failed`), `ScopeCopy.title(for:options:)` (`PreviewSheet.swift`), `TCSourceChecks.settingsCopy()?.consentSaveFailed`, `GlassCheckboxStyle`.
- Produces: `struct ConsentSection: View`; `enum ConsentScopeRows { static func nextScopes(reported: [String], options: [ConsentScope], toggling: ConsentScope, granted: Bool) -> Set<String> }` (pure, in `ConsentSection.swift`); `SettingsLegacyWords.{consentHeading, appliesFromNow, alwaysIncluded, optionalEachOne, credit, alwaysOn, nothingPreselected}`.

- [ ] **Step 1: Write the failing pure test** (Review Focus 2)

```swift
import XCTest
@testable import TraceCommonsApp

final class ConsentScopesSectionTests: XCTestCase {
    private func scope(_ name: String, alwaysOn: Bool = false, grants: Bool = true) -> ConsentScope {
        ConsentScope(name: name, description: "", alwaysOn: alwaysOn, grantsDataUse: grants)
    }

    /// The list sent is built from what the daemon reports, never from the
    /// ticks: two quick presses cannot race a scope neither touched.
    func test_theTickReadsTheDaemonNeverTheDraft() {
        let options = [scope("debugging_evaluation", alwaysOn: true), scope("benchmark_only"), scope("public_attribution", grants: false)]
        let next = ConsentScopeRows.nextScopes(reported: ["debugging_evaluation", "benchmark_only"],
                                               options: options, toggling: options[2], granted: true)
        XCTAssertEqual(next, ["debugging_evaluation", "benchmark_only", "public_attribution"])
        let off = ConsentScopeRows.nextScopes(reported: ["debugging_evaluation", "benchmark_only"],
                                              options: options, toggling: options[1], granted: false)
        XCTAssertEqual(off, ["debugging_evaluation"])
    }

    /// Always-on is always in the list, whatever the daemon reported.
    func test_alwaysOnIsNeverDropped() {
        let options = [scope("debugging_evaluation", alwaysOn: true), scope("benchmark_only")]
        let next = ConsentScopeRows.nextScopes(reported: [], options: options, toggling: options[1], granted: false)
        XCTAssertEqual(next, ["debugging_evaluation"])
    }
}
```

Check `ConsentScope`'s memberwise initialiser in `Models.swift` before writing; if it is `Decodable`-only, add an `init(name:description:alwaysOn:grantsDataUse:)` in the test target's support file.

- [ ] **Step 2: Extend the parity table**

```swift
        Section(glass: "Views/Settings/ConsentSection.swift",
                bindings: ["model.consentScopes", "model.status.consentScopes", "model.status.loggedIn",
                           "model.setConsentScopes(", "ConsentScopeRows.nextScopes("],
                copySources: ["ScopeCopy.title(for:", "scope.description", "settingsCopy()?.consentSaveFailed",
                              "SettingsLegacyWords.consentHeading", "SettingsLegacyWords.appliesFromNow",
                              "SettingsLegacyWords.alwaysIncluded", "SettingsLegacyWords.optionalEachOne",
                              "SettingsLegacyWords.credit", "SettingsLegacyWords.nothingPreselected"],
                confirmations: [],
                accessibility: ["GlassCheckboxStyle()", ".accessibilityElement(children: .combine)"]),
```

- [ ] **Step 3: Run** `swift test --filter 'ConsentScopesSectionTests|SettingsParityTests'`. Expected: FAIL (no `ConsentScopeRows`, no file).

- [ ] **Step 4: Write the section**

One `Toggle` per scope with `GlassCheckboxStyle()`; the label is the title in `GlassTokens.TypeScale.bodyStrong` over `scope.description` in `caption`; `.disabled(scope.alwaysOn || busy || !model.status.loggedIn)`; always-on rows are `isOn: .constant(true)` with a `GlassTag(SettingsLegacyWords.alwaysOn)`; `.accessibilityElement(children: .combine)` on each row so VoiceOver reads the title, description and the native switch value. The Swift `"Granted"`/`"Not granted"` accessibility values are not carried over: the native switch supplies the state. `"Not granted"` contains a function word and counts in the dump, so `SettingsView.swift` may read 39 after this task; that is a legitimate lowering (a Swift word replaced by a system value), and the baseline entry is set to what the dump reads, never predicted. The press calls:

```swift
    private func setScope(_ scope: ConsentScope, granted: Bool) {
        guard !busy, model.status.loggedIn, !scope.alwaysOn else { return }
        let scopes = ConsentScopeRows.nextScopes(
            reported: model.status.consentScopes, options: model.consentScopes, toggling: scope, granted: granted)
        saveError = nil
        busy = true
        Task {
            if case .failed = await model.setConsentScopes(Array(scopes)) {
                saveError = TCSourceChecks.settingsCopy()?.consentSaveFailed
            }
            busy = false
        }
    }
```

and `isOn` reads `model.status.consentScopes.contains(scope.name)`, never a draft. The failure line is a `GlassNotice(tone: .outside)`.

- [ ] **Step 5: Move the seven sentences into the words table; dump stays 40; register the file in the scan; run** `swift test --filter 'ConsentScopesSectionTests|SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|ConsentBindingTests'`. Expected: PASS.

- [ ] **Step 6: Commit** `git commit -m "Draw consent scopes on glass in Settings"`.

### Task 4: Public profile and the go-public sheet

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/PublicProfileSection.swift` (section + `GoPublicSheet`)
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/GoPublicGateTests.swift` (new)

**Interfaces:**
- Consumes: `model.publicProfile` (`DaemonClient.PublicProfile`: `handle`, `bio`, `publicSince`), `model.claimHandle(_:bio:)`, `model.leaveRoster()`, `model.profileBusy`, `model.profileOutcome` (`.none`, `.published(cached:)`, `.left(cached:)`, `.refused(label)`, `.leaveRefused(label)`), `model.clearProfileOutcome()`, `PublicProfileCopy.*`, `PublicProfileCopyCheck.failures()`, `GlassSheet`, `GlassTextField`, `GlassCheckboxStyle`.
- Produces: `struct PublicProfileSection: View`, `struct GoPublicSheet: View`, `enum GoPublicGate { static func canGoPublic(acknowledged: Bool, handle: String, busy: Bool) -> Bool }`; `SettingsLegacyWords.{publishedLines: [String], neverLines: [String], doNotTrustProfileWording}`.

- [ ] **Step 1: Write the failing gate test**

```swift
import XCTest
@testable import TraceCommonsApp

final class GoPublicGateTests: XCTestCase {
    func test_goPublicNeedsAnAcknowledgementAndAHandle() {
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: false, handle: "zaki", busy: false))
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: true, handle: "   ", busy: false))
        XCTAssertFalse(GoPublicGate.canGoPublic(acknowledged: true, handle: "zaki", busy: true))
        XCTAssertTrue(GoPublicGate.canGoPublic(acknowledged: true, handle: "zaki", busy: false))
    }
}
```

- [ ] **Step 2: Extend the parity table**

```swift
        Section(glass: "Views/Settings/PublicProfileSection.swift",
                bindings: ["model.publicProfile", "model.claimHandle(", "model.leaveRoster()", "model.profileBusy",
                           "model.profileOutcome", "model.clearProfileOutcome()", "GoPublicGate.canGoPublic(",
                           "utf8.count)/280"],
                copySources: ["PublicProfileCopy.heading", "PublicProfileCopy.footnote", "PublicProfileCopy.listHandlePublicly",
                              "PublicProfileCopy.goPublicConfirm", "PublicProfileCopy.handleLabel", "PublicProfileCopy.bioLabel",
                              "PublicProfileCopy.saveProfile", "PublicProfileCopy.leaveRoster", "PublicProfileCopy.published",
                              "PublicProfileCopy.publishedNotCached", "PublicProfileCopy.leftRoster",
                              "PublicProfileCopy.leftRosterNotCached", "PublicProfileCopy.failureSentence(",
                              "PublicProfileCopy.leaveFailureSentence(", "PublicProfileCopy.onRosterSince(",
                              "PublicProfileCopy.goPublicHeadline", "PublicProfileCopy.notNow", "PublicProfileCopy.goPublicFootnote",
                              "PublicProfileCopy.goPublicHandleLabel", "PublicProfileCopy.goPublicBioLabel",
                              "PublicProfileCopy.publishedHeading", "PublicProfileCopy.neverHeading",
                              "PublicProfileCopy.goPublicAcknowledgement", "PublicProfileCopyCheck.failures()",
                              "SettingsLegacyWords.publishedLines", "SettingsLegacyWords.neverLines",
                              "SettingsLegacyWords.doNotTrustProfileWording"],
                confirmations: ["GlassSheet("],
                accessibility: [".accessibilityLabel(", "GlassCheckboxStyle()"]),
```

- [ ] **Step 3: Run** `swift test --filter 'GoPublicGateTests|SettingsParityTests'`. Expected: FAIL.

- [ ] **Step 4: Write the section and the sheet**

The section: off the roster, a `GlassCard` with `PublicProfileCopy.listHandlePublicly` and a `Button(PublicProfileCopy.goPublicConfirm)` in `GlassButtonStyle(.glass)` that sets `showingGoPublic = true`; on the roster, a `GlassEyebrowCard(PublicProfileCopy.heading)` (D-15 default) with `GlassTextField(PublicProfileCopy.handleLabel, text: $handleDraft)`, a `TextEditor` for the bio styled with `glassType(.body)` and `.scrollContentBackground(.hidden)` inside a `GlassWell`, the byte counter, Save in `GlassButtonStyle(.primary)` disabled on `model.profileBusy || handleDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty`, Leave in `.glass`, `.accessibilityLabel("\(PublicProfileCopy.heading): \(handle)")`. Keep `seedProfileDraft()` and `publishedSignature` verbatim. The outcome sentence and the footnote follow; `profileCopyDefects` becomes a `GlassNotice(tone: .outside, title: SettingsLegacyWords.doNotTrustProfileWording)`.

The sheet, presented with `.sheet(isPresented: $showingGoPublic)`:

```swift
struct GoPublicSheet: View {
    var onDismiss: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var acknowledged = false
    @State private var handle = ""
    @State private var bio = ""

    var body: some View {
        GlassSheet(title: PublicProfileCopy.goPublicHeadline) {
            HStack(alignment: .top, spacing: GlassTokens.Space.cardGap) {
                column(PublicProfileCopy.publishedHeading, SettingsLegacyWords.publishedLines)
                column(PublicProfileCopy.neverHeading, SettingsLegacyWords.neverLines)
            }
            GlassTextField(PublicProfileCopy.goPublicHandleLabel, text: $handle)
            // bio editor as in the section
            Toggle(PublicProfileCopy.goPublicAcknowledgement, isOn: $acknowledged)
                .toggleStyle(GlassCheckboxStyle())
            if case .refused(let label) = model.profileOutcome {
                GlassNotice(tone: .outside) { Text(PublicProfileCopy.failureSentence(label)) }
            }
            HStack {
                Spacer(minLength: 0)
                Button(PublicProfileCopy.notNow, action: onDismiss).buttonStyle(GlassButtonStyle(.glass))
                Button(PublicProfileCopy.goPublicConfirm) { model.claimHandle(handle, bio: bio) }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .disabled(!GoPublicGate.canGoPublic(acknowledged: acknowledged, handle: handle, busy: model.profileBusy))
            }
            Text(PublicProfileCopy.goPublicFootnote).glassType(GlassTokens.TypeScale.caption)
        }
        .frame(width: 560)
        .onChange(of: outcomeIsSettled) { _, settled in if settled { onDismiss() } }
        .onAppear { model.clearProfileOutcome() }
    }
}
```

`column(_:_:)` is a `GlassWell` with the eyebrow and one `Text` per line; `outcomeIsSettled` is the legacy computed property verbatim; `GoPublicGate.canGoPublic` is the legacy `canGoPublic` as a pure function.

- [ ] **Step 5: Move the eight sentences (seven column lines, the defect heading) into the words table as `publishedLines`, `neverLines`, `doNotTrustProfileWording`; dump stays 40; register the file; run** `swift test --filter 'GoPublicGateTests|SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests'`. Expected: PASS.

- [ ] **Step 6: Commit** `git commit -m "Draw the public profile and go-public sheet on glass"`.

### Task 5: Watched folders and the shared source row

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/GlassSourceRow.swift`
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/WatchedFoldersSection.swift`
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/GlassSourceRowTests.swift` (new)

**Interfaces:**
- Consumes: `SourceKind` (`.claudeCode`, `.codex`, `.geminiCli`, `.cline`, `.opencode`; `allCases`, `displayName`, `rawValue`), `SourceCandidate` (`source`, `path`, `exists`, `evidence(now:)`, `decodeList(from:)`), `SourceChoice` (`.watch(path:)`, `.off`, `.undecided`), `TCSourceChecks.settingsCopy() -> SourceSettingsCopy?` (`heading`, `explanation`, `saveFailed`, `unavailable`, `retry`, `selectedFolder`, `noCandidate`, `watchCandidate`, `chooseFolder`, `tools[kind.rawValue]` with `explanation`, `key`, `unsetScansConventional`, `chooseFolder`, `decline`), `TCSourceChecks.checkLine(tool:sourceMode:)`, `TCDiscovery.sourcesJSON()`, `model.setSourceRoot(_:_:) async -> Bool`, `model.refreshSettings()`, `GlassFolderButton`, `GlassButtonStyle(.glass)`.
- Produces: `struct GlassSourceRow: View` with exactly `SourceRootRow`'s initialiser (`kind:candidate:choice:reportedMode:onWatchCandidate:onChoose:onDecline:`), `GlassSourceRow.chooseFolder() -> String?` (the `NSOpenPanel`, moved), `enum SourceRowState { static func answer(copy: SourceSettingsCopy, kind: SourceKind, reportedMode: String?, choice: SourceChoice, candidate: SourceCandidate?) -> SourceRowAnswer }` (pure), `struct WatchedFoldersSection: View`. Phase 2's Folders step consumes `GlassSourceRow`.

- [ ] **Step 1: Write the failing pure test**

```swift
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class GlassSourceRowTests: XCTestCase {
    /// A reported MODE is authoritative: the candidate's evidence never
    /// replaces the core's sentence for a watched or declined source.
    func test_aReportedModeOutranksTheCandidate() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let candidate = SourceCandidate(source: .codex, path: "/tmp/x", exists: true, sessionCount: 3, lastModified: nil)
        let answer = SourceRowState.answer(copy: copy, kind: .codex, reportedMode: "watch", choice: .undecided, candidate: candidate)
        XCTAssertEqual(answer.line, TCSourceChecks.checkLine(tool: copy.tools["codex"]!.key, sourceMode: "watch"))
        XCTAssertNil(answer.candidatePath)
    }

    /// Undecided with no mode shows the candidate, and with no candidate the
    /// core's no-candidate sentence, never a path this shell invented.
    func test_undecidedShowsTheCandidateOrTheCoresNoCandidateLine() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let none = SourceRowState.answer(copy: copy, kind: .geminiCli, reportedMode: nil, choice: .undecided, candidate: nil)
        XCTAssertEqual(none.line, copy.tools["gemini-cli"]?.explanation == nil ? copy.noCandidate : nil)
        XCTAssertNil(none.candidatePath)
    }

    /// The row keeps the legacy initialiser so the Folders step and Settings
    /// share it unchanged.
    func test_theRowKeepsTheLegacySignature() throws {
        let source = try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Settings/GlassSourceRow.swift"), encoding: .utf8)
        for label in ["kind:", "candidate:", "choice:", "reportedMode:", "onWatchCandidate:", "onChoose:", "onDecline:"] {
            XCTAssertTrue(source.contains(label), "GlassSourceRow lacks \(label)")
        }
    }
}
```

Check `SourceCandidate`'s initialiser in `TCShellCore/SourceCandidate.swift` and `SourceKind.geminiCli.rawValue` before running; adjust the test's constructor and the `"gemini-cli"` key to what the type declares.

- [ ] **Step 2: Extend the parity table**

```swift
        Section(glass: "Views/Settings/WatchedFoldersSection.swift",
                bindings: ["TCSourceChecks.settingsCopy()", "SourceKind.allCases", "TCDiscovery.sourcesJSON()",
                           "SourceCandidate.decodeList(", "model.setSourceRoot(", "model.refreshSettings()",
                           "routingSourceModes", "opencodeSourceMode", "GlassSourceRow("],
                copySources: ["copy.heading", "copy.explanation", "copy.saveFailed", "copy.unavailable", "copy.retry"],
                confirmations: ["GlassSourceRow.chooseFolder()"],
                accessibility: []),
```

- [ ] **Step 3: Run** `swift test --filter 'GlassSourceRowTests|SettingsParityTests'`. Expected: FAIL.

- [ ] **Step 4: Write `GlassSourceRow` and `SourceRowState`**

`SourceRowState.answer` is `SourceRootRow.answerLine`'s branching as data: `struct SourceRowAnswer: Equatable { let line: String?; let candidatePath: String?; let evidence: String?; let selectedPath: String? }`. The row is a `GlassCard` with `kind.displayName` in `bodyStrong`, the tool explanation in `caption`, the answer lines, then the buttons: `Button(copy.watchCandidate)` (only when `candidate?.exists == true`, disabled when `choice == .watch(path: candidate.path)`), `GlassFolderButton(tool.chooseFolder ?? copy.chooseFolder) { if let path = Self.chooseFolder() { onChoose(path) } }`, `Button(tool.decline)` disabled when `choice == .off`; all in `GlassButtonStyle(.glass)`. `WatchedFoldersSection` is the legacy `watchedFolders` body with `GlassSourceRow` in place of `SourceRootRow`, `GlassNotice(tone: .outside)` for `copy.saveFailed`, and `Button(copy.retry) { model.refreshSettings() }`.

- [ ] **Step 5: Register both files in the scan; run** `swift test --filter 'GlassSourceRowTests|SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|SourceCheckBindingTests|OnboardingRootsRowsTests'`. Expected: PASS; `SettingsView.swift` dump still 40 (this section authors no sentence).

- [ ] **Step 6: Commit** `git commit -m "Draw watched folders on glass with a shared source row"`.

### Task 6: Tools (the local proxy)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/ToolsSection.swift`
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/ToolsSectionTests.swift` (new)

**Interfaces:**
- Consumes: `model.routingCopy: RoutingCopy?`, `model.routingForm: RoutingForm` (`on`, `port: UInt16`, `tokenDir`), `model.routingDiscovery` (`found`), `model.routingChecking`, `model.routingProbeLine`, `model.status.routing` (`state`, `derived`, `lastRefreshAt`), `model.routingEvidence`, `model.routingCalls`, `model.applyIronWire(_:)`, `model.discoverRouting()`, `model.refreshRoutedTools()`, `RoutingSurface.*`, `TCRoutingCopy.lastChecked(when:)`, `GlassToggleStyle(.settings)`, `GlassExpander`, `GlassTag`, `GlassKeyValueList`.
- Produces: `struct ToolsSection: View`; `enum RoutingPortInput { static func accept(_ value: Int, into form: RoutingForm) -> RoutingForm }` (pure: the `UInt16(exactly:) > 0` guard); `GlassTag.Tone` mapping `RoutingTone -> GlassTag.Tone` as `ToolsSection.tone(_:)`.

- [ ] **Step 1: Write the failing pure test**

```swift
import XCTest
import TCShellCore
@testable import TraceCommonsApp

final class ToolsSectionTests: XCTestCase {
    /// Out of range is left as it was, never clamped: port 0 is the
    /// ask-the-kernel sentinel the daemon refuses.
    func test_anInvalidPortLeavesTheFormUnchanged() {
        var form = RoutingForm(on: true, port: 8787, tokenDir: "")
        form = RoutingPortInput.accept(0, into: form)
        XCTAssertEqual(form.port, 8787)
        form = RoutingPortInput.accept(70_000, into: form)
        XCTAssertEqual(form.port, 8787)
        form = RoutingPortInput.accept(9000, into: form)
        XCTAssertEqual(form.port, 9000)
    }

    /// Only the four routing tones exist; none maps to a failure look.
    func test_routingTonesNeverReadAsFailed() {
        for tone in [RoutingTone.clear, .held, .attention, .neutral] {
            XCTAssertNotEqual(ToolsSection.tone(tone), .outside)
        }
        XCTAssertEqual(ToolsSection.tone(.clear), .on)
        XCTAssertEqual(ToolsSection.tone(.attention), .ask)
    }
}
```

Check `RoutingForm`'s initialiser in `TCShellCore/RoutingSurface.swift` first.

- [ ] **Step 2: Extend the parity table**

```swift
        Section(glass: "Views/Settings/ToolsSection.swift",
                bindings: ["model.routingCopy", "model.routingForm", "model.routingDiscovery", "model.routingChecking",
                           "model.routingProbeLine", "model.status.routing.state", "model.status.routing.derived",
                           "model.status.routing.lastRefreshAt", "model.routingEvidence", "model.routingCalls",
                           "model.applyIronWire(", "model.discoverRouting()", "model.refreshRoutedTools()",
                           "RoutingPortInput.accept("],
                copySources: ["RoutingSurface.toolRows(", "RoutingSurface.discoveryLine(", "RoutingSurface.connecting(",
                              "RoutingSurface.overrideIsCollapsed(", "RoutingSurface.stateLine(", "RoutingSurface.tone(forState:",
                              "RoutingSurface.showsLastChecked(", "TCRoutingCopy.lastChecked(",
                              "copy.toolsHeading", "copy.intro", "copy.toggle", "copy.connect", "copy.lookAgain",
                              "copy.overrideTitle", "copy.portTitle", "copy.portNote", "copy.folderTitle", "copy.chooseFolder",
                              "copy.folderNote", "copy.checking", "copy.apply", "copy.appliesAtOnce", "copy.derivedOrigin"],
                confirmations: ["GlassSourceRow.chooseFolder()"],
                accessibility: ["GlassToggleStyle(.settings)", ".accessibilityLabel(copy.portTitle)",
                                ".accessibilityLabel(copy.folderTitle)", ".accessibilityElement(children: .combine)"]),
```

- [ ] **Step 3: Run** `swift test --filter 'ToolsSectionTests|SettingsParityTests'`. Expected: FAIL.

- [ ] **Step 4: Write the section** as the legacy `routing` body in glass: tool rows as `GlassTableRow`s with `Text(row.name)` and `GlassTag(row.word, tone: Self.tone(row.tone))`; the switch `Toggle(copy.toggle, isOn:)` with `GlassToggleStyle(.settings)`; `routingState` as `GlassStatusLabel(line, status:)`; the Connect button `GlassButtonStyle(.primary)` only when `model.routingDiscovery.found && !form.on`; Look again `.link`; the override in `GlassExpander(copy.overrideTitle, isOpen:)` with the `routingOverrideOpen ?? !RoutingSurface.overrideIsCollapsed(...)` binding, a `TextField` with `format: .number.grouping(.never)` whose setter is `routingDraft = RoutingPortInput.accept(value, into: form)`, and `GlassFolderButton(copy.chooseFolder) { if let path = GlassSourceRow.chooseFolder() { ... } }`; Apply in `.glass`; `.onAppear { model.discoverRouting(); model.refreshRoutedTools() }`. No sentence is authored.

- [ ] **Step 5: Register the file; run** `swift test --filter 'ToolsSectionTests|SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|RoutingBindingTests|RoutingCallTests'`. Expected: PASS.

- [ ] **Step 6: Commit** `git commit -m "Draw the Tools routing card on glass"`.

### Task 7: The redaction witness

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/WitnessSection.swift`
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`

**Interfaces:**
- Consumes: everything in inventory row 9; `WitnessForm` (`url`, `signingAddress`, `measurements`, `canConfigure`, `fromStatus(_:)`), `WitnessTone`, `NativeFlowNotice` is replaced by `GlassNotice`.
- Produces: `struct WitnessSection: View`; `WitnessSection.tone(_ tone: WitnessTone) -> GlassStatus` (`.refused -> .outside`, `.attention -> .ask`, `.held -> .ask`, `.clear -> .on`, `.neutral -> .off`), kept separate from `ToolsSection.tone` for the reason `SettingsView.witnessTone` gives.

- [ ] **Step 1: Extend the parity table**

```swift
        Section(glass: "Views/Settings/WitnessSection.swift",
                bindings: ["model.witnessCopy", "model.witnessState", "model.witnessStateCode", "model.witnessStatus?.refusal",
                           "model.witnessStatus?.pinnedMeasurementLine", "model.witnessLabel", "model.witnessBusy", "model.witnessCalls",
                           "ironwireAttestedBodies", "tokenDistributionsContribution", "tokenStorage",
                           "model.inferenceEvidenceBusy", "model.tokenContributionBusy", "model.tokenStorageNotice",
                           "model.inferenceEvidenceSaveFailed", "model.tokenContributionSaveFailed",
                           "model.configureWitness(", "model.clearWitness()", "model.setInferenceEvidence(",
                           "model.setTokenContribution(", "model.setLocalTokenCapture(", "model.cleanTokenStorage(discard:",
                           "model.refreshWitness()", "WitnessForm.fromStatus(", "form.canConfigure"],
                copySources: ["WitnessSurface.stateLine(", "WitnessSurface.tone(forState:", "WitnessSurface.lastResultLine(",
                              "WitnessSurface.lastResultTone(", "WitnessSurface.offersConfigure(", "WitnessSurface.offersClear(",
                              "copy.heading", "copy.intro", "copy.certificateMeans", "copy.clear", "copy.clearNote",
                              "copy.appliesAtOnce", "copy.urlTitle", "copy.signingAddressTitle", "copy.measurementsTitle",
                              "copy.measurementsNote", "copy.configure", "copy.inferenceHeading", "copy.inferenceDisclosure",
                              "copy.inferenceCaptureNote", "copy.inferenceScopeNote", "copy.inferenceEnabled", "copy.inferenceDisabled",
                              "copy.inferenceEnable", "copy.inferenceDisable", "copy.inferenceConfirm", "copy.inferenceCancel",
                              "copy.inferenceSaveFailed", "copy.tokenHeading", "copy.tokenDisclosure", "copy.tokenCaptureNote",
                              "copy.tokenScopeNote", "copy.tokenEnabled", "copy.tokenDisabled", "copy.tokenEnable", "copy.tokenDisable",
                              "copy.tokenConfirm", "copy.tokenCancel", "copy.tokenSaveFailed", "storage.captureLabel",
                              "storage.captureNotice", "storage.captureConfirmation", "storage.cancelLabel", "storage.stateLine",
                              "storage.scopeNote", "storage.cleanupLabel", "storage.discardLabel", "storage.confirmLabel",
                              "storage.discardConfirmation"],
                confirmations: ["showingInferenceDisclosure", "showingTokenDisclosure", "showingTokenCapture", "showingTokenDiscard",
                                ".confirmationDialog("],
                accessibility: [".accessibilityLabel(copy.urlTitle)", ".accessibilityLabel(copy.signingAddressTitle)",
                                ".accessibilityLabel(copy.measurementsTitle)", ".accessibilityElement(children: .combine)"]),
```

- [ ] **Step 2: Run** `swift test --filter SettingsParityTests`. Expected: FAIL.

- [ ] **Step 3: Write the section**: the legacy `witness`, `inferenceEvidence`, `tokenContribution`, `witnessState`, `witnessFields` bodies on `GlassEyebrowCard`, `GlassStatusLabel`, `GlassTag`, `GlassTextField` (url, signing address), a `TextEditor` in a `GlassWell` with `GlassTokens.TypeScale.mono` for the measurements, `GlassNotice(tone: .outside)` for the two save-failed lines, buttons in `GlassButtonStyle(.glass)`, the four `confirmationDialog`s verbatim (system dialogs are HIG; they carry the core's sentences). The `.opacity(copy.tokenHeading == nil ? 0 : 1).disabled(copy.tokenHeading == nil)` pair is kept.

- [ ] **Step 4: Register the file; run** `swift test --filter 'SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|WitnessBindingTests|WitnessReviewBusyTests'`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Draw the redaction witness on glass"`.

### Task 8: Private AI pointer and the route disclosure

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/PrivateAISection.swift` (section + `RouteDisclosureGlassBody` + `RouteDisclosureUnreadableGlassLine`)
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`

**Interfaces:**
- Consumes: `model.privateInferenceCopy?.{settingsTitle, settingsMoved, destination}`, `navigation?.section = .privateInference` (kept; Phase 4 Task 3 retargets to `MonitorDestination.inference`), `model.routeDisclosureState` (`.shown(RouteDisclosure)`, `.loading`, `.unreadable`), `model.routeDisclosureUnreadableCopy?.{title, panel}`, `model.refreshRouteDisclosure()`, `RouteDisclosure` (`copy.route`, `copy.localFilter`, `copy.witness?.{heading, addressLabel, signingLabel, measurementsLabel, check, classifier, origin}`, `facts.witness?.{url, signingAddress, pinnedMeasurements}`, `copy.attestedBodies`, `copy.receipts`).
- Produces: `struct PrivateAISection: View { var navigation: MainWindowNavigation? }`, `struct RouteDisclosureGlassBody: View { let disclosure: RouteDisclosure }` (Phase 4 Task 5 reuses it in the hosted preview), `struct RouteDisclosureUnreadableGlassLine: View { let line: String? }`.

- [ ] **Step 1: Extend the parity table**

```swift
        Section(glass: "Views/Settings/PrivateAISection.swift",
                bindings: ["model.privateInferenceCopy", "navigation?.section = .privateInference",
                           "model.routeDisclosureState", "model.routeDisclosureUnreadableCopy", "model.refreshRouteDisclosure()"],
                copySources: ["copy.settingsTitle", "copy.settingsMoved", "copy.destination",
                              "copy.route", "copy.localFilter", "witness.heading", "witness.addressLabel", "witness.signingLabel",
                              "witness.measurementsLabel", "witness.check", "witness.classifier", "witness.origin",
                              "copy.attestedBodies", "copy.receipts", "facts.url", "facts.signingAddress", "facts.pinnedMeasurements"],
                confirmations: [],
                accessibility: [".accessibilityElement(children: .combine)"]),
```

- [ ] **Step 2: Run** `swift test --filter SettingsParityTests`. Expected: FAIL.

- [ ] **Step 3: Write the section.** The pointer is a `GlassEyebrowCard(copy.settingsTitle)` with `copy.settingsMoved` and `Button(copy.destination)` in `GlassButtonStyle(.link)`; the disclosure is a `GlassEyebrowCard(disclosure.copy.title)` holding `RouteDisclosureGlassBody` (a `GlassKeyValueList` of the witness facts in mono, the sentences in `caption`), `ProgressView().controlSize(.small)` while loading, and `RouteDisclosureUnreadableGlassLine`: a `GlassStatusLabel(line ?? model.routeDisclosureUnreadableCopy?.title ?? "", status: .ask)`, so the unreadable state is drawn with a glyph and words even when the Rust's panel sentence could not be read, and the scan's no-bare-dot rule holds (`MonitorWords` is debug-only until Phase 4, so it is not read here). `.onAppear { model.refreshRouteDisclosure() }`.

- [ ] **Step 4: Register the file; run** `swift test --filter 'SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|RouteDisclosureRefreshTests|SettingsPointerTests'`. Expected: PASS. `SettingsPointerTests` reads `SettingsView.swift` for the pointer; it keeps passing until Task 10 moves the body, where it is repointed.

- [ ] **Step 5: Commit** `git commit -m "Draw the Private AI pointer and route disclosure on glass"`.

### Task 9: Projects (folder modes from the core's table)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Settings/ProjectsSection.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` (words table gains "No projects seen yet.")
- Modify: `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs` (the "arming confirmation" `SURFACES` row's path: `TraceCommonsApp/Views/SettingsView.swift` → `TraceCommonsApp/Views/Settings/ProjectsSection.swift`)
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/ProjectModeChoicesTests.swift` (new)

**Interfaces:**
- Consumes: `model.projects: [ProjectRow]` (`displayLabel`, `isUnresolvedBucket`, `offerableModes: [ProjectMode]`, `mode`), `model.setProjectMode(_:mode:)`, `model.lastActionError`, `ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(project:count:))`, `ProjectCopy.unresolvedBucketNote`, `ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())` (`choices: [Choice]` with `mode`, `label`, `line`), `GlassPicker`, `GlassPickerOption`.
- Produces: `struct ProjectsSection: View`; `enum ProjectModeChoices { static func options(for modes: [ProjectMode], copy: ContributionModeCopy) -> [GlassPickerOption<ProjectMode>] }` (pure); the picker's words are the core's.

- [ ] **Step 1: Write the failing pure test** (this pins the owner's "Ask me / Automatic / Never" to the core's table)

```swift
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class ProjectModeChoicesTests: XCTestCase {
    /// The core's contribution-mode table names every ProjectMode the daemon
    /// accepts, by the daemon's own mode string.
    func test_theCoreNamesEveryProjectMode() throws {
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        for mode in [ProjectMode.ask, .autoUpload, .ignore] {
            XCTAssertNotNil(copy.choice(for: mode.rawValue), "no core label for \(mode.rawValue)")
        }
    }

    /// The picker offers only the modes the row can take, in the row's
    /// order, each labelled by the core.
    func test_optionsFollowOfferableModes() throws {
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        let options = ProjectModeChoices.options(for: [.ask, .ignore], copy: copy)
        XCTAssertEqual(options.map(\.value), [.ask, .ignore])
        XCTAssertEqual(options.map(\.title), [copy.choice(for: "notify_only")?.label, copy.choice(for: "ignore")?.label])
    }
}
```

`GlassPickerOption` exposes `value` and `title`; check `Controls.swift:301-315` for the property names and adjust.

- [ ] **Step 2: Extend the parity table**

```swift
        Section(glass: "Views/Settings/ProjectsSection.swift",
                bindings: ["model.projects", "model.setProjectMode(", "model.lastActionError", "armingCandidate",
                           "project.offerableModes", "project.isUnresolvedBucket", "project.displayLabel",
                           "ProjectModeChoices.options("],
                copySources: ["ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(",
                              "ProjectCopy.unresolvedBucketNote", "TCCoreCopy.contributionModeCopyJSON()",
                              "SettingsLegacyWords.noProjectsYet"],
                confirmations: [".confirmationDialog(", "presenting: armingCandidate"],
                accessibility: ["GlassPicker("]),
```

- [ ] **Step 3: Run** `swift test --filter 'ProjectModeChoicesTests|SettingsParityTests'`. Expected: FAIL.

- [ ] **Step 4: Write the section.** One `GlassTableRow` per project: `project.displayLabel`, the bucket note in `caption` when `isUnresolvedBucket`, and `GlassPicker(project.displayLabel, selection: Binding<ProjectMode?>(get: { project.mode }, set: { wanted in ... }), options: ProjectModeChoices.options(for: project.offerableModes, copy: copy), placeholder: copy.title)`; the setter keeps the legacy rule (arming goes through `armingCandidate`, everything else calls `model.setProjectMode` directly; a nil selection is ignored). The arming `confirmationDialog` is the legacy one verbatim (`presenting: armingCandidate`, no `.destructive` role). `model.lastActionError` is a `GlassNotice(tone: .outside)` with a dismiss `Button` that sets `model.lastActionError = nil` (what `ActionNoticeDismissTests` requires). When the core's table has not loaded, the picker is not drawn and the row shows nothing for its mode.

- [ ] **Step 5: Lower `ProjectRow.swift`'s baseline?** No: `ProjectCopy.modeChoiceLabel` is still used by `OnboardingProjectsView`'s tag words until Phase 2; leave the entry at 3. Repoint the `.rs` row; run `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central`. Expected: PASS once `ProjectsSection.swift` contains `TCCoreCopy.armingOfferCopyJSON`.

- [ ] **Step 6: Register the file; run** `swift test --filter 'ProjectModeChoicesTests|SettingsParityTests|GlassSurfaceRulesTests|ShellWordingTests|ActionNoticeDismissTests'`. Expected: PASS; dump `SettingsView.swift` 40.

- [ ] **Step 7: Commit** `git commit -m "Draw Projects on glass with the core's folder-mode words"`.

### Task 10: Switch the window to the glass content and retire the legacy bodies

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift:419-432` (`SettingsContent(navigation:section:)` → `GlassSettingsContent(navigation:section:)`, drop `.tcScreen()`)
- Modify: `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` (delete `SettingsView`, `SettingsContent`'s body and `GoPublicDialog`; keep `SettingsContent.consentHeading`, `auditHeading`, `auditSentence` as the words table's sources, `SettingsLegacyWords`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift:196` (`case .settings: SettingsView(navigation:)` → `ScrollView { ForEach(SettingsSection.allCases.filter { $0 != .compute }) { GlassSettingsContent(navigation: navigation, section: $0) } }` so the legacy main window keeps a Settings destination until Phase 4)
- Modify: `macos/Sources/TraceCommonsApp/DebugScreenshot.swift:654-658` (`SettingsContent()` → `VStack { ForEach(SettingsSection.allCases...) { GlassSettingsContent(section:) } }`)
- Delete: `macos/Sources/TraceCommonsApp/Views/SourceRootRow.swift`, `macos/Sources/TraceCommonsApp/Views/RouteDisclosureView.swift` (after moving `SessionSendDisclosureView` to `Views/Settings/PrivateAISection.swift`'s sibling file `Views/SessionSendDisclosureView.swift` unchanged, since `PreviewSheet` uses it), `macos/Sources/TraceCommonsApp/Views/ConsentScopesView.swift` stays (onboarding, Phase 2)
- Modify tests: `SettingsSectionsTests.swift:28-49` (read `Views/Settings/GlassSettingsContent.swift` for `case .x:` and each section file for its heading source), `SettingsPointerTests.swift:18` (read `Views/Settings/PrivateAISection.swift`), `ActionNoticeDismissTests.swift:37` (names)
- Test: `macos/Tests/TraceCommonsAppTests/SettingsParityTests.swift` (add the unavailable-branch test)

**Interfaces:**
- Consumes: everything Tasks 1-9 produced.
- Produces: `MonitorSettingsWindow` draws glass; `SettingsView.swift` is a words table of exactly 40 sentences. `SettingsContent` shrinks to a namespace `enum SettingsContent { static let consentHeading; static let auditHeading; static func auditSentence(_:project:) }`, because `SettingsSections.swift:34,41` read `SettingsContent.consentHeading` and `.auditHeading` for the list; the view struct of that name is gone.

- [ ] **Step 1: Write the failing test** (Review Focus 1)

```swift
    /// Every section draws its own loading or unavailable state when the
    /// daemon has not answered, rather than nothing and rather than a
    /// control that reads as working. The marker is the branch on the
    /// optional the section reads first.
    func test_everySectionHasAnUnavailableBranch() throws {
        let guards: [String: String] = [
            "Views/Settings/ConnectionSection.swift": "if let settings = model.daemonSettings",
            "Views/Settings/WatchingSection.swift": "if let settings = model.daemonSettings",
            "Views/Settings/ConsentSection.swift": "!model.status.loggedIn",
            "Views/Settings/WatchedFoldersSection.swift": "copy.unavailable",
            "Views/Settings/ToolsSection.swift": "if let copy = model.routingCopy",
            "Views/Settings/WitnessSection.swift": "if let copy = model.witnessCopy",
            "Views/Settings/PrivateAISection.swift": "case .loading:",
            "Views/Settings/ProjectsSection.swift": "model.projects.isEmpty",
            "Views/Settings/StartupSection.swift": "if let status = notificationStatus",
        ]
        for (file, marker) in guards {
            XCTAssertTrue(try Self.text(file).contains(marker), "\(file) has no unavailable branch (\(marker))")
        }
    }

    /// The window draws the glass content and nothing else.
    func test_theWindowDrawsGlassContent() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("GlassSettingsContent(navigation: navigation, section: section)"))
        XCTAssertFalse(window.contains("SettingsContent(navigation:"))
        XCTAssertFalse(window.contains(".tcScreen()"))
    }
```

- [ ] **Step 2: Run** `swift test --filter SettingsParityTests`. Expected: FAIL on `test_theWindowDrawsGlassContent`.

- [ ] **Step 3: Make the changes listed under Files.** Move every remaining Swift-authored sentence from the deleted bodies into `SettingsLegacyWords` so the dump reads exactly 40. Update the three tests named above.

- [ ] **Step 4: Run the whole gate**

```
cd macos && swift test
python3 ../scripts/design-tokens/generate.py --check
cd .. && cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central
```
Expected: all PASS; `TC_WORDING_DUMP=1 swift test --filter ShellWordingTests` shows `SettingsView.swift: 40` and no `Views/Settings/` entry.

- [ ] **Step 5: Commit and open the Phase 1 PR**

```bash
git commit -m "Draw every Settings section on glass"
```

PR body: the inventory table above, the dump before/after, and the D-15 and D-17 defaults taken. Verification pastes the three commands' output. Tick the MIT OR Apache-2.0 attestation.

---

## Phase 2: Onboarding internals in TCDesign

**Spec:** #1152 "Onboarding / first run: a single pane over the scene, with
StepProgress"; #1173 R12; the 2026-09-25 FTUX concept (#1030) as amended by
the owner on 2026-09-28; D-3, D-4, D-5, D-6, D-16.

**How the concept maps onto today's steps.** `OnboardingNavigation.Step`
runs welcome → (roots) → connect → consent → (privacyScan) → projects →
done. `FirstRunProgress` already labels them Folders (roots), Join
(connect), Uses (consent), Scan, Projects. The concept's Join → Folders →
Uses order inverts Folders and Join; on macOS Join needs the daemon and the
daemon needs the roots (D-3). The concept's Sharing moment, tiers, Tools and
Rules steps do not exist today (D-5), nor Private AI in onboarding (D-6).
Phase 2 rebuilds the existing steps' content on TCDesign, in place, keeps
every daemon call and guard, and records the concept's deltas in Task 10 as
decisions and DRAFT sentences. Nothing new is built on the concept.

**Fail-closed guards preserved (every task's tests pin the one it touches):**

| Guard | Where | Pinned by |
|---|---|---|
| The first-run window draws only while `model.requiresOnboarding`, and closes when it turns false | `FirstRunWindowView` | `FirstRunProgressTests.test_theWindowIsGatedOnRequiresOnboarding` (exists) |
| Roots: Continue disabled until Claude Code and Codex are answered (`roots.isComplete`); nothing pre-selected | `OnboardingRootsView` | Task 3 |
| Join never authorizes sharing: `enroll` carries no scopes; a link fills the field and names the issuer, pressing is the person's | `OnboardingConnectContent`, `AppDelegate.application(_:open:)` | Task 4 |
| Uses: nothing optional pre-ticked; Continue applies `set_consent_scopes` and advances only on `.succeeded`; failure keeps the ticks | `ConsentScopesContent`, `OnboardingCoordinatorView.advanceFromConsent`, `OnboardingNavigation` | Task 5, `OnboardingNavigationTests` (exists) |
| Scan: `acknowledgeNearAINotice()` fires only when the scan is chosen; no copy, no choices, no Continue | `OnboardingPrivacyScanContent` | Task 6 |
| Projects: only Ignore is offered, never Automatic; the bucket is never armable | `OnboardingProjectsContent` | Task 7 |
| Done: `markOnboardingComplete()` only from Done's button; notifications asked only from a button under the purpose sentence | `OnboardingDoneContent`, `Notifier.configure` | Task 8, `NotificationAuthorizationTests` (exists) |

**Inventory of the legacy steps** (wording counts from `ShellWordingTests`):

| Step | File | Controls | Bindings | Copy sources | Sentences authored |
|---|---|---|---|---|---|
| Welcome | `OnboardingWelcomeView.swift` | Get started, What gets removed? | — | `TCOnboardingCopy.load()?.welcomeBody` | 8 |
| Folders | `OnboardingRootsView.swift` | `SourceRootRow` × `SourceKind.allCases`, Continue | `SessionRoots` (`subscript`, `watch(_:)`, `isComplete`, `settingsJSON()`), `TCDiscovery.sourcesJSON()`, `SourceCandidate.decodeList`, `model.isStartingDaemon`, `model.startDaemon(at:settingsJSON:)`, `model.configDirectory`, `TCDaemon.TCError.rootsNotDeclared` | — | 5 |
| Join | `OnboardingConnectView.swift` | invite `TextField`, Look up, Join host, Continue (already connected), `NearAiJoinView`, `NearAccountConnectView` | `InviteLink.parse`, `PendingInvite.shared.take()`/`.value`, `model.enroll(invite:)`, `model.status.loggedIn` | — | 7 |
| Join (NEAR AI) | `NearAiJoinView.swift` | commons field, Join, `CredentialSection(requiresSession: true)` | `model.credentialStatus.sessionState == CredentialSurface.statePresent`, `model.nearAiAccountEnroll(commons:)` | `copy.nearAiEnroll*`, `model.witnessCopy?.wallet?.commons`, `TCNearAiEnroll.line/tone` | 0 |
| Join (wallet) | `NearAccountConnectView.swift` | commons/account fields, Check, Start, Cancel | `model.nativeWalletFlow(action:flowID:commons:account:)`, `openURL` | `model.witnessCopy?.wallet.*`, flow messages | 0 |
| Uses | `ConsentScopesView.swift` | one row per scope, Continue with N permissions | `model.consentScopes`, `initialSelection`, `onContinue(Set<String>)` | `ScopeCopy.title(for:options:)`, `scope.description` | 7 |
| Scan | `OnboardingPrivacyScanView.swift` | two radio rows, Continue | `model.daemonSettings?.nearAIConfigured`, `model.acknowledgeNearAINotice()` | `PrivacyScanCopy.decode(fromJSON: TCCoreCopy.privacyScanCopyJSON())` | 0 (core) |
| Projects | `OnboardingProjectsView.swift` | Ignore/Ignored per project, Continue | `model.projects`, `model.setProjectMode(_:mode:)`, `model.lastActionError` | `ProjectCopy.unresolvedBucketNote`, `ProjectCopy.modeChoiceLabel` (tag words) | 4 |
| Done | `OnboardingDoneView.swift` | Start at login / Not now, Allow / Not now, Done | `LoginItemManager.currentState/register()`, `Notifier.shared.authorizationStatus()/requestAuthorization()`, `onFinish` | `Notifier.copy?.{doneBody, notificationOffer, notNow, notificationAllow, notificationAllowed, notificationDenied, systemSettings}`, `Notifier.purpose` | 8 |
| Coordinator | `OnboardingCoordinatorView.swift` | Back | `OnboardingNavigation`, `WhatGetsRemovedSheet` | — | 5 |
| What gets removed | `WhatGetsRemovedSheet.swift` | Close | — | `TCScrubInfo` (check) | 4 |
| Admission | `AdmissionPreparationView.swift` | backend field, confirm, `SettingsLink` | `model.prepareAdmissionSession(entryID:backend:)`, `inferenceEvidenceEnabled` | `model.witnessCopy?.admission.*` | 0 |

Each step is rebuilt **in place**: the file keeps its path (both ratchets
key on it), its `*View` wrapper and `*Content` struct names (the screenshot
hook and `NativeOnboardingRenderTests` construct them), and its sentence
count, with the sentences gathered into an `enum <File>Words` at the foot of
the file.

### Task 1: The step order, the progress labels and the parity harness

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/FirstRunViews.swift` (no behaviour change; the doc comment records D-3)
- Test: `macos/Tests/TraceCommonsAppTests/OnboardingParityTests.swift` (new)
- Test: `macos/Tests/TraceCommonsAppTests/FirstRunProgressTests.swift` (add the order test)

**Interfaces:**
- Consumes: `OnboardingNavigation.Step`, `FirstRunProgress(step:folders:scan:)`, `FirstRunWords`.
- Produces: `OnboardingParityTests.Step` table and helper; the recorded order Folders, Join, Uses, Scan, Projects.

- [ ] **Step 1: Write the failing order test** (pins D-3's default so a later change is deliberate)

```swift
    /// On a fresh install the folders come before Join, because the daemon
    /// cannot start without them and Join needs the daemon (D-3). The
    /// progress bar shows the real order, not the concept's.
    func test_foldersPrecedeJoinOnAFreshInstall() throws {
        let progress = try XCTUnwrap(FirstRunProgress(step: .roots, folders: true, scan: false))
        XCTAssertEqual(progress.labels, [FirstRunWords.folders, FirstRunWords.join, FirstRunWords.uses, FirstRunWords.projects])
        XCTAssertEqual(progress.current, 0)
    }
```

- [ ] **Step 2: Write the parity harness**

```swift
import XCTest

/// Each onboarding step keeps its daemon calls, its guards and its copy
/// sources through the rebuild. The table is the inventory in the plan.
final class OnboardingParityTests: XCTestCase {
    struct Step {
        let file: String
        let bindings: [String]
        let copySources: [String]
        let guards: [String]
    }

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let steps: [Step] = []

    func test_everyStepKeepsItsBindingsGuardsAndCopy() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            for needle in step.bindings + step.copySources + step.guards {
                XCTAssertTrue(source.contains(needle), "\(step.file) lacks \(needle)")
            }
        }
    }

    /// No rebuilt step reads the legacy palette.
    func test_noStepReadsTheLegacyPalette() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(step.file) reads TC.")
            XCTAssertFalse(source.contains("CommunityBrand"), "\(step.file) reads CommunityBrand")
        }
    }
}
```

- [ ] **Step 3: Run** `swift test --filter 'FirstRunProgressTests|OnboardingParityTests'`. Expected: PASS (the order test passes today; the harness is empty). Commit: `git commit -m "Record the first-run step order and add the onboarding parity harness"`.

### Task 2: Welcome

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingWelcomeView.swift` (rebuild `OnboardingWelcomeContent` in place; delete `WireframeGlobe`, `SignalArc`)
- Test: `OnboardingParityTests.swift`

**Interfaces:**
- Consumes: `TCOnboardingCopy.load()?.welcomeBody`, `GlassPane`, `GlassTokens.TypeScale.display`, `GlassButtonStyle(.primary)`, `GlassButtonStyle(.link)`.
- Produces: `OnboardingWelcomeView(onGetStarted:onWhatGetsRemoved:)`, `OnboardingWelcomeContent(onGetStarted:onWhatGetsRemoved:)` unchanged signatures; `enum OnboardingWelcomeWords` holding the eight sentences verbatim.

- [ ] **Step 1: Add the table row**

```swift
        Step(file: "Views/OnboardingWelcomeView.swift",
             bindings: ["onGetStarted", "onWhatGetsRemoved"],
             copySources: ["TCOnboardingCopy.load()?.welcomeBody", "OnboardingWelcomeWords.headline",
                           "OnboardingWelcomeWords.promiseLine", "OnboardingWelcomeWords.lede",
                           "OnboardingWelcomeWords.scrubbing", "OnboardingWelcomeWords.whatGetsRemoved",
                           "OnboardingWelcomeWords.footer"],
             guards: ["GlassButtonStyle(.primary)", ".keyboardShortcut(.defaultAction)"]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL (`OnboardingWelcomeWords` absent).

- [ ] **Step 3: Rebuild the content** (D-16 default): a `VStack` inside the first-run pane with the headline in `GlassTokens.TypeScale.display` (three `Text`s so the promise lines keep their emphasis, as `GlassTag(_, tone: .accent)` eyebrows rather than a painted highlight), the lede in `body`, the scrubbing sentence verbatim, `Button(OnboardingWelcomeWords.whatGetsRemoved, action: onWhatGetsRemoved).buttonStyle(GlassButtonStyle(.link))`, `Button(OnboardingWelcomeWords.getStarted, action: onGetStarted).buttonStyle(GlassButtonStyle(.primary)).keyboardShortcut(.defaultAction)`, the footer in `GlassTokens.TypeScale.eyebrow`. `.accessibilityElement(children: .combine)` with the full headline as `.accessibilityLabel`, as today. Gather the sentences:

```swift
/// This screen's sentences, verbatim from the shared design spec. The
/// scrubbing concession is load-bearing and must not be softened.
enum OnboardingWelcomeWords {
    static let headline = "You decide what gets contributed."
    static let promiseLine1 = "Nothing is sent"
    static let promiseLine2 = "unless you say so."
    static let lede = """
        Coding agents get better when there are real transcripts to learn \
        from. Almost all of that data is locked inside companies. Trace \
        Commons is a shared pool that isn't.
        """
    static let scrubbing = """
        Before anything leaves this machine it is scrubbed locally for secrets, \
        keys, and tokens. That scrubbing is good and it is not perfect — which \
        is why you get to look first.
        """
    static let whatGetsRemoved = "What gets removed?"
    static let getStarted = "Get started"
    static let footer = "Scrubbed locally · shown to you · sent only on your word"
    static let wordmark = "Trace Commons — Contributor"
}
```

Compare the dump before and after: `OnboardingWelcomeView.swift` must read 8 both times; if the rebuilt file reads 7 because a literal the scanner counted was deleted with the globe (none of its strings are sentences, so it should not), restore the sentence rather than lowering the entry.

- [ ] **Step 4: Register `"Views/OnboardingWelcomeView.swift"` in `GlassSurfaceRulesTests.files`; update `DebugScreenshot.swift:614-618`'s size comment (the globe ladder no longer applies; keep the 900-wide capture); run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|NativeOnboardingRenderTests'`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Draw the Welcome step on glass"`.

### Task 3: Folders (the roots step)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingRootsView.swift` (rebuild in place on `GlassSourceRow`)
- Test: `OnboardingParityTests.swift`, `macos/Tests/TraceCommonsAppTests/OnboardingRootsRowsTests.swift` (exists; keep green)

**Interfaces:**
- Consumes: `GlassSourceRow` (Phase 1 Task 5), `SessionRoots`, `TCDiscovery.sourcesJSON()`, `SourceCandidate.decodeList`, `model.startDaemon(at:settingsJSON:)` with `.refused(reason)`, `.running`, `.needsRoots`, `.starting`.
- Produces: `OnboardingRootsView(configDirectory:onStarted:)` unchanged; `OnboardingRootsView.offeredKinds` kept (Settings reads it until Phase 1 Task 5 switched to `SourceKind.allCases`; keep both for one release); `enum OnboardingRootsWords` with the five sentences.

- [ ] **Step 1: Add the table row**

```swift
        Step(file: "Views/OnboardingRootsView.swift",
             bindings: ["SessionRoots()", "roots.watch(", "roots.isComplete", "roots.settingsJSON()",
                        "TCDiscovery.sourcesJSON()", "SourceCandidate.decodeList(", "model.isStartingDaemon",
                        "model.startDaemon(at:", "GlassSourceRow(", "TCDaemon.TCError.rootsNotDeclared"],
             copySources: ["OnboardingRootsWords.heading", "OnboardingRootsWords.readsTranscripts",
                           "OnboardingRootsWords.answerForBoth", "OnboardingRootsWords.optionalRows",
                           "OnboardingRootsWords.answerBeforeContinuing"],
             guards: [".disabled(!roots.isComplete || model.isStartingDaemon)"]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild**: heading in `GlassTokens.TypeScale.heading`, the three explanation paragraphs in `body`, one `GlassSourceRow` per `SourceKind.allCases` with the same closures, `GlassNotice(tone: .outside)` for `failure`, Continue in `GlassButtonStyle(.primary)` with the guard verbatim. `ActionNoticeDismissTests` names `OnboardingRootsView.failure`; keep the property.

- [ ] **Step 4: Register the file; run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|OnboardingRootsRowsTests|ActionNoticeDismissTests'`. Expected: PASS; dump `OnboardingRootsView.swift` 5.

- [ ] **Step 5: Commit** `git commit -m "Draw the Folders step on glass"`.

### Task 4: Join (invite, NEAR AI, wallet)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingConnectView.swift`, `Views/NearAiJoinView.swift`, `Views/NearAccountConnectView.swift`, `Views/AdmissionPreparationView.swift`, `Views/NativeFlowNotice.swift` (rebuild in place; `NativeFlowNotice` becomes a `GlassNotice` wrapper keeping its `(message:glyph:tone:)` initialiser so `AppDelegate`-adjacent callers stay)
- Test: `OnboardingParityTests.swift`, `macos/Tests/TraceCommonsAppTests/JoinNeverSharesTests.swift` (new)

**Interfaces:**
- Consumes: `InviteLink`, `DeepLink`, `PendingInvite`, `model.enroll(invite:)`, `model.nearAiAccountEnroll(commons:)`, `model.nativeWalletFlow(...)`, `CredentialSection(copy:requiresSession:)` (legacy, hosted until Phase 4 Task 7 moves it; it is `TC`-based, so `NearAiJoinView.swift` is not added to the scan until then), `GlassTextField`, `GlassNotice`.
- Produces: `OnboardingConnectView(onEnrolled:)`, `OnboardingConnectContent(onEnrolled:previewPhase:previewText:)`, `NearAiJoinView(onEnrolled:)`, `NearAccountConnectView(onBusyChanged:onEnrolled:)`, `AdmissionPreparationView(entryID:)` unchanged; `enum OnboardingConnectWords` (7 sentences).

- [ ] **Step 1: Write the failing guard test**

```swift
import XCTest
@testable import TraceCommonsApp

/// Joining never authorizes sharing (concept rule 2, and the coordinator's
/// "Call ordering"): `enroll` carries no scopes, and a link only fills the
/// field.
final class JoinNeverSharesTests: XCTestCase {
    func test_enrollCarriesNoScopes() throws {
        let source = try OnboardingParityTests.text("Views/OnboardingConnectView.swift")
        XCTAssertTrue(source.contains("model.enroll(invite: link.raw)"))
        XCTAssertFalse(source.contains("scopes:"), "Join must not send scopes; Uses applies them")
    }

    func test_aLinkFillsTheFieldAndStops() throws {
        let source = try OnboardingParityTests.text("Views/OnboardingConnectView.swift")
        XCTAssertTrue(source.contains("pendingInvite.take()"))
        let consume = try XCTUnwrap(source.range(of: "private func consumePendingInvite()"))
        let body = source[consume.lowerBound...].prefix(400)
        XCTAssertTrue(body.contains("resolve()"))
        XCTAssertFalse(body.contains("join("), "a link must never enrol by itself")
    }

    func test_anEmptyInviteParameterIsDropped() {
        XCTAssertNil(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=")!))
        XCTAssertEqual(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE")!),
                       "https://issuer.example/onboard#CODE")
    }
}
```

- [ ] **Step 2: Add the table row**

```swift
        Step(file: "Views/OnboardingConnectView.swift",
             bindings: ["InviteLink.parse(", "PendingInvite.shared", "pendingInvite.take()", "model.enroll(invite:",
                        "model.status.loggedIn", "NearAiJoinView(onEnrolled:", "NearAccountConnectView(onBusyChanged:"],
             copySources: ["OnboardingConnectWords.deadInvite", "OnboardingConnectWords.heading",
                           "OnboardingConnectWords.pasteTheLink", "OnboardingConnectWords.alreadyConnected",
                           "OnboardingConnectWords.inviteIsFor(", "OnboardingConnectWords.connectingTo(",
                           "OnboardingConnectWords.join("],
             guards: [".onChange(of: pendingInvite.value)", "case .deadInvite:"]),
```

- [ ] **Step 3: Run** `swift test --filter 'JoinNeverSharesTests|OnboardingParityTests'`. Expected: FAIL on the words.

- [ ] **Step 4: Rebuild** the four files: `GlassTextField(OnboardingConnectWords.linkPrompt, text: $inviteText)` (the prompt `"https://…/onboard#…"` is not a sentence), Look up and Join in `GlassButtonStyle(.glass)` / `.primary`, the dead-invite line as `GlassStatusLabel(OnboardingConnectWords.deadInvite, status: .outside)`, `GlassSectionRule` between the three ways in, `NearAiJoinView` and `NearAccountConnectView` on `GlassCard` with `GlassTextField`s and `GlassNotice` for refusals, `AdmissionPreparationView` likewise. `NativeFlowNotice(message:glyph:tone:)` maps `tone == "refused"` to `GlassNotice(tone: .outside)` and anything else to `.ask`, drawing the glyph only through the notice's own status. Keep `InviteLink`, `DeepLink`, `NativeWalletView`, `AdmissionPreparation` unchanged.

- [ ] **Step 5: Register `OnboardingConnectView.swift`, `NearAccountConnectView.swift`, `AdmissionPreparationView.swift`, `NativeFlowNotice.swift` in the scan (not `NearAiJoinView.swift`, which hosts `CredentialSection` until Phase 4 Task 7); run** `swift test --filter 'JoinNeverSharesTests|OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|NearAiJoinViewTests|NearAccountConnectTests|NearAiOnboardingSessionTests|NativeOnboardingRenderTests|AdmissionPlacementTests'`. Expected: PASS; dump `OnboardingConnectView.swift` 7.

- [ ] **Step 6: Commit** `git commit -m "Draw the Join step on glass"`.

### Task 5: Uses (the consent scopes step)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/ConsentScopesView.swift` (rebuild `ConsentScopesContent` in place)
- Test: `OnboardingParityTests.swift`, `macos/Tests/TraceCommonsAppTests/UsesStepTests.swift` (new)

**Interfaces:**
- Consumes: `model.consentScopes`, `ScopeCopy.title(for:options:)`, `GlassCheckboxStyle`, `GlassButtonStyle(.primary)`.
- Produces: `ConsentScopesView(onContinue:initialSelection:)`, `ConsentScopesContent(onContinue:initialSelection:)` unchanged; `enum ConsentScopesWords` (7 sentences, including the Continue label as `static func continueWith(_ total: Int) -> String`); `enum UsesStep { static func continueLabel(alwaysOn: Int, selected: Int) -> String; static func startsUnticked(_ initial: Set<String>) -> Bool }` (pure).

- [ ] **Step 1: Write the failing pure test** (D-4 default: nothing optional pre-ticked; always-on included and locked)

```swift
import XCTest
@testable import TraceCommonsApp

final class UsesStepTests: XCTestCase {
    func test_nothingOptionalStartsTicked() {
        XCTAssertTrue(UsesStep.startsUnticked([]))
        XCTAssertFalse(UsesStep.startsUnticked(["benchmark_only"]))
    }

    /// The count includes the always-on permission the upload carries.
    func test_theContinueCountIncludesAlwaysOn() {
        XCTAssertEqual(UsesStep.continueLabel(alwaysOn: 1, selected: 0), ConsentScopesWords.continueWith(1))
        XCTAssertEqual(UsesStep.continueLabel(alwaysOn: 1, selected: 2), ConsentScopesWords.continueWith(3))
    }

    /// Always-on rows are locked and on; they are never a tappable control.
    func test_alwaysOnRowsAreLockedOn() throws {
        let source = try OnboardingParityTests.text("Views/ConsentScopesView.swift")
        XCTAssertTrue(source.contains("isOn: .constant(true)"))
        XCTAssertTrue(source.contains(".disabled(scope.alwaysOn)"))
    }
}
```

- [ ] **Step 2: Add the table row**

```swift
        Step(file: "Views/ConsentScopesView.swift",
             bindings: ["model.consentScopes", "initialSelection", "onContinue(selected)", "UsesStep.continueLabel(",
                        "grantsDataUse", "alwaysOn"],
             copySources: ["ScopeCopy.title(for:", "scope.description", "ConsentScopesWords.heading",
                           "ConsentScopesWords.changeLater", "ConsentScopesWords.alwaysIncluded",
                           "ConsentScopesWords.optionalEachOne", "ConsentScopesWords.credit",
                           "ConsentScopesWords.withdrawLater", "ConsentScopesWords.continueWith("],
             guards: ["GlassCheckboxStyle()", ".keyboardShortcut(.defaultAction)"]),
```

- [ ] **Step 3: Run** `swift test --filter 'UsesStepTests|OnboardingParityTests'`. Expected: FAIL.

- [ ] **Step 4: Rebuild**: the three groups as `GlassEyebrowCard`s (`alwaysIncluded`, `optionalEachOne`, `credit`); each scope a `Toggle` with `GlassCheckboxStyle()` whose label is the title in `bodyStrong` over the description in `caption`; always-on rows `isOn: .constant(true)` and `.disabled(scope.alwaysOn)` with a `GlassTag(ConsentScopesWords.alwaysOn)`; `.accessibilityElement(children: .combine)` per row; the withdraw-later line; Continue in `.primary` with `UsesStep.continueLabel(alwaysOn:selected:)` and `.keyboardShortcut(.defaultAction)`. The selection stays local `@State` seeded from `initialSelection`, as today.

- [ ] **Step 5: Register the file; run** `swift test --filter 'UsesStepTests|OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|OnboardingNavigationTests'`. Expected: PASS; dump `ConsentScopesView.swift` 7.

- [ ] **Step 6: Commit** `git commit -m "Draw the Uses step on glass"`.

### Task 6: Scan (the extra privacy scan)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingPrivacyScanView.swift` (rebuild in place; the `.rs` `SURFACES` row pins this path and `TCCoreCopy.privacyScanCopyJSON`, which stays)
- Test: `OnboardingParityTests.swift`

**Interfaces:**
- Consumes: `PrivacyScanCopy` (`title`, `localAlways`, `offer`, `disclosure`, `localOnly`, `withNear`), `model.acknowledgeNearAINotice()`, `GlassPicker`.
- Produces: `OnboardingPrivacyScanView(onContinue:)`, `OnboardingPrivacyScanContent(onContinue:previewChoice:)` unchanged; `Choice` unchanged.

- [ ] **Step 1: Add the table row**

```swift
        Step(file: "Views/OnboardingPrivacyScanView.swift",
             bindings: ["model.daemonSettings?.nearAIConfigured == true", "model.acknowledgeNearAINotice()",
                        "if choice == .localPlusScan"],
             copySources: ["TCCoreCopy.privacyScanCopyJSON()", "copy.title", "copy.localAlways", "copy.offer",
                           "copy.disclosure", "copy.localOnly", "copy.withNear", "Text(verbatim:"],
             guards: ["if let copy {", "GlassPicker("]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL (no `GlassPicker`).

- [ ] **Step 3: Rebuild**: the explanation as three `Text(verbatim:)` paragraphs (the core's sentences may hold `*`), the choice as `GlassPicker(copy.title, selection: $choice, options: [GlassPickerOption(copy.localOnly, value: Choice.localOnly, dot: .off), GlassPickerOption(copy.withNear, value: .localPlusScan, dot: .ask)], placeholder: copy.title)` with `selection` a `Binding<Choice?>` that never writes nil; Continue in `.primary`, calling `acknowledgeNearAINotice()` only on `.localPlusScan`. With no copy the body draws nothing, as today.

- [ ] **Step 4: Register the file; run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests'` and `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Draw the Scan step on glass"`.

### Task 7: Projects (what to watch)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingProjectsView.swift` (rebuild in place)
- Modify: `macos/Sources/TCShellCore/ProjectRow.swift` (delete `ProjectCopy.modeChoiceLabel` once no caller remains; lower `ShellWordingTests.wordingBaseline["TCShellCore/ProjectRow.swift"]` from 3 to the dump's new value, which is the deliberate lowering the baseline comment describes, because the words left for the core's `ContributionModeCopy`)
- Test: `OnboardingParityTests.swift`

**Interfaces:**
- Consumes: `model.projects`, `model.setProjectMode(_:mode:)`, `ProjectCopy.unresolvedBucketNote`, `ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON())` for the "Ask me" / "Never" tag words (`choice(for: "notify_only")?.label`, `choice(for: "ignore")?.label`), `GlassTag`, `GlassButtonStyle(.glass)`.
- Produces: `OnboardingProjectsView(onContinue:)`, `OnboardingProjectsContent(onContinue:)` unchanged; `enum OnboardingProjectsWords` (4 sentences).

- [ ] **Step 1: Add the table row**

```swift
        Step(file: "Views/OnboardingProjectsView.swift",
             bindings: ["model.projects", "model.setProjectMode(project, mode: isIgnored ? .ask : .ignore)",
                        "model.lastActionError", "project.isUnresolvedBucket"],
             copySources: ["ProjectCopy.unresolvedBucketNote", "TCCoreCopy.contributionModeCopyJSON()",
                           "OnboardingProjectsWords.heading", "OnboardingProjectsWords.everyProjectAsksFirst",
                           "OnboardingProjectsWords.everyProjectAsksFirstIgnore", "OnboardingProjectsWords.noProjectsYet"],
             guards: ["Button(isIgnored ? "]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild**: each project a `GlassTableRow` with the label, a `GlassTag(choice.label, tone: isIgnored ? .neutral : .on)` from the core's table (no tag when the table has not loaded), the bucket note, and the Ignore/Ignored `Button` in `.glass`; `GlassNotice(tone: .outside)` for `lastActionError` with its dismiss. `auto_upload` is never offered here (the doc comment stays).

- [ ] **Step 4: Register the file; run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|ProjectRowTests'`. Expected: PASS; dump `OnboardingProjectsView.swift` 4; `ProjectRow.swift` at its new lowered count.

- [ ] **Step 5: Commit** `git commit -m "Draw the Projects step on glass with the core's mode words"`.

### Task 8: Done (login item and notifications)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingDoneView.swift` (rebuild in place)
- Test: `OnboardingParityTests.swift`

**Interfaces:**
- Consumes: `LoginItemManager`, `Notifier`, `GlassCard`, `GlassStatusLabel`, `GlassButtonStyle`.
- Produces: `OnboardingDoneView(onFinish:)`, `OnboardingDoneContent(onFinish:)` unchanged; `enum OnboardingDoneWords` (8 sentences).

- [ ] **Step 1: Add the table row**

```swift
        Step(file: "Views/OnboardingDoneView.swift",
             bindings: ["LoginItemManager.currentState", "LoginItemManager.register()", "Notifier.shared.authorizationStatus()",
                        "Notifier.shared.requestAuthorization()", "Button(OnboardingDoneWords.done, action: onFinish)"],
             copySources: ["Notifier.copy?.doneBody", "Notifier.copy?.notificationOffer", "Notifier.copy?.notNow",
                           "Notifier.copy?.notificationAllow", "Notifier.copy?.notificationAllowed",
                           "Notifier.copy?.notificationDenied", "Notifier.copy?.systemSettings", "Notifier.purpose",
                           "OnboardingDoneWords.setUpNothingSent", "OnboardingDoneWords.startAtLoginQuestion",
                           "OnboardingDoneWords.needsToBeRunning", "OnboardingDoneWords.willStartNextLogin",
                           "OnboardingDoneWords.almostThere", "OnboardingDoneWords.couldNotTurnOn("],
             guards: [".disabled(notificationRequestPending)", "notificationStatus == .notDetermined"]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild**: `GlassStatusLabel(OnboardingDoneWords.setUpNothingSent, status: .on)` as the heading row, the done body, the two offers as `GlassCard`s with their two buttons (`.glass` for Not now, `.primary` for the affirmative), the outcomes as `caption` text, Done in `.primary` with `.keyboardShortcut(.defaultAction)` and `.disabled(notificationRequestPending)`. The `.task` and `.onReceive` refreshes stay.

- [ ] **Step 4: Register the file; run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|OnboardingCompletionTests|NotificationAuthorizationTests'`. Expected: PASS; dump 8.

- [ ] **Step 5: Commit** `git commit -m "Draw the Done step on glass"`.

### Task 9: The coordinator chrome and the what-gets-removed sheet

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/OnboardingCoordinatorView.swift` (Back bar and the four inline notices on glass; sequencing untouched)
- Modify: `macos/Sources/TraceCommonsApp/Views/WhatGetsRemovedSheet.swift` (rebuild in place as a `GlassSheet`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/FirstRunViews.swift` (nothing but the doc comment: "The steps are glass from Phase 2")
- Test: `OnboardingParityTests.swift`

**Interfaces:**
- Consumes: `OnboardingNavigation` (unchanged), `GlassBreadcrumb` for Back, `GlassNotice`, `GlassSheet`, `TCScrubInfo` (check the sheet's source for its exact bridge call before editing).
- Produces: `OnboardingCoordinatorView(startAt:onStep:onComplete:)` unchanged; `enum OnboardingCoordinatorWords` (5 sentences); `WhatGetsRemovedSheet()` unchanged (4 sentences in `WhatGetsRemovedWords`).

- [ ] **Step 1: Add the rows**

```swift
        Step(file: "Views/OnboardingCoordinatorView.swift",
             bindings: ["OnboardingNavigation(step: startAt)", "navigation.beginConsentSave(", "navigation.finishConsentSave(",
                        "model.setConsentScopes(scopes)", "navigation.enrolled(visit: visit)", "OnboardingDoneView(onFinish: onComplete)"],
             copySources: ["OnboardingCoordinatorWords.back", "OnboardingCoordinatorWords.settingsLoading",
                           "OnboardingCoordinatorWords.couldNotSave", "OnboardingCoordinatorWords.scanNoLongerAvailable",
                           "OnboardingCoordinatorWords.scanNotIncluded"],
             guards: [".disabled(navigation.consentSaveInProgress)", "case .failed:", "consentSaveFailed = true"]),
        Step(file: "Views/WhatGetsRemovedSheet.swift",
             bindings: [], copySources: ["WhatGetsRemovedWords."], guards: ["GlassSheet("]),
```

- [ ] **Step 2: Run** `swift test --filter OnboardingParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild** the Back bar as `GlassBreadcrumb([GlassCrumb(OnboardingCoordinatorWords.back)], backLabel: OnboardingCoordinatorWords.back, onBack: { navigation.enter(previous) })`, the four notices as `GlassNotice(tone: .ask)` (settings loading, scan not included) and `GlassNotice(tone: .outside)` (could not save, scan no longer available) with the fallback Continue in `.glass`. The sheet: `GlassSheet(title:)` over its existing list, Close in `.glass`.

- [ ] **Step 4: Register both files; run** `swift test --filter 'OnboardingParityTests|GlassSurfaceRulesTests|ShellWordingTests|OnboardingNavigationTests|FirstRunProgressTests'`. Expected: PASS; dumps 5 and 4.

- [ ] **Step 5: Commit** `git commit -m "Draw the first-run chrome and the what-gets-removed sheet on glass"`.

### Task 10: The concept's deltas, recorded (decision-blocked; no code)

**Files:**
- Modify: this plan (the table below is the deliverable; nothing in `macos/` changes)
- Test: `macos/Tests/TraceCommonsAppTests/OnboardingParityTests.swift` (one test that the unapproved steps do not exist)

**Interfaces:**
- Produces: the DRAFT register the owner approves or strikes; a test pinning that no Sharing, Tools or Rules step was built.

- [ ] **Step 1: Write the pin**

```swift
    /// The concept's Sharing, Tools and Rules steps are decision-blocked
    /// (D-5, D-6). Until the owner approves them and the core exports their
    /// framing, no step by those names exists.
    func test_unapprovedConceptStepsDoNotExist() throws {
        let navigation = try Self.text("OnboardingNavigation.swift")
        for name in ["case sharing", "case tools", "case rules", "case quickSetup", "case customSetup"] {
            XCTAssertFalse(navigation.contains(name), "\(name) was built before its decision")
        }
    }
```

Run: `swift test --filter OnboardingParityTests`. Expected: PASS. Commit: `git commit -m "Pin the unapproved first-run steps as absent"`.

- [ ] **Step 2: The register.** Each row names what exists in the core, what is DRAFT, and what wiring is missing. DRAFT lines are what a screen would need a sentence *for*; the sentences themselves are not written here.

| Concept element | Core copy that exists | DRAFT, NEEDS APPROVAL (a sentence is needed for...) | Wiring missing on macOS | Decision |
|---|---|---|---|---|
| Sharing moment on Uses: "Ask me each time" / "Share automatically" | `tc_automatic_contribution_copy_json` (`path_ask_first`, `path_automatic` with scrub scope and limit; `BRIDGE_ONLY` row), `tc_automatic_grant_copy_json` (the disclosure lines `TracesStore` already renders) | the step's heading; the picker's label; the line that says Ask-me finishes setup; the line that says automatic leads through the disclosures | a first-run route into the Queue's grant path (`AppModel.acceptArmingOffer` takes an `ArmingOffer` minted by the daemon's offer; onboarding has none); the witness disclosure before the grant | D-5 |
| Quick setup / Custom setup tiers | none | both tier names and their one-line descriptions; "Custom setup instead" | none (navigation only) | D-5 |
| Tools step: "Not seeing your tool above?" add box | none | the prompt; the dropped-folder outcome | daemon source detection for an arbitrary folder (not built, #1030 "Mocked for later") | D-5 |
| Rules step: per-repo rule plus past sessions "N of M selected" | `tc_contribution_mode_copy_json` (the three rule labels), `tc_project_ignore_copy_json`, `tc_arming_offer_copy_json` | the step's heading; the past-sessions explanation; the count line | per-session listing and a way to contribute a chosen subset (no backend); repos tagged with their tool (K11) | D-5 |
| Private AI switch on Uses (custom tier) | `tc_inference_connection_copy_json` (`BRIDGE_ONLY`), `PrivateInferenceCopy.offer*` (the Queue's offer card) | the card's placement sentence on an onboarding step | `set_private_ai` over IPC (`notAvailableYet`, Z6); feeding the witness disclosure | D-6 |
| Join's "Skip: watch only" and "contributing needs a near.ai account" | none | both lines | `account_session_status` (`notAvailableYet`, Z13) for the signed-in state | D-5 |
| Returning-user "Welcome back" as re-consent after a void | `tc_grant_void_notice` (the void card `ShellNotices` draws) | the card's heading and its two buttons | a re-grant path from the void notice (today: "Turn back on" re-arms, `rearmGrantVoid`) | D-5 |
| Passkey popups P-1..P-7 | none | all | WebAuthn client (Z11); hidden in the first release by #1030 rule 12 | none needed now |

- [ ] **Step 3: Open the Phase 2 PR** with the inventory, the guard table, the dumps before and after (every rebuilt file at its old count; `ProjectRow.swift` lowered deliberately with the reason), the register above, and the D-3, D-4, D-16 defaults taken. Verification pastes `swift test`, `generate.py --check` and the copy-ratchet output. Tick the attestation.

---

## Phase 3: Skills and Compute

**Spec:** #1152 "Compute: a Settings section, including pause, resume and
withdraw"; "No glass design exists" for Skills (task statement); D-7, D-8.
The minimum: put the existing content on TCDesign surfaces in the Monitor,
keep every binding, and record the design questions for Ron and the owner.

**Inventory.**

| Screen | File | Controls | Bindings | Copy sources |
|---|---|---|---|---|
| Compute | `ComputeView.swift` (`ComputeView(model:)`, `ComputeContent(model:allowance:)`) | allowance `TextField`, Enable, Resume, Pause, Disable, Retry | `ComputeModel.snapshot` (`copy.{introduction, allowanceLabel, allowanceDetail, resume, pause, disable, enable}`, `title`, `detail`, `canEnable`, `canResume`, `canPause`, `available`, `consentGranted`, `ramAllowanceGib`), `controlsBusy`, `quitWasRefused`, `copy?.{quitRefused, unavailable, retry}`, `failureLabel`, `perform(.enable(ramAllowanceGiB:))`, `perform(.resume)`, `perform(.pause)`, `perform(.disable)`, `retryOpen()` | all the core's (`ComputeCopy`); the file authors no sentence |
| Skills | `SkillLearningView.swift` (`SkillLearningView(record:copy:)`) | Learn, name/applicability/procedure editors, Review, Edit skill, Approve and test, Review install, Install (check `InstalledSkillPanel`), Roll back, Inspect runs, Open fixture source | `model.skillLearningState(for:)`, `model.learnSkill(from:)`, `reviewSkill(from:draft:)`, `testSkill(from:)`, `editSkill(from:)`, `reviewSkillInstall(from:)`, `installSkill(from:)`, `rollbackSkill(from:)`, `ensureLocalInstalledSkillStatus(for:)`, `TCSkillLearning.validateDraftJSON`, `TCSkillLearning.errorLine(label:)` | `SkillLearningCopy` (`model.skillLearningCopy`), `model.publicRunCopy?.evidenceKindLabel(for:)`; the file is in `ShellWordingTests.rustOwnedSurfaces` and must stay sentence-free |

Skills is reached today from History → View session (`SessionDetailView`),
per `HistoryRecord`. The Monitor's History page lists
`DaemonData.HistoryRow`s (`submissionId`) and has no selection and no
inspector for a row. D-7's default puts Skills in the History page's
inspector for the selected row, resolved to the `HistoryRecord` by
`submissionID`.

**Design questions for Ron (recorded in the Phase 3 PR; settled
2026-10-06):**
1. Whether Compute's four controls are one segmented control or four glass
   buttons; and whether the allowance field is inline or in a well.
   **Settled: four glass buttons and the field as built in #1258.**
2. Whether Skills is a History-inspector section (default) or a Home card
   that opens a page, as Missions does. **Settled: the History inspector,
   as built in #1258.**
3. The look of the skill editor's three fields and the evaluation report's
   per-trial rows (`SkillTrialRow`), which have no counterpart in #1146.
   **Settled: as built in #1258.**
4. Whether `InstalledSkillPanel`'s rollback warrants a confirmation.
   **Settled: yes.** Roll back opens a confirmation in the core's words
   (`SkillLearningCopy.rollback_confirm_*`, `rollback_keep`; DRAFT, NEEDS
   APPROVAL as new wording), and only its confirm rolls back
   (`ComputeSkillsParityTests.test_aSkillRollbackIsConfirmedFirst`).

### Task 1: Compute on glass in the Settings window

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/ComputeView.swift` (rebuild in place; keep `ComputeView(model:)` and `ComputeContent(model:allowance:)`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift:421-422` (wrap `ComputeView(model: compute)` in the same `ScrollView` and padding the other sections get, no `.tcScreen()`)
- Test: `macos/Tests/TraceCommonsAppTests/ComputeSkillsParityTests.swift` (new), `macos/Tests/TraceCommonsAppTests/ComputeNavigationTests.swift` (exists; `testUnavailableSurfaceRendersWithoutTraceModel` renders `ComputeContent` with `ImageRenderer` and must stay green)

**Interfaces:**
- Consumes: `ComputeModel`, `GlassEyebrowCard`, `GlassCard`, `GlassTextField`, `GlassButtonStyle(.glass)`, `GlassNotice`.
- Produces: `ComputeContent(model:allowance:)` on glass; `enum ComputeAllowance { static func parse(_ text: String) -> UInt64? }` (the `UInt64 > 0` guard, pure).

- [ ] **Step 1: Write the failing tests**

```swift
import XCTest
@testable import TraceCommonsApp

final class ComputeSkillsParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Zero and non-numbers are refused; the daemon refuses an allowance of
    /// nothing and a typed word is not an allowance.
    func test_theAllowanceParsesOnlyAPositiveInteger() {
        XCTAssertNil(ComputeAllowance.parse(""))
        XCTAssertNil(ComputeAllowance.parse("0"))
        XCTAssertNil(ComputeAllowance.parse("8 GiB"))
        XCTAssertEqual(ComputeAllowance.parse("8"), 8)
    }

    func test_computeKeepsEveryControlAndCopySource() throws {
        let source = try Self.text("Views/ComputeView.swift")
        for needle in ["ComputeContent(model:", "snapshot.copy.introduction", "snapshot.copy.allowanceLabel",
                       "snapshot.copy.allowanceDetail", "snapshot.copy.resume", "snapshot.copy.pause",
                       "snapshot.copy.disable", "snapshot.copy.enable", "snapshot.canEnable", "snapshot.canResume",
                       "snapshot.canPause", "snapshot.available", "snapshot.consentGranted", "model.controlsBusy",
                       "model.quitWasRefused", "copy.quitRefused", "copy.unavailable", "copy.retry", "model.failureLabel",
                       ".enable(ramAllowanceGiB:", ".perform(.resume)", ".perform(.pause)", ".perform(.disable)",
                       "model.retryOpen()", "ComputeAllowance.parse(", "GlassTextField("] {
            XCTAssertTrue(source.contains(needle), "ComputeView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter ComputeSkillsParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild** `ComputeContent`: the refusal line as `GlassNotice(tone: .outside)`; the introduction in `body`; the snapshot's `title`/`detail` as a `GlassCard` with `.accessibilityElement(children: .combine)`; the allowance as `GlassTextField(snapshot.copy.allowanceLabel, text: $allowance)` when `canEnable`, else the figure in `GlassTokens.TypeScale.number`; the controls in `.glass`, disabled exactly as today (`enable` also on `ComputeAllowance.parse(allowance) == nil`); `ProgressView()` while busy; the unavailable branch with Retry. The `.onChange(of: model.snapshot?.ramAllowanceGib, initial: true)` seeding stays in `ComputeView`.

- [ ] **Step 4: Register `"Views/ComputeView.swift"` in the scan; run** `swift test --filter 'ComputeSkillsParityTests|GlassSurfaceRulesTests|ComputeNavigationTests|ShellWordingTests'`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Draw Compute on glass in Settings"`.

### Task 2: The Skills panel in the History inspector

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/SkillLearningView.swift` (rebuild in place on glass; keep `SkillLearningView(record:copy:)` and the private sub-structs' names)
- Create: `macos/Sources/TraceCommonsApp/Views/Monitor/HistoryInspector.swift` (`HistoryDetailInspector(row:)`; Phase 4 Task 6 adds Withdraw and the public-run editor to it)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift` (`HistoryPage` gains a `selection: Binding<String>` of `submissionId`; `HistoryRowView` becomes selectable with the same full-width highlight the Traces tree uses)
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift` (`@SceneStorage("monitor.selectedHistory") private var selectedHistory = ""`; the `.home` inspector case draws `HistoryDetailInspector` when `homePage == .history && !selectedHistory.isEmpty`, else `HomeSummaryInspector`)
- Test: `ComputeSkillsParityTests.swift`, `macos/Tests/TraceCommonsAppTests/SkillLearningRenderingTests.swift` and `SkillLearningTests.swift` (exist; keep green)

**Interfaces:**
- Consumes: `AppModel.history: [HistoryRecord]` (`submissionID`), `model.skillLearningCopy: SkillLearningCopy?`, `SkillLearningView(record:copy:)`, `GlassPane`, `GlassEyebrowCard`, `GlassTextField`, `GlassExpander`, `GlassTag`, `GlassButtonStyle`.
- Produces: `struct HistoryDetailInspector: View { let row: DaemonData.HistoryRow }` resolving `model.history.first { $0.submissionID == row.submissionId }`; `enum HistorySelection { static func record(for submissionId: String, in history: [HistoryRecord]) -> HistoryRecord? }` (pure); `MonitorWindowView.selectedHistory`.

- [ ] **Step 1: Write the failing tests**

```swift
    func test_theInspectorResolvesTheRecordBySubmissionId() throws {
        let json = """
        {"submission_id":"s-1","project_label":"payments-api","agent_name":"codex","status":"accepted","submitted_at":"2026-10-01T00:00:00Z"}
        """
        let record = try JSONDecoder().decode(HistoryRecord.self, from: Data(json.utf8))
        XCTAssertEqual(HistorySelection.record(for: "s-1", in: [record])?.submissionID, "s-1")
        XCTAssertNil(HistorySelection.record(for: "s-2", in: [record]))
    }

    func test_skillsLivesInTheHistoryInspector() throws {
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SkillLearningView(record: record, copy: copy)"))
        XCTAssertTrue(inspector.contains("model.skillLearningCopy"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("HistoryDetailInspector("))
        XCTAssertTrue(window.contains("@SceneStorage(\"monitor.selectedHistory\")"))
    }

    func test_skillLearningKeepsEveryCallAndAuthorsNothing() throws {
        let source = try Self.text("Views/SkillLearningView.swift")
        for needle in ["model.skillLearningState(for:", "model.learnSkill(from:", "model.reviewSkill(from:",
                       "model.testSkill(from:", "model.editSkill(from:", "model.reviewSkillInstall(from:",
                       "model.installSkill(from:", "model.rollbackSkill(from:", "model.ensureLocalInstalledSkillStatus(for:",
                       "TCSkillLearning.validateDraftJSON(", "TCSkillLearning.errorLine(label:", "copy.rollbackDisclosure",
                       "copy.evaluationDisclosure", "GlassTextField("] {
            XCTAssertTrue(source.contains(needle), "SkillLearningView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
    }
```

Check `HistoryRecord`'s decoder (`Models.swift:528-560`) for the minimum keys and adjust the JSON.

- [ ] **Step 2: Run** `swift test --filter ComputeSkillsParityTests`. Expected: FAIL.

- [ ] **Step 3: Rebuild `SkillLearningView`** on glass: the heading as `GlassEyebrowCard(copy.heading)`, the promise in `caption`, each stage as today with `GlassTextField` for the name, a `TextEditor` in a `GlassWell` for applicability and procedure (keeping the 150 ms validation `.task(id: draft)`), the evaluation contract in a `GlassWell`, `GlassTag` for the gate result and per-trial pass/fail, `GlassExpander` for "Inspect runs" and "Model output", the buttons in `.primary`/`.glass`, failures as `GlassNotice(tone: .outside)`. Every `minHeight: 44` stays (large text).

- [ ] **Step 4: Write the inspector and the selection**

```swift
import SwiftUI
import TCDesign
import TCShellCore

/// The selected History row's details (D-7). Skills first; Phase 4 adds
/// Withdraw and the public-run editor beside it.
struct HistoryDetailInspector: View {
    let row: DaemonData.HistoryRow
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let record = HistorySelection.record(for: row.submissionId, in: model.history),
                   let copy = model.skillLearningCopy {
                    SkillLearningView(record: record, copy: copy)
                }
            }
        }
    }
}

enum HistorySelection {
    static func record(for submissionId: String, in history: [HistoryRecord]) -> HistoryRecord? {
        history.first { $0.submissionID == submissionId }
    }
}
```

`HistoryPage` takes `selection: Binding<String>`; `HistoryRowView` is wrapped in a `Button(action: { selection = row.submissionId })` with `.buttonStyle(.plain)` and `.accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)`, highlighted with `GlassColor.ink(0.12)` behind the row when selected, matching `TracesTreeView`'s selection. `HomeTabView` passes the binding through.

- [ ] **Step 5: Register `"Views/SkillLearningView.swift"` and `"Views/Monitor/HistoryInspector.swift"` in the scan; run** `swift test --filter 'ComputeSkillsParityTests|GlassSurfaceRulesTests|SkillLearningRenderingTests|SkillLearningTests|SkillLearningClientTests|HomeTests|ShellWordingTests'`. Expected: PASS; `SkillLearningView.swift` still absent from the dump.

- [ ] **Step 6: Commit** `git commit -m "Put Skills in the History inspector on glass"`.

### Task 3: Record the design questions and open the Phase 3 PR

- [ ] **Step 1:** Put the four design questions above and the D-7, D-8 defaults taken in the PR body, with a screenshot request for Ron (the agent cannot capture; ask Ron to run `TRACE_COMMONS_MONITOR=1` — until Phase 4 — and review on macOS 26 and 14).
- [ ] **Step 2:** Run the whole gate (`swift test`, `generate.py --check`, the copy ratchet); paste the output; tick the attestation.

---

## Phase 4: The R15 cutover

**Spec:** #1173 R15 ("Retire Tauri on macOS, once R6, R7, R9, R11 and R12
match"; on `main` the comments call the release switch R15); #1152
"Screens: every screen in the current app maps to the new shell. Nothing
becomes unreachable", "Behaviour that must not depend on the inspector",
"Acceptance"; D-1, D-2, D-9 to D-14, D-18, D-19.

**What the legacy main window and menu bar do that the Monitor does not, and
where each goes.** Verified against `TraceCommonsAppMain.swift`,
`MainWindowView.swift`, `MenuBarView.swift`, `AppDelegate.swift`,
`DebugScreenshot.swift`, `SelfTest.swift`, `macos/scripts/run-demo.sh`:

| Duty | Today | After R15 | Task |
|---|---|---|---|
| Live daemon data | `MainWindowView` reads `AppModel`; the Monitor's stores read `DaemonDataWiring.sample` | the stores read `model.daemonData`, re-attached when the daemon restarts, as `MenuBarStripLabel` does | 1 |
| Window IDs | `WindowID.main` ("trace-commons-main"), `.monitor`, `.firstRun`, `.menuPreview` | `.main` deleted; `.monitor` is the main window; `.firstRun`; `.menuPreview` debug-only | 11 |
| `ShellNotices` (attached daemon, grant void, arming rewording, gate held, legacy migration) | `MainWindowView.swift`, drawn by both windows (`ShellNoticesPlacementTests`) | `Views/ShellNotices.swift` on TCDesign, drawn by the Monitor's main pane and the first-run pane | 2 |
| Navigation and external openers (`OpenMainWindow`, `MainWindowNavigation.section`) | notifications' Review, Dock reopen, invite links, quit refusal, `TRACE_COMMONS_SHOW_WINDOW`, `MainWindowCommands` ⌘1-7, the glass panel's `openMain(_:)` | `MonitorDestination` + `OpenMonitor` (pending-until-handler) + `LaunchRouting`; `MonitorCommands` ⌘1-3 tabs, ⌘⇧M | 3 |
| The onboarding gate | `MainWindowView.traceContent` draws `OnboardingCoordinatorView` while `model.requiresOnboarding` | the first-run window opens at launch and on every `OpenMonitor` request while `requiresOnboarding`; the Monitor opens otherwise | 4 |
| Queue: `PreviewSheet` (Search, What's in it, Transcript with `transcript-copy-all`, Permissions; witness review; verdict/correction; admission; send disclosure; certificates), the three `HealthBanner`s (`model.health`, `budgetHealth`, `witnessCapacityHealth`), `UndoBar` outside the inspector, `PrivateInferenceOfferCard`, `ArmingOfferCard`, Submit all / as verdict, `NotOfferedDisclosure`, surviving-secret line, `WeekBand` | `QueueView.swift`, `QueueFolderRow.swift`, `PreviewSheet.swift` | hosted in the Traces tab: the sheet from the inspector's "Look inside"; the health banners, offer cards and the undo notice above the tree; Submit all on the folder row; `WeekBand` dropped (the chart footer and legend carry the week) | 5 |
| History: Withdraw with confirmation and outcome, session detail (public-run editor), roster (`CommunitySection`), `CreditRecordView`, quarantine explanations | `HistoryView.swift`, `SessionDetailView.swift`, `SessionContributionOverview.swift`, `WithdrawalCopy.swift`, `CreditRecordView.swift` | hosted in `HistoryDetailInspector` (withdraw, public run) and a History-page `GlassNotice` for held explanations; roster and `CreditRecordView` dropped from the window (the Home credit card and `publicProfile` in Settings carry them) | 6 |
| Private AI: the switch with `offerWhat`/`offerExposure`, `CredentialSection`, `HarnessListSection`, `PrivateInferenceActivationView`'s startup gate | `PrivateInferenceView.swift`, `PrivateInferenceActivationView.swift` | hosted in the Inference tab's inspector (`InferenceAccount.swift`); `MenuBarContent.performPrivateInferenceTray` → `PrivateInferenceTray` in TCShellCore | 7 |
| Insights (`InsightsView`, `InsightsStoreSelection` from `CommandLine.arguments`), Mission drafts (`MissionDraftsView`, `MissionDraftsModel`) | sidebar destinations; never start services | Home pages `.insights` and `.missionDrafts`; the Monitor starts services on appear (D-11) | 8 |
| Menu bar: `MenuBarLabel` (the mark with the badge), `MenuBarContent` (the AppKit menu), pause words, `Format` | `MenuBarView.swift` | the glass strip and panel, release default; `Format` → TCShellCore; pause words stay in `MenuBarView.swift` until D-12 | 9 |
| Undo | `AppModel.undo` / `undoApproval()` (five seconds, the `UndoBar`); `TracesStore.lastContributed` + `pendingUndo` in `SessionInspectorView` only | the glass undo notice above the tree, visible with the inspector hidden (spec "Behaviour that must not depend on the inspector") | 5 |
| Quit prompt and refusal | `AppDelegate.applicationShouldTerminate` → `QuitConfirmation.granted`; refusal sets `navigation.section = .compute` and `OpenMainWindow.request()` | unchanged prompt; refusal calls `OpenMonitor.request(.settings(.compute))` | 3 |
| Notifications | `Notifier.shared.onReview = { OpenMainWindow.request() }` | `OpenMonitor.request(.traces(entryId: nil))` | 3 |
| Sparkle / updates | `UpdateController.shared.start()` from `Launcher.startServices`; "Check Now" in Settings | unchanged; the Startup section (Phase 1) holds Check Now | 11 (no change, pinned) |
| `TRACE_COMMONS_SHOW_WINDOW`, `TRACE_COMMONS_APPEARANCE`, `TRACE_COMMONS_SCREENSHOT_DIR`, `TRACE_COMMONS_SELFTEST_OUT`, `TRACE_COMMONS_ONBOARD_SELFTEST_*`, `TRACE_COMMONS_RESUME_CHECK_OUT`, `TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT`, `TRACE_COMMONS_QUIT_AFTER_SHOT`, `TRACE_COMMONS_CONTRIBUTOR_DIR` | `TraceCommonsAppMain.swift`, `DebugScreenshot.swift`, `SelfTest.swift`, `DesignSystem.swift`, `run-demo.sh`, `release-apps.yml` (credential check) | all kept; `SHOW_WINDOW` opens the Monitor (or the first-run window); `APPEARANCE` moves to `TraceCommonsAppMain` alone (D-2); `TRACE_COMMONS_GLASS_MENU`, `TRACE_COMMONS_MONITOR`, `TRACE_COMMONS_FIRST_RUN` deleted; `TRACE_COMMONS_MENU_PREVIEW`, `TRACE_COMMONS_SAMPLE` debug-only | 11 |
| `DebugScreenshot` (eleven legacy renders), `SelfTest` (AppModel only) | | `DebugScreenshot` renders the glass screens and the hosted surfaces; `SelfTest` unchanged | 11 |
| Accessibility identifiers | `"transcript-copy-all"` in `PreviewSheet.swift` | kept (the sheet is hosted) | 5 |
| Tests naming legacy views | listed in "What `origin/main` holds" | each retargeted in the task that moves its subject; `LegacyShellRetiredTests` pins the deletions | 12 |
| Copy ratchet paths (`MainWindowView.swift`, `QueueView.swift`, `SettingsView.swift`) and wording baseline entries | | repointed or deleted in the task that moves the file | 2, 5, 9, 12 |
| CI: `macos-15-fallback-tests` runs `TCDesignTests` only; `make-app-bundle.sh`; `release-apps.yml` | | the fallback job also runs `GlassSurfaceRulesTests` and `MonitorNavigationTests`; the bundle script and release workflow need no change (pinned by a test that no env flag is read in release code paths) | 13 |

### Task 1: The Monitor on the live client

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift:54-87` (the stores take `client: nil` and attach the live client; sample only under `#if DEBUG` and `TRACE_COMMONS_SAMPLE`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/TracesStore.swift`, `InferenceStore.swift`, `HomeStore.swift` (each gains `attach(_ client: (any DaemonDataClient)?)` and a nil-client idle state, as `MenuPanelStore` has; check `MenuPanelStore.attach` for the shape)
- Test: `macos/Tests/TraceCommonsAppTests/MonitorNavigationTests.swift` (new; this task adds the live-client pin)

**Interfaces:**
- Consumes: `AppModel.liveData: LiveDaemonClient?` (`@Published`), `AppModel.daemonData`, `MenuPanelStore.attach(_:)`.
- Produces: `TracesStore.attach(_:)`, `InferenceStore.attach(_:)`, `HomeStore.attach(_:)`; `MonitorWindowView.sampleClient() -> (any DaemonDataClient)?` returning a sample client only in debug builds with `TRACE_COMMONS_SAMPLE` set, else nil.

- [ ] **Step 1: Write the failing pin** (mirrors `test_thePanelUsesTheLiveClient`)

```swift
import XCTest
@testable import TraceCommonsApp

final class MonitorNavigationTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The Monitor's three stores read the app's live client, re-attached
    /// whenever the daemon restarts; sample data is debug-only and opt-in.
    func test_theMonitorUsesTheLiveClient() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertFalse(window.contains("DaemonDataWiring.sample(choice.set)"), "a store is built on sample data")
        for store in ["traces", "inference", "home"] {
            XCTAssertTrue(window.contains("\(store).attach(client)"), "\(store) is never attached to the live client")
        }
        XCTAssertTrue(window.contains(".task(id: model.liveData.map(ObjectIdentifier.init))"))
        let sample = try XCTUnwrap(window.range(of: "static func sampleClient()"))
        let body = window[sample.lowerBound...].prefix(600)
        XCTAssertTrue(body.contains("#if DEBUG"), "sample data must be debug-only")
        XCTAssertTrue(body.contains("TRACE_COMMONS_SAMPLE"))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter MonitorNavigationTests`. Expected: FAIL.

- [ ] **Step 3: Implement.** In `MonitorWindowView`:

```swift
    @State private var traces = TracesStore(client: nil)
    @State private var inference = InferenceStore(client: nil)
    @State private var home = HomeStore(client: nil)

    /// Sample data, debug builds only, when `TRACE_COMMONS_SAMPLE` names a
    /// set; nil otherwise, and then the live client is attached below.
    static func sampleClient() -> (any DaemonDataClient)? {
        #if DEBUG
        let choice = sampleChoice(ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"])
        guard ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] != nil else { return nil }
        if choice.unknown { NSLog("TRACE_COMMONS_SAMPLE unrecognised; fallback %@", choice.set.rawValue) }
        return DaemonDataWiring.sample(choice.set)
        #else
        return nil
        #endif
    }
```

and on the body: `.task(id: model.liveData.map(ObjectIdentifier.init)) { let client = Self.sampleClient() ?? model.daemonData; traces.attach(client); inference.attach(client); home.attach(client); async let a: () = traces.run(); async let b: () = inference.run(); async let c: () = home.run(); _ = await (a, b, c) }`. Each store's `attach` replaces its client and resets to `.loading`; `run()` with a nil client returns after marking the core down (`ScreenState.coreDown`, which the tabs already draw). Keep the `sample:`/`sampleUnknown:` markers on `TracesStore` for the debug path.

- [ ] **Step 4: Run** `swift test --filter 'MonitorNavigationTests|TracesTreeTests|HomeTests|MonitorInferenceDotTests|MonitorReviewTests|MenuBarGlassPanelTests'`. Expected: PASS (the store tests construct their own sample clients).

- [ ] **Step 5: Commit** `git commit -m "Attach the Monitor's stores to the live daemon client"`.

### Task 2: `ShellNotices` on TCDesign, in its own file

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/ShellNotices.swift` (`ShellNotices`, `AttachedDaemonNotice`, `GrantVoidNotices`, `GrantVoidNoticeCard`, `ArmingRewordingNotices`, `ArmingRewordedNoticeCard`, `GateHeldNoticeCard`, `LegacyMigrationNoticeCard`, moved from `MainWindowView.swift:742-1111` and redrawn on `GlassNotice`; plus a wording-free `HealthBanner(health:onAction:)` rebuilt on `GlassNotice`, without the legacy one's two Swift sentences: the `.help("Not wired up in this build.")` developer hint is dropped (the button is simply absent when `onAction == nil`) and the `"Needs attention. \(health.title)"` accessibility label becomes the notice's tone plus `health.title`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift` (remove the moved structs; delete `DaemonStartupNotice`, whose three sentences leave the shell in Task 7 for the core's core-down words; move `CenteredNotice`, which authors nothing, to `PreviewSheet.swift`, its only remaining caller)
- Modify: `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs` (the "grant void notice" and "arming rewording notice" rows: `TraceCommonsApp/Views/MainWindowView.swift` → `TraceCommonsApp/Views/ShellNotices.swift`)
- Modify: `macos/Tests/TraceCommonsAppTests/ShellNoticesPlacementTests.swift` (reads `ShellNotices.swift` for the cards and both windows for `ShellNotices()`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/FirstRunViews.swift` (draw `ShellNotices()` above the step: a void during first run is told there too)
- Test: `macos/Tests/TraceCommonsAppTests/ShellNoticesPlacementTests.swift`

**Interfaces:**
- Consumes: `TCAttach.copy()`, `TCConsentCopy.voidNoticeJSON(forVoid:)`, `GrantVoidNotice.decode(fromJSON:)`, `notice.rearmTarget(for:)`, `TCConsentCopy.armingRewordedNoticeJSON(forRewording:)`, `ArmingRewordedNotice`, `model.gateHeldNotice`, `model.legacyMigrationNotice`, `model.acknowledgeGrantVoid(id:)`, `model.rearmGrantVoid(id:projectID:)`, `model.acknowledgeArmingRewording(id:)`, `model.askFirst(projectID:)`, `model.acknowledgeLegacyInviteMigration()`, `GlassNotice`.
- Produces: the same struct names and initialisers; `HealthBanner(health:onAction:)` on glass, drawn by Phase 4 Task 5's `TracesOffersBar` for `model.health`, `model.budgetHealth` and `model.witnessCapacityHealth`, as `QueueContent` draws them today (`QueueView.swift:79-88`).

- [ ] **Step 1: Rewrite the placement test to the new file and run it to fail**

```swift
    func test_theMonitorAndTheFirstRunPaneBothShowTheNotices() throws {
        for file in ["Views/MonitorWindowView.swift", "Views/Monitor/FirstRunViews.swift"] {
            XCTAssertTrue(try MonitorNavigationTests.text(file).contains("ShellNotices()"), "\(file) does not draw the notices")
        }
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        for card in ["AttachedDaemonNotice()", "GrantVoidNotices(", "ArmingRewordingNotices(", "GateHeldNoticeCard(", "LegacyMigrationNoticeCard("] {
            XCTAssertTrue(notices.contains(card), "ShellNotices lacks \(card)")
        }
        XCTAssertNil(notices.range(of: #"\bTC\."#, options: .regularExpression))
    }
```

Run: `swift test --filter ShellNoticesPlacementTests`. Expected: FAIL.

- [ ] **Step 2: Move and redraw.** Each card is `GlassNotice(tone: .ask, title: notice.title) { ... }` with the body, reasons and the buttons (`Button(notice.acknowledge, action:)` in `.glass`, the rearm/ask-first button in `.primary`) and `.accessibilityElement(children: .contain)` + `.accessibilityLabel(Text(notice.title))` as today; the attached-daemon notice `tone: .off`; refusals in `GlassColor.textPrimary` with a `GlassStatusLabel(failed, status: .ask)`. The moved cards author no sentence; `ShellNotices.swift` must not appear in the dump at all. The sentences that stay behind in `MainWindowView.swift` (`DaemonStartupNotice`'s three, `HealthBanner`'s two, the subtitles, the watch chip, the pause menu) are deleted with the file in Task 11; the dump decides the file's count in between, and the baseline entry is set to whatever it reads, lowered deliberately.

- [ ] **Step 3: Repoint the two `.rs` rows; run** `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central` and `swift test --filter 'ShellNoticesPlacementTests|GlassSurfaceRulesTests|ShellWordingTests|LegacyMigrationNoticeModelTests'` (register `"Views/ShellNotices.swift"` in the scan). Expected: PASS.

- [ ] **Step 4: Commit** `git commit -m "Move the shell notices to their own file on glass"`.

### Task 3: `MonitorDestination`, `OpenMonitor` and `LaunchRouting`

**Files:**
- Create: `macos/Sources/TraceCommonsApp/MonitorNavigation.swift`
- Modify: `macos/Sources/TraceCommonsApp/MainWindowNavigation.swift` (`section: MainWindowView.Section` → `pending: MonitorDestination?`; `displaysInsights`/`displaysCompute` deleted; `activateServicesIfNeeded` keeps its signature but no longer reads a section: D-11)
- Modify: `macos/Sources/TraceCommonsApp/AppDelegate.swift:98-99,112,131` (`OpenMonitor.request(.settings(.compute))`, `OpenMonitor.request()`, `OpenMonitor.request()`)
- Modify: `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift` (`OpenMainWindow` → `OpenMonitor`; `Launcher.launch` installs the handler; `Notifier.shared.onReview = { OpenMonitor.request(.traces(entryId: nil)) }`; `TRACE_COMMONS_SHOW_WINDOW` → `OpenMonitor.request()`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift` (consume `navigation.pending` on appear and on change: set `tab`, `homePage`, `selectedSession`, open Settings at a section via `openSettings()` plus `@AppStorage("settings.section")`? No: `@SceneStorage("settings.section")` is per scene and cannot be written from outside; add `MainWindowNavigation.settingsSection: SettingsSection?` that `MonitorSettingsWindow` reads on appear and clears)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift:802-806` (`openMain(_ section: MainWindowView.Section)` → `open(_ destination: MonitorDestination)` calling `OpenMonitor.request(destination)`; `openMain(.queue)` → `.traces(entryId: nil)`, `.history` → `.home(.history)`, `.privateInference` → `.inference`, `.settings` → `.settings(.watchedFolders)`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Settings/PrivateAISection.swift` (`navigation?.section = .privateInference` → `OpenMonitor.request(.inference)`)
- Modify tests: `ComputeNavigationTests.swift` (the three `MainWindowView.Section` loops become `MonitorDestination` cases), `WindowServiceActivationTests.swift` (no section), `SettingsParityTests` (the Private AI binding needle becomes `OpenMonitor.request(.inference)`)
- Test: `macos/Tests/TraceCommonsAppTests/MonitorNavigationTests.swift`

**Interfaces:**
- Produces:

```swift
/// Where an outside caller wants the Monitor to go. Nothing here sends.
enum MonitorDestination: Equatable, Sendable {
    case home(HomeTabView.Page)
    case inference
    case traces(entryId: String?)
    case settings(SettingsSection)
}

/// Opening the Monitor from outside a SwiftUI view: a notification action,
/// a Dock click, an invite link, a quit refusal, `TRACE_COMMONS_SHOW_WINDOW`.
/// Requests before the handler exists are held and replayed once, as
/// `OpenMainWindow` did; the last destination wins.
enum OpenMonitor {
    @MainActor static var handler: ((MonitorDestination?) -> Void)? { didSet { replayIfPending() } }
    @MainActor private static var pending: (held: Bool, destination: MonitorDestination?) = (false, nil)
    @MainActor static func request(_ destination: MonitorDestination? = nil)
    @MainActor static func reset()   // tests only
}

/// Which window a request opens: first run while onboarding is required,
/// the Monitor otherwise. Pure.
enum LaunchRouting {
    enum Window: Equatable { case firstRun, monitor }
    static func window(requiresOnboarding: Bool) -> Window
}
```

- [ ] **Step 1: Write the failing tests** (Review Focus 4)

```swift
    @MainActor
    func test_aRequestBeforeTheHandlerIsReplayedOnce() {
        OpenMonitor.reset()
        var opened: [MonitorDestination?] = []
        OpenMonitor.request(.traces(entryId: nil))
        OpenMonitor.request(.settings(.compute))
        XCTAssertTrue(opened.isEmpty)
        OpenMonitor.handler = { opened.append($0) }
        XCTAssertEqual(opened.count, 1, "a held request replays exactly once")
        XCTAssertEqual(opened.first, .settings(.compute), "the last destination wins")
        OpenMonitor.request(nil)
        XCTAssertEqual(opened.count, 2)
    }

    func test_firstRunWinsWhileOnboardingIsRequired() {
        XCTAssertEqual(LaunchRouting.window(requiresOnboarding: true), .firstRun)
        XCTAssertEqual(LaunchRouting.window(requiresOnboarding: false), .monitor)
    }

    /// Every outside opener goes through OpenMonitor; nothing names the
    /// deleted main window.
    func test_everyOpenerUsesOpenMonitor() throws {
        let delegate = try Self.text("AppDelegate.swift")
        XCTAssertTrue(delegate.contains("OpenMonitor.request(.settings(.compute))"), "quit refusal must land on Compute")
        XCTAssertTrue(delegate.contains("OpenMonitor.request()"))
        XCTAssertFalse(delegate.contains("OpenMainWindow"))
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("Notifier.shared.onReview = { OpenMonitor.request(.traces(entryId: nil)) }"))
        XCTAssertTrue(main.contains("TRACE_COMMONS_SHOW_WINDOW"))
        XCTAssertFalse(main.contains("WindowID.main"))
        let panel = try Self.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertFalse(panel.contains("MainWindowView.Section"))
        XCTAssertTrue(panel.contains("OpenMonitor.request("))
    }
```

- [ ] **Step 2: Run** `swift test --filter MonitorNavigationTests`. Expected: FAIL.

- [ ] **Step 3: Implement.** `OpenMonitor.request` with a handler: `handler(destination)`; without: `pending = (true, destination)`. The handler installed in `Launcher.launch`: `NSApp.activate(ignoringOtherApps: true); navigation.pending = destination; switch LaunchRouting.window(requiresOnboarding: model.requiresOnboarding) { case .firstRun: openWindow(id: WindowID.firstRun); case .monitor: openWindow(id: WindowID.monitor) }` and, for `.settings(let section)`, `navigation.settingsSection = section; openSettings()` (the `Launcher` view gains `@Environment(\.openSettings)`). The Monitor consumes `navigation.pending` in `.onChange(of: navigation.pending, initial: true)`: `.home(let page)` → `tab = .home; homePage = page`; `.inference` → `tab = .inference`; `.traces(let id)` → `tab = .traces; if let id { Self.review(id, selection: &selectedSession, showsInspector: &showsInspector) }`; then `navigation.pending = nil`. `MonitorSettingsWindow` reads `navigation.settingsSection` on appear into `section` and clears it. D-14: a deep link while onboarded still only opens the Monitor; the comment on `AppDelegate.application(_:open:)` says so.

- [ ] **Step 4: Run** `swift test --filter 'MonitorNavigationTests|ComputeNavigationTests|WindowServiceActivationTests|QuitCoordinatorTests|SettingsParityTests|MenuBarGlassPanelTests'`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Route every outside opener to the Monitor by destination"`.

### Task 4: The first-run window as the onboarding gate

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift` (`Launcher.launch`: after `activateServices()`, `OpenMonitor.request()` once `model.startup` and `model.status` have first answered; the app is regular, so a window opens on a normal launch as today)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/FirstRunViews.swift` (on `requiresOnboarding` turning false: `dismissWindow(id: WindowID.firstRun)` then `OpenMonitor.request(.home(.overview))`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift` (while `model.requiresOnboarding`, the main pane draws `ShellNotices()` and a `GlassNotice(tone: .ask, title: MonitorWords.signedOut)` with a `Button` that calls `OpenMonitor.request()` (which routes to first run) instead of the tabs' content; `MonitorWords.signedOut` is the core's word, approved in #1218)
- Test: `MonitorNavigationTests.swift`

**Interfaces:**
- Consumes: `model.requiresOnboarding`, `model.markOnboardingComplete()`, `MonitorWords.signedOut`, `LaunchRouting`.
- Produces: the gate; `FirstRunWindowView` unchanged in signature.

- [ ] **Step 1: Write the failing pins**

```swift
    func test_theMonitorNeverShowsTabsWhileOnboardingIsRequired() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("if model.requiresOnboarding {"), "the Monitor must gate on requiresOnboarding")
        XCTAssertTrue(window.contains("MonitorWords.signedOut"))
    }

    func test_finishingFirstRunOpensTheMonitor() throws {
        let firstRun = try Self.text("Views/Monitor/FirstRunViews.swift")
        XCTAssertTrue(firstRun.contains("dismissWindow(id: WindowID.firstRun)"))
        XCTAssertTrue(firstRun.contains("OpenMonitor.request(.home(.overview))"))
    }
```

- [ ] **Step 2: Run** `swift test --filter MonitorNavigationTests`. Expected: FAIL. **Step 3:** implement as listed. **Step 4:** run `swift test --filter 'MonitorNavigationTests|FirstRunProgressTests|OnboardingCompletionTests'`; PASS. **Step 5:** commit `git commit -m "Gate the Monitor on first run and hand off when it completes"`.

### Task 5: Traces gap closure (hosted preview sheet, offer cards, undo above the tree, Submit all, surviving secret)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Monitor/TracesOffers.swift` (`TracesOffersBar`: `PrivateInferenceOfferCard` and `ArmingOfferCard` moved from `QueueView.swift:1236-1330` unchanged in body, the glass undo notice moved out of `SessionInspectorView.pendingUndo`, and the surviving-secret line)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/TracesViews.swift` (`TracesTreeView` draws `TracesOffersBar` above the tree; the folder row gains Submit all via `store.approveFolder`; `SessionInspectorView` gains "Look inside" presenting `PreviewSheet(entry:preloaded:)` as a `.sheet`; `pendingUndo` moves out)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/TracesStore.swift` (`approveFolder(projectId:)` and `cancelFolder(projectId:)` actions on `DaemonDataClient`; `residualSecretLine(for:)` via `TCCoreCopy.residualSecretLine`)
- Modify: `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift` (no `tcScreen()` on the hosted root; keep everything else, including `transcript-copy-all`)
- Delete: `macos/Sources/TraceCommonsApp/Views/QueueView.swift`, `Views/QueueFolderRow.swift` (its ignore confirmation already exists in `TracesViews.swift:203-206`), `Views/SessionContributionOverview.swift` stays (Task 6)
- Modify: `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs` ("arming offer" → `TraceCommonsApp/Views/Monitor/TracesOffers.swift`; "surviving secret" → `TraceCommonsApp/Views/Monitor/TracesStore.swift`; "ignore project" `QueueFolderRow.swift` → `TraceCommonsApp/Views/Monitor/TracesViews.swift`)
- Modify: `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (delete the `QueueView.swift` and `QueueFolderRow.swift` entries: their sentences leave the shell; the Traces tab reads `MonitorTracesCopy`. Confirm with the dump that no sentence moved into `TracesOffers.swift`; the two cards author none)
- Modify tests: `QueueEntryOriginTests`, `PreviewSlotTests`, `PreviewSizingTests` (check each for `QueueView`/`QueueContent` references and retarget to `TracesOffers`/`PreviewSheet`)
- Test: `macos/Tests/TraceCommonsAppTests/TracesGapTests.swift` (new)

**Interfaces:**
- Consumes: `AppModel.undo`, `undoApproval()`, `dismissUndo()`, `armingOffer`, `acceptArmingOffer(_:)`, `declineArmingOffer(_:)`, `answerPrivateInferenceOffer(accepted:)`, `privateInferenceOfferDue` (check the exact name in `QueueContent`), `loadCaptureSample`, `PreviewSheet(entry:preloaded:)`, `QueueEntry` (legacy) resolved from `DaemonData.QueueEntry` by `entryId` through `model.awaitingDecision`, `TCCoreCopy.residualSecretLine(...)` with the arguments `QueueView.swift:434` passes, `DaemonDataClient.approveFolder(projectId:)`.
- Produces: `struct TracesOffersBar: View { let store: TracesStore }`; `TracesStore.approveFolder(_ projectId: String) async`; `TracesStore.residualSecretLine(for entryId: String) -> String?`; `enum QueueEntryBridge { static func legacyEntry(for entryId: String, in awaiting: [QueueEntry]) -> QueueEntry? }` (pure).

- [ ] **Step 1: Write the failing tests** (Review Focus 3 and the spec's inspector-independence rule)

```swift
import XCTest
@testable import TraceCommonsApp

final class TracesGapTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The undo notice and the consent offers live above the tree, so they
    /// are visible with the inspector hidden (spec, "Behaviour that must not
    /// depend on the inspector").
    func test_undoAndOffersDoNotDependOnTheInspector() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct SessionInspectorView"))
        let treeBody = tree[treeView.lowerBound..<inspector.lowerBound]
        XCTAssertTrue(treeBody.contains("TracesOffersBar(store: store)"))
        let inspectorBody = tree[inspector.lowerBound...]
        XCTAssertFalse(inspectorBody.contains("pendingUndo"), "undo must not live only in the inspector")
    }

    /// Hosted legacy content paints no ground of its own inside a glass
    /// surface (Review Focus 3).
    func test_hostedLegacyContentPaintsNoGround() throws {
        for file in ["Views/PreviewSheet.swift", "Views/Monitor/TracesOffers.swift"] {
            let source = try Self.text(file)
            XCTAssertFalse(source.contains(".tcScreen()"), "\(file) paints TC.ground inside glass")
        }
    }

    /// The surviving-secret line and the arming offer still read the core.
    func test_theCoreWordsStillReachTheTracesTab() throws {
        XCTAssertTrue(try Self.text("Views/Monitor/TracesStore.swift").contains("TCCoreCopy.residualSecretLine("))
        XCTAssertTrue(try Self.text("Views/Monitor/TracesOffers.swift").contains("TCCoreCopy.armingOfferCopyJSON("))
        for banner in ["HealthBanner(health: health", "HealthBanner(health: budget", "HealthBanner(health: witness"] {
            XCTAssertTrue(try Self.text("Views/Monitor/TracesOffers.swift").contains(banner), "the \(banner) banner left the shell")
        }
        XCTAssertTrue(try Self.text("Views/Monitor/TracesViews.swift").contains("PreviewSheet(entry:"))
        XCTAssertTrue(try Self.text("Views/PreviewSheet.swift").contains("\"transcript-copy-all\""))
    }

    func test_theLegacyQueueIsGone() {
        XCTAssertFalse(FileManager.default.fileExists(atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/QueueView.swift").path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/QueueFolderRow.swift").path))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter TracesGapTests`. Expected: FAIL.

- [ ] **Step 3: Implement.** `TracesOffersBar` draws, in order: the three health banners (`HealthBanner(health:onAction:)` from Task 2, for `model.health` with the same `onAction` `QueueContent` wires at `QueueView.swift:79-84`, then `model.budgetHealth`, then `model.witnessCapacityHealth`); the arming offer card when `model.armingOffer != nil`; the Private AI offer card when the legacy `QueueContent` condition holds (copy the condition verbatim from `QueueView.swift:110-118`); the undo notice (`GlassNotice(tone: .ask, title: contributed.toast.line)` with Undo in `.primary`, moved from `SessionInspectorView` with its refusal line); the surviving-secret line as `GlassStatusLabel(line, status: .ask)` for the selected session. The folder row's Submit all is a `Button` in `GlassButtonStyle(.submit(done: false))` calling `store.approveFolder(folder.id)`, drawn only when the core's eligibility offers Contribute for the folder (reuse `EligibilitySurface.offersContribute`). "Look inside" in the inspector: `Button(words.lookInside)` — check `MonitorTracesCopy` for the word; if absent, this button is decision-blocked (the core must export it) and the sheet opens from the session row's existing Review action instead. The sheet presents the legacy `PreviewSheet(entry:preloaded:)` with `QueueEntryBridge.legacyEntry(for:in:)`.

- [ ] **Step 4: Repoint the `.rs` rows and the wording baseline; run** `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central` and `swift test --filter 'TracesGapTests|ShellWordingTests|TracesTreeTests|MonitorReviewTests|PreviewSlotTests|PreviewSizingTests|QueueEntryOriginTests|AdmissionPlacementTests|ConsentBindingTests|SheetEligibilityInvalidationTests'`. Expected: PASS.

- [ ] **Step 5: Commit** `git commit -m "Close the Traces gaps and delete the legacy queue"`.

### Task 6: History gap closure (Withdraw, session detail, held explanations)

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/HistoryInspector.swift` (hosts `SessionContributionOverview(record:)`-equivalent status, `SessionWithdrawalAction`, `WithdrawalOutcomeView`, `WithdrawalConfirmationView` and `PublicRunEditor` beside the Skills panel)
- Modify: `macos/Sources/TraceCommonsApp/Views/SessionDetailView.swift` (`PublicRunEditor` becomes internal; `SessionDetailView` deleted after the inspector hosts its parts)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift` (`HistoryPage` draws `GlassNotice(tone: .ask, title: MonitorWords.heldForReview)` with `HistoryView.contributorFacingExplanations(in:)`'s lines moved to `enum HeldExplanations` in `HistoryInspector.swift`; `QuarantineExplanationTests` retargets)
- Delete: `macos/Sources/TraceCommonsApp/Views/HistoryView.swift`, `Views/CreditRecordView.swift` (after moving `PrivacyManifestoView` if anything else uses it: check with `grep -rn PrivacyManifestoView macos/Sources`)
- Modify: `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (delete the `HistoryView.swift` 26 and `CreditRecordView.swift` 9 entries. The Swift-authored held-review body `HistoryView.heldReviewBody` cannot move into `HistoryInspector.swift`, because no file may be added to the baseline; the core's `MonitorScreensCopy.heldExplanation` (approved in #1218) replaces it, and `QuarantineExplanationTests.testHeldCopyDoesNotClaimAHumanReader` reads `MonitorWords.heldExplanation` instead. The explanation lines themselves are the daemon's, filtered by `HeldExplanations.lines(in:)`, which authors nothing.)
- Modify: `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs` (`WithdrawalCopy.swift` rows unchanged; it is hosted, not moved)
- Test: `macos/Tests/TraceCommonsAppTests/HistoryGapTests.swift` (new)

**Interfaces:**
- Consumes: `model.withdraw(_:)`, `ContributionStatusPresentation.offersWithdraw(_:)`, `WithdrawalCopy`, `SessionWithdrawalAction`, `WithdrawalConfirmationView(...)`, `WithdrawalOutcomeView(result:)`, `PublicRunEditor`, `model.loadSessionDetail(_:)`, `model.publishPublicRun(_:draft:)`, `model.unpublishPublicRun(_:)`, `model.publicRunCopy?.contributionStatusLabel(for:)`, `MonitorWords.{unknown, heldForReview, heldExplanation}`.
- Produces: `HistoryDetailInspector` with status, withdraw, public run, skills; `enum HistoryStatusLine { static func text(status: String, label: String?, unknown: String) -> String }` (pure: the core's label, else the core's unknown word, never the raw status); `HeldExplanations.lines(in:)`.

- [ ] **Step 1: Write the failing tests** (Review Focus 5)

```swift
import XCTest
import TCShellCore
@testable import TraceCommonsApp

final class HistoryGapTests: XCTestCase {
    /// An unrecognised status reads the core's unavailable word, is
    /// terminal, and offers no Withdraw (owner rule; D-19).
    func test_anUnknownStatusIsTerminalAndOffersNoWithdraw() {
        XCTAssertEqual(HistoryStatusLine.text(status: "zzz", label: nil, unknown: "Status unavailable"), "Status unavailable")
        XCTAssertEqual(HistoryStatusLine.text(status: "accepted", label: "Accepted", unknown: "Status unavailable"), "Accepted")
        XCTAssertFalse(ContributionStatusPresentation.offersWithdraw("zzz"))
    }

    func test_theInspectorHostsWithdrawAndThePublicRunEditor() throws {
        let inspector = try MonitorNavigationTests.text("Views/Monitor/HistoryInspector.swift")
        for needle in ["SessionWithdrawalAction(", "ContributionStatusPresentation.offersWithdraw(", "PublicRunEditor(",
                       "model.loadSessionDetail(", "MonitorWords.unknown", "HistoryStatusLine.text("] {
            XCTAssertTrue(inspector.contains(needle), "HistoryInspector lacks \(needle)")
        }
    }

    func test_heldExplanationsComeFromTheCore() throws {
        let home = try MonitorNavigationTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("MonitorWords.heldExplanation"))
        XCTAssertFalse(FileManager.default.fileExists(atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/HistoryView.swift").path))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter HistoryGapTests`. Expected: FAIL. **Step 3:** implement as listed; the hosted withdraw views keep their `TC.` tokens (D-1) and are wrapped in a `GlassCard`. **Step 4:** run `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central` and `swift test --filter 'HistoryGapTests|ShellWordingTests|QuarantineExplanationTests|SessionPublicationTests|RefusedOutcomeTests|HomeTests|ComputeSkillsParityTests'`; PASS. **Step 5:** commit `git commit -m "Close the History gaps and delete the legacy history screens"`.

### Task 7: Private AI gap closure (account, tools, switch)

**Files:**
- Create: `macos/Sources/TraceCommonsApp/Views/Monitor/InferenceAccount.swift` (`InferenceAccountSection`: hosts `CredentialSection(copy:prominent: true)`, `HarnessListSection(copy:)` and `PrivateAISwitchCard` extracted from `PrivateInferenceContent`'s `DisclosureGroup` with its `Toggle`, `offerWhat`, `offerExposure`, the state line from the tone, `servingLine`, `settingsAppliesAtOnce`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/InferenceViews.swift` (`PrivateAIInspectorView` draws `InferenceAccountSection` above the tools list; while `model.startup` is `.needsRoots` the Inference tab's main pane draws the Folders step (`OnboardingRootsView(configDirectory:onStarted:)`), as `PrivateInferenceActivationView` did, and while it is `.starting` or `.refused` it draws the tab's existing core-down state (`ScreenState.coreDown`, the core's `MonitorWords.table?.coreUnreachable`), not the deleted `DaemonStartupNotice`)
- Delete: `macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift` (keep `PrivateInferenceIndicator` by moving it to `InferenceAccount.swift`), `Views/PrivateInferenceActivationView.swift`
- Create: `macos/Sources/TCShellCore/PrivateInferenceTray.swift` (`enum PrivateInferenceTray { enum Action { case stopAnswering, openDestination }; static func action(on: Bool) -> Action; static func label(on: Bool, copy: PrivateInferenceCopy) -> String; static func perform(on: Bool, turnOff: () -> Void, open: () -> Void) }`, moved from `MenuBarContent`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift` (`MenuBarContent.performPrivateInferenceTray` → `PrivateInferenceTray.perform`)
- Modify: `macos/Tests/TraceCommonsAppTests/PrivateInferenceMenuBarTests.swift` (`MenuBarContent.*` → `PrivateInferenceTray.*`; `privateInferenceSymbol` → `PrivateInferenceIndicator.palette(...)`-based glyph in `InferenceAccount.swift`)
- Test: `macos/Tests/TraceCommonsAppTests/InferenceGapTests.swift` (new)

**Interfaces:**
- Consumes: `model.privateInferenceCopy`, `model.privateInferenceState`, `model.privateInferenceCalls`, `model.daemonSettings?.privateInferenceOn`, `model.daemonSettings?.privateInference`, `model.privateInferenceBusy`, `model.applyPrivateInference(_:)`, `PrivateInferenceSurface.{tone, stateLine, servingLine}`, `CredentialSection`, `HarnessListSection`.
- Produces: `InferenceAccountSection`, `PrivateAISwitchCard`, `PrivateInferenceTray`.

- [ ] **Step 1: Write the failing tests**

```swift
import XCTest
import TCShellCore
@testable import TraceCommonsApp

final class InferenceGapTests: XCTestCase {
    /// The menu never turns Private AI on: off is the one write, on opens
    /// the destination where the exposure sentence is.
    func test_theTrayTurnsItOffAndOpensToTurnOn() {
        var turnedOff = 0, opened = 0
        PrivateInferenceTray.perform(on: false, turnOff: { turnedOff += 1 }, open: { opened += 1 })
        XCTAssertEqual((turnedOff, opened), (0, 1))
        PrivateInferenceTray.perform(on: true, turnOff: { turnedOff += 1 }, open: { opened += 1 })
        XCTAssertEqual((turnedOff, opened), (1, 2))
    }

    /// The switch sits with the exposure sentence; the state line reads the
    /// tone, never the switch.
    func test_theSwitchCarriesTheExposureSentence() throws {
        let account = try MonitorNavigationTests.text("Views/Monitor/InferenceAccount.swift")
        for needle in ["copy.offerWhat", "copy.offerExposure", "model.applyPrivateInference(",
                       "PrivateInferenceSurface.stateLine(", "PrivateInferenceSurface.tone(", "CredentialSection(copy:",
                       "HarnessListSection(copy:", "copy.settingsAppliesAtOnce"] {
            XCTAssertTrue(account.contains(needle), "InferenceAccount lacks \(needle)")
        }
        XCTAssertFalse(account.contains("privateInferenceOn ? .on"), "the state must come from the tone, not the switch")
    }

    func test_theLegacyDestinationIsGone() {
        XCTAssertFalse(FileManager.default.fileExists(atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/PrivateInferenceView.swift").path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: GlassSurfaceRulesTests.root.appendingPathComponent("Views/PrivateInferenceActivationView.swift").path))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter InferenceGapTests`. Expected: FAIL. **Step 3:** implement; the hosted `CredentialSection`/`HarnessListSection` keep `TC.` (D-1); `PrivateAISwitchCard` is new and glass (register it: it lives in `InferenceAccount.swift`, which hosts legacy views, so the scan registers only `TCShellCore/PrivateInferenceTray.swift` via a `TCShellCoreTests` sentence check — it authors none). **Step 4:** run `swift test --filter 'InferenceGapTests|PrivateInferenceMenuBarTests|PrivateInferenceActivationTests|PrivateInferenceDestinationTests|CredentialCancelTests|CredentialRenewalTests|BalanceBindingTests|FundingTests|ShellWordingTests'` and the copy ratchet; PASS. **Step 5:** commit `git commit -m "Host the Private AI account, tools and switch in the Inference tab"`.

### Task 8: Insights and Mission drafts reachable from Home

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/HomeViews.swift` (`HomeTabView.Page` gains `.insights`, `.missionDrafts`; `HomeOverview` gains two `GlassEyebrowCard`s with the core's words — check `TCInsights.copy()["insights_title"]` and `MissionDraftsModel.copy["title"]` for the headings; the pages are `GlassBreadcrumb` + the hosted `InsightsView(storeSelection:)` / `MissionDraftsView(model:)`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MonitorWindowView.swift` (takes `insightsStoreSelection: InsightsStoreSelection` and `missionDrafts: MissionDraftsModel` from `TraceCommonsAppMain`)
- Test: `MonitorNavigationTests.swift`

**Interfaces:**
- Consumes: `InsightsView(storeSelection:)`, `MissionDraftsView(model:)`, `MissionDraftsModel.loadCopy()`, `InsightsStoreSelection.parse(arguments:)`.
- Produces: `HomeTabView.Page.insights`, `.missionDrafts`; `MonitorWindowView(insightsStoreSelection:missionDrafts:)`.

- [ ] **Step 1: Write the failing pin**

```swift
    func test_insightsAndMissionDraftsAreReachableFromHome() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("case insights"))
        XCTAssertTrue(home.contains("case missionDrafts"))
        XCTAssertTrue(home.contains("InsightsView(storeSelection:"))
        XCTAssertTrue(home.contains("MissionDraftsView(model:"))
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("MonitorWindowView(insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts)"))
    }
```

- [ ] **Step 2: Run** `swift test --filter MonitorNavigationTests`. Expected: FAIL. **Step 3:** implement; `InsightsView`'s delete actions and `MissionDraftsView`'s delete confirmation are untouched (both are plain SwiftUI, no `TC.`). D-9: `MissionsPage` ships as built. D-11: say in the PR that the Insights-only launch no longer avoids starting services. **Step 4:** run `swift test --filter 'MonitorNavigationTests|InsightsModelTests|InsightsStoreRoutingIntegrationTests|MissionDraftsModelTests|MissionsTests|HomeTests'`; PASS. **Step 5:** commit `git commit -m "Reach Insights and mission drafts from Home"`.

### Task 9: The glass menu bar as the release default

**Files:**
- Create: `macos/Sources/TCShellCore/Format.swift` (`public enum Format { bytes, when, tomorrowMorning }`, moved from `MenuBarView.swift:450-469`)
- Modify: `macos/Sources/TraceCommonsApp/Views/MenuBarView.swift` (delete `MenuBarLabel`, `MenuBarGlyph`, `MenuBarContent`; keep `enum MenuBarWords { resume, pause, pauseHour, pauseMorning, pauseIndefinite; static func pauseUntil(_:) }` until D-12)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift` (`MenuBarContent.*Label` → `MenuBarWords.*`, `MenuBarContent.pauseUntil` → `MenuBarWords.pauseUntil`; drop the file's `#if DEBUG`)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuPanelStore.swift` (drop `#if DEBUG`)
- Modify: `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (`MenuBarView.swift` 11 → the dump's value after the move, which is the three pause sentences; `MainWindowView.swift` deleted in Task 11)
- Modify: `macos/Tests/TraceCommonsAppTests/MenuBarGlyphRenderTests.swift` (renders `MenuBarGlyph`; retarget to `GlassMenuBarStrip` under each `Condition` with a badge, keeping its "every state renders and a count adds ink" assertion)
- Test: `MonitorNavigationTests.swift`

**Interfaces:**
- Produces: `Format` in TCShellCore; `MenuBarWords`; the glass `MenuBarExtra(.window)` as the only menu-bar item.

- [ ] **Step 1: Write the failing pin**

```swift
    func test_theGlassMenuBarIsTheOnlyMenuBarItem() throws {
        let main = try Self.text("TraceCommonsAppMain.swift")
        XCTAssertFalse(main.contains("TRACE_COMMONS_GLASS_MENU"))
        XCTAssertFalse(main.contains("MenuBarContent("))
        XCTAssertEqual(main.components(separatedBy: "MenuBarExtra(").count - 1, 1, "exactly one menu-bar item")
        XCTAssertTrue(main.contains(".menuBarExtraStyle(.window)"))
        let menu = try Self.text("Views/MenuBarView.swift")
        XCTAssertFalse(menu.contains("struct MenuBarContent"))
        XCTAssertTrue(menu.contains("enum MenuBarWords"))
    }
```

- [ ] **Step 2: Run** `swift test --filter MonitorNavigationTests`. Expected: FAIL. **Step 3:** implement; `TraceCommonsAppMain` keeps one `MenuBarExtra` whose label is `MenuBarStripLabel` and whose content is `MenuBarGlassPanel`, `.menuBarExtraStyle(.window)`; `Launcher` loses its `#if DEBUG menuPanel` and always takes the store. D-10: the override rows ship disabled. **Step 4:** run `swift test --filter 'MonitorNavigationTests|MenuBarGlassPanelTests|MenuBarGlyphRenderTests|MenuBarStatusTests|ShellWordingTests|PrivateInferenceMenuBarTests'`; PASS; dump `MenuBarView.swift` at its lowered value. **Step 5:** commit `git commit -m "Make the glass menu bar the only menu-bar item"`.

### Task 10: `DebugScreenshot` on the glass screens

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/DebugScreenshot.swift` (render: `TracesTreeView` over the live store, `MenuBarGlassPanel(ownsSurface: true)`, `ConsentScopesContent`, `OnboardingWelcomeContent`, `OnboardingProjectsContent`, `OnboardingDoneContent`, both `OnboardingConnectContent` states, every `GlassSettingsContent(section:)`, `HistoryDetailInspector` for the first history row, `OnboardingPrivacyScanContent`, `WitnessReviewConsent`, `PreviewSheet` as today; drop `MenuBarPreview`, `QueueContent`, `SettingsContent()`, `CreditRecordView`, `WithdrawalConfirmationCapture` (moved to `HistoryInspector.swift` if kept))
- Test: `macos/Tests/TraceCommonsAppTests/NativeOnboardingRenderTests.swift` (exists; it renders views with `ImageRenderer` and reads `TRACE_COMMONS_SCREENSHOT_DIR`; keep green)

- [ ] **Step 1:** Make the edits; `swift build` must succeed with no reference to a deleted view. **Step 2:** run `swift test --filter 'NativeOnboardingRenderTests|MenuBarGlyphRenderTests'`; PASS. **Step 3:** commit `git commit -m "Point the screenshot hook at the glass screens"`.

### Task 11: The release default: no window gates, no legacy windows

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift` (delete `Window("Trace Commons", id: WindowID.main)`, `MainWindowCommands`, `MonitorWindowCommands`'s "Monitor"/"First run" items, the `#if DEBUG` around the Monitor, Settings and first-run scenes and around `TRACE_COMMONS_MONITOR`/`TRACE_COMMONS_FIRST_RUN`; keep the menu-preview window and `TRACE_COMMONS_MENU_PREVIEW` under `#if DEBUG` (D-18); add `MonitorCommands` (⌘1 Home, ⌘2 Inference, ⌘3 Traces, ⌘⇧M via `PrivateInferenceTray`); `TRACE_COMMONS_APPEARANCE` handling stays here alone (D-2); `WindowID.main` deleted; the Monitor window's `.defaultSize` and `.windowResizability(.contentMinSize)` kept)
- Modify: every `Views/Monitor/*.swift` and `Views/MonitorWindowView.swift`: remove the file-level `#if DEBUG` / `#endif`
- Delete: `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift`, `Views/MainWindowNavigation.swift`'s section remnants (the class stays as the services gate and `pending` holder), `Views/BrandMark.swift`, `Views/CommunityBrand.swift` (their last users were `MainWindowView`, `MenuBarLabel`, `OnboardingWelcomeContent` (Phase 2), `SettingsView.profilePanel` (Phase 1), `DesignSystem.swift`'s `TC.MenuBar` block, which is deleted with it), `Views/ActionMessageBanner.swift` (replaced by `GlassNotice` everywhere; check `grep -rn ActionMessageBanner macos/Sources`)
- Modify: `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (delete `MainWindowView.swift` 14, `BrandMark.swift` 1, `ActionMessageBanner.swift` 2 — their sentences leave the shell: the Monitor's subtitles are the core's `MonitorScreensCopy`, the watch chip's "Paused"/"Watching" are `MonitorWords.paused`/`.watching`, the pause menu is `MenuBarWords`; confirm each with the dump)
- Modify: `macos/scripts/run-demo.sh` (drop `--env TRACE_COMMONS_MONITOR=...`; `TRACE_COMMONS_SHOW_WINDOW` now opens the Monitor)
- Modify: `macos/Tests/TraceCommonsAppTests/AccentContrastTests.swift`, `LineHeightScalingTests.swift` (they read `TC.*`; keep until Task 14, where they are deleted with `DesignSystem.swift`)
- Test: `macos/Tests/TraceCommonsAppTests/LegacyShellRetiredTests.swift` (new)

- [ ] **Step 1: Write the failing pins**

```swift
import XCTest
@testable import TraceCommonsApp

final class LegacyShellRetiredTests: XCTestCase {
    static let root = GlassSurfaceRulesTests.root

    func test_theLegacyWindowsAreGone() {
        for file in ["Views/MainWindowView.swift", "Views/BrandMark.swift", "Views/CommunityBrand.swift",
                     "Views/QueueView.swift", "Views/HistoryView.swift", "Views/PrivateInferenceView.swift"] {
            XCTAssertFalse(FileManager.default.fileExists(atPath: Self.root.appendingPathComponent(file).path), "\(file) still exists")
        }
    }

    /// The glass windows are the release default: no DEBUG gate and no env
    /// flag stands between a release build and the Monitor, Settings or the
    /// first-run window. The menu preview and sample data stay debug-only.
    func test_theGlassWindowsHaveNoGate() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        for flag in ["TRACE_COMMONS_GLASS_MENU", "TRACE_COMMONS_MONITOR\"", "TRACE_COMMONS_FIRST_RUN", "WindowID.main", "MainWindowView("] {
            XCTAssertFalse(main.contains(flag), "\(flag) survives")
        }
        for scene in ["MonitorWindowView(", "MonitorSettingsWindow(", "FirstRunWindowView("] {
            let at = try XCTUnwrap(main.range(of: scene)).lowerBound
            let before = main[..<at]
            let opens = before.components(separatedBy: "#if DEBUG").count - 1
            let closes = before.components(separatedBy: "#endif").count - 1
            XCTAssertEqual(opens, closes, "\(scene) is inside #if DEBUG")
        }
        XCTAssertTrue(main.contains("TRACE_COMMONS_MENU_PREVIEW"))
        for file in ["Views/MonitorWindowView.swift", "Views/Monitor/TracesViews.swift", "Views/Monitor/HomeViews.swift",
                     "Views/Monitor/InferenceViews.swift", "Views/Monitor/FirstRunViews.swift", "Views/Monitor/MenuBarGlassPanel.swift",
                     "Views/Monitor/MissionsViews.swift", "Views/Monitor/FlowMapView.swift", "Views/Monitor/FlowMapScene.swift",
                     "Views/Monitor/HomeStore.swift", "Views/Monitor/InferenceStore.swift", "Views/Monitor/MenuPanelStore.swift"] {
            XCTAssertFalse(try MonitorNavigationTests.text(file).hasPrefix("#if DEBUG"), "\(file) is debug-only")
        }
    }

    /// Sample data never reaches a release build.
    func test_sampleDataStaysDebugOnly() throws {
        let shellCore = Self.root.deletingLastPathComponent().appendingPathComponent("TCShellCore/DataContract")
        for file in ["SampleDaemonClient.swift", "SampleDaemonData.swift"] {
            let source = try String(contentsOf: shellCore.appendingPathComponent(file), encoding: .utf8)
            XCTAssertTrue(source.hasPrefix("#if DEBUG"), "\(file) must stay debug-only")
        }
        let wiring = try MonitorNavigationTests.text("DaemonDataWiring.swift")
        XCTAssertTrue(wiring.contains("#if DEBUG\n    /// Sample data"))
    }

    /// The shipped hooks scripts and CI rely on are still read.
    func test_theScriptHooksSurvive() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("TRACE_COMMONS_SHOW_WINDOW"))
        XCTAssertTrue(main.contains("TRACE_COMMONS_APPEARANCE"))
        XCTAssertTrue(main.contains("DebugScreenshot.scheduleIfRequested(model: model)"))
        XCTAssertTrue(main.contains("SelfTest.runIfRequested(model: model)"))
        XCTAssertTrue(main.contains("UpdateController.shared.start()"))
        XCTAssertTrue(main.contains("Notifier.shared.configure()"))
        let demo = try String(contentsOf: Self.root.deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("scripts/run-demo.sh"), encoding: .utf8)
        XCTAssertFalse(demo.contains("TRACE_COMMONS_MONITOR"))
        XCTAssertTrue(demo.contains("TRACE_COMMONS_SHOW_WINDOW"))
    }
}
```

- [ ] **Step 2: Run** `swift test --filter LegacyShellRetiredTests`. Expected: FAIL. **Step 3:** implement as listed. **Step 4:** run the whole suite `swift test`, the copy ratchet, `generate.py --check`; PASS; the dump lists no deleted file and no new one. **Step 5:** commit `git commit -m "Make the glass windows the release default and delete the legacy shell"`.

### Task 12: The ratchets and the tests that named the legacy shell

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs` (every `SURFACES` path verified against the tree; `swift_sources() >= 86` still holds — count with `find macos/Sources -name '*.swift' | wc -l` and paste)
- Modify: `macos/Tests/TCShellCoreTests/ShellWordingTests.swift` (the baseline matches the dump exactly; the comment "There are 96 Swift sources" updated to the count)
- Modify: `macos/Tests/TraceCommonsAppTests/SettingsSectionsTests.swift`, `SettingsPointerTests.swift`, `ActionNoticeDismissTests.swift`, `ComputeNavigationTests.swift`, `PrivateInferenceMenuBarTests.swift`, `QuarantineExplanationTests.swift`, `MenuBarGlyphRenderTests.swift`, `ShellNoticesPlacementTests.swift` (each already retargeted in its task; this task re-reads each for a stale name and runs them together)

- [ ] **Step 1:** `grep -rn 'MainWindowView\|MenuBarContent\|SettingsContent\|QueueView\|QueueContent\|HistoryView\.\|PrivateInferenceView\|CommunityBrand' macos/Tests crates/trace-commons-contributor-ffi/tests` must print nothing but the `LegacyShellRetiredTests` assertions; paste the output.
- [ ] **Step 2:** run `cd macos && swift test` and the copy ratchet; PASS with counts pasted. **Step 3:** commit `git commit -m "Retarget the ratchets and tests at the glass shell"`.

### Task 13: CI, the bundle script and the release workflow

**Files:**
- Modify: `.github/workflows/clients.yml:164-166` (`swift test --filter TCDesignTests` → `swift test --filter 'TCDesignTests|GlassSurfaceRulesTests|MonitorNavigationTests|LegacyShellRetiredTests'`; the job already builds the FFI dylib the app test target links; confirm by reading lines 155-163 before editing and, if it does not, add `cargo build -p trace-commons-contributor-ffi` as the step before)
- Modify: `macos/scripts/make-app-bundle.sh:1-7` (the header says "a menu-bar app needs a bundle so that LSUIElement ... exist"; the app has been regular since `LSUIElement` was removed; correct the comment only)
- No change: `.github/workflows/release-apps.yml`, `scripts/ci/verify-macos-entitlements.sh`, `macos/scripts/make-release-dmg.sh` (the DMG builds `swift build --configuration release` with no env flag; `LegacyShellRetiredTests.test_theGlassWindowsHaveNoGate` is the proof the release binary opens the glass windows)
- Test: the CI run itself; paste the `macos-15-fallback-tests` and `macos-app-tests` job URLs in the PR

- [ ] **Step 1:** edit; **Step 2:** `actionlint .github/workflows/clients.yml` if installed, else `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/clients.yml'))"`; **Step 3:** commit `git commit -m "Run the glass shell's scan on the macOS 15 fallback job"`.

### Task 14: Delete `DesignSystem.swift` (gated on D-1)

**Files:**
- Delete: `macos/Sources/TraceCommonsApp/Views/DesignSystem.swift`, `macos/Tests/TraceCommonsAppTests/AccentContrastTests.swift`, `LineHeightScalingTests.swift` (their subject is `TC`; `TCDesignTests.TextContrastTests` and `LargeTextTests` cover the glass tokens)
- Test: `macos/Tests/TraceCommonsAppTests/LegacyShellRetiredTests.swift`

- [ ] **Step 1: Write the gate test**

```swift
    /// The legacy palette is gone from the app target: no file reads TC. or
    /// CommunityBrand. This is the same count as
    /// `git grep -c 'CommunityBrand\|\bTC\.' -- macos/Sources/TraceCommonsApp`.
    func test_noFileReadsTheLegacyPalette() throws {
        var offenders: [String] = []
        let enumerator = try XCTUnwrap(FileManager.default.enumerator(at: Self.root, includingPropertiesForKeys: nil))
        for case let url as URL in enumerator where url.pathExtension == "swift" {
            let source = try String(contentsOf: url, encoding: .utf8)
            if source.range(of: #"\bTC\."#, options: .regularExpression) != nil || source.contains("CommunityBrand") {
                offenders.append(url.lastPathComponent)
            }
        }
        XCTAssertEqual(offenders, [], "files still on the legacy palette: \(offenders.sorted())")
    }
```

- [ ] **Step 2: Run** `swift test --filter LegacyShellRetiredTests/test_noFileReadsTheLegacyPalette`. Expected, under D-1's default: FAIL, naming exactly `PreviewSheet.swift`, `SessionContributionOverview.swift`, `WithdrawalCopy.swift`, `CredentialSection.swift`, `HarnessListView.swift`, `BalanceRow.swift`, `FundingRow.swift`, `CertificateSection.swift`, `ScrubbingCaveat.swift`, `WhatGetsRemovedSheet.swift` (if not rebuilt), `NearAiJoinView.swift`, `SessionDetailView.swift` (`PublicRunEditor`), `DesignSystem.swift`. Paste the list into the PR as the residue D-1 decides on.

- [ ] **Step 3:** If the owner chose "wait": rebuild each residue file on TCDesign in its own task (each is the shape of Phase 1's tasks: a parity row, a words table at the old count, the scan), then delete `DesignSystem.swift` and the two tests, and run the gate: PASS. If the owner chose "host": mark the test `XCTSkip("D-1: legacy palette retained until the hosted surfaces are rebuilt")` with the residue list in the skip message, leave `DesignSystem.swift`, and open the follow-up issue listing the residue.

- [ ] **Step 4:** commit `git commit -m "Delete the legacy design system"` or `git commit -m "Record the legacy palette residue R15 ships with"`.

### Task 15: Rollback, the owner's manual checklist, and the PR

**Rollback.** The Phase 4 PR lands as one squash commit `<sha>` through the
merge queue. To roll back:

1. `git revert <sha>` on a branch off `main`; open a PR titled "Revert the
   R15 cutover"; it re-adds the legacy shell, the ratchet entries and the
   `#if DEBUG` gates in one step because the squash removed them in one step.
2. Do not publish the Sparkle appcast for the R15 build until the checklist
   below passes on a signed DMG; the previous appcast stays current, so a
   contributor who has not yet updated is unaffected, and one who has can
   be offered the previous version by re-signing it (`release-apps.yml`'s
   `sign_update` step, run by hand on the previous DMG).
3. The daemon, its settings file and the keychain are untouched by R15
   (every write still goes through `AppModel` and `DaemonDataClient`), so a
   reverted app reads the same state.

**Manual verification checklist for the owner** (agents cannot run the UI;
each line is something only a person at the machine can see). Run on macOS
26 and on a macOS 14 or 15 machine, from the signed DMG launched in Finder
with no shell environment:

- [ ] The menu bar shows the glass strip; clicking it opens the panel; Pause → For 1 hour, then Resume, change the watching pill; Private AI Off is the only write; "Settings" opens ⌘,; "Quit" shows the core's quit prompt.
- [ ] A fresh state directory opens the first-run window on launch; Folders → Join → Uses → Projects → Done; the Monitor opens on Done; relaunch opens the Monitor, not first run.
- [ ] Quitting between Join and Done, then relaunching, resumes first run at Uses (the `TRACE_COMMONS_RESUME_CHECK_OUT` self-test says the same).
- [ ] A `tracecommons://enroll?invite=...` link fills Join's field and names the issuer and does nothing else; while onboarded, it opens the Monitor (D-14).
- [ ] Clicking the Dock icon with no window open opens the Monitor; a digest notification's Review opens the Monitor on Traces.
- [ ] ⌘Q with Compute running shows the prompt; a refused stop opens Settings at Compute.
- [ ] Settings (⌘,): every one of the twelve sections draws; with the daemon stopped each shows its unavailable state and no control reads as on.
- [ ] Traces: contribute a session with the inspector hidden; the undo notice appears above the tree and Undo works within the core's deadline; "Look inside" opens the preview sheet with Search, Transcript (Copy everything) and Permissions; the arming confirmation and the ignore confirmation use the core's words.
- [ ] History: select a row; the inspector shows status, Withdraw (only on statuses the core allows), the public-run editor and Skills; a row with an unknown status reads the core's unavailable word and offers no Withdraw.
- [ ] Inference: sign in, connect a tool (the exposure sheet appears first), turn the switch off from the menu, see the state line follow the tone.
- [ ] Home → Missions, Insights, Mission drafts each open and return by breadcrumb; Insights' delete works.
- [ ] System Settings → Accessibility: Reduce Transparency makes every pane opaque `paneOpaque` and no hosted legacy content shows a different ground; Increase Contrast changes edges and text; Reduce Motion removes the pane slide and the map dashes.
- [ ] VoiceOver: the tabs announce their badge and dot text; a tree row announces its name, state and actions; a consent row announces its title, description and switch state; the Settings list is arrow-key navigable; every icon button has a name.
- [ ] Larger text (System Settings → Displays → Text size): no label clips in the toolbar, the tabs, the pickers or the first-run progress.
- [ ] `macos/scripts/run-demo.sh` with `TRACE_COMMONS_SHOW_WINDOW=1` brings the Monitor up; with `TRACE_COMMONS_SCREENSHOT_DIR` set it writes the glass captures.

**The PR.** Title "Make the glass windows the release default (R15)". Body:
the duty table at the top of Phase 4 with each row's "after" state as
landed; the decisions taken by default (D-1, D-2, D-9 to D-14, D-18, D-19);
the dumps before and after; the residue from Task 14; the rollback steps;
the checklist above as checkboxes for the owner; Verification with the pasted
output of `swift test`, `generate.py --check`, the copy ratchet, and the two
CI job URLs. The cargo boxes in the template are N/A (no Rust change beyond
a test's path table; say so). Tick the MIT OR Apache-2.0 attestation.

---

## Self-review (run before opening the plan's PR)

1. **Spec coverage.** #1152 "Screens" maps every screen: Home (exists),
   Traces (Task 4.5 closes the gaps), Inference (4.7), History (4.6),
   Missions (4.8), Compute (3.1), Insights (4.8), onboarding (Phase 2),
   Settings (Phase 1), menu bar (4.9). "Behaviour that must not depend on
   the inspector": 4.5. "Settings navigation": Phase 1. Reduce Transparency,
   Increase Contrast, Reduce Motion, contrast floors, VoiceOver: the scan in
   every phase plus the checklist. The macOS 14 fallback: 4.13 widens the
   fallback job; the material itself is `TCDesign`'s. Not covered, by
   decision: light appearance (D-2), the concept's new steps (D-5, D-6),
   `DesignSystem.swift`'s deletion under D-1's default (4.14 records it).
2. **Placeholder scan.** No "TBD", "TODO", "later", "appropriate",
   "similar to". Two places say "check ... before editing" (a constructor
   signature, a copy key); those are instructions to read a named file,
   with the fallback stated.
3. **Type consistency.** `GlassSettingsContent(navigation:section:)` (1.1,
   1.10, 4.10); `GlassSourceRow(kind:candidate:choice:reportedMode:onWatchCandidate:onChoose:onDecline:)`
   (1.5, 2.3); `MonitorDestination` cases (4.3, 4.4, 4.9, Phase 1's
   `PrivateAISection` retarget); `OpenMonitor.request(_:)` (4.3, 4.4, 4.9);
   `HistoryDetailInspector(row:)` (3.2, 4.6); `HistorySelection.record(for:in:)`
   (3.2); `PrivateInferenceTray.perform(on:turnOff:open:)` (4.7, 4.9);
   `ComputeContent(model:allowance:)` (3.1, `ComputeNavigationTests`);
   `SkillLearningView(record:copy:)` (3.2); `SettingsLegacyWords` members
   named identically in the parity table and the tasks (1.1-1.10).
4. **Review Focus.** Each of the five lines names its task and test:
   1 → 1.10 `test_everySectionHasAnUnavailableBranch`; 2 → 1.3
   `test_theTickReadsTheDaemonNeverTheDraft`; 3 → 4.5
   `test_hostedLegacyContentPaintsNoGround`; 4 → 4.3
   `test_aRequestBeforeTheHandlerIsReplayedOnce`; 5 → 4.6
   `test_anUnknownStatusIsTerminalAndOffersNoWithdraw`.
