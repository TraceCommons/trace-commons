import XCTest

@testable import TraceCommonsApp

/// R11 of #1173: the Settings window's section list (spec, "Settings
/// navigation") chooses which part of the existing settings is drawn.
@MainActor
final class SettingsSectionsTests: XCTestCase {
    /// The spec's sections, in its order, plus Compute. Every one is
    /// drawn in the body; Notifications and Updates follow Startup.
    func test_theSectionsAreTheSpecsInItsOrder() {
        XCTAssertEqual(SettingsSection.allCases, [
            .connection, .startup, .notifications, .updates, .watching, .consent, .publicProfile,
            .watchedFolders, .tools, .privateAI, .witness, .projects, .changes, .compute,
        ])
        XCTAssertEqual(Set(SettingsSection.allCases.map(\.symbol)).count, SettingsSection.allCases.count)
    }

    /// The list is #1146's twelve (`sections.ts`), in his order and his
    /// words, from the core: Notifications and Updates sit under "Startup &
    /// notifications", and nothing truncates to "Private AI on this com…".
    func test_theListIsRonsTwelveInTheCoresWords() throws {
        XCTAssertEqual(SettingsSection.listed, [
            .connection, .startup, .watching, .consent, .publicProfile, .watchedFolders, .tools,
            .privateAI, .witness, .projects, .changes, .compute,
        ])
        let nav = try XCTUnwrap(MonitorWords.table?.settingsNav)
        XCTAssertEqual(SettingsSection.listed.map { $0.listRow(nav).text }, [
            "Connection", "Startup & notifications", "Watching", "Data uses", "Public profile",
            "Watched folders", "Tools", "Private AI", "Redaction witness", "Projects",
            "Change log", "Compute",
        ])
        XCTAssertNil(SettingsSection.notifications.navName(nav))
        XCTAssertNil(SettingsSection.updates.navName(nav))
    }

    /// A section whose copy has not loaded is a disabled placeholder row,
    /// never a vanished one; a loaded name is the row's text.
    func test_aSectionWithNoCopyIsAPlaceholderNotAMissingRow() {
        XCTAssertEqual(SettingsSection.ListRow.row(title: nil), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: ""), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: "Private AI"), .init(text: "Private AI", enabled: true))
        for section in SettingsSection.listed {
            XCTAssertEqual(section.listRow(nil), .init(text: "—", enabled: false), "\(section)")
        }
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
        // #1146's head: Connected or Not connected as the h2 and Ready or
        // Local only as the chip, neither before the core has answered.
        for needle in ["guard model.statusRead == .answered else { return nil }",
                       "model.status.loggedIn ? SettingsLegacyWords.connected : SettingsLegacyWords.notConnected",
                       "GlassChip(glass: model.status.loggedIn ? SettingsLegacyWords.connectionReady",
                       ": SettingsLegacyWords.connectionLocalOnly,"] {
            XCTAssertTrue(connection.contains(needle), "ConnectionSection.swift lacks \(needle)")
        }
        XCTAssertNotEqual(SettingsLegacyWords.connectionReady, SettingsLegacyWords.connectionLocalOnly)
        XCTAssertFalse(SettingsLegacyWords.connectionReady.isEmpty)
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
    /// source, read in the section's own file. Startup, Watching and Uses
    /// take #1146's two-level card heads instead (an eyebrow over an h2);
    /// the modal's section rule still names each by the list's word.
    func test_eachSectionIsHeadedByTheListsTitleSource() throws {
        let headings: [SettingsSection: (file: String, source: String)] = [
            .connection: ("ConnectionSection", "GlassEyebrowCard(SettingsWords.connection, title: connectionTitle)"),
            .startup: ("StartupSection",
                       "GlassEyebrowCard(SettingsLegacyWords.desktopEyebrow, title: SettingsLegacyWords.desktopTitle)"),
            .notifications: ("StartupSection", "if let heading = Notifier.copy?.notificationHeading"),
            .updates: ("StartupSection", "GlassEyebrowCard(SettingsWords.updates)"),
            .watching: ("WatchingSection",
                        "GlassEyebrowCard(SettingsLegacyWords.discoveryEyebrow, title: SettingsLegacyWords.discoveryTitle)"),
            .consent: ("ConsentSection",
                       "GlassEyebrowCard(SettingsLegacyWords.consentEyebrow, title: SettingsLegacyWords.consentHeading)"),
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

    /// #1146's watcher card: drawn only once the core's status is read, a
    /// glass chip for Watching or Paused, the caption, and Pause watcher and
    /// Resume watcher, each enabled only where it changes something. Every
    /// word is the core's.
    func test_watchingHasRonsWatcherCard() throws {
        let source = try SettingsParityTests.text("Views/Settings/WatchingSection.swift")
        for needle in ["if model.statusRead == .answered {\n            watcher(paused: model.status.paused, armed: Self.controlsArmed(",
                       "GlassEyebrowCard(SettingsLegacyWords.watcherEyebrow, title: SettingsLegacyWords.watcherTitle)",
                       "GlassChip(glass: paused ? SettingsLegacyWords.watcherPaused : SettingsLegacyWords.watcherWatching,",
                       "Button(SettingsLegacyWords.pauseWatcher) { model.pause(until: nil) }",
                       ".disabled(paused || !armed)", "Button(SettingsLegacyWords.resumeWatcher) { model.resume() }",
                       ".disabled(!paused || !armed)", "Text(SettingsLegacyWords.watcherCaption)",
                       "lastReadFailed: model.statusReadFailed, startup: model.startup"] {
            XCTAssertTrue(source.contains(needle), "WatchingSection.swift lacks \(needle)")
        }
        // #1273 review: a stale status (answered once, its last read failed,
        // or the core not running) never arms Pause or Resume.
        XCTAssertTrue(WatchingSection.controlsArmed(read: .answered, lastReadFailed: false, startup: .running))
        XCTAssertFalse(WatchingSection.controlsArmed(read: .answered, lastReadFailed: true, startup: .running))
        XCTAssertFalse(WatchingSection.controlsArmed(read: .answered, lastReadFailed: false, startup: .starting))
        XCTAssertFalse(WatchingSection.controlsArmed(read: .answered, lastReadFailed: false, startup: .refused("x")))
        XCTAssertFalse(WatchingSection.controlsArmed(read: .failed, lastReadFailed: true, startup: .running))
        let shell = try XCTUnwrap(MonitorWords.table?.shell)
        for word in [shell.watcherEyebrow, shell.watcherTitle, shell.watcherWatching, shell.watcherPaused,
                     shell.watcherCaption, shell.settingsRefresh, shell.consentEyebrow, shell.desktopEyebrow,
                     shell.desktopTitle, shell.discoveryEyebrow, shell.discoveryTitle] {
            XCTAssertFalse(word.isEmpty)
        }
        XCTAssertNotEqual(shell.watcherWatching, shell.watcherPaused)
        // Every card with a re-read link uses the core's word for it.
        for file in ["WatchingSection", "ConsentSection", "StartupSection"] {
            let text = try SettingsParityTests.text("Views/Settings/\(file).swift")
            XCTAssertTrue(text.contains("Button(SettingsLegacyWords.refresh"), "\(file) has no Refresh")
        }
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
