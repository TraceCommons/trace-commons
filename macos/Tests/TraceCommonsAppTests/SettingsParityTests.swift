import XCTest

@testable import TraceCommonsApp

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
                              "SettingsLegacyWords.notConnected", "SettingsLegacyWords.extraScanConfigured",
                              "SettingsStateRow(title: SettingsLegacyWords.extraScanConfigured"],
                confirmations: [],
                accessibility: []),
        Section(glass: "Views/Settings/StartupSection.swift",
                bindings: ["LoginItemManager.currentState", "LoginItemManager.register()", "LoginItemManager.unregister()",
                           "Notifier.shared.authorizationStatus()", "Notifier.shared.requestAuthorization()",
                           "NSApplication.didBecomeActiveNotification",
                           "updates.currentVersion", "updates.mode", "updates.lastCheckDate", "updates.canCheckNow",
                           "updates.checkNow()", "NSPasteboard.general.setString",
                           "if let status = notificationStatus"],
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
        Section(glass: "Views/Settings/ConsentSection.swift",
                bindings: ["model.consentScopes", "model.status.consentScopes", "model.status.loggedIn",
                           "model.setConsentScopes(", "ConsentScopeRows.nextScopes(",
                           "ConsentScopeRows.isOn(scope: scope, granted: granted, unavailable: unavailable)",
                           "ConsentScopeRows.isEnabled(scope: scope, busy: busy, unavailable: unavailable)"],
                copySources: ["ScopeCopy.title(for:", "scope.description", "settingsCopy()?.consentSaveFailed",
                              "SettingsLegacyWords.consentHeading", "SettingsLegacyWords.appliesFromNow",
                              "SettingsLegacyWords.alwaysIncluded", "SettingsLegacyWords.optionalEachOne",
                              "SettingsLegacyWords.credit", "SettingsLegacyWords.nothingPreselected"],
                confirmations: [],
                accessibility: ["GlassCheckboxStyle()", ".accessibilityElement(children: .combine)"]),
        Section(glass: "Views/Settings/WatchingSection.swift",
                bindings: ["quiescenceSecs", "digestIntervalSecs", "queueTtlDays", "localNotifications",
                           "model.status.paused"],
                copySources: ["SettingsLegacyWords.sessionFinishedAfter(", "SettingsLegacyWords.atMostOneNotification(",
                              "SettingsLegacyWords.undecidedDropped(", "SettingsLegacyWords.notificationsRenderedHere",
                              "SettingsLegacyWords.pausedNothingSent",
                              "SettingsStateRow(title: SettingsLegacyWords.notificationsRenderedHere"],
                confirmations: [],
                accessibility: []),
        Section(glass: "Views/Settings/ChangesSection.swift",
                bindings: ["model.audit", "model.refreshAudit()"],
                copySources: ["SettingsLegacyWords.auditHeading", "SettingsLegacyWords.nothingChanged",
                              "SettingsLegacyWords.auditSentence("],
                confirmations: [],
                accessibility: [".accessibilityElement(children: .combine)"]),
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
        Section(glass: "Views/Settings/WatchedFoldersSection.swift",
                bindings: ["TCSourceChecks.settingsCopy()", "SourceKind.allCases", "TCDiscovery.sourcesJSON()",
                           "SourceCandidate.decodeList(", "model.setSourceRoot(", "model.refreshSettings()",
                           "routingSourceModes", "opencodeSourceMode", "GlassSourceRow("],
                copySources: ["copy.heading", "copy.explanation", "copy.saveFailed", "copy.unavailable", "copy.retry"],
                confirmations: [],
                accessibility: []),
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
        Section(glass: "Views/Settings/GlassSourceRow.swift",
                bindings: ["SourceRowState.answer(", "static func chooseFolder()"],
                copySources: ["copy.watchCandidate", "tool.decline", "tool.chooseFolder"],
                confirmations: ["NSOpenPanel"],
                accessibility: []),
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
    /// The yes/no state of a check row is words and a glyph, not a colour.
    func test_stateRowCarriesItsStateInWords() throws {
        let source = try Self.text("Views/Settings/SettingsStateRow.swift")
        XCTAssertTrue(source.contains(".accessibilityLabel(SettingsLegacyWords.stateLabel(title, isOn))"))
        XCTAssertTrue(source.contains("checkmark.circle.fill"))
        XCTAssertEqual(SettingsLegacyWords.stateLabel("X", true), "X: yes")
        XCTAssertEqual(SettingsLegacyWords.stateLabel("X", false), "X: no")
    }

    /// The notification refresh must hang on a node that exists while the
    /// status is still nil, never on a container whose only child is the
    /// conditional card (an empty container may never run its modifiers).
    func test_notificationRefreshIsOnAnAlwaysPresentContainer() throws {
        let source = try Self.text("Views/Settings/StartupSection.swift")
        let start = try XCTUnwrap(source.range(of: "struct NotificationsSection"))
        let body = String(source[start.lowerBound...])
        XCTAssertTrue(body.contains("Color.clear.frame(width: 0, height: 0)"), "no always-present anchor")
        XCTAssertFalse(body.contains("Group {"), "refresh chained onto a conditional-only Group")
        let anchor = try XCTUnwrap(body.range(of: "Color.clear.frame(width: 0, height: 0)"))
        let task = try XCTUnwrap(body.range(of: ".task { await refreshStatus() }"))
        XCTAssertLessThan(anchor.lowerBound, task.lowerBound)
        XCTAssertEqual(body.components(separatedBy: "Notifier.shared.authorizationStatus()").count - 1, 1,
                       "the refresh closure is duplicated")
    }

    /// The witness card's refresh and its unavailable branch hang on an
    /// always-present container; the token block keeps its hide-and-disable
    /// pair; the four dialogs are system confirmation dialogs; and a refusal
    /// is never drawn as anything but outside.
    func test_witnessSectionShape() throws {
        let source = try Self.text("Views/Settings/WitnessSection.swift")
        let unavailable = try XCTUnwrap(source.range(of: "} else {\n"))
        let anchor = try XCTUnwrap(source.range(of: "Color.clear.frame(width: 0, height: 0)"))
        let appear = try XCTUnwrap(source.range(of: ".onAppear {"))
        XCTAssertLessThan(unavailable.lowerBound, anchor.lowerBound)
        XCTAssertLessThan(anchor.lowerBound, appear.lowerBound)
        XCTAssertEqual(source.components(separatedBy: "model.refreshWitness()").count - 1, 1)
        XCTAssertTrue(source.contains(".opacity(copy.tokenHeading == nil ? 0 : 1)"))
        XCTAssertTrue(source.contains(".disabled(copy.tokenHeading == nil)"))
        XCTAssertEqual(source.components(separatedBy: ".confirmationDialog(").count - 1, 4)
        XCTAssertTrue(source.contains("GlassNotice(tone: .outside)"))
        XCTAssertFalse(source.contains("NativeFlowNotice"))
        XCTAssertEqual(WitnessSection.tone(.refused), .outside)
        XCTAssertEqual(WitnessSection.tone(.attention), .ask)
        XCTAssertEqual(WitnessSection.tone(.held), .ask)
        XCTAssertEqual(WitnessSection.tone(.clear), .on)
        XCTAssertEqual(WitnessSection.tone(.neutral), .off)
    }

    func test_theGlassContentDrawsEverySection() throws {
        let source = try Self.text("Views/Settings/GlassSettingsContent.swift")
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("case .\(section.rawValue):"), "GlassSettingsContent lacks .\(section.rawValue)")
        }
    }
}
