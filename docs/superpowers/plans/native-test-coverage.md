# Native test coverage (K8, #1173)

K8 ports the remaining tests that read React or Tauri files, so the native
macOS app is covered once Tauri retires on macOS (R15). This note is the
record of that work: where each of the six frontend tests landed, which
macOS tests will need to move with Ron's redesign, and what the signing and
entitlements checks will need once there is no Tauri bundle to compare
against.

Only macOS is covered. Nothing here adds Windows or GTK tests for these six;
that coverage is still missing.

## 1. The six `.test.mjs` files

| Tauri test | What it pinned | Where it is now |
|---|---|---|
| `tauri-desktop/frontend/src/features/onboarding/flow1.test.mjs` | The scope picker, the grant blockers and `requestGrant`, Decide later, Back, the witness disclosure, withdraw-and-confirm, the optional "connecting inference" step. | **Covered by #1207 (K5).** Flow 1's Rust module (`flow1.rs`) is not on `main` yet -- K5 is open, not merged -- so there is nothing in the core to write a Rust test against, and this file's own behaviour must not be touched per the K8 brief. K5's own description names the step machine, grant blockers and Back as exactly what it is moving into the core. |
| `tauri-desktop/frontend/src/features/onboarding/inference-connection.test.mjs` | Two things bundled in one file: (a) parsing/refusing malformed offers, current-connection and select-result payloads from the daemon, and (b) `inferenceView`'s state machine -- `loading` / `sign_in` / `none` / `installed` / `install` / `choose` (with `otherDevice` / `reselect` / `expectedVersion`) -- plus `installTarget` and `needsAccountSignIn`. | **(a) already in the core**, more strictly than the Tauri parsers: `crates/trace-commons-contributor/src/inference_connection.rs` (`list_offers`, `current_selection`, `select`) and `crates/trace-commons-contributor/src/daemon/inference_connection.rs` (the IPC handlers, five methods, ~40 existing tests covering malformed/partial payloads, `pending_install`, `reselection_required`, `revocation_applied`, one-installed-device-per-account). **(b) has no Swift screen.** No macOS source references `inference_connection_offers` / `_current` / `_select` / `_install` at all; `CoreCopyExportTests.testTheRemainingTablesCrossAsJSONObjects` already documents this ("the connecting-inference step" is one of "the screens macOS does not have yet"). Listed as **waiting for Ron's screen** -- when it exists, `inferenceView`'s branches should become a Swift test against that screen's view-state function, built on the IPC facts the core already validates. |
| `tauri-desktop/frontend/src/lib/tauri/automatic-grant-copy.test.mjs` | `parseAutomaticGrantCopy`: exactly one of `patterns_only` / `model_scrubbed` is ever present, an unknown/missing `disclosure` is refused, and `scrubDisclosureLines` orders `[scope, limit, no_review]`. | **Already in the core and already tested there** (`consent_copy.rs`: `AutomaticGrantCopy`, `automatic_grant_copy`, `the_patterns_only_grant_copy_never_carries_the_model_scrub_wording`, `the_model_scrubbed_grant_copy_is_the_agreed_wording`) -- this landed with K3 (#1176), not K5. The invariant is enforced by the type itself (`disclosure: &'static str` is set from a closed two-variant `Disclosure` enum; the two scrub fields are `Option`s set in the same match arm), so "an unknown disclosure" and "both present" cannot be constructed from this function at all. **Ported as a new Swift test**, `CoreCopyExportTests.testThePatternsOnlyGrantCopyNeverCarriesTheModelScrubWording`, decoding the real `tc_automatic_contribution_copy_json` export and asserting the same shape. Only the `patterns_only` half is reachable this way: `automatic_contribution_copy` reads configuration only and can never answer `model_scrubbed` (that needs `automatic_gate::folder_disclosure` over a folder's certificates, exported over no ABI yet). The `model_scrubbed` half, and `scrubDisclosureLines`' render order, belong to the Flow 1 re-grant screens K5 is moving into the core -- **covered by #1207** for the part this repo cannot reach today. |
| `tauri-desktop/frontend/src/lib/tauri/quit-confirmation-copy.test.mjs` | Each of the three roles (`hosting` / `attached` / `unavailable`) keeps its own sentence; an unknown role or a missing/empty field is refused. | **Already in the core**, per-role body text (`crates/trace-commons-contributor/src/quit_copy.rs`, `QuitRole` is a closed three-variant enum, five existing tests). **Already partly in Swift**: `TCShellCore/QuitPrompt.swift` decodes it, and `CoreCopyExportTests.testTheNoWatcherQuitPromptClaimsNothingItCannotKnow` already held the `unavailable` role. **Ported the other two**: `CoreCopyExportTests.testEveryQuitRoleKeepsItsOwnSentenceAgainstARealDaemon` exercises `hosting` and `attached` against a real, in-process `TCDaemon` (one hosting, one attached to it), the same pattern `DaemonStartupTests` uses. **Ported the malformed-payload half** as a new file, `TCShellCoreTests/QuitPromptTests.swift`, mirroring `ProjectIgnoreCopyTests`'s shape (fixture JSON, empty field / missing field / not-JSON / nil all refused). `CoreCopyExportTests.testEachRoleDecodesWithItsOwnBody` decodes all three roles from the real export and checks each keeps its own body; it lives in `TCBridgeTests` because `TCShellCoreTests` does not link the core. **Gap found, not fixed**: `QuitPrompt.role` is a plain `String`; decode only checks every field is non-empty, so it does not actually refuse an unrecognized role string the way the Tauri parser does (`role: "embedded"` would decode successfully in Swift today). This cannot happen from the real ABI -- `QuitRole` is a closed Rust enum with no fourth case to serialize -- so it was left as a documented gap rather than a fix, since fixing it is a production change outside this port. |
| `tauri-desktop/frontend/src/features/waiting/health-recovery.test.mjs` | Only the `near-ai-notice-not-acknowledged` health label offers the notice-recovery confirmation; it is `ready` only once the shared privacy-scan copy has loaded, else `loading` / `unavailable`. | **Already in Swift**, structured differently because the architectures differ: `HealthCopy.forLabel("near-ai-notice-not-acknowledged")` decodes `PrivacyScanCopy` straight out of the linked-in dylib (`TCCoreCopy.privacyScanCopyJSON()`), with no IPC round trip, so there is no "still loading" interval the way there is in Tauri (which fetches this copy from the daemon over IPC and can observe it mid-flight or failed). **Ported as** `HealthQueueReviewTests.testOnlyTheNearAiNoticeLabelOffersTheSharedRecoveryCopy`, asserting the label's `HealthCopy` carries exactly the core's `recoveryTitle` / `recoveryDetail` / `recoveryAction`, and that no other label does, the other two privacy holds (`privacy-filter-canary-failed`, `pii-filter-unavailable`) included. |
| `tauri-desktop/frontend/src/features/history/withdrawal-eligibility.test.mjs` | `canWithdrawStatus`: withdrawable exactly for `{accepted, submitted, quarantined, awaiting_pii_backstop}`, not for `{withdrawn, revoked, purged, expired, unknown}`. `historyStatusLabel`: `awaiting_pii_backstop` reads with the core's shared label when it has arrived, else "Status unavailable"; known statuses keep fixed labels either way. | **Already extensively in Swift**, and already cross-referencing this issue: `SessionPublicationTests` (`testEveryTerminalContributionStateSuppressesWithdrawal`, `testServerPreAcceptanceStatesUseTheNonDistributedWithdrawalCopy`, `testAnUnknownStatusIsConfirmedWithTheCorePrompt`, `testSharedCopyLabelsEveryContributionStateAndPermittedUse`) and `WithdrawalCopy.swift`/`PublicRunModels.swift` (`ContributionStatusPresentation.isTerminal`, `PublicRunCopy.contributionStatusLabel`). **Closed two small gaps**: added `awaiting_pii_backstop` to the non-terminal loop, and asserted its and `quarantined`'s exact core labels ("Waiting for privacy review" / "Held for privacy review", matching the Tauri fixture's `status_awaiting_pii_backstop` verbatim) in `testSharedCopyLabelsEveryContributionStateAndPermittedUse`. **Fails closed, as Tauri does** (the owner's decision on #1212's review, 2026-10-02): `isTerminal` is now an allowlist of the six open statuses the core names, so an unrecognized one is terminal and `ContributionStatusPresentation.offersWithdraw`, which both Withdraw buttons read, answers `false` for it, matching Tauri's `canWithdrawStatus`. It used to be a denylist of the four closed statuses and offered Withdraw on an unknown status. New test: `testAnUnrecognizedContributionStateIsTreatedAsTerminalAndOffersNoWithdraw`. **Still different from Tauri**: macOS offers Withdraw on `received` and `rejected`, which Tauri's allowlist does not name; not changed here. **Separate finding, not fixed**: `HistoryRow.statusSentence` in `HistoryView.swift` is a second, private, hand-written status table that does not consult `PublicRunCopy.contributionStatusLabel` at all, so today it shows the generic "Not in the commons" for `awaiting_pii_backstop` rather than the core's "Waiting for privacy review". Worth a follow-up, left alone here since it is a display wire-up, not an eligibility or copy-ownership gap. |

New/changed test files: `macos/Tests/TCBridgeTests/CoreCopyExportTests.swift`,
`macos/Tests/TCShellCoreTests/QuitPromptTests.swift` (new),
`macos/Tests/TraceCommonsAppTests/HealthQueueReviewTests.swift`,
`macos/Tests/TraceCommonsAppTests/SessionPublicationTests.swift`.

## 2. macOS tests that will break with the redesign

These all scan `SettingsView.swift` or `PreviewSheet.swift`'s own source text
(by locating a declaration's signature and reading to its closing brace,
sometimes stripping comments/strings first) because a SwiftUI `body` holding
`@State` or an `@EnvironmentObject` cannot be built, rendered or reflected
outside a running window. Ron's redesign (R6, R7, R9, R11 -- the three-pane
monitor, Settings, the preview/review surface) will move or rewrite both
files wholesale, so every test below breaks on sight (missing signature,
moved section, wrong line count) the day that lands, not because the
invariant stopped being true. None of them should be deleted quietly: each
pins something a real incident or review caught, named in its own doc
comment. The general carry-forward move is the same for all of them: once a
view/view-model boundary exists that can be exercised without a live window
(an `@Observable`/`ObservableObject` the redesign already needs for Ron's own
preview-gallery and sample-data work, C2), re-assert the same invariant
against that object's public state/bindings instead of the view's source
text, and delete the source-scanning version in the same PR that replaces
the surface it was reading.

### `TCShellCoreTests/SettingsPointerTests.swift` (SettingsView.swift)

- `testTheSettingsEntryShowsThePointerSentence` -- the Private AI entry in
  Settings shows the core's `copy.settingsMoved` sentence and navigates to
  `.privateInference` (a pointer to where the switch now lives, not a dead
  end).
- `testTheSwitchIsNoLongerOnTheSettingsEntry` -- Settings itself holds
  neither the switch (`copy.settingsToggle`) nor the write
  (`model.applyPrivateInference`); both belong only on the destination.

Carry forward: whatever Ron's Settings surface becomes, it must still (a)
point a contributor who goes looking for the switch to wherever it actually
lives, worded by the core, and (b) never hold a second copy of that switch
or its write path. Re-pin both halves against the new Settings screen (or
its view-model) once it exists.

### `TraceCommonsAppTests/SourceCheckBindingTests.swift` (SettingsView.swift)

- `testTheSessionSourceRowsAreDrivenByTheModeAndNotTheBoolean` -- each tool's
  Connection row is driven by `settings.routingSourceModes.<tool>` (the
  three-state mode string: watch/off/unset), never collapsed to a boolean
  "configured" flag that conflates `off` and `unset`; and the section writes
  no hand-rolled sentence fragments ("sessions folder set", "usual place").

Carry forward: the specific defect this guards against is a row that reads
true/false instead of the mode, printing one sentence for two different
facts. Re-pin against whatever replaces the Connection section with the same
three-state mode binding, and keep the forbidden-fragment list (source
wording belongs in `source_copy.rs`, not the view).

### `TraceCommonsAppTests/WitnessBindingTests.swift` (SettingsView.swift, the witness card)

- `testTheRefusedToneMapsToRefusedAndNeverToAttention` -- a refusal paints as
  `.refused`, never the softer `.attention`.
- `testTheToneBridgeHasNoDefaultArm` -- every ABI tone case is spelled out
  explicitly; no `default` that would absorb a future case silently.
- `testTheCardDoesNotPaintItselfThroughTheRoutingBridge` -- the witness
  card's tone comes from the witness ABI range, never the disjoint routing
  one.
- `testTheToneIsTakenFromTheStateAndNotFromTheSentence` -- tone is read off
  the state code, never recovered by pattern-matching the rendered sentence.
- `testAnUnnameableStateRendersNoSentenceOfTheCardsOwn` -- an unrecognized
  state renders no sentence at all (an `if let` with no `else`), not a
  made-up one.
- `testTheToneIsComputedOutsideTheSentencesConditional` -- the tone still
  paints even when the sentence branch does not fire.
- `testEveryRefusalIsOfferedAWayOut` -- a refusing, unpinned witness is never
  a dead end with no path to configure one.
- `testTheFieldsStayLiveInARefusal` -- the pin-a-measurement fields stay
  editable while refused (the way out above has to be reachable).
- `testEachFieldIsBoundToItsOwnProperty` -- the URL field writes the URL
  property, the signing-key field writes the key property, etc. -- never
  cross-wired.
- `testConfigureIsRefusedUntilSomethingIsPinned` -- Configure cannot be
  pressed with nothing pinned (which would refuse every future submission).
- `testTheCountIsRenderedAsASentenceAndNeverAsABareNumeral` -- the pin count
  is always the Rust's sentence, never a bare number.
- `testANullCountLineRendersNothingRatherThanAPlaceholder` -- an unreadable
  count renders nothing, not a placeholder implying zero or "unknown".
- `testTheMeasurementsEditorIsPreFilledFromWhatWasRead` -- the editor opens
  pre-filled with what was actually read, not blank.
- `testTheEditorDoesNotRewriteAPin` -- nothing silently rewrites an entry
  between read and write.
- `testNoWordingIsAuthoredOnThisCard` -- no sentence on the card is a Swift
  literal; all of it is core copy.
- `testTheCardNeverClaimsATraceIsCleanOrAttested` -- the card never
  summarises a certificate as "clean" or "attested".
- `testTheCardRendersNothingWithoutTheSharedPayload` -- no payload, no
  fallback wording -- the card renders nothing.
- `testTheCardCarriesNoRestartNotice` -- no "restart to apply" text; a
  changed witness is read on the next upload.
- `testAWriteRepublishesWhatWasReadBack` -- after a write, the card
  republishes what the ABI echoed back, not the request it sent.
- `testTheStateIsNeverDerivedFromTheConfigurationHavingAUrl` -- trust state
  always comes from `tc_witness_trust_state`, never inferred from "does the
  config have a URL".

Carry forward: this is twenty tests encoding one review's worth of findings
about a single card. Whatever Ron's witness/routing surface looks like, it
should carry an equivalent binding-level test suite (ideally against a
view-model's published properties rather than view source), covering at
minimum: tone source-of-truth and its closed switch, the card's own silence
when data is missing or unreadable, no shell-authored wording, round-trip
fidelity (publish what was read back, not what was asked for), and that a
refusal is always escapable. Treat this file as the acceptance checklist for
whatever replaces the card, not as twenty independent one-offs.

### `TraceCommonsAppTests/RoutingBindingTests.swift` (SettingsView.swift, the IronWire routing card)

- `testThePortAndFolderFieldsAreLiveOnlyWhileTheSwitchIsOn` -- fields are
  live exactly when the local-service switch is on, not its inverse.
- `testOnlyTheSwitchAndTheTwoButtonsWriteTheDeclaration` -- only the switch
  and Apply write `set_settings`; no field setter writes behind the scenes.
- `testTheSwitchWritesOnlyItsOwnFieldAndThatSpellsNull` -- turning the switch
  off writes a null form, carrying whatever was on screen rather than
  stamping a value into it.
- `testTheCardAsksWhatTheMachineKnowsWhenItAppears` -- the card asks the
  model (not a file, not nothing) for the discovered port on appear.
- `testTheDiscoveryOfferIsTheSharedSentence` -- "no IronWire found" and
  similar are ordinary states with the shared sentence, not error states.
- `testADiscoveredPortNeverReplacesADeclaredOne` -- a stale discovered port
  never overwrites a port the contributor declared.
- `testThePortAndFolderCollapseOnlyOnceSomethingWasDiscovered` -- the manual
  fields stay open by default and only collapse once discovery actually
  answered.
- `testThePerToolWordsComeFromTheProbeAndNeverFromTheSwitch` -- each tool's
  reachability word comes from the probe, never read off the switch state.
- `testNoStylingDecisionIsMadeAgainstARenderedString` -- tone/styling is
  never chosen by `contains`-matching the rendered sentence (the exact shape
  of the "unreachable" matched as "reachable" bug).
- `testTheSentenceAndTheStampReadTheSameDaemonState` -- the "last checked"
  stamp and the status sentence are gated on the same state, never two.
- `testTheStatusSentenceIsPaintedFromTheStateAndNotFromItsOwnText` -- the
  sentence's own tone/paint comes from the state, not from parsing itself.
- `testNoStateOnThisCardIsPaintedAsAFault` -- the "not yet checked" state is
  its own thing, neither off nor broken, and is only reachable through the
  shared bridge.
- `testAppearingWithADeadProxyWritesTheSentenceAndNotOnlyTheWords` -- the
  probe call on appear writes the full state, not only a words-only path.
- `testTheProbeSentenceIsRenderedUnchanged` -- the daemon's probe sentence is
  shown verbatim, nothing wrapped around it.
- `testTheCardPrintsNoWordsOfItsOwn` -- no literal but punctuation and empty
  `TextField` placeholders.
- `testTheCardCarriesNoRestartNotice` -- no restart-to-apply text here
  either.
- `testTheCardRendersNothingWithoutTheSharedPayload` -- same rule as the
  witness card: no payload, no fallback wording.

(`RoutingOriginDecodeTests.testOriginComesOnlyFromReportedDerivedFlag`, in
the same file, is a plain `Codable` decode test with no view-source
dependency and does not need to move.)

Carry forward: same approach as the witness card above -- this is the
acceptance checklist for the IronWire routing surface. Preserve, at minimum:
field liveness gated on the switch (not its inverse), a single declared
writer, discovery never clobbering a declaration, tone/state read from ABI
values and never recovered from rendered text, and the shared-payload /
no-payload rule.

### `TraceCommonsAppTests/ConsentBindingTests.swift` (PreviewSheet.swift, the consent/Contribute surface)

- `testContributeIsArmedByTheEnrollmentAndNotByTheSummaryArriving` --
  Contribute arms on `summary?.enrolled`, never on `summary != nil` (a
  summary can exist from an unenrolled device's placeholder identity).
- `testContributeIsDisarmedWhenTheSentencesCouldNotBeRead` -- Contribute
  stays disarmed when the consent copy itself (`consent != nil`) failed to
  decode -- no claim is made that cannot be printed.
- `testTheTooltipIsEmptyRatherThanWrongWhenTheSentencesAreMissing` -- the
  tooltip is silent, not wrong, under the same missing-copy condition.
- `testTheStatementIsReadFromTheSharedCopy` -- the gate statement reads
  `consent?.gateStatement`; the old `ReadGate.statement` constant is banned.
- `testNoSentenceIsAuthoredOnThisSurface` -- `canContribute` / `gateHelp` /
  `gateStatement` contain no shell-written string literal.

Carry forward: the core guarantee is "no claim without a pin, no approval
without the sentences that explain it". Re-pin against whatever Ron's
review/Contribute surface becomes: arming keyed on enrollment (not mere
arrival of data), silence rather than a wrong tooltip when copy fails to
decode, and the gate statement read from the core, never authored locally.

### `TraceCommonsAppTests/AdmissionPlacementTests.swift` (PreviewSheet.swift)

- `testTheFooterDrawsTheAdmissionPreparationControl` -- the
  admission-evidence control is drawn in the footer, on every preview.
- `testTheFailureBranchNoLongerHidesTheAdmissionPreparationControl` --
  it is NOT drawn only inside the failed-preview branch (the bug this test
  exists to catch: a contributor had to fail a review once to find it).
- `testTheMovedControlIsStillGatedOnTheEnrolment` -- the footer still checks
  `admissionEvidenceOffered` before the control, in that order.

Carry forward: wherever the admission-evidence control lands in the redesign
(a footer, a persistent toolbar, whatever the three-pane layout uses), it
must be reachable from every preview, not only a failure path, and must stay
gated on the same enrollment check.

### `TCShellCoreTests/ShellWordingTests.swift` (both files, by line count only)

This is a different shape of fragility: a ratchet over every Swift source
file's count of sentence-like string literals, with
`"TraceCommonsApp/Views/SettingsView.swift": 40` and
`"TraceCommonsApp/Views/PreviewSheet.swift": 38` as two of its ~30 pinned
entries. The redesign will add or remove literal sentences in both files
long before it finishes moving their wording to the core, so these two
numbers will go stale on the first wording-bearing commit, independent of
whether that commit is correct.

Carry forward: when SettingsView.swift/PreviewSheet.swift are replaced,
delete their entries from `wordingBaseline` only once their replacement
views hold zero shell-authored wording (matching the `rustOwnedSurfaces`
list), the same way K3/K4 retired entries for the files they moved copy out
of. Until then, update the two numbers in the same commit that changes
either file's literal count, same as any other baseline move in this file.

## 3. Signing and entitlements checks compared against Tauri's bundle

Both comparisons live in `scripts/ci/test-verify-macos-entitlements.sh`, the
PR-time harness for `scripts/ci/verify-macos-entitlements.sh` (which itself
is already bundle-agnostic: it reads the bundle id and executable name out
of whichever `Info.plist` it is pointed at, and only the keychain access
group `KXSWJN7WY8.ai.tracecommons.shell` and team id `KXSWJN7WY8` are
literal). Per the brief, nothing here is changed -- this is the list for
when Tauri's macOS bundle goes away.

- **"both apps request the same keychain access group"** (lines ~61-67):
  reads `macos/entitlements.plist` and `tauri-desktop/src-tauri/entitlements.plist`
  directly and asserts their `keychain-access-groups` arrays are textually
  equal. This is the thing actually worth keeping: the contributor crate's
  `ACCESS_GROUP` (`crates/trace-commons-contributor/src/daemon/os_secret_store.rs`,
  currently a private `const` = `"KXSWJN7WY8.ai.tracecommons.shell"`) is one
  constant, and a credential one app writes to the keychain has to be
  readable by the other, for as long as both apps exist on a contributor's
  machine. **Once Tauri retires on macOS**, there is no second plist to diff
  against. Stand it alone by asserting `macos/entitlements.plist`'s
  `keychain-access-groups` against that Rust constant directly, rather than
  against a second app: export it (`pub const`, or a small `tc_access_group()`
  C ABI getter next to the other `tc_*_json` copy exports) so the shell
  script -- or a Swift test, once one exists that can shell out or link the
  constant -- compares the committed plist to the single source of truth
  instead of to a sibling app's file. Until Tauri actually stops shipping a
  macOS build, leave the two-plist diff in place: it is still true, and it
  is the only thing today that would catch the two plists drifting apart.
- **The "desktop" fixture bundle** (lines ~96-114): builds a fixture `.app`
  using Tauri's real bundle id (`ai.tracecommons.desktop`), executable name
  (`trace-commons-tauri-desktop`) and entitlements
  (`tauri-desktop/src-tauri/entitlements.plist`), signs it, embeds the
  native shell's provisioning profile in it, and asserts
  `verify-macos-entitlements.sh` refuses it for the App ID mismatch
  (`profile is for ai.tracecommons.shell, not ai.tracecommons.desktop`).
  This is testing `verify-macos-entitlements.sh`'s own cross-wiring check,
  not anything about Tauri specifically -- any second bundle id would do.
  **Once Tauri retires on macOS**, replace the fixture's identity with a
  synthetic one (e.g. `ai.tracecommons.other-app` / `OtherApp`, built the
  same way the `good`/`bare` fixtures already are, rather than from a real
  product's files) so the mismatch test no longer depends on
  `tauri-desktop/src-tauri/entitlements.plist` existing or being kept in
  sync with anything. The comment block above the fixture already says as
  much ("There is no positive Tauri fixture... only the Apple Developer
  account can issue [it]") -- this is the negative half, and it never needed
  Tauri's files to be real, only present and distinct from the native
  shell's.
- Not a comparison, but adjacent: `verify-macos-entitlements.sh`'s own
  header comment ("It serves both macOS apps: the native shell... and the
  Tauri desktop app...") and the CI wiring (`release-apps.yml`'s `macos` job
  invokes it once, against `macos/.build/TraceCommons.app`; no job invokes it
  against a Tauri-built macOS app) should be re-read once R15 lands: the
  comment should drop the Tauri half, and if `tauri-desktop/src-tauri/entitlements.plist`
  is deleted along with Tauri's macOS target, the two items above are the
  only places that reference it.
