import Foundation
import XCTest
@testable import TraceCommonsApp

/// Analyze's file panel opens where the harnesses keep their traces, with
/// hidden folders shown, since both live under hidden folders in the home
/// folder that a default panel hides.
final class TraceFilePanelTests: XCTestCase {
    private let home = URL(fileURLWithPath: "/Users/someone", isDirectory: true)
    private var claude: URL { home.appendingPathComponent(".claude/projects", isDirectory: true) }
    private var codex: URL { home.appendingPathComponent(".codex/sessions", isDirectory: true) }

    private func start(_ source: String, existing: [URL]) -> URL {
        let present = Set(existing.map(\.standardizedFileURL.path))
        return TraceFileStart.directory(source: source, home: home) { present.contains($0.standardizedFileURL.path) }
    }

    func testClaudeCodesFolderComesFirstThenCodexsThenHome() {
        XCTAssertEqual(start("claude_code", existing: [claude, codex]), claude)
        XCTAssertEqual(start("claude_code", existing: [codex]), codex)
        XCTAssertEqual(start("claude_code", existing: []), home)
        XCTAssertEqual(start("trajectory", existing: [claude, codex]), claude)
        XCTAssertEqual(start("trajectory", existing: [codex]), codex)
        XCTAssertEqual(start("trajectory", existing: []), home)
    }

    func testCodexChosenOpensCodexsFolderWhenItExists() {
        XCTAssertEqual(start("codex", existing: [claude, codex]), codex)
        XCTAssertEqual(start("codex", existing: [claude]), claude)
        XCTAssertEqual(start("codex", existing: []), home)
    }

    @MainActor
    func testAnalyzeOpensThePanelAtThatFolderWithHiddenFilesShown() throws {
        let panel = try MonitorNavigationTests.text("Views/FolderPanel.swift")
        XCTAssertTrue(panel.contains("panel.showsHiddenFiles = true"))
        XCTAssertTrue(panel.contains("panel.directoryURL = TraceFileStart.directory(source: source"))
        // Still one panel in the app, made in one place.
        XCTAssertEqual(panel.components(separatedBy: "NSOpenPanel()").count - 1, 1)
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("FolderPanel.chooseTraceFile(source: source)"))
        XCTAssertFalse(view.contains(".fileImporter(isPresented: $choosingFile"))
    }
}
