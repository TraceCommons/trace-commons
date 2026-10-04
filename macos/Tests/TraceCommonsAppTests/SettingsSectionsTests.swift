import XCTest

@testable import TraceCommonsApp

/// R11 of #1173: the Settings window's section list (spec, "Settings
/// navigation") chooses which part of the existing settings is drawn.
@MainActor
final class SettingsSectionsTests: XCTestCase {
    /// The spec's sections, in its order, plus Compute. Notifications and
    /// Updates have rows of their own, after Startup, so they can be found
    /// from the list.
    func test_theSectionsAreTheSpecsInItsOrder() {
        XCTAssertEqual(SettingsSection.allCases, [
            .connection, .startup, .notifications, .updates, .watching, .consent, .publicProfile,
            .watchedFolders, .tools, .privateAI, .witness, .projects, .changes, .compute,
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

    /// With no copy loaded, every section whose title comes from copy has
    /// `title()` nil and draws as the disabled placeholder; the others keep
    /// their words. With the copy loaded, each row is that copy's heading.
    func test_aNilTitleDrawsTheDisabledPlaceholderRow() {
        let none = SettingsSection.TitleSources()
        let fromCopy: [SettingsSection] = [.notifications, .watchedFolders, .tools, .privateAI, .witness]
        for section in SettingsSection.allCases {
            let row = section.listRow(none)
            if fromCopy.contains(section) {
                XCTAssertNil(section.title(none), "\(section)")
                XCTAssertEqual(row, .init(text: "—", enabled: false), "\(section)")
            } else {
                XCTAssertNotNil(section.title(none), "\(section)")
                XCTAssertTrue(row.enabled, "\(section)")
            }
        }

        let loaded = SettingsSection.TitleSources(
            notifications: "N", watchedFolders: "F", tools: "T", privateAI: "P", witness: "W", compute: "C")
        let expected: [SettingsSection: String] = [
            .notifications: "N", .watchedFolders: "F", .tools: "T", .privateAI: "P", .witness: "W", .compute: "C",
        ]
        for (section, text) in expected {
            XCTAssertEqual(section.listRow(loaded), .init(text: text, enabled: true), "\(section)")
        }
        XCTAssertEqual(SettingsSection.updates.listRow(none).text, SettingsWords.updates)
    }

    /// Every section but Compute (its own view) draws something when
    /// chosen, and between them the sections reach every block the main
    /// window draws, each exactly once: nothing in Settings is reachable
    /// only by scrolling the main window.
    func test_everySectionsContentIsReachable() {
        let all = SettingsContent.parts(for: nil)
        XCTAssertEqual(all, SettingsContent.Part.allCases)

        var reached: [SettingsContent.Part] = []
        for section in SettingsSection.allCases {
            let parts = SettingsContent.parts(for: section)
            if section == .compute {
                XCTAssertTrue(parts.isEmpty)
            } else {
                XCTAssertFalse(parts.isEmpty, "\(section) draws nothing")
            }
            reached += parts
        }
        XCTAssertEqual(reached.count, Set(reached).count, "a block is drawn by two sections")
        XCTAssertEqual(Set(reached), Set(all), "a block no section reaches")
    }

    /// Startup no longer hides Notifications and Updates behind its row.
    func test_notificationsAndUpdatesAreTheirOwnSections() {
        XCTAssertEqual(SettingsContent.parts(for: .startup), [.loginItem])
        XCTAssertEqual(SettingsContent.parts(for: .notifications), [.notifications])
        XCTAssertEqual(SettingsContent.parts(for: .updates), [.updates])
        XCTAssertEqual(SettingsContent.parts(for: .privateAI), [.privateInference, .routeDisclosure])
    }
}
