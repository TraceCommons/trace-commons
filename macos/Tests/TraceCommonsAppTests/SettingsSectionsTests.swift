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

    /// Every section but Compute (its own view) draws its own glass view
    /// when chosen, and no two sections draw the same one: nothing in
    /// Settings is reachable only by scrolling the main window, and nothing
    /// is drawn twice when the main window lists every section.
    func test_everySectionsContentIsReachable() throws {
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        var drawn: [String] = []
        for section in SettingsSection.allCases {
            let view = try XCTUnwrap(Self.view(drawnFor: section, in: content), "no arm for .\(section.rawValue)")
            if section == .compute {
                XCTAssertEqual(view, "EmptyView()")
            } else {
                XCTAssertNotEqual(view, "EmptyView()", "\(section) draws nothing")
                drawn.append(view)
            }
        }
        XCTAssertEqual(drawn.count, Set(drawn).count, "a view is drawn by two sections: \(drawn)")
    }

    /// Connection carries the account contribution card, and the card's
    /// heading and controls are the core's words, not this shell's.
    func test_connectionDrawsTheAccountContributionCardInCoreWords() throws {
        let connection = try SettingsParityTests.text("Views/Settings/ConnectionSection.swift")
        XCTAssertTrue(connection.contains("ContributionAccountCard()"), "Connection no longer draws the card")
        for field in [
            "accountContributionHeading", "accountContributionRefreshAction",
            "accountContributionInviteCode", "accountContributionRedeemAction",
        ] {
            XCTAssertTrue(connection.contains("copy.\(field)"), "the card does not read \(field)")
        }
    }

    /// Startup no longer hides Notifications and Updates behind its row, and
    /// Private AI draws the route disclosure with its pointer.
    func test_notificationsAndUpdatesAreTheirOwnSections() throws {
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertEqual(Self.view(drawnFor: .startup, in: content), "StartupSection()")
        XCTAssertEqual(Self.view(drawnFor: .notifications, in: content), "NotificationsSection()")
        XCTAssertEqual(Self.view(drawnFor: .updates, in: content), "UpdatesSection()")
        XCTAssertEqual(Self.view(drawnFor: .privateAI, in: content), "PrivateAISection(onPointer: onPrivateAI)")

        let startup = try SettingsParityTests.text("Views/Settings/StartupSection.swift")
        let startupBody = try XCTUnwrap(startup.components(separatedBy: "struct NotificationsSection").first)
        XCTAssertFalse(startupBody.contains("NotificationsSection()"), "Startup draws Notifications too")
        XCTAssertFalse(startupBody.contains("UpdatesSection()"), "Startup draws Updates too")

        let privateAI = try SettingsParityTests.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(privateAI.contains("model.routeDisclosureState"), "Private AI no longer draws the route disclosure")
    }

    /// Each section's heading is the one the list names it by: the same
    /// source, read in the section's own file.
    func test_eachSectionIsHeadedByTheListsTitleSource() throws {
        let headings: [SettingsSection: (file: String, source: String)] = [
            .connection: ("ConnectionSection", "GlassEyebrowCard(SettingsWords.connection)"),
            .startup: ("StartupSection", "GlassEyebrowCard(SettingsWords.startup)"),
            .notifications: ("StartupSection", "if let heading = Notifier.copy?.notificationHeading"),
            .updates: ("StartupSection", "GlassEyebrowCard(SettingsWords.updates)"),
            .watching: ("WatchingSection", "GlassEyebrowCard(SettingsWords.watching)"),
            .consent: ("ConsentSection", "GlassEyebrowCard(SettingsLegacyWords.consentHeading)"),
            .publicProfile: ("PublicProfileSection", "GlassEyebrowCard(PublicProfileCopy.heading)"),
            .watchedFolders: ("WatchedFoldersSection", "GlassEyebrowCard(copy.heading)"),
            .tools: ("ToolsSection", "GlassEyebrowCard(copy.toolsHeading)"),
            .privateAI: ("PrivateAISection", "GlassEyebrowCard(copy.settingsTitle)"),
            .witness: ("WitnessSection", "GlassEyebrowCard(copy.heading)"),
            .projects: ("ProjectsSection", "GlassEyebrowCard(SettingsWords.projects)"),
            .changes: ("ChangesSection", "GlassEyebrowCard(SettingsLegacyWords.auditHeading)"),
        ]
        for section in SettingsSection.allCases where section != .compute {
            let heading = try XCTUnwrap(headings[section], "no heading recorded for \(section)")
            let source = try SettingsParityTests.text("Views/Settings/\(heading.file).swift")
            XCTAssertTrue(source.contains(heading.source), "\(heading.file) is not headed by \(heading.source)")
        }
        // The two the words table holds are the list's own two.
        XCTAssertEqual(SettingsLegacyWords.consentHeading, SettingsContent.consentHeading)
        XCTAssertEqual(SettingsLegacyWords.auditHeading, SettingsContent.auditHeading)
    }

    /// The view a section's `case` arm draws in `GlassSettingsContent`.
    static func view(drawnFor section: SettingsSection, in content: String) -> String? {
        let arm = "case .\(section.rawValue):"
        guard let line = content.split(separator: "\n").first(where: {
            $0.trimmingCharacters(in: .whitespaces).hasPrefix(arm)
        }) else { return nil }
        return line.trimmingCharacters(in: .whitespaces).dropFirst(arm.count)
            .trimmingCharacters(in: .whitespaces)
    }
}
