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
             copySources: ["TCCoreCopy.privacyScanCopyJSON()", "Text(copy.title)", "Text(verbatim: copy.localAlways)",
                           "Text(verbatim: copy.offer)", "Text(verbatim: copy.disclosure)",
                           "GlassPickerOption(copy.localOnly,", "GlassPickerOption(copy.withNear,"],
             guards: ["if let copy {", "GlassPicker(copy.title,", ".buttonStyle(GlassButtonStyle(.primary))",
                      ".keyboardShortcut(.defaultAction)", "set: { if let picked = $0 { choice = picked } }"]),
    ]

    /// Rows the table must hold; each task that adds a step raises it, so a
    /// dropped row fails here instead of passing silently.
    static let minimumSteps = 5

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

    /// No rebuilt step reads the legacy palette.
    func test_noStepReadsTheLegacyPalette() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(step.file) reads TC.")
            XCTAssertFalse(source.contains("CommunityBrand"), "\(step.file) reads CommunityBrand")
        }
    }
}
