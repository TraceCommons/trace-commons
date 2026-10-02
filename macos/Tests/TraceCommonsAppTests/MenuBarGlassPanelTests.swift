import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R13 of #1173: the glass menu-bar panel's status row follows the menu
/// bar's own state, and the panel's content is the shipping menu's.
final class MenuBarGlassPanelTests: XCTestCase {
    /// The dot follows `MenuBarStatus`'s precedence: attention, then
    /// decisions owed, then paused, then idle.
    func test_theDotFollowsTheMenuBarState() {
        XCTAssertEqual(MenuBarPanelStatus.dot(.attention), .outside)
        XCTAssertEqual(MenuBarPanelStatus.dot(.count("3")), .ask)
        XCTAssertEqual(MenuBarPanelStatus.dot(.paused), .off)
        XCTAssertEqual(MenuBarPanelStatus.dot(.idle), .on)
    }

    /// The badge counts decisions owed, and draws nothing at zero or when
    /// the core did not say.
    func test_theBadgeCountsDecisionsOwedOnly() {
        XCTAssertEqual(MenuBarPanelStatus.badge(3), 3)
        XCTAssertNil(MenuBarPanelStatus.badge(0))
        XCTAssertNil(MenuBarPanelStatus.badge(nil))
    }

    /// The panel adds a frame, a status row and a badge; every row and
    /// action is the shipping menu's own `MenuBarContent`, so the panel
    /// authors no menu item of its own.
    func test_thePanelReusesTheMenusContent() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("MenuBarContent(navigation: navigation)"))
        XCTAssertFalse(source.contains("Button("), "the panel adds an action the menu does not have")
    }
}
