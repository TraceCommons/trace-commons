import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Alejo's launch feedback on first run (2026-11-05): say the benefit
/// before asking for a tool, name the tools it works with, and make the
/// sharing choice read what it does. The words are DRAFT, NEEDS APPROVAL.
@MainActor
final class AlejoFeedbackTests: XCTestCase {
    private func words() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private func source(_ path: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(path), encoding: .utf8)
    }

    func test_welcomeAndToolsSayTheBenefitFirst() throws {
        let copy = try words()
        XCTAssertTrue(copy.welcome.body.contains("privacy VPN for your AI"))
        XCTAssertTrue(copy.welcome.body.contains("earn credit"))
        for tool in ["Claude Code", "Codex", "OpenCode", "terminal"] {
            XCTAssertTrue(copy.tools.subtitle.contains(tool), tool)
        }
        let tools = try source("Views/FirstRun/ToolsScreen.swift")
        XCTAssertTrue(tools.contains("Text(copy.tools.subtitle)"))
    }

    func test_usesButtonSaysWhetherTracesAreShared() throws {
        let uses = try words().uses
        XCTAssertEqual(uses.dontShareNow, "Don't share traces now")
        XCTAssertEqual(uses.shareTraces, "Share traces")
        let screen = try source("Views/FirstRun/UsesScreen.swift")
        XCTAssertTrue(screen.contains("? copy.uses.shareTraces : copy.uses.dontShareNow,"))
        XCTAssertTrue(screen.contains("subdued: !UsesScreenLayout.hasSelection(runner.state, options: options),"))
        XCTAssertFalse(screen.contains("copy.join.skip"))
        // Dark for not sharing, the primary for sharing.
        let frame = try source("Views/FirstRun/FirstRunFrame.swift")
        XCTAssertTrue(frame.contains("GlassButtonStyle(footer.subdued ? .secondary : .primary, size: .stage)"))
    }
}
