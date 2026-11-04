import XCTest

/// Every window that shows the contributor's traces carries the shared
/// notices (grant voids, arming rewordings, a gate hold, the attached
/// daemon and the legacy-invite migration). The monitor had none of them,
/// so a void during monitor use was never told. The first-run pane tells a
/// void too: one can arrive while someone is still setting up. Since
/// 2026-10-19 the Monitor draws them as popover notices over its tabs
/// (`MonitorNotices`); the first-run pane keeps the stack.
@MainActor
final class ShellNoticesPlacementTests: XCTestCase {
    func test_theMonitorAndTheFirstRunPaneBothShowTheNotices() throws {
        XCTAssertTrue(try MonitorNavigationTests.text("Views/Monitor/FirstRunViews.swift").contains("ShellNotices()"),
                      "the first-run pane does not draw the notices")
        XCTAssertTrue(try MonitorNavigationTests.text("Views/MonitorWindowView.swift").contains("MonitorNotices("),
                      "the Monitor does not draw the notices")
        let monitor = try MonitorNavigationTests.text("Views/Monitor/MonitorNotices.swift")
        for notice in ["model.isAttachedDaemon", "model.status.grantVoids", "model.status.armingRewordings",
                       "model.gateHeldNotice", "model.legacyMigrationNotice", "model.showsLegacyMigrationOfferCard",
                       "LegacyMigrationMoveControls(offer: offer, placement: .notice)"] {
            XCTAssertTrue(monitor.contains(notice), "the Monitor's notices lack \(notice)")
        }
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        for card in ["AttachedDaemonNotice()", "GrantVoidNotices(", "ArmingRewordingNotices(", "GateHeldNoticeCard(", "LegacyMigrationNoticeCard(", "LegacyMigrationOfferCard("] {
            XCTAssertTrue(notices.contains(card), "ShellNotices lacks \(card)")
        }
        XCTAssertNil(notices.range(of: #"\bTC\."#, options: .regularExpression))
        XCTAssertFalse(notices.contains("struct HealthBanner"), "the glass health banners live in TracesHealth.swift")
    }

    /// One definition of the stack and of each card, in the new file only,
    /// so the windows cannot drift on what they tell. (The legacy window
    /// that once defined them is gone: `LegacyShellRetiredTests`.)
    func test_theCardsAreDefinedOnce() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        for name in ["ShellNotices", "AttachedDaemonNotice", "GrantVoidNotices", "GrantVoidNoticeCard",
                     "ArmingRewordingNotices", "ArmingRewordedNoticeCard", "GateHeldNoticeCard", "LegacyMigrationNoticeCard",
                     "LegacyMigrationOfferCard", "LegacyMigrationMoveControls"] {
            XCTAssertEqual(notices.components(separatedBy: "struct \(name): View").count - 1, 1, "\(name) is not defined once")
        }
        XCTAssertEqual(notices.components(separatedBy: "GrantVoidNotices(").count - 1, 1)
    }

    /// Each card's buttons call that card's own action: Re-arm re-arms,
    /// Ask first asks first, and acknowledging only acknowledges. Checked
    /// per card (from its `struct` to the next declaration), by the whole
    /// button call through its closing parenthesis or brace, and by the
    /// number of buttons the card draws, so a swapped action, a dropped
    /// button or an extra one fails here.
    func test_eachCardsButtonsCallItsOwnActions() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        let acknowledge = "Button(notice.acknowledge, action: onAcknowledge)"
        let cards: [(name: String, buttons: [String])] = [
            ("AttachedDaemonNotice", []),
            ("LegacyMigrationNoticeCard", [acknowledge]),
            ("LegacyMigrationOfferCard", []),
            ("LegacyMigrationMoveControls", [
                "Button(offer.action) { Task { await model.migrateLegacyInvite(invite: invite) } }",
                "Button(offer.dismiss, action: onDismiss)",
                "Button(offer.nearAILoginCancel) { model.cancelLegacyMigrationNearAILogin() }",
            ]),
            ("GrantVoidNoticeCard", ["Button(action, action: onRearm)", acknowledge]),
            ("ArmingRewordedNoticeCard", ["Button(action, action: onAskFirst)", acknowledge]),
            ("GateHeldNoticeCard", ["Button(action) { onAskFirst(projectID) }"]),
        ]
        for card in cards {
            let start = try XCTUnwrap(notices.range(of: "struct \(card.name): View {"), "\(card.name) is missing")
            let rest = notices[start.upperBound...]
            let end = rest.range(of: #"\n(private |fileprivate )?struct "#, options: .regularExpression)?.lowerBound
                ?? rest.endIndex
            let body = String(rest[..<end])
            for button in card.buttons {
                XCTAssertEqual(body.components(separatedBy: button).count - 1, 1, "\(card.name) lacks \(button)")
            }
            // Both call forms: `Button(` and the trailing-closure `Button {`.
            let drawn = body.components(separatedBy: "Button(").count - 1
                + body.components(separatedBy: "Button {").count - 1
            XCTAssertEqual(drawn, card.buttons.count, "\(card.name) draws a button this test does not pin")
        }
    }

    /// The card's Move and the one in Settings look the same; each is named
    /// for its place, so a screen reader can tell them apart
    /// (`docs/ui-copy-guide.md` 5). The controls take the place explicitly
    /// rather than inferring it from the dismiss.
    func test_theTwoMoveButtonsAreNamedForTheirPlace() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        XCTAssertEqual(
            notices.components(separatedBy: ".accessibilityLabel(offer.actionName(for: placement))").count - 1, 1)
        XCTAssertTrue(notices.contains("LegacyMigrationMoveControls(offer: offer, placement: .notice, onDismiss: onDismiss)"))
        let settings = try MonitorNavigationTests.text("Views/Settings/ConnectionSection.swift")
        XCTAssertTrue(settings.contains("LegacyMigrationMoveControls(offer: offer, placement: .settings)"))
        XCTAssertTrue(settings.contains("model.isLegacyMigrationOfferDrawn"))
        // Move only while the daemon offers it; a held refusal draws without it.
        XCTAssertTrue(notices.contains("model.offersLegacyMigrationMove"))
    }

    /// A card is a glass notice, its state carries words (the title is a
    /// status label), and no literal sentence is written here.
    func test_theCardsAreGlassNoticesWithNoSentenceOfTheirOwn() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        XCTAssertGreaterThanOrEqual(notices.components(separatedBy: "GlassNotice(").count - 1, 6)
        XCTAssertNil(notices.range(of: #"Text\(\s*""#, options: .regularExpression), "a literal Text(\"...\")")
        XCTAssertNil(notices.range(of: #"Button\(\s*""#, options: .regularExpression), "a literal Button(\"...\")")
    }
}
