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

    func test_theGlassContentDrawsEverySection() throws {
        let source = try Self.text("Views/Settings/GlassSettingsContent.swift")
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("case .\(section.rawValue):"), "GlassSettingsContent lacks .\(section.rawValue)")
        }
    }
}
