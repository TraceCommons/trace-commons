import XCTest

@testable import TraceCommonsApp

/// R11 of #1173: the Settings window's section list (spec, "Settings
/// navigation") chooses which part of the existing settings is drawn.
@MainActor
final class SettingsSectionsTests: XCTestCase {
    /// The Settings IA review's order (owner, 2026-11-04, D1-D20): Account,
    /// Capture, Sharing, Private AI, App, Advanced. Every section is drawn in
    /// the body; Watched folders follows Watching and Notifications follows
    /// Startup.
    func test_theSectionsAreTheReviewsGroupedOrder() {
        XCTAssertEqual(SettingsSection.allCases, [
            .account, .payouts, .referrals,
            .watching, .watchedFolders, .tools, .insights,
            .projects, .consent, .publicProfile, .changes,
            .privateAI,
            .general, .startup, .notifications, .updates,
            .witness, .compute,
        ])
        XCTAssertEqual(Set(SettingsSection.allCases.map(\.symbol)).count, SettingsSection.allCases.count)
        let groups: [SettingsSection.Group] = [.account, .capture, .sharing, .privateAI, .app, .advanced]
        XCTAssertEqual(SettingsSection.allCases.map(\.group).reduce(into: [SettingsSection.Group]()) {
            if $0.last != $1 { $0.append($1) }
        }, groups, "a group is split")
    }

    /// The list names sixteen rows in the core's words, with a hairline
    /// before the first row of every group but the first (D17: no group
    /// headings). Watched folders sits under Watching (D12) and Notifications
    /// under "Startup & notifications"; Updates has its own row (D19).
    func test_theListIsGroupedInTheCoresWords() throws {
        XCTAssertEqual(SettingsSection.listed, [
            .account, .payouts, .referrals, .watching, .tools, .insights, .projects, .consent, .publicProfile,
            .changes, .privateAI, .general, .startup, .updates, .witness, .compute,
        ])
        let nav = try XCTUnwrap(MonitorWords.table?.settingsNav)
        XCTAssertEqual(SettingsSection.listed.map { $0.listRow(nav).text }, [
            "Account", "Payouts", "Referrals", "Watching", "Tools", "Insights", "Projects", "Data uses",
            "Public profile", "Change log", "Private AI", "General", "Startup & notifications", "Updates",
            "Redaction witness", "Compute",
        ])
        XCTAssertNil(SettingsSection.notifications.navName(nav))
        XCTAssertNil(SettingsSection.watchedFolders.navName(nav))
        let rows = SettingsSection.listed
        XCTAssertEqual(rows.filter { SettingsSection.startsGroup($0, in: rows) },
                       [.watching, .projects, .privateAI, .general, .witness])
        // Payouts is listed only while something in it applies (D2); the
        // hairlines follow the rows actually drawn.
        let withoutPayouts = SettingsModal.rows(payoutsShown: false)
        XCTAssertFalse(withoutPayouts.contains(.payouts))
        XCTAssertEqual(SettingsModal.rows(payoutsShown: true), rows)
        XCTAssertFalse(SettingsSection.startsGroup(.referrals, in: withoutPayouts))
    }

    /// Before first run only the sections that write nothing first run asks
    /// are open (R-43). Account and Private AI are open; Payouts and
    /// Referrals wait, as there is no account to pay yet.
    func test_onlyTheSectionsFirstRunDoesNotOwnAreOpenBeforeOnboarding() {
        XCTAssertEqual(Set(SettingsSection.allCases.filter(\.availableBeforeOnboarding)),
                       [.account, .privateAI, .general, .startup, .notifications, .updates, .compute])
    }

    /// A section whose copy has not loaded is a disabled placeholder row,
    /// never a vanished one; a loaded name is the row's text.
    func test_aSectionWithNoCopyIsAPlaceholderNotAMissingRow() {
        XCTAssertEqual(SettingsSection.ListRow.row(title: nil), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: ""), .init(text: "—", enabled: false))
        XCTAssertEqual(SettingsSection.ListRow.row(title: "Private AI"), .init(text: "Private AI", enabled: true))
        for section in SettingsSection.listed where section != .general {
            XCTAssertEqual(section.listRow(nil), .init(text: "—", enabled: false), "\(section)")
        }
    }

    /// General's name is this app's own word and its content never waits on
    /// the core, so its row is never the placeholder: with the core's table
    /// missing it still reads General and still opens the section.
    func test_generalsRowNeverWaitsOnTheCoresTable() {
        XCTAssertEqual(SettingsSection.general.navName(nil), SettingsWords.general)
        XCTAssertEqual(SettingsSection.general.listRow(nil), .init(text: SettingsWords.general, enabled: true))
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

    /// Account (was Connection, D1) carries the account contribution card in
    /// the core's words, the connected line with the account's login (D6),
    /// the NEAR AI login without Forget (D3, D8), and Sign out over Set up
    /// again at its foot.
    func test_accountDrawsItsCardsInCoreWords() throws {
        let account = try SettingsParityTests.text("Views/Settings/AccountSection.swift")
        XCTAssertTrue(account.contains("ContributionAccountCard()"), "Account no longer draws the card")
        for field in [
            "accountContributionHeading", "accountContributionRefreshAction",
            "accountContributionInviteCode", "accountContributionRedeemAction",
        ] {
            XCTAssertTrue(account.contains("copy.\(field)"), "the card does not read \(field)")
        }
        // #1146's head: the connected line as the h2 and Ready or Local only
        // as the chip, neither before the core has answered.
        for needle in ["guard model.statusRead == .answered else { return nil }",
                       "GlassChip(glass: model.status.loggedIn ? SettingsLegacyWords.connectionReady",
                       ": SettingsLegacyWords.connectionLocalOnly,",
                       "AccountAttentionCard()", "AccountLoginMismatchBlock()", "AccountLoginChangeButton()",
                       "AccountDevicesView()", "LegacyMigrationSettingsCard()", "AccountLeaveCard()",
                       "SetUpAgainControl()"] {
            XCTAssertTrue(account.contains(needle), "AccountSection.swift lacks \(needle)")
        }
        // Sign out comes before Set up again, its body above its button.
        let body = try XCTUnwrap(account.range(of: "Text(copy.signOutBody)"))
        let button = try XCTUnwrap(account.range(of: "Button(copy.signOut)"))
        let setUpAgain = try XCTUnwrap(account.range(of: "SetUpAgainControl()\n"))
        XCTAssertLessThan(body.lowerBound, button.lowerBound)
        XCTAssertLessThan(button.lowerBound, setUpAgain.lowerBound)
        XCTAssertNotEqual(SettingsLegacyWords.connectionReady, SettingsLegacyWords.connectionLocalOnly)
        XCTAssertFalse(SettingsLegacyWords.connectionReady.isEmpty)
    }

    /// "Connected with {provider}" only for an enrolled account whose login
    /// is known and which needs no attention (D6, F1); otherwise the plain
    /// words.
    func test_theConnectedLineNamesTheLoginOnlyWhenItIsTheAccountsKnownLogin() throws {
        let copy = try XCTUnwrap(AccountLoginChangeModel.shared.copy)
        func title(_ loggedIn: Bool, _ login: String?, attention: Bool = false, signedIn: Bool? = true) -> String {
            AccountSection.connectedTitle(
                loggedIn: loggedIn, accountSignedIn: signedIn, accountLogin: login, needsAttention: attention,
                copy: copy)
        }
        XCTAssertEqual(title(true, "github"), "Connected with GitHub")
        XCTAssertEqual(title(true, "github", attention: true), SettingsLegacyWords.connected)
        XCTAssertEqual(title(true, nil), SettingsLegacyWords.connected)
        XCTAssertEqual(title(true, "myspace"), SettingsLegacyWords.connected)
        XCTAssertEqual(title(false, "github"), SettingsLegacyWords.notConnected)
        // An account that never signs in, or an older daemon: as before.
        XCTAssertEqual(title(true, "github", signedIn: nil), "Connected with GitHub")
    }

    /// After Sign out the account stays enrolled, but its session is gone:
    /// the card says so in the core's "Not signed in", never "Connected
    /// with {provider}", draws no Ready chip, and offers the core's "Sign
    /// in", which signs the account back in (owner, 2026-11-04).
    func test_aSignedOutAccountSaysNotSignedInAndOffersSignIn() throws {
        let copy = try XCTUnwrap(AccountLoginChangeModel.shared.copy)
        XCTAssertEqual(AccountSection.head(loggedIn: true, accountSignedIn: false), .signedOut)
        XCTAssertEqual(AccountSection.head(loggedIn: true, accountSignedIn: true), .connected)
        XCTAssertEqual(AccountSection.head(loggedIn: true, accountSignedIn: nil), .connected)
        XCTAssertEqual(AccountSection.head(loggedIn: false, accountSignedIn: false), .notEnrolled)
        let signedOut = AccountSection.connectedTitle(
            loggedIn: true, accountSignedIn: false, accountLogin: "github", needsAttention: false, copy: copy)
        XCTAssertEqual(signedOut, MonitorWords.signedOut)
        XCTAssertFalse(signedOut.isEmpty)
        XCTAssertEqual(MonitorWords.table?.signIn, "Sign in", "the notice's approved word")
        XCTAssertEqual(MonitorWindowView.openFirstRun, MonitorWords.table?.signIn, "one key for both")
        XCTAssertFalse(AccountSection.Head.signedOut.showsChip, "a signed-out account wears no Ready chip")
        XCTAssertTrue(AccountSection.Head.connected.showsChip)
        XCTAssertTrue(AccountSection.Head.notEnrolled.showsChip)

        let account = try SettingsParityTests.text("Views/Settings/AccountSection.swift")
        for needle in [
            // The core's "Sign in", the signed-out notice's own key (#1918),
            // and the account's own way back in (NEAR AI or native, the
            // daemon's choice), as the payout card's Sign in again runs it.
            "if let signIn = MonitorWords.table?.signIn, !signIn.isEmpty {",
            "Button(signIn) { Task { await model.signInToAccount() } }",
            ".disabled(model.accountSigningIn)",
            // One Sign in in the section: the NEAR AI login card and the
            // devices card, whose own Sign in again would be a second, wait
            // for the account to be signed in.
            "if head != .signedOut, let copy = model.privateInferenceCopy {",
            "if head != .signedOut {\n                AccountDevicesView()",
        ] {
            XCTAssertTrue(account.contains(needle), "AccountSection.swift lacks \(needle)")
        }
    }

    /// Forget is drawn nowhere (D3); Private AI's card offers only the way
    /// in (D8); Account keeps the move from the keychain.
    func test_forgetIsGoneAndPrivateAIKeepsOnlyTheWayIn() {
        for placement in [CredentialSection.Placement.account, .signInShortcut] {
            XCTAssertFalse(CredentialSection.draws(.forget, in: placement))
            XCTAssertFalse(CredentialSection.draws(.none, in: placement))
            XCTAssertTrue(CredentialSection.draws(.obtain, in: placement))
            XCTAssertTrue(CredentialSection.draws(.cancel, in: placement))
        }
        XCTAssertTrue(CredentialSection.draws(.migrate, in: .account))
        XCTAssertFalse(CredentialSection.draws(.migrate, in: .signInShortcut))
        // The shortcut steps aside while a login is held, unless the balance
        // asks for a sign-in; it stays before a login and while unread.
        XCTAssertFalse(CredentialSection.shows(action: .forget, balanceAction: .none, in: .signInShortcut))
        XCTAssertFalse(CredentialSection.shows(action: .migrate, balanceAction: .none, in: .signInShortcut))
        XCTAssertTrue(CredentialSection.shows(action: .forget, balanceAction: .obtain, in: .signInShortcut))
        XCTAssertTrue(CredentialSection.shows(action: .obtain, balanceAction: .none, in: .signInShortcut))
        XCTAssertTrue(CredentialSection.shows(action: .none, balanceAction: .none, in: .signInShortcut))
        XCTAssertTrue(CredentialSection.shows(action: .forget, balanceAction: .none, in: .account))
    }

    /// Payouts holds the payout link and the payouts-need-a-login line, and
    /// is present only while one of them is (D2).
    func test_payoutsIsPresentOnlyWhileSomethingInItApplies() throws {
        XCTAssertFalse(PayoutsSection.present(linkVisible: false, loginLine: false))
        XCTAssertTrue(PayoutsSection.present(linkVisible: true, loginLine: false))
        XCTAssertTrue(PayoutsSection.present(linkVisible: false, loginLine: true))
        let privateAI = try SettingsParityTests.text("Views/Settings/PrivateAISection.swift")
        XCTAssertFalse(privateAI.contains("PayoutLinkView("), "Private AI still draws the payout link")
        XCTAssertFalse(privateAI.contains("PayoutsNeedLoginLine()"), "Private AI still draws the payouts line")
        XCTAssertTrue(privateAI.contains("AccountLoginChangeButton(offersMismatchChange: true, carded: true)"),
                      "Private AI lost #1912's Use a different account (D10)")
    }

    /// Startup no longer hides Notifications and Updates behind its row, and
    /// Private AI draws the route disclosure with its pointer.
    func test_notificationsAndUpdatesAreTheirOwnSections() throws {
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        XCTAssertEqual(Self.view(drawnFor: .startup, in: content), "StartupSection()")
        XCTAssertEqual(Self.view(drawnFor: .notifications, in: content), "NotificationsSection()")
        XCTAssertEqual(Self.view(drawnFor: .updates, in: content), "UpdatesSection()")
        XCTAssertEqual(Self.view(drawnFor: .privateAI, in: content), "PrivateAISection()")
        XCTAssertEqual(Self.view(drawnFor: .insights, in: content), "InsightsSection()")
        XCTAssertEqual(Self.view(drawnFor: .tools, in: content), "ToolsSection()")

        let startup = try SettingsParityTests.text("Views/Settings/StartupSection.swift")
        let startupBody = try XCTUnwrap(startup.components(separatedBy: "struct NotificationsSection").first)
        XCTAssertFalse(startupBody.contains("NotificationsSection()"), "Startup draws Notifications too")
        XCTAssertFalse(startupBody.contains("UpdatesSection()"), "Startup draws Updates too")

        let privateAI = try SettingsParityTests.text("Views/Settings/PrivateAISection.swift")
        XCTAssertTrue(privateAI.contains("model.routeDisclosureState"), "Private AI no longer draws the route disclosure")
    }

    /// Each section's heading is the one the list names it by: the same
    /// source, read in the section's own file. Startup, Watching and Uses
    /// use their specific card titles; the section rule still names each
    /// by the list's word.
    func test_eachSectionIsHeadedByTheListsTitleSource() throws {
        let headings: [SettingsSection: (file: String, source: String)] = [
            .general: ("GeneralSection", "GlassEyebrowCard(SettingsWords.appearance)"),
            .account: ("AccountSection", "GlassEyebrowCard(Self.eyebrow, title: connectionTitle, accessory: {"),
            // Payouts' cards head themselves (the payout link's title).
            .payouts: ("PayoutsSection", "PayoutLinkView(onVisibility:"),
            .insights: ("InsightsSettingsCard", "GlassEyebrowCard(text(\"title\"))"),
            .startup: ("StartupSection",
                       "GlassEyebrowCard(SettingsLegacyWords.desktopEyebrow, title: SettingsLegacyWords.desktopTitle, accessory: {"),
            .notifications: ("StartupSection", "if let heading = Notifier.copy?.notificationHeading"),
            .updates: ("StartupSection", "GlassEyebrowCard(SettingsWords.updates)"),
            .watching: ("WatchingSection",
                        "GlassEyebrowCard(SettingsLegacyWords.discoveryEyebrow, title: SettingsLegacyWords.discoveryTitle, accessory: {"),
            .consent: ("ConsentSection",
                       "GlassEyebrowCard(SettingsLegacyWords.consentEyebrow, title: SettingsLegacyWords.consentHeading, accessory: {"),
            .publicProfile: ("PublicProfileSection", "GlassEyebrowCard(PublicProfileCopy.heading)"),
            .watchedFolders: ("WatchedFoldersSection", "GlassEyebrowCard(copy.heading)"),
            .tools: ("ToolsSection", "GlassEyebrowCard(copy.toolsHeading)"),
            // The section's own panels, headed by the standard-settings rule
            // (owner, 2026-10-10); the modal's rule names it by the list's word.
            .privateAI: ("PrivateAISection", "PrivateAISettingsPanels(store: store)"),
            .referrals: ("ReferralsSection", "GlassEyebrowCard(surface.copy.title)"),
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
                       "GlassEyebrowCard(SettingsLegacyWords.watcherEyebrow, title: SettingsLegacyWords.watcherTitle, accessory: {",
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
        // Every card that re-reads has the refresh icon button, named in
        // the core's word (owner, 2026-10-20: it was a "Refresh" link).
        for file in ["WatchingSection", "ConsentSection", "StartupSection"] {
            let text = try SettingsParityTests.text("Views/Settings/\(file).swift")
            XCTAssertTrue(text.contains("GlassRefreshButton(SettingsLegacyWords.refresh"), "\(file) has no refresh")
        }
    }

    /// The digest interval line at one hour, the shortest interval the
    /// daemon allows, is the core's singular, never "every 1 hours"
    /// (poldsam's #1298 review, finding 2). Other intervals keep the plural.
    func test_theDigestIntervalLineAtOneHourIsTheCoresSingular() throws {
        let words = try XCTUnwrap(ShellWords.table?.settings)
        XCTAssertFalse(words.atMostOneNotificationOne.isEmpty)
        XCTAssertEqual(SettingsLegacyWords.atMostOneNotification(1), words.atMostOneNotificationOne)
        XCTAssertFalse(SettingsLegacyWords.atMostOneNotification(1).contains("1 hours"))
        XCTAssertEqual(SettingsLegacyWords.atMostOneNotification(4),
                       ShellWords.fill(words.atMostOneNotification, ["hours": "4"]))
        XCTAssertTrue(SettingsLegacyWords.atMostOneNotification(4).contains("4 hours"))
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
