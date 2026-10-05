# Native First-Run Port Implementation Plan (replaces #1235)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace #1235 with a native SwiftUI glass port of Ron's #1030 first run: Quick setup (Join, Folders, Uses) and Custom setup (Join, Tools, Rules, Uses), the Sharing moment ending in the grant, the Private AI switch, and the passkey popups, wired to the real daemon.

**Architecture:** The views are new TCDesign compositions of Ron's #1030 screens, one file per screen under `macos/Sources/TraceCommonsApp/Views/FirstRun/`. All decisions live in one pure value, `FirstRunState` (in `TCShellCore`), and one pure function, `FirstRunPlan.calls(for:at:)`, that turns it into the ordered daemon calls; `AppModel` only executes them. The existing hosts stay (`FirstRunWindowView` and the legacy `MainWindowView` gate), and the type names #1242 depends on are kept. Every word comes from `FirstRunCopy` and the existing core tables.

**Tech Stack:** Swift 6 package in `macos/` (SwiftUI, AppKit, AuthenticationServices via the existing `PlatformPasskeyProvider`), the `TCDesign` target, XCTest. No new dependencies.

**Spec:** Ron's #1030 (`origin/ftux:docs/superpowers/specs/2026-09-25-ftux-glass-flows-design.md`; screens in `origin/ftux:tauri-desktop/frontend/src/features/ftux/components/`, state rules in `ftux-model.ts`); the owner decisions of 2026-09-28 (six points) and 2026-10-04; the backend plan `docs/superpowers/plans/2026-10-04-first-run-backend.md`, whose PR this one stacks on.

**Copy in force:** this copy, on `glass-phase-2-first-run`. The copy on `first-run-backend` predates 52028da57 (still `enrolled: Bool`; no `setSourceSettings`, `startedSettingsJSON` or `signedIn`) and is superseded for Tasks 2, 3, 5, 6 and 7.

## Global Constraints

- Ron's order and screens are the design. Quick: Join, Folders, Uses. Custom: Join, Tools, Rules, Uses. No chooser screen: Quick is the default and "Custom setup instead" switches (`ftux-page.tsx:89`, `ftux-model.ts:12-23`).
- Join collects the invite and shows its host (`TCInvite.issuerHost`); the real `enroll`, `invite_lookup` and near.ai sign-in run after Folders (Quick) or Tools (Custom) has started the daemon, because the daemon refuses to start until Claude Code and Codex are answered (`settings.rs:1363`).
- A tool that is not installed still needs an answer: its row shows "Get {tool}" and "I don’t use it", and Continue waits (owner, 2026-10-04). Never write `off` for a question nobody answered.
- Uses: the always-on use is shown unticked and required; Start stays disabled until it is ticked (owner, 2026-09-28 point 4). The daemon's floor is unchanged; the box gates the button only.
- Uses comes before the sharing decision; Automatic leads through the scrub disclosure and the witness disclosure, then `grant_automatic` (points 2 and 4). The core chooses the sharing wording (`tc_automatic_contribution_copy_json`).
- Rule and sharing choices read Ask me / Automatic / Never from `tc_contribution_mode_copy_json` everywhere (2026-10-02).
- Past sessions: the selection is a person-made approve; "Include every past session" selects each session (2026-10-04).
- Swift authors no sentence. New files hold no literal of two or more words with a function word (`ShellWordingTests`); every string comes from `FirstRunCopy`, `TCSourceChecks.settingsCopy()`, `ContributionModeCopy`, `PrivateInferenceSurface`, `AutomaticGrantCopy`, `ScopeCopy` (existing) or the route disclosure.
- macOS 14 floor, Liquid Glass fallback below 26; screens never branch on the OS.
- Keep for #1242: `OnboardingNavigation`, `OnboardingCoordinatorView(startAt:onStep:onComplete:)`, `FirstRunWindowView`, `FirstRunProgress.paneWidth`, `AppModel.requiresOnboarding`, `markOnboardingComplete()`.
- Ron owns #1030: tell him about this port; never push to `ftux`.
- Commit style and verification as the backend plan; plus `python3 scripts/design-tokens/generate.py --check` and `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central`.

## Review Focus

1. **The daemon fails to start after Folders** (bad folder, socket in use): Join's deferred invite must not be lost and nothing may claim to be joined; the step shows the core's `watcher_start_failed` and stays. Pinned in Task 2 (`test_aFailedStartKeepsTheInviteAndJoinsNothing`).
2. **The invite is dead when finally looked up** (pasted on Join, rejected after Folders): the person is returned to Join with the core's invite error, their folder answers kept. Pinned in Task 2 (`test_aDeadInviteReturnsToJoinWithAnswersKept`).
3. **Automatic chosen but the grant is refused** (`automatic-grant-witness-changed`, `arming-terms-unavailable`): setup still finishes on Ask me, and says so; it never reports sharing as automatic. Pinned in Task 9 (`test_aRefusedGrantFinishesOnAskMe`).
4. **A person goes Back after the daemon started** (Uses → Join after Folders): earlier answers stay, and nothing is enrolled twice. Pinned in Task 2 (`test_backAfterStartDoesNotEnrollTwice`).
5. **Skip: watch only, then Automatic** (no account): Automatic needs an account, so the Sharing picker offers Ask me only and Start finishes watching. Pinned in Task 2 (`test_watchOnlyCannotChooseAutomatic`).

---

## File Structure

| File | Responsibility |
|---|---|
| `macos/Sources/TCShellCore/FirstRun/FirstRunState.swift` | Every answer the person gives; pure; `Codable` for restore |
| `macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift` | Tiers, step lists, `canContinue`, Back |
| `macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift` | State → ordered daemon calls at each commit point |
| `macos/Sources/TraceCommonsApp/FirstRunRunner.swift` | Executes a plan's calls against `AppModel`/`DaemonClient`, maps outcomes back into state |
| `macos/Sources/TraceCommonsApp/Views/FirstRun/FirstRunFrame.swift` | Ron's frame: pane, tier eyebrow, `GlassStepProgress`, Back, notices, footer |
| `.../FirstRun/JoinScreen.swift`, `FoldersScreen.swift`, `ToolsScreen.swift`, `RulesScreen.swift`, `UsesScreen.swift` | One per #1030 screen |
| `.../FirstRun/ToolAnswerRow.swift` | Ron's tool row (`tool-row.tsx`) |
| `.../FirstRun/SharingDisclosures.swift` | Scrub disclosure sheet, witness disclosure sheet |
| `.../FirstRun/PasskeySheets.swift` | P-1, P-2, P-5, P-7 |
| `macos/Sources/TraceCommonsApp/Views/OnboardingCoordinatorView.swift`, `OnboardingNavigation.swift` | Kept names; now host the new flow |
| `macos/Sources/TraceCommonsApp/Views/Monitor/FirstRunViews.swift` | Kept host; progress from `FirstRunNavigation` |

The legacy step views (`OnboardingWelcomeView`, `OnboardingRootsView`, `OnboardingConnectView`, `ConsentScopesView` as a step, `OnboardingPrivacyScanView`, `OnboardingProjectsView`, `OnboardingDoneView`, `WhatGetsRemovedSheet`) are deleted when Task 11 rewires the hosts; their `ShellWordingTests` baseline entries are removed and their `SURFACES` rows re-pointed to the new files. Deleting `OnboardingRootsView` also removes its `ALLOWED` entry ("Which folders may this") in `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs`; the test fails on an entry that no longer matches, so this is not optional.

---

### Task 1: Base the branch

- [ ] **Step 1:** `git switch -c glass-phase-2-first-run origin/glass-phase-1-settings`, then `git merge origin/main` (brings #1187's identity and invite code) and `git merge first-run-backend` (the backend PR's branch). Count commits after each merge (`git rev-list --count`).
- [ ] **Step 2:** Run `cargo build -p trace-commons-contributor-ffi && (cd macos && swift test)` — Expected: the same pass/fail set as `glass-phase-1-settings` plus the backend tests. Record the baseline counts in the PR body.
- [ ] **Step 3: Commit** the merges (no other change).

### Task 2: First-run state, navigation and plan

**Files:** the three `TCShellCore/FirstRun/*.swift`; test `macos/Tests/TCShellCoreTests/FirstRunPlanTests.swift`, `FirstRunNavigationTests.swift`

**Interfaces:**
- Produces:
  - `public enum FirstRunTier: String, Codable { quick, custom }`
  - `public enum FirstRunStep: String, Codable { join, folders, tools, rules, uses }`; `FirstRunNavigation.steps(for: FirstRunTier) -> [FirstRunStep]`
  - `public struct FirstRunState: Codable, Equatable { tier; step; invite: String; issuerHost: String?; account: AccountAnswer (.none, .watchOnly, .nearAI, .passkey(name)); toolAnswers: [SourceKind: SourceChoice]; addedFolders: [AddedFolder]; rules: [String /*project_id*/: ProjectMode]; pastSelections: [String /*project_id*/: Set<String> /*session_id*/]; scopes: Set<String>; sharing: SharingPath (.askMe, .automatic); privateAI: Bool; daemonStarted: Bool; startedSettingsJSON: String?; enrolledInvite: String? /*trimmed*/; signedIn: Bool }`, plus `mutating func answer(_ kind: SourceKind, _ choice: SourceChoice)` and `mutating func add(_ folder: AddedFolder)`, which keep one answer per kind (a later answer drops that kind's added folder, and an added folder drops that kind's row answer). `daemonStarted`, `startedSettingsJSON`, `enrolledInvite` and `signedIn` are daemon facts the runner records; navigation (Back, `returnToJoin(afterDeadInvite:)`) never clears them.
  - `FirstRunNavigation.canContinue(_ state: FirstRunState, candidates: [SourceCandidate], requiredScope: String?) -> Bool` (Folders/Tools: every offered kind answered, including missing ones; Uses: `requiredScope` ticked)
  - `public enum FirstRunCall: Equatable { startDaemon(settingsJSON: String), setSourceSettings(settingsJSON: String), lookupInvite(String), enroll(String), signInNearAI, setConsentScopes([String]), setProjectMode(projectID: String, ProjectMode), includePastSessions(projectID: String, [String]), setPrivateAI(Bool), grantAutomatic(witness: String?), markComplete }`
  - `FirstRunPlan.calls(for state: FirstRunState, at commit: CommitPoint) -> [FirstRunCall]` with `enum CommitPoint { leaveRoots /*Folders or Tools*/, start /*Uses*/ }`. Order at `leaveRoots` (nothing at all while the roots are unfinished): `startDaemon` if not `daemonStarted`, otherwise `setSourceSettings` carrying only the declarations that differ from `startedSettingsJSON` (a key withdrawn since the start is sent as `{"mode":"off"}`, because `set_settings` merges and an absent key leaves the daemon watching; an unknown `startedSettingsJSON` sends the whole declaration; nothing when unchanged); then `lookupInvite` and `enroll`, both skipped when the trimmed invite equals `enrolledInvite` (a spent last use would answer `exhausted`); then `signInNearAI` when chosen and not `signedIn`. Order at `start`: `setConsentScopes`, every `setProjectMode`, every `includePastSessions`, `setPrivateAI` (Custom only), `grantAutomatic` only if `sharing == .automatic` and the account is not `.watchOnly`, then `markComplete`.

- [ ] **Step 1: Write the failing tests** — `test_quickHasJoinFoldersUses`, `test_customHasJoinToolsRulesUses`, `test_aMissingToolMustBeAnswered`, `test_startWaitsForTheRequiredUse`, `test_theDaemonStartsBeforeAnyJoinCall`, `test_scopesAreSavedBeforeTheGrant`, `test_watchOnlyCannotChooseAutomatic`, `test_backAfterStartDoesNotEnrollTwice`, `test_quickNeverSetsARuleOrIncludesPastSessions`, and the two Review Focus runner tests named in Task 3 as plan-level cases (`test_aFailedStartKeepsTheInviteAndJoinsNothing` asserts `calls(at: .leaveRoots)` is recomputed with the invite intact after `daemonStarted` stays false; a pure-function stand-in, the guarantee is Task 3's). Also `test_anEnrolledInviteIsNotLookedUpAgain`, `test_aNewInviteAfterEnrollingIsLookedUpAndEnrolled`, `test_aFolderChangedAfterStartReachesTheDaemon`, `test_aFolderWithdrawnAfterStartIsTurnedOff`, `test_aDeadInviteKeepsAnEarlierEnrolment`, `test_aLaterAnswerReplacesAnAddedFolder`; `test_backAfterStartDoesNotEnrollTwice` asserts the exact (empty) call list. `test_watchOnlyJoinsNothing` keeps a pasted invite with `.watchOnly` and asserts exactly `[.startDaemon]`, so only the account check stops the join; `test_noCandidatesStillNeedsADeclaration` pins that with no candidates discovered Folders and Tools stay closed until Claude Code and Codex are answered (`settingsJSON() != nil` is the only backstop there).
- [ ] **Step 2: Run** `cd macos && swift test --filter "FirstRunPlanTests|FirstRunNavigationTests"` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Model Ron's first run as one state and its daemon calls"`

### Task 3: The runner

**Files:** `macos/Sources/TraceCommonsApp/FirstRunRunner.swift`; test `macos/Tests/TraceCommonsAppTests/FirstRunRunnerTests.swift` (a recording fake for each call)

**Interfaces:**
- Consumes: Task 2; `AppModel.startDaemon(at:settingsJSON:completion:)`, `enroll(invite:)`, `setConsentScopes(_:)`, `setProjectMode(_:mode:)`, `applyPrivateInference(_:)`, `markOnboardingComplete()`; backend `DaemonClient.inviteLookup`, `includePastSessions`, `grantAutomatic`, `accountSignIn`.
- Records: `startDaemon` success sets `daemonStarted` and `startedSettingsJSON`; `setSourceSettings` success replaces `startedSettingsJSON` with the full current declaration; `enroll` success sets `enrolledInvite` to the trimmed invite; `signInNearAI` success sets `signedIn`. A failure records nothing.
- Produces: `@MainActor final class FirstRunRunner: ObservableObject { @Published var state: FirstRunState; @Published var failure: FirstRunFailure?; func commit(_ point: CommitPoint) async }`; `enum FirstRunFailure: Equatable { startFailed, inviteDead, enrollFailed, scopesFailed, grantRefused(label: String) }`, each mapped to a core sentence by the screens.

- [ ] **Step 1: Write the failing tests** — `test_aFailedStartKeepsTheInviteAndJoinsNothing` (a failing fake `startDaemon`; asserts the resulting state: invite kept, `daemonStarted` false, `enrolledInvite` nil, step unchanged, no lookup or enroll recorded by the fakes), `test_aDeadInviteReturnsToJoinWithAnswersKept`, `test_aRefusedGrantFinishesOnAskMe` (completion still marked; `state.sharing == .askMe`; failure `grantRefused`), `test_callsRunInPlanOrderAndStopAtTheFirstFailure`.
- [ ] **Step 2–4:** run `swift test --filter FirstRunRunnerTests` (FAIL), implement, run (PASS).
- [ ] **Step 5: Commit** — `git commit -m "Run the first run's daemon calls in order"`

### Task 4: Ron's frame

**Files:** `Views/FirstRun/FirstRunFrame.swift`; test `macos/Tests/TraceCommonsAppTests/FirstRunFrameTests.swift` (source scan, house pattern)

**Interfaces:** `struct FirstRunFrame<Content: View>: View { init(copy: FirstRunCopy, state: Binding<FirstRunState>, onBack: (() -> Void)?, footer: FirstRunFooter, content:) }` — `GlassPane`, the tier name as eyebrow, `GlassStepProgress(labels: copy.frame.steps(for: tier), current:)`, a `.link` Back, `GlassNotice` for the runner's failure, and the footer (`.link` "Custom setup instead" on Quick's Folders only, `.primary` Continue/Start with its disabled help from `copy.frame.answerEveryTool`).

- [ ] Steps: test `test_theFrameDrawsTheTierAndRonsStepLabels`, `test_customSetupInsteadAppearsOnlyOnQuickFolders`; implement; pass; commit `"Draw Ron's first-run frame in glass"`.

### Task 5: Join

**Files:** `Views/FirstRun/JoinScreen.swift`; test `JoinScreenTests.swift`
Ron's `join-screen.tsx:21-236` in TCDesign: title and body; invite `GlassCard` (`GlassTextField` + `.glass` "Look up"; the host line from `TCInvite.issuerHost` before the daemon runs, the joined line with the pay range after `inviteLookup`, the invite error as `GlassNotice(.outside)`); the near.ai `GlassCard` (`.glass` sign-in, `GlassStatusLabel(.on)` "Signed in"); the passkey `GlassCard` (opens Task 10's sheets; shown, since the owner chose to build the passkey flow on 2026-10-04, which overrides #1030 rule 12's hiding); the quiet no-sharing card; footer "Skip: watch only" until an account exists, then "Continue". Once `enrolledInvite` is set the invite field is read-only (the joined line shows instead), so an enrolled person cannot paste a second invite; the plan still looks up and enrolls a changed invite rather than dropping it.
- [ ] Steps: tests `test_joinShowsTheHostBeforeTheDaemonRuns`, `test_skipReadsWatchOnlyUntilAnAccountExists`, `test_joinAuthorsNoSentence`; implement; pass; commit `"Port Ron's Join screen"`.

### Task 6: Folders (Quick) and the tool row

**Files:** `Views/FirstRun/FoldersScreen.swift`, `ToolAnswerRow.swift`; tests `FoldersScreenTests.swift`
Ron's `tool-row.tsx`: `GlassCard` with `GlassToolTile(.tool(..), large: true)`, name, mono path, the evidence line (`SourceCandidate.evidence()`, existing); a found tool's `GlassPicker` with "Watch this folder" (`.on` dot) and "I don’t use it" (`.off`), placeholder unanswered; `GlassFolderButton` "Choose a different folder for {tool}" (`NSOpenPanel`); a missing tool shows "Install it, then this row asks again.", `.secondary` "Get {tool}", **and** the same picker limited to "I don’t use it" (owner decision). Rows come from `TCDiscovery.sourcesJSON()` (no daemon) and write through `FirstRunState.answer(_:_:)` / `add(_:)`. Continue commits `.leaveRoots`.
- [ ] Steps: tests `test_aMissingToolStillAsksForAnAnswer`, `test_continueIsDisabledUntilEveryRowIsAnswered`, `test_choosingAFolderWatchesThatPath`; implement; pass; commit `"Port Ron's Folders screen"`.

### Task 7: Tools (Custom) with "add your tool"

**Files:** `Views/FirstRun/ToolsScreen.swift`; test `ToolsScreenTests.swift`
The Folders rows in compact form plus Ron's add tile (`GlassCard` + `GlassToolTile(.folder)`, click opens `NSOpenPanel`, drop accepts a folder URL). The picked folder goes to `TCDiscovery.describeFolderJSON`, decoded with `FolderMatch.decodeList` (never `SourceCandidate.decodeList`, which drops the trajectory row and would turn two matches into one): one match re-points that kind ("Added by you"); OpenCode and trajectory both matching asks which with a `GlassPicker`; a trajectory match sets `SessionRoots.trajectory`; no match shows the refusal line. Rows and picked folders write through `FirstRunState.answer(_:_:)` and `add(_:)`, never `toolAnswers` or `addedFolders` directly, so a later "I don’t use it" drops the added folder (test `test_aLaterOffDropsTheAddedFolder`). Continue commits `.leaveRoots`.
- [ ] Steps: tests `test_aRecognisedFolderRepointsItsKind`, `test_anAmbiguousFolderAsksWhichTool`, `test_anUnrecognisedFolderIsRefused`, `test_theCaptionNamesNoUnsupportedTool` (no "Theia", no "SSH" in the rendered copy); implement; pass; commit `"Port Ron's Tools screen with folder recognition"`.

### Task 8: Rules (Custom) with the past-session picker

**Files:** `Views/FirstRun/RulesScreen.swift`; test `RulesScreenTests.swift`
Ron's `rules-screen.tsx:138-237`. Card 1, `GlassEyebrowCard` "Repos found in {tools} sessions": `GlassTableRow` per `listProjects` row, mono path, "{n} sessions", a `GlassPicker` over `ContributionModeCopy` (Ask me / Automatic / Never). Card 2, "Past sessions, by folder" with "{selected} of {total} selected": per project a mixed-state `Toggle` with `GlassCheckboxStyle` "Include every past session in {folder}" (selects every selectable `session_id`), a `GlassExpander` listing `listPastSessions` rows (date and size for `not_queued`; title and duration when present), "Show all {count}" / "Show fewer", a Never folder disabled with "{count} · rule is Never", `still_active` rows disabled. Setting a folder to Never clears its selection (`ftux-model.ts` `applyRule`).
- [ ] Steps: tests `test_includeEverySelectsEachSelectableSession`, `test_neverClearsTheFolderSelection`, `test_aStillActiveSessionCannotBeTicked`, `test_rowsWithoutATitleShowDateAndSize`; implement; pass; commit `"Port Ron's Rules screen with the past-session picker"`.

### Task 9: Uses, Sharing and Private AI

**Files:** `Views/FirstRun/UsesScreen.swift`, `SharingDisclosures.swift`; tests `UsesScreenTests.swift`, `SharingDisclosuresTests.swift`
Ron's `uses-screen.tsx:32-217`: `GlassEyebrowCard` "How your traces may be used": the always-on scope unticked with `GlassTag` "required"; a mixed "All optional uses" toggle and a `GlassExpander` holding the optional scopes and the handle scope (titles `ScopeCopy.title`, descriptions from `consentOptions`). Sharing `GlassCard`: `GlassPicker` Ask me / Automatic (labels `ContributionModeCopy`; Automatic absent for watch-only), its line from `AutomaticGrantCopy.pathAskFirst` / `pathAutomatic` + scrub scope and limit, fallbacks "Loading sharing copy…" and "Sharing copy unavailable. Starting is disabled.". Custom adds the Private AI `GlassCard` with a `Toggle` (`GlassToggleStyle`) worded by `PrivateInferenceSurface`, disabled until that copy loads. Footer note and `.primary` "Start sharing", disabled until the required use is ticked. Start with Automatic presents `SharingDisclosures`: a `GlassSheet` for the scrub disclosure (`AutomaticGrantCopy.disclosure`, `noReview`, the scope and limit), then a `GlassSheet` for the witness (route disclosure body plus `witnessStatus` state line; records the signing address), then commits `.start` with `Flow1Progress` fed to `TCFlow1.grantRequestJSON` to decide `grantAutomatic`'s witness argument.
- [ ] Steps: tests `test_theRequiredUseStartsUnticked`, `test_startIsDisabledUntilTheRequiredUseIsTicked`, `test_automaticShowsBothDisclosuresBeforeTheGrant`, `test_askMeGrantsNothing`, `test_privateAIAppearsOnlyInCustom`, `test_aRefusedGrantFinishesOnAskMe` (screen shows the failure line); move `tc_automatic_contribution_copy_json` and `tc_inference_connection_copy_json` from `BRIDGE_ONLY` to `SURFACES` with these files; implement; pass; commit `"Port Ron's Uses screen with the Sharing moment"`.
- Deviations recorded after review: only `tc_automatic_contribution_copy_json` moved to `SURFACES`. `tc_inference_connection_copy_json` stays in `BRIDGE_ONLY`, because the Private AI card reads `model.privateInferenceCopy` (`TCPrivateInference.copyJSON`) and no first-run file calls it; Tasks 11 and 12 should not expect that row. Start's failures are worded by new `FirstRunCopy.uses` lines (`sharing_refused`, `scopes_failed`, `rules_failed`, `private_ai_failed`) and Private AI's `write_unconfirmed`, not by the folder-override refusal sentence, which says "Nothing changed" after setup has finished. The grant-refused notice is set after `markComplete`; Task 11's host must carry `runner.failure` past completion (for example into a post-setup banner) rather than rely on the Uses frame staying up. `copy.privateAi.loading` stays unused: `AppModel` decodes the Private AI copy synchronously.

### Task 10: Passkey sheets

**Files:** `Views/FirstRun/PasskeySheets.swift`; test `PasskeySheetsTests.swift`
P-1 Choose, P-2 Name (`GlassTextField`, the two name errors, the `GlassNotice(.ask)` warning), P-5 Verify (`accountBind`), P-7 Welcome back, as `GlassSheet`s; P-3, P-4 and P-6 are the system sheets `NativePasskeyCoordinator.perform(_:label:)` raises. Cancel at Verify signs out and Join shows the signed-out notice.
- [ ] Steps: tests `test_aBlankNameIsRefusedWithRonsLine`, `test_cancelAtVerifySignsOut`, `test_noSimulatedSheetExists` (no "Simulated" string, no imitation of a system sheet); implement; pass; commit `"Port Ron's passkey sheets onto the native passkey coordinator"`.

### Task 11: Rewire the hosts and retire #1235's screens

**Files:** `OnboardingNavigation.swift`, `Views/OnboardingCoordinatorView.swift`, `Views/Monitor/FirstRunViews.swift`, `Views/MainWindowView.swift`; delete the legacy step views listed under File Structure; tests: retire `OnboardingParityTests.test_unapprovedConceptStepsDoNotExist`, `UsesStepTests.test_alwaysOnRowsAreLockedOn`, `FirstRunProgressTests.test_foldersPrecedeJoinOnAFreshInstall` (each replaced by the Task 2 test that states the new rule, named in the commit); update `ShellWordingTests.wordingBaseline`, `GlassSurfaceRulesTests.files` (append the new files; remove deleted ones deliberately, saying why), `swift_copy_surface_is_central.rs` `SURFACES`.
`OnboardingCoordinatorView(startAt:onStep:onComplete:)` keeps its signature and hosts `FirstRunRunner`; `FirstRunWindowView` keeps `FirstRunProgress.paneWidth`.
- [ ] Remove the `OnboardingRootsView.swift` entry from `ALLOWED` in `swift_copy_surface_is_central.rs` with the view, and run `cargo test -p trace-commons-contributor-ffi --test swift_copy_surface_is_central`.
- [ ] Steps: run the full gate (Global Constraints); fix only what the rewire breaks; commit `"Host Ron's first run in both windows and retire the old steps"`.

### Task 12: Replace #1235, restack, update the review page

- [ ] **Step 1:** Ask the owner before any force-push: either force-push `glass-phase-2-first-run` to `glass-phase-2-onboarding` (keeps PR #1235) or open a new PR and close #1235.
- [ ] **Step 2:** Restack #1236, #1241 and #1242 on the new branch, one at a time, re-running `swift test` after each; the expected conflicts are `GlassSurfaceRulesTests.swift` (#1236), `NearAiJoinView.swift`, `ProjectsSection.swift`, `OnboardingParityTests.swift`, `ActionNoticeDismissTests.swift` (#1241), and the coordinator, `FirstRunViews.swift`, `ConsentScopesView.swift`, `PrivateInferenceActivationView.swift` and their tests (#1242). Count commits after each history operation.
- [ ] **Step 3:** Update the #1235 section of the review artifact (https://claude.ai/artifact/YNjwXSsrnsCugV5793uCd9) with code-derived wireframes of the new screens beside Ron's #1030 screens.
- [ ] **Step 4:** Tell Ron on #1030 that the native port exists, linking the PR.
