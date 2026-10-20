import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A batch card for traces from several tools names none of them (#1490
/// review): no first tool's logo, and a contribute note that does not say
/// "What Claude Code read from its memory".
@MainActor
final class TraceTransferMixedToolsTests: XCTestCase {
    func test_aMixedBatchDrawsNoToolsLogo() throws {
        let one = MapTransfer(id: "1", folder: "f", tool: "codex", action: .contribute)
        let other = MapTransfer(id: "2", folder: "g", tool: "claude-code", action: .contribute)
        let mixed = try XCTUnwrap(MapTransfer.batch([other, one]))
        XCTAssertTrue(mixed.mixedTools)
        XCTAssertNil(TraceTransferCard.toolMark(mixed), "a mixed batch draws the neutral mark")
        XCTAssertEqual(TraceTransferCard.toolMark(one), SourceKind.codex.glassTool)
        let same = try XCTUnwrap(MapTransfer.batch([one, MapTransfer(id: "3", folder: "f", tool: "codex", action: .contribute)]))
        XCTAssertEqual(TraceTransferCard.toolMark(same), SourceKind.codex.glassTool)
    }

    func test_aMixedBatchsNoteNamesNoTool() throws {
        let words = try XCTUnwrap(FlowMapScene.words)
        let note = TraceTransferCard.state(.contribute, tool: nil, words: words).note
        XCTAssertEqual(note, words.transferContributeNoteMixed)
        XCTAssertFalse(note.isEmpty)
        XCTAssertFalse(note.contains("{"))
        for tool in SourceKind.allCases { XCTAssertFalse(note.contains(tool.displayName), tool.displayName) }
        XCTAssertTrue(TraceTransferCard.state(.contribute, tool: "Codex", words: words).note.contains("Codex"))
        XCTAssertEqual(TraceTransferCard.state(.keep, tool: nil, words: words).note, words.transferKeepNote)
        let source = try GlassSurfaceRulesTests.text("Views/Monitor/TraceTransferCard.swift")
        XCTAssertTrue(source.contains("let state = Self.state(transfer.action, tool: transfer.mixedTools ? nil : name, words: words)"))
    }
}
