import XCTest

@testable import TraceCommonsApp

/// How the glass Connection section (`ConnectionSection`) is wired to the session-source
/// rows underneath it.
///
/// `SourceCheckExportTests` proves the sentences; it cannot see which
/// property this view hands them. A row that passed `claudeRootConfigured`
/// -- `mode == "watch"`, and so false for `off` as well as for `unset` --
/// would pass that suite and still print one sentence for two different
/// facts, which is the defect this change removes.
///
/// A SwiftUI `body` holding an `@EnvironmentObject` cannot be built or
/// reflected outside a running window, so this asserts against the view's own
/// source, as `RoutingBindingTests` does. It fails loudly rather than
/// silently: if the section cannot be located the test says so instead of
/// passing over nothing.
final class SourceCheckBindingTests: XCTestCase {
    /// `.../macos/Sources/TraceCommonsApp/Views/Settings/ConnectionSection.swift`
    private static let viewPath = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TraceCommonsAppTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/ConnectionSection.swift")

    /// The section's `body`, from its signature to the row helper that
    /// follows it. Deliberately a slice and not the whole file: the row
    /// helper's own definition names the properties this test forbids, in a
    /// doc comment explaining why, and a whole-file scan would either match
    /// that or have to be weakened until it matched nothing.
    private func connectionSection(
        file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = try? String(contentsOf: Self.viewPath, encoding: .utf8) else {
            XCTFail("could not read \(Self.viewPath.path)", file: file, line: line)
            return nil
        }
        guard let start = text.range(of: "var body: some View {") else {
            XCTFail("the Connection section has moved or been renamed", file: file, line: line)
            return nil
        }
        guard let end = text.range(of: "private func sourceLine(", range: start.upperBound..<text.endIndex)
        else {
            XCTFail("the row helper after the Connection body has moved", file: file, line: line)
            return nil
        }
        return String(text[start.upperBound..<end.lowerBound])
    }

    /// The rows are asked for by MODE, and this view writes no sentence about
    /// anybody's session folder.
    func testTheSessionSourceRowsAreDrivenByTheModeAndNotTheBoolean() {
        guard let section = connectionSection() else { return }
        XCTAssertTrue(
            section.contains(
                "sourceLine(TCSourceChecks.claude, settings.routingSourceModes.claude)"),
            "the Claude row is not bound to its source mode: \(section)")
        XCTAssertTrue(
            section.contains(
                "sourceLine(TCSourceChecks.codex, settings.routingSourceModes.codex)"),
            "the Codex row is not bound to its source mode: \(section)")
        for tool in ["gemini", "cline"] {
            XCTAssertTrue(section.contains(
                "sourceLine(TCSourceChecks.\(tool), settings.routingSourceModes.\(tool))"))
        }
        // #1146 lists OpenCode as well, by its mode, unset when unread.
        XCTAssertTrue(section.contains(
            "sourceLine(TCSourceChecks.opencode, settings.opencodeSourceMode ?? \"unset\")"))
        for forbidden in [
            "claudeRootConfigured", "codexRootConfigured", "sessions folder set", "usual place",
        ] {
            XCTAssertFalse(
                section.contains(forbidden),
                "the Connection section still names \(forbidden): \(section)")
        }
    }
}
