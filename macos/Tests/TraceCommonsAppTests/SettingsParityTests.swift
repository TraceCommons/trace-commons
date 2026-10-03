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

    func test_theGlassContentDrawsEverySection() throws {
        let source = try Self.text("Views/Settings/GlassSettingsContent.swift")
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("case .\(section.rawValue):"), "GlassSettingsContent lacks .\(section.rawValue)")
        }
    }
}
