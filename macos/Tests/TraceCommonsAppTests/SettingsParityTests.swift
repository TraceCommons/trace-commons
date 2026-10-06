import TCDesign
import TCShellCore
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
        /// Needles the glass file must not contain.
        var forbidden: [String] = []
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
                           // The write and its `nextScopes` list are the model's
                           // (`AppModel.toggleConsentScope`), so a section switch
                           // cannot drop them; `SettingsSectionStateTests` pins it.
                           "model.toggleConsentScope(", "model.consentWriteRefused",
                           "ConsentScopeRows.isOn(scope: scope, granted: granted, unavailable: unavailable)",
                           "ConsentScopeRows.isEnabled(scope: scope, busy: model.consentWriteBusy, unavailable: unavailable)"],
                copySources: ["ScopeCopy.title(for:", "scope.description", "settingsCopy()?.consentSaveFailed",
                              "ConsentScopeRows.refusalLine(", "MonitorScreensCopy.decode(",
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
        Section(glass: "Views/Settings/ProjectsSection.swift",
                bindings: ["model.projects", "model.setProjectMode(", "model.lastActionError", "armingCandidate",
                           "project.offerableModes", "project.isUnresolvedBucket", "project.displayLabel",
                           "ProjectModeChoices.options("],
                copySources: ["ProjectArmingCopy.decode(fromJSON: TCCoreCopy.armingOfferCopyJSON(",
                              "ProjectCopy.unresolvedBucketNote", "TCCoreCopy.contributionModeCopyJSON()",
                              "SettingsLegacyWords.noProjectsYet"],
                confirmations: [".glassModal(isPresented:", "GlassConfirmation(", "if let project = armingCandidate"],
                accessibility: ["GlassPicker("],
                forbidden: ["ProjectCopy.modeChoiceLabel"]),
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
                confirmations: ["GlassModal(", ".glassModal(isPresented: $showingGoPublic)"],
                accessibility: [".accessibilityLabel(", "GlassCheckboxStyle()"]),
        Section(glass: "Views/Settings/WatchedFoldersSection.swift",
                bindings: ["TCSourceChecks.settingsCopy()", "SourceKind.allCases", "TCDiscovery.sourcesJSON()",
                           "SourceCandidate.decodeList(", "model.saveSourceRoot(", "model.sourceRootBusy",
                           "model.sourceRootSaveFailed", "model.refreshSettings()",
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
                confirmations: ["FolderPanel.choose()"],
                accessibility: ["GlassToggleStyle(.settings)", ".accessibilityLabel(copy.portTitle)",
                                ".accessibilityLabel(copy.folderTitle)", ".accessibilityElement(children: .combine)"]),
        Section(glass: "Views/Settings/PrivateAISection.swift",
                bindings: ["model.privateInferenceCopy", "navigation?.section = .privateInference",
                           "model.routeDisclosureState", "model.routeDisclosureUnreadableCopy", "model.refreshRouteDisclosure()"],
                copySources: ["copy.settingsTitle", "copy.settingsMoved", "copy.destination",
                              "copy.route", "copy.localFilter", "witness.heading", "witness.addressLabel", "witness.signingLabel",
                              "witness.measurementsLabel", "witness.check", "witness.classifier", "witness.origin",
                              "copy.attestedBodies", "copy.receipts", "facts.url", "facts.signingAddress", "facts.pinnedMeasurements",
                              "disclosure.copy.title", "model.routeDisclosureUnreadableCopy?.title",
                              "model.routeDisclosureUnreadableCopy?.panel", "MonitorScreensCopy.decode(",
                              "monitorScreensCopyJSON()"],
                confirmations: [],
                accessibility: [".accessibilityElement(children: .combine)"]),
        Section(glass: "Views/Settings/WitnessSection.swift",
                bindings: ["model.witnessCopy", "= model.witnessState\n", "model.witnessStateCode", "model.witnessStatus?.refusal",
                           "model.witnessStatus?.pinnedMeasurementLine", "model.witnessLabel", "model.witnessBusy", "model.witnessCalls",
                           "ironwireAttestedBodies", "tokenDistributionsContribution", "tokenStorage",
                           "model.inferenceEvidenceBusy", "model.tokenContributionBusy", "model.tokenStorageNotice",
                           "model.inferenceEvidenceSaveFailed", "model.tokenContributionSaveFailed",
                           "model.configureWitness(", "model.clearWitness()", "model.setInferenceEvidence(",
                           "model.setTokenContribution(", "model.setLocalTokenCapture(", "model.cleanTokenStorage(discard:",
                           "model.refreshWitness()", "WitnessForm.fromStatus(", "form.canConfigure"],
                copySources: ["WitnessSurface.stateLine(", "WitnessSurface.tone(forState:", "WitnessSurface.lastResultLine(",
                              "WitnessSurface.lastResultTone(", "WitnessSurface.offersConfigure(", "WitnessSurface.offersClear(",
                              "copy.heading", "copy.intro", "copy.certificateMeans", "Button(copy.clear)", "copy.clearNote",
                              "copy.appliesAtOnce", "copy.urlTitle", "copy.signingAddressTitle", "copy.measurementsTitle",
                              "copy.measurementsNote", "copy.configure", "copy.inferenceHeading", "copy.inferenceDisclosure",
                              "copy.inferenceCaptureNote", "copy.inferenceScopeNote", "copy.inferenceEnabled", "copy.inferenceDisabled",
                              "Button(copy.inferenceEnable)", "Button(copy.inferenceDisable)", "copy.inferenceConfirm", "copy.inferenceCancel",
                              "copy.inferenceSaveFailed", "copy.tokenHeading", "copy.tokenDisclosure", "copy.tokenCaptureNote",
                              "copy.tokenScopeNote", "copy.tokenEnabled", "copy.tokenDisabled", "Button(copy.tokenEnable ?? \"\")", "Button(copy.tokenDisable ?? \"\")",
                              "copy.tokenConfirm", "copy.tokenCancel", "copy.tokenSaveFailed", "storage.captureLabel",
                              "storage.captureNotice", "storage.captureConfirmation", "storage.cancelLabel", "storage.stateLine",
                              "storage.scopeNote", "storage.cleanupLabel", "storage.discardLabel", "storage.confirmLabel",
                              "storage.discardConfirmation"],
                confirmations: ["showingInferenceDisclosure", "showingTokenDisclosure", "showingTokenCapture", "showingTokenDiscard",
                                ".glassModal(isPresented:", "GlassConfirmation("],
                accessibility: [".accessibilityLabel(copy.urlTitle)", ".accessibilityLabel(copy.signingAddressTitle)",
                                "GlassTextArea(copy.measurementsTitle,", ".accessibilityElement(children: .combine)"]),
        Section(glass: "Views/Settings/GlassSourceRow.swift",
                bindings: ["SourceRowState.answer(", "FolderPanel.choose()"],
                copySources: ["copy.watchCandidate", "tool.decline", "tool.chooseFolder"],
                confirmations: ["FolderPanel.choose()"],
                accessibility: []),
    ]

    func test_everyLegacyBindingAndCopySourceHasAGlassHome() throws {
        for section in Self.sections {
            let source = try Self.text(section.glass)
            for needle in section.bindings + section.copySources + section.confirmations + section.accessibility {
                XCTAssertTrue(source.contains(needle), "\(section.glass) lacks \(needle)")
            }
            for needle in section.forbidden {
                XCTAssertFalse(source.contains(needle), "\(section.glass) must not use \(needle)")
            }
        }
    }

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
        // Only the anchor's own modifier and closing braces lie between the
        // anchor and `.onAppear`, so the refresh hangs on the outer container.
        let between = source[anchor.upperBound..<appear.lowerBound]
        XCTAssertEqual(
            between.filter { !$0.isWhitespace },
            ".accessibilityHidden(true)}}",
            "something other than closing braces sits between the anchor and .onAppear")
        let closure = String(source[appear.upperBound...].prefix(while: { $0 != "}" }))
        XCTAssertTrue(closure.contains("model.refreshWitness()"), "onAppear does not refresh")
        XCTAssertEqual(source.components(separatedBy: "model.refreshWitness()").count - 1, 1)
        // Four whole-window confirmations, each with its cancel; Discard is
        // the one destructive answer (and its button reads destructive).
        XCTAssertEqual(source.components(separatedBy: ".cancel(").count - 1, 4)
        XCTAssertEqual(source.components(separatedBy: ".destructive(storage.confirmLabel)").count - 1, 1)
        XCTAssertEqual(source.components(separatedBy: "role: .destructive").count - 1, 1)
        XCTAssertTrue(source.contains(".accessibilityHidden(copy.tokenHeading == nil)"))
        XCTAssertTrue(source.contains(".opacity(copy.tokenHeading == nil ? 0 : 1)"))
        XCTAssertTrue(source.contains(".disabled(copy.tokenHeading == nil)"))
        XCTAssertEqual(source.components(separatedBy: ".glassModal(isPresented:").count - 1, 4)
        XCTAssertFalse(source.contains(".confirmationDialog("))
        // A save failure keeps the core's refusal glyph and tone, as legacy
        // `NativeFlowNotice` did; a missing wallet copy is still a refusal.
        XCTAssertFalse(source.contains("GlassNotice(tone: .outside)"), "the save failure's tone is chosen in Swift")
        XCTAssertTrue(source.contains(
            "GlassFlowNotice(message: copy.inferenceSaveFailed, glyph: copy.wallet?.refusedGlyph ?? \"\", tone: copy.wallet?.refusedTone)"))
        XCTAssertTrue(source.contains(
            "GlassFlowNotice(message: copy.tokenSaveFailed ?? \"\", glyph: copy.wallet?.refusedGlyph ?? \"\", tone: copy.wallet?.refusedTone)"))
        XCTAssertEqual(GlassFlowNotice.status(forTone: "refused"), .outside)
        XCTAssertEqual(GlassFlowNotice.status(forTone: "neutral"), .off)
        XCTAssertEqual(GlassFlowNotice.status(forTone: ""), .off)
        XCTAssertEqual(GlassFlowNotice.status(forTone: nil), .outside, "an unread tone is drawn as healthy")
        let notice = try Self.text("Views/Settings/GlassFlowNotice.swift")
        XCTAssertTrue(notice.contains("if !glyph.isEmpty { Text(glyph) }"), "the core's glyph is not drawn")
        XCTAssertTrue(notice.contains("let ink = status == .outside ? status.textColor : GlassColor.textSecondary"))
        XCTAssertTrue(notice.contains(".foregroundStyle(ink)"), "the tone never reaches the words")
        XCTAssertFalse(source.contains("NativeFlowNotice"))
        XCTAssertEqual(WitnessSection.tone(.refused), .outside)
        XCTAssertEqual(WitnessSection.tone(.attention), .ask)
        XCTAssertEqual(WitnessSection.tone(.held), .ask)
        XCTAssertEqual(WitnessSection.tone(.clear), .on)
        XCTAssertEqual(WitnessSection.tone(.neutral), .off)
    }

    /// The refresh hangs on the section's outer container, which always has
    /// a child (the disclosure card), and is not repeated; the three views
    /// stay standalone structs; the destination is read, never typed; and
    /// loading is a spinner, not a shown route.
    func test_privateAISectionShape() throws {
        let source = try Self.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(source.contains("        }\n        .onAppear { model.refreshRouteDisclosure() }\n"),
                      "refresh is not adjacent to the outer container's closing brace")
        XCTAssertEqual(source.components(separatedBy: "model.refreshRouteDisclosure()").count - 1, 1)
        XCTAssertEqual(source.components(separatedBy: ".onAppear").count - 1, 1)
        for name in ["struct PrivateAISection: View", "struct RouteDisclosureGlassBody: View",
                     "struct RouteDisclosureUnreadableGlassLine: View"] {
            XCTAssertEqual(source.components(separatedBy: name).count - 1, 1, "\(name) is not declared exactly once")
        }
        XCTAssertTrue(source.contains("Button(copy.destination)"))
        XCTAssertTrue(source.contains(".buttonStyle(GlassButtonStyle(.link))"))
        for arm in ["case .shown(let disclosure):", "case .loading:", "case .unreadable:"] {
            XCTAssertEqual(source.components(separatedBy: arm).count - 1, 1, "\(arm) is not drawn exactly once")
        }
        XCTAssertTrue(source.contains("GlassStatusLabel(words, status: .ask)"),
                      "unreadable is not drawn with a glyph and words")
        XCTAssertTrue(source.contains("GlassSpinner(standalone: true)"))
        XCTAssertFalse(source.lowercased().contains("private inference"))
        XCTAssertFalse(source.contains("MonitorWords"))
    }

    /// The unreadable state is never a bare dot: with neither of the core's
    /// sentences it reads the core's "unknown" word, and only then a dash.
    func test_unreadableLineNeverDrawsEmpty() {
        typealias Line = RouteDisclosureUnreadableGlassLine
        XCTAssertEqual(Line.text(line: "panel", fallback: "title", unknown: "unk"), "panel")
        XCTAssertEqual(Line.text(line: nil, fallback: "title", unknown: "unk"), "title")
        XCTAssertEqual(Line.text(line: nil, fallback: nil, unknown: "unk"), "unk")
        XCTAssertFalse(Line.text(line: nil, fallback: nil, unknown: nil).isEmpty)
        let source = try? Self.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(source?.contains("GlassWarningGlyph()") ?? false)
        XCTAssertTrue(source?.contains(".accessibilityLabel(words)") ?? false)
    }

    /// The route disclosure is read paragraph by paragraph: its body
    /// contains its children rather than merging them into one utterance.
    func test_routeDisclosureBodyIsNavigableLineByLine() throws {
        let source = try Self.text("Views/Settings/PrivateAISection.swift")
        let start = try XCTUnwrap(source.range(of: "struct RouteDisclosureGlassBody: View"))
        let end = try XCTUnwrap(source.range(of: "struct RouteDisclosureUnreadableGlassLine", range: start.upperBound..<source.endIndex))
        let body = String(source[start.upperBound..<end.lowerBound])
        XCTAssertFalse(body.contains(".accessibilityElement(children: .combine)"), "the disclosure is one VoiceOver element")
        XCTAssertTrue(body.contains(".accessibilityElement(children: .contain)"))
    }

    func test_measurementsRowIsOmittedWhenNothingIsPinned() throws {
        func items(_ pins: [String]) throws -> [GlassKeyValueList.Item] {
            let pinsJSON = pins.map { "\"\($0)\"" }.joined(separator: ",")
            let facts = try JSONDecoder().decode(RouteDisclosure.WitnessFacts.self, from: Data("""
                {"state":"on","url":"u","signing_address":"s","pinned_measurements":[\(pinsJSON)],"origin":"o"}
                """.utf8))
            let witness = try JSONDecoder().decode(RouteDisclosure.WitnessCopy.self, from: Data("""
                {"heading":"h","address_label":"A","signing_label":"S","measurements_label":"M","check":"c","origin":"o"}
                """.utf8))
            return RouteDisclosureGlassBody.witnessItems(witness: witness, facts: facts)
        }
        XCTAssertEqual(try items([]).map(\.label), ["A", "S"])
        XCTAssertEqual(try items(["p1", "p2"]).map(\.label), ["A", "S", "M"])
        XCTAssertEqual(try items(["p1", "p2"]).last?.value, "p1\np2")
    }

    /// The switch draws every section the list offers, by its own case.
    func test_theGlassContentDrawsEverySection() throws {
        let source = try Self.text("Views/Settings/GlassSettingsContent.swift")
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("case .\(section.rawValue):"), "GlassSettingsContent lacks .\(section.rawValue)")
        }
    }

    /// Sections that read nothing from the daemon: the login item, the
    /// system's notification permission and the update feed are all local.
    static let localOnlySections: Set<SettingsSection> = [.startup, .notifications, .updates]

    /// Each daemon-reading section's branch for "the daemon has not answered",
    /// and what that branch draws. Every branch is the absent case written
    /// first, so the text up to its closing brace is exactly what it draws.
    static let daemonAbsentBranches: [SettingsSection: (file: String, branches: [(marker: String, draws: String)])] = [
        .connection: ("ConnectionSection", [
            ("if model.statusRead != .answered {", "SettingsReadNotice(model.statusRead"),
            ("if model.daemonSettings == nil {", "SettingsReadNotice(model.settingsRead"),
        ]),
        .watching: ("WatchingSection", [("if model.daemonSettings == nil {", "SettingsReadNotice(model.settingsRead")]),
        .consent: ("ConsentSection", [("if model.statusRead != .answered {", "SettingsReadNotice(model.statusRead")]),
        .publicProfile: ("PublicProfileSection", [
            ("} else if model.publicProfileRead != .answered {", "SettingsReadNotice(model.publicProfileRead"),
        ]),
        .watchedFolders: ("WatchedFoldersSection", [("if model.daemonSettings == nil {", "Text(copy.unavailable)")]),
        .tools: ("ToolsSection", [("if model.daemonSettings == nil {", "SettingsReadNotice(model.settingsRead")]),
        .witness: ("WitnessSection", [
            ("if model.witnessRead != .answered {", "SettingsReadNotice(model.witnessRead"),
            ("if model.daemonSettings == nil {", "SettingsReadNotice(model.settingsRead"),
        ]),
        .privateAI: ("PrivateAISection", [("case .loading:", "GlassSpinner(standalone: true)")]),
        .projects: ("ProjectsSection", [("if model.projectsRead != .answered {", "SettingsReadNotice(model.projectsRead")]),
        .changes: ("ChangesSection", [("if model.auditRead != .answered {", "SettingsReadNotice(model.auditRead")]),
    ]

    /// Every section that reads the daemon draws its own loading or
    /// unavailable state before the daemon answers, rather than nothing,
    /// rather than an answer read from a default, and rather than a control
    /// that reads as working (Review Focus 1). A branch that draws only a
    /// hidden `Color.clear` does not count.
    func test_everySectionHasAnUnavailableBranch() throws {
        for section in SettingsSection.allCases where section != .compute {
            if Self.localOnlySections.contains(section) {
                XCTAssertNil(Self.daemonAbsentBranches[section], "\(section) is both local-only and daemon-read")
                continue
            }
            let entry = try XCTUnwrap(Self.daemonAbsentBranches[section], "\(section) has no daemon-absent branch recorded")
            let source = try Self.text("Views/Settings/\(entry.file).swift")
            for (marker, draws) in entry.branches {
                guard let start = source.range(of: marker) else {
                    XCTFail("\(entry.file) has no daemon-absent branch `\(marker)`")
                    continue
                }
                let rest = source[start.upperBound...]
                let end = rest.firstIndex(of: "}") ?? rest.endIndex
                let branch = String(rest[..<end])
                XCTAssertTrue(branch.contains(draws), "\(entry.file)'s `\(marker)` branch draws no \(draws): \(branch)")
                XCTAssertFalse(
                    branch.contains("Color.clear") && !branch.contains(draws),
                    "\(entry.file)'s `\(marker)` branch draws only a hidden frame")
            }
        }
        // The shared state is the glass spinner, a progress indicator to
        // VoiceOver (R-20), while a read is in flight, and the core's failure
        // line once it has failed or the core is down: never a spinner that
        // runs for good.
        let awaiting = try Self.text("Views/Settings/SettingsStateRow.swift")
        XCTAssertTrue(awaiting.contains("struct SettingsAwaiting: View"))
        XCTAssertTrue(awaiting.contains("GlassSpinner(standalone: true)"))
        XCTAssertTrue(awaiting.contains("struct SettingsReadNotice: View"))
        let awaitingArm = try XCTUnwrap(awaiting.range(of: "case .awaiting:"))
        let failedArm = try XCTUnwrap(awaiting.range(of: "case .failed, .coreDown:"))
        let answeredArm = try XCTUnwrap(awaiting.range(of: "case .answered:"))
        XCTAssertTrue(awaiting[awaitingArm.upperBound..<failedArm.lowerBound].contains("SettingsAwaiting()"))
        let failedDraws = awaiting[failedArm.upperBound..<answeredArm.lowerBound]
        XCTAssertTrue(failedDraws.contains("SettingsUnavailable("), "a failed read draws no failure line")
        XCTAssertFalse(failedDraws.contains("SettingsAwaiting") || failedDraws.contains("ProgressView"),
                       "a failed read spins")
        // `.unknown` is the placeholder held until the first answer; a real
        // status always carries a schema version.
        XCTAssertFalse(DaemonStatus.unknown.answered)
    }

    /// The answers read from a default are drawn only after the branch above:
    /// "Not connected", "No projects seen yet." and "Nothing has been changed."
    /// would otherwise state a value nothing reported.
    func test_defaultReadAnswersFollowTheAbsentBranch() throws {
        // Each answer waits on the read it is drawn from: `status` answering
        // says nothing about `list_audit`, `list_projects` or the profile.
        for (file, gate, answer) in [
            ("ConnectionSection", "model.statusRead != .answered {", "SettingsLegacyWords.notConnected"),
            ("ConnectionSection", "model.statusRead != .answered {", "SettingsLegacyWords.queuedNothingSent"),
            ("ProjectsSection", "model.projectsRead != .answered {", "SettingsLegacyWords.noProjectsYet"),
            ("ChangesSection", "model.auditRead != .answered {", "SettingsLegacyWords.nothingChanged"),
            ("PublicProfileSection", "model.publicProfileRead != .answered {", "optInCard\n"),
        ] {
            let source = try Self.text("Views/Settings/\(file).swift")
            let branch = try XCTUnwrap(source.range(of: gate), "\(file) has no `\(gate)` branch")
            let drawn = try XCTUnwrap(source.range(of: answer), "\(file) no longer draws \(answer)")
            XCTAssertLessThan(branch.lowerBound, drawn.lowerBound, "\(file) draws \(answer) before its read answers")
        }
        // The placeholder comparison is not an answer to anything a section
        // draws; the per-read states replace it.
        for section in SettingsSection.allCases {
            guard let entry = Self.daemonAbsentBranches[section] else { continue }
            let source = try Self.text("Views/Settings/\(entry.file).swift")
            XCTAssertFalse(source.contains("model.status.answered"), "\(entry.file) still gates on status.answered")
        }
    }

    /// The roster date is the profile card's accessory. Left unlabelled, the
    /// first trailing closure binds to `GlassEyebrowCard`'s `action:`, which
    /// makes the whole card a button and builds the tag without drawing it.
    func test_rosterDateIsTheProfileCardsAccessory() throws {
        let source = try Self.text("Views/Settings/PublicProfileSection.swift")
        XCTAssertTrue(source.contains("GlassEyebrowCard(PublicProfileCopy.heading, accessory: {"))
        XCTAssertFalse(source.contains("} content: {"))
    }

    /// Settings (the Monitor's modal since #1241 Task 10) draws the glass
    /// content and nothing else.
    func test_theWindowDrawsGlassContent() throws {
        for rel in ["Views/Monitor/SettingsModal.swift", "Views/MonitorWindowView.swift"] {
            let source = try Self.text(rel)
            // The glass view's name ends in the legacy one's, so the legacy
            // call is looked for with the glass calls taken out.
            XCTAssertFalse(source.replacingOccurrences(of: "GlassSettingsContent(", with: "").contains("SettingsContent("), rel)
            XCTAssertFalse(source.contains(".tcScreen()"), rel)
        }
        let modal = try Self.text("Views/Monitor/SettingsModal.swift")
        XCTAssertTrue(modal.contains("GlassSettingsContent(navigation: navigation, section: item, onPrivateAI: onPrivateAI)"))
    }
}
