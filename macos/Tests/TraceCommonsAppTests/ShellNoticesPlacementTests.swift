import XCTest

/// Every window that shows the contributor's traces carries the shared
/// notice stack (grant voids, arming rewordings, a gate hold, the attached
/// daemon and the legacy-invite migration). The monitor had none of them,
/// so a void during monitor use was never told. The first-run pane tells a
/// void too: one can arrive while someone is still setting up.
@MainActor
final class ShellNoticesPlacementTests: XCTestCase {
    func test_theMonitorAndTheFirstRunPaneBothShowTheNotices() throws {
        for file in ["Views/MonitorWindowView.swift", "Views/Monitor/FirstRunViews.swift"] {
            XCTAssertTrue(try MonitorNavigationTests.text(file).contains("ShellNotices()"), "\(file) does not draw the notices")
        }
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        for card in ["AttachedDaemonNotice()", "GrantVoidNotices(", "ArmingRewordingNotices(", "GateHeldNoticeCard(", "LegacyMigrationNoticeCard("] {
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
                     "ArmingRewordingNotices", "ArmingRewordedNoticeCard", "GateHeldNoticeCard", "LegacyMigrationNoticeCard"] {
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

    /// A card is a glass notice, its state carries words (the title is a
    /// status label), and no literal sentence is written here.
    func test_theCardsAreGlassNoticesWithNoSentenceOfTheirOwn() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        XCTAssertGreaterThanOrEqual(notices.components(separatedBy: "GlassNotice(").count - 1, 5)
        XCTAssertNil(notices.range(of: #"Text\(\s*""#, options: .regularExpression), "a literal Text(\"...\")")
        XCTAssertNil(notices.range(of: #"Button\(\s*""#, options: .regularExpression), "a literal Button(\"...\")")
    }
}
