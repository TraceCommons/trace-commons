import TCBridge
import TCDesign
import XCTest

@testable import TraceCommonsApp

/// Review 1 item 1 of #1229: the Connection card's dot for a session source.
/// An unset Claude Code or Codex source is read from its usual place (the
/// core's `unset_scans_conventional`), so it must never wear the "off" dot a
/// declined tool wears. Only a declined source is `.off`, and an answer the
/// core's copy cannot back is never drawn as on or off.
final class ConnectionSourceStatusTests: XCTestCase {
    private func coreCopy() throws -> SourceSettingsCopy {
        try XCTUnwrap(TCSourceChecks.settingsCopy(), "the core's source copy did not decode")
    }

    func test_unsetClaudeAndCodexAreNotDrawnOff() throws {
        let copy = try coreCopy()
        for tool in [TCSourceChecks.claude, TCSourceChecks.codex] {
            let status = ConnectionSection.sourceStatus(tool: tool, mode: "unset", copy: copy)
            XCTAssertNotEqual(status, GlassStatus.off, "unset \(tool) is read from the usual place, not off")
            XCTAssertEqual(status, GlassStatus.on, "unset \(tool) is a tool in use")
        }
    }

    func test_declinedSourcesAreOff() throws {
        let copy = try coreCopy()
        for tool in [TCSourceChecks.claude, TCSourceChecks.codex, TCSourceChecks.gemini, TCSourceChecks.cline] {
            XCTAssertEqual(ConnectionSection.sourceStatus(tool: tool, mode: "off", copy: copy), GlassStatus.off)
        }
    }

    func test_watchedSourcesAreOn() throws {
        let copy = try coreCopy()
        for tool in [TCSourceChecks.claude, TCSourceChecks.codex, TCSourceChecks.gemini, TCSourceChecks.cline] {
            XCTAssertEqual(ConnectionSection.sourceStatus(tool: tool, mode: "watch", copy: copy), GlassStatus.on)
        }
    }

    /// Gemini CLI and Cline open nothing while unset: not in use, but not a
    /// decision either, so neither on nor off.
    func test_unsetNonScanningSourcesAreNeitherOnNorOff() throws {
        let copy = try coreCopy()
        for tool in [TCSourceChecks.gemini, TCSourceChecks.cline] {
            let status = ConnectionSection.sourceStatus(tool: tool, mode: "unset", copy: copy)
            XCTAssertNotEqual(status, GlassStatus.on, "unset \(tool) opens nothing")
            XCTAssertNotEqual(status, GlassStatus.off, "unset \(tool) was never declined")
        }
    }

    /// Fail closed: with no core copy, or a mode the shell does not know,
    /// nothing says whether the source is read, so it is neither on nor off.
    func test_unbackedAnswersAreNeitherOnNorOff() throws {
        let copy = try coreCopy()
        let cases: [(String, String, SourceSettingsCopy?)] = [
            (TCSourceChecks.claude, "unset", nil),
            (TCSourceChecks.codex, "unset", nil),
            (TCSourceChecks.claude, "something-new", copy),
            (TCSourceChecks.codex, "", copy),
        ]
        for (tool, mode, copy) in cases {
            let status = ConnectionSection.sourceStatus(tool: tool, mode: mode, copy: copy)
            XCTAssertNotEqual(status, GlassStatus.on, "\(tool)/\(mode)/copy:\(copy != nil)")
            XCTAssertNotEqual(status, GlassStatus.off, "\(tool)/\(mode)/copy:\(copy != nil)")
        }
    }
}
