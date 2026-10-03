import TCShellCore
import XCTest

@testable import TraceCommonsApp

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

    static let steps: [Step] = [
        Step(file: "Views/OnboardingWelcomeView.swift",
             bindings: ["var onGetStarted: () -> Void", "var onWhatGetsRemoved: () -> Void",
                        "Button(OnboardingWelcomeWords.getStarted, action: onGetStarted)",
                        "Button(OnboardingWelcomeWords.whatGetsRemoved, action: onWhatGetsRemoved)"],
             copySources: ["TCOnboardingCopy.load()?.welcomeBody",
                           "Text(OnboardingWelcomeWords.headline)",
                           "GlassTag(OnboardingWelcomeWords.promiseLine1, tone: .accent)",
                           "GlassTag(OnboardingWelcomeWords.promiseLine2, tone: .accent)",
                           "Text(OnboardingWelcomeWords.lede)",
                           "Text(OnboardingWelcomeWords.scrubbing)",
                           "Text(OnboardingWelcomeWords.footer)"],
             guards: [".buttonStyle(GlassButtonStyle(.primary))", ".buttonStyle(GlassButtonStyle(.link))",
                      ".keyboardShortcut(.defaultAction)"]),
        Step(file: "Views/OnboardingRootsView.swift",
             bindings: ["SessionRoots()", "roots.watch(", "roots.isComplete", "roots.settingsJSON()",
                        "TCDiscovery.sourcesJSON()", "SourceCandidate.decodeList(", "model.isStartingDaemon",
                        "model.startDaemon(at:", "GlassSourceRow(kind:", "Button(OnboardingRootsWords.continueButton)", "TCDaemon.TCError.rootsNotDeclared"],
             copySources: ["Text(OnboardingRootsWords.heading)", "Text(OnboardingRootsWords.readsTranscripts)",
                           "Text(OnboardingRootsWords.answerForBoth)", "Text(OnboardingRootsWords.optionalRows)",
                           "failure = OnboardingRootsWords.answerBeforeContinuing"],
             guards: [".disabled(!roots.isComplete || model.isStartingDaemon)"]),
        Step(file: "Views/OnboardingConnectView.swift",
             bindings: ["InviteLink.parse(", "PendingInvite.shared", "pendingInvite.take()", "model.enroll(invite:",
                        "model.status.loggedIn", "NearAiJoinView(onEnrolled:", "NearAccountConnectView(onBusyChanged:"],
             copySources: ["OnboardingConnectWords.deadInvite", "OnboardingConnectWords.heading",
                           "OnboardingConnectWords.pasteTheLink", "OnboardingConnectWords.alreadyConnected",
                           "OnboardingConnectWords.inviteIsFor(", "OnboardingConnectWords.connectingTo(",
                           "OnboardingConnectWords.join("],
             guards: [".onChange(of: pendingInvite.value)", "case .deadInvite:"]),
        Step(file: "Views/ConsentScopesView.swift",
             bindings: ["_selected = State(initialValue: initialSelection)", "onContinue(selected)",
                        "UsesStep.continueLabel(", "model.consentScopes.filter(\\.alwaysOn)",
                        "model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }",
                        "model.consentScopes.filter { !$0.alwaysOn && !$0.grantsDataUse }"],
             copySources: ["ScopeCopy.title(for: scope.name, options: model.consentScopes)",
                           "Text(scope.description)", "Text(ConsentScopesWords.heading)",
                           "Text(ConsentScopesWords.changeLater)",
                           "GlassEyebrowCard(ConsentScopesWords.alwaysIncluded)",
                           "GlassEyebrowCard(ConsentScopesWords.optionalEachOne)",
                           "GlassEyebrowCard(ConsentScopesWords.credit)",
                           "Text(ConsentScopesWords.withdrawLater)", "ConsentScopesWords.continueWith(alwaysOn + selected)"],
             guards: [".toggleStyle(GlassCheckboxStyle())", ".keyboardShortcut(.defaultAction)",
                      ".disabled(scope.alwaysOn)", "isOn: .constant(true))", ".disabled(model.consentScopes.isEmpty)"]),
        Step(file: "Views/OnboardingPrivacyScanView.swift",
             bindings: ["model.daemonSettings?.nearAIConfigured == true", "model.acknowledgeNearAINotice()",
                        "if choice == .localPlusScan", "Button(OnboardingPrivacyScanWords.continueButton)"],
             copySources: ["TCCoreCopy.privacyScanCopyJSON()", "Text(verbatim: copy.title)", "Text(verbatim: copy.localAlways)",
                           "Text(verbatim: copy.offer)", "Text(verbatim: copy.disclosure)",
                           "GlassPickerOption(copy.localOnly,", "GlassPickerOption(copy.withNear,"],
             guards: ["if let copy {", "GlassPicker(copy.title,", ".buttonStyle(GlassButtonStyle(.primary))",
                      ".keyboardShortcut(.defaultAction)", "set: { if let picked = $0 { choice = picked } }"]),
        Step(file: "Views/OnboardingProjectsView.swift",
             bindings: ["ForEach(model.projects) { project in", "if !model.status.answered {",
                        "let target = OnboardingProjectsModes.target(from: project.mode)",
                        "model.setProjectMode(project, mode: target)",
                        "if project.isUnresolvedBucket {", "ProjectErrorNotice()",
                        "Button(OnboardingProjectsWords.continueButton)"],
             copySources: ["Text(ProjectCopy.unresolvedBucketNote)", "Text(OnboardingProjectsWords.heading)",
                           "Text(model.projects.isEmpty ? OnboardingProjectsWords.everyProjectAsksFirst : OnboardingProjectsWords.everyProjectAsksFirstIgnore)",
                           "Text(OnboardingProjectsWords.noProjectsYet)",
                           "GlassEyebrowCard(OnboardingProjectsWords.projects)",
                           "label: (ProjectMode) -> String = ProjectCopy.modeChoiceLabel) -> String? {",
                           "GlassTag(tag, tone: isIgnored ? .neutral : .ask)"],
             guards: ["if let tag = OnboardingProjectsModes.name(project.mode) {",
                      "if let move = OnboardingProjectsModes.name(target) {",
                      "Button(move) {",
                      ".accessibilityLabel([project.displayLabel, move].joined(separator: \", \"))",
                      ".buttonStyle(GlassButtonStyle(.glass))", ".buttonStyle(GlassButtonStyle(.primary))",
                      ".keyboardShortcut(.defaultAction)"]),
        Step(file: "Views/OnboardingDoneView.swift",
             bindings: ["LoginItemManager.currentState", "LoginItemManager.register()", "Notifier.shared.authorizationStatus()",
                        "Notifier.shared.requestAuthorization()", "Button(OnboardingDoneWords.done, action: onFinish)"],
             copySources: ["Notifier.copy?.doneBody", "Notifier.copy?.notificationOffer", "Notifier.copy?.notNow",
                           "Notifier.copy?.notificationAllow ??", "Notifier.copy?.notificationAllowed",
                           "Notifier.copy?.notificationDenied", "Notifier.copy?.systemSettings", "Notifier.purpose",
                           "OnboardingDoneWords.setUpNothingSent", "OnboardingDoneWords.startAtLoginQuestion",
                           "OnboardingDoneWords.needsToBeRunning", "OnboardingDoneWords.willStartNextLogin",
                           "OnboardingDoneWords.almostThere", "OnboardingDoneWords.couldNotTurnOn("],
             guards: [".disabled(notificationRequestPending)", "notificationStatus == .notDetermined",
                      "GlassStatusLabel(OnboardingDoneWords.setUpNothingSent, status: .on)",
                      ".buttonStyle(GlassButtonStyle(.primary))", ".buttonStyle(GlassButtonStyle(.glass))",
                      ".keyboardShortcut(.defaultAction)"]),
    ]

    /// Rows the table must hold; each task that adds a step raises it, so a
    /// dropped row fails here instead of passing silently.
    static let minimumSteps = 7

    func test_theTableKeepsEveryRowAdded() {
        XCTAssertGreaterThanOrEqual(Self.steps.count, Self.minimumSteps)
    }

    func test_everyStepKeepsItsBindingsGuardsAndCopy() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            for needle in step.bindings + step.copySources + step.guards {
                XCTAssertTrue(source.contains(needle), "\(step.file) lacks \(needle)")
            }
        }
    }

    /// The Folders step scrolls exactly once, whichever host draws it: the
    /// wrapper owns the only ScrollView and the activation host adds none.
    func test_foldersStepScrollsOnce() throws {
        let roots = try Self.text("Views/OnboardingRootsView.swift")
        XCTAssertEqual(roots.components(separatedBy: "ScrollView {").count - 1, 1)
        let host = try Self.text("Views/PrivateInferenceActivationView.swift")
        XCTAssertFalse(host.contains("ScrollView {"), "the activation host nests a second ScrollView")
    }

    /// The Scan step acknowledges the third-party notice only when the scan
    /// is chosen: the call sits inside the `.localPlusScan` block, and it is
    /// the file's only call.
    func test_scanAcknowledgesOnlyWhenTheScanIsChosen() throws {
        let source = try Self.text("Views/OnboardingPrivacyScanView.swift")
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let call = "model.acknowledgeNearAINotice()"
        XCTAssertEqual(source.components(separatedBy: call).count - 1, 1, "the call must appear exactly once")
        let opener = "if choice == .localPlusScan {"
        let start = try XCTUnwrap(source.range(of: opener))
        let block = try XCTUnwrap(source[start.upperBound...].firstIndex(of: "}")).self
        XCTAssertTrue(source[start.upperBound..<block].contains(call),
                      "the acknowledgement left the .localPlusScan block")
    }

    /// Projects offers Never and Ask me, never Automatic: arming is asked for
    /// later, from a preview, through the core's confirmation, so the
    /// unresolved bucket can never be armed from here either. The one mode
    /// call is the toggle, and no mode list that could carry Automatic is
    /// read.
    func test_projectsNeverOffersAutomatic() throws {
        let source = try Self.text("Views/OnboardingProjectsView.swift")
        XCTAssertFalse(source.contains(".autoUpload"), "onboarding Projects names Automatic")
        XCTAssertFalse(source.contains("offerableModes"), "onboarding Projects reads a list that can hold Automatic")
        XCTAssertFalse(source.contains("ProjectModeChoices"), "onboarding Projects draws the Settings picker")
        XCTAssertEqual(source.components(separatedBy: "model.setProjectMode(").count - 1, 1)
    }

    /// The toggle moves between Ask me and Never only: Automatic is never
    /// the target, whatever mode the row is in.
    func test_projectsToggleMovesOnlyBetweenAskMeAndNever() {
        XCTAssertEqual(OnboardingProjectsModes.target(from: .ask), .ignore)
        XCTAssertEqual(OnboardingProjectsModes.target(from: .ignore), .ask)
        XCTAssertEqual(OnboardingProjectsModes.target(from: .autoUpload), .ignore)
    }

    /// A mode the core's table does not name has no name here, so neither
    /// its tag nor the toggle that moves to it is drawn: no control is
    /// wordless.
    func test_aModeTheCoreDoesNotNameDrawsNoTagOrToggle() throws {
        let json = """
            {"title":"t","mixed":"m","choices":[{"mode":"notify_only","label":"Ask me","line":"l"}],
             "override_active":"o","clear":"c","auto_partial":"a"}
            """
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: json))
        let label: (ProjectMode) -> String = { copy.label(for: $0) ?? "" }
        XCTAssertNil(OnboardingProjectsModes.name(.ignore, label: label))
        XCTAssertEqual(OnboardingProjectsModes.name(.ask, label: label), "Ask me")
        // With the core's own table, both words are there.
        XCTAssertEqual(OnboardingProjectsModes.name(.ignore), "Never")
        XCTAssertEqual(OnboardingProjectsModes.name(.ask), "Ask me")
    }

    /// The error notice both Projects surfaces draw is dismissible, by the
    /// core's word or the banner's own.
    func test_theProjectErrorIsDismissible() throws {
        let source = try Self.text("Views/Settings/ProjectsSection.swift")
        XCTAssertTrue(source.contains("struct ProjectErrorNotice: View {"))
        XCTAssertTrue(source.contains(
            "Button(Self.dismissLabel ?? ActionMessageBanner.dismissWord) { model.lastActionError = nil }"))
    }

    /// Done is the only place onboarding is marked complete, and the only
    /// place the system is asked about notifications is a button under the
    /// purpose sentence. The file itself never completes onboarding or
    /// calls the model; its one exit is `onFinish`.
    func test_doneCompletesOnlyFromItsButtonAndAsksOnlyFromAButton() throws {
        let source = try Self.text("Views/OnboardingDoneView.swift")
        XCTAssertFalse(source.contains("markOnboardingComplete"), "Done completes onboarding itself")
        XCTAssertEqual(source.components(separatedBy: "Button(OnboardingDoneWords.done, action: onFinish)").count - 1, 1)
        XCTAssertFalse(source.contains("onFinish()"), "Done calls onFinish from somewhere other than its button")
        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let purpose = try XCTUnwrap(flat.range(of: "Text(Notifier.purpose)"))
        let open = "Button(Notifier.copy?.notificationAllow ?? \"\") {"
        XCTAssertEqual(flat.components(separatedBy: open).count - 1, 1)
        let allow = try XCTUnwrap(flat.range(of: open))
        XCTAssertLessThan(purpose.lowerBound, allow.lowerBound, "the ask must sit under the purpose sentence")
        let close = try XCTUnwrap(flat.range(of: ".buttonStyle(GlassButtonStyle(.primary))", range: allow.upperBound..<flat.endIndex))
        let span = String(flat[allow.upperBound..<close.lowerBound])
        // The helper is called exactly once, from inside the Allow button's closure.
        XCTAssertEqual(span.components(separatedBy: "requestAuthorization()").count - 1, 1, "the Allow button must call the helper")
        let outside = flat.replacingCharacters(in: allow.lowerBound..<close.lowerBound, with: "")
        let defined = outside.components(separatedBy: "private func requestAuthorization()").count - 1
        XCTAssertEqual(defined, 1)
        // Outside the span the name appears only as the helper's definition and the one system call.
        XCTAssertEqual(outside.components(separatedBy: "requestAuthorization()").count - 1, 2,
                       "requestAuthorization() is called from somewhere other than the Allow button")
        XCTAssertEqual(flat.components(separatedBy: "Notifier.shared.requestAuthorization()").count - 1, 1)
    }

    /// Content structs hold no ScrollView; the wrapper scrolls once.
    func test_doneScrollsOnce() throws {
        let source = try Self.text("Views/OnboardingDoneView.swift")
        XCTAssertEqual(source.components(separatedBy: "ScrollView {").count - 1, 1)
    }

    /// The refreshes hang on the always-present container, never on a
    /// conditional branch.
    func test_doneRefreshesHangOnTheAlwaysPresentContainer() throws {
        let source = try Self.text("Views/OnboardingDoneView.swift")
            .split(whereSeparator: \.isWhitespace).joined(separator: " ")
        XCTAssertTrue(source.contains(".padding(GlassTokens.Space.panePadding) .frame(maxWidth: .infinity, alignment: .leading) .task { await refreshStatus() } .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification))"))
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
