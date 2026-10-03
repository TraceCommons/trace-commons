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
    /// so the windows cannot drift on what they tell.
    func test_theCardsAreDefinedOnceAndNotInTheLegacyFile() throws {
        let notices = try MonitorNavigationTests.text("Views/ShellNotices.swift")
        let main = try MonitorNavigationTests.text("Views/MainWindowView.swift")
        for name in ["ShellNotices", "AttachedDaemonNotice", "GrantVoidNotices", "GrantVoidNoticeCard",
                     "ArmingRewordingNotices", "ArmingRewordedNoticeCard", "GateHeldNoticeCard", "LegacyMigrationNoticeCard"] {
            XCTAssertEqual(notices.components(separatedBy: "struct \(name): View").count - 1, 1, "\(name) is not defined once")
            XCTAssertFalse(main.contains("struct \(name): View"), "\(name) is still defined in MainWindowView.swift")
        }
        XCTAssertEqual(notices.components(separatedBy: "GrantVoidNotices(").count - 1, 1)
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
