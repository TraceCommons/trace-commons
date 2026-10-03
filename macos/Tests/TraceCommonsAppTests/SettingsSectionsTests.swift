import XCTest

@testable import TraceCommonsApp

/// R11 of #1173: the Settings window's section list (spec, "Settings
/// navigation") chooses which part of the existing settings is drawn.
@MainActor
final class SettingsSectionsTests: XCTestCase {
    /// The spec's sections, in its order, plus Compute.
    func test_theSectionsAreTheSpecsInItsOrder() {
        XCTAssertEqual(SettingsSection.allCases, [
            .connection, .startup, .watching, .consent, .publicProfile, .watchedFolders,
            .tools, .privateAI, .witness, .projects, .changes, .compute,
        ])
        XCTAssertEqual(Set(SettingsSection.allCases.map(\.symbol)).count, SettingsSection.allCases.count)
    }

    /// A section whose copy has not loaded is a disabled placeholder row,
    /// never a vanished one; a loaded title is the row's text.
    func test_aSectionWithNoCopyIsAPlaceholderNotAMissingRow() {
        XCTAssertEqual(SettingsSection.ListRow.row(title: nil), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: ""), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: "Private AI"), .init(text: "Private AI", enabled: true))
    }

    /// The list draws a row for every section, whatever its copy: no row is
    /// behind an `if let` on a title.
    func test_theListDrawsEverySection() throws {
        let source = try String(contentsOf: Self.source("Views/MonitorWindowView.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains("SettingsSection.ListRow.row(title: item.title("))
        XCTAssertFalse(source.contains("if let title = item.title("))
    }

    /// Every section but Compute (its own view) is drawn by `SettingsContent`
    /// when chosen: a section in the list with nothing behind it would show
    /// an empty pane, which is the selection not matching the content.
    func test_everySectionHasContent() throws {
        let source = try String(contentsOf: Self.source("Views/SettingsView.swift"), encoding: .utf8)
        for section in SettingsSection.allCases where section != .compute {
            XCTAssertTrue(source.contains("shows(.\(section.rawValue))"), "\(section) draws nothing")
        }
    }

    /// The list and the section say the same words: the headings the list
    /// shows are the ones the sections draw.
    func test_theListUsesTheSectionsOwnHeadings() throws {
        let source = try String(contentsOf: Self.source("Views/SettingsView.swift"), encoding: .utf8)
        XCTAssertTrue(source.contains("TCSectionHeader(title: Self.consentHeading)"))
        XCTAssertTrue(source.contains("TCSectionHeader(title: Self.auditHeading)"))
        for word in [SettingsWords.connection, SettingsWords.startup, SettingsWords.watching, SettingsWords.projects] {
            XCTAssertTrue(source.contains("TCSectionHeader(title: \"\(word)\")"), word)
        }
    }

    private static func source(_ path: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
    }
}
