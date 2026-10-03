import TCShellCore
import XCTest

@testable import TraceCommonsApp

final class ToolsSectionTests: XCTestCase {
    /// Out of range is left as it was, never clamped: port 0 is the
    /// ask-the-kernel sentinel the daemon refuses.
    func test_anInvalidPortLeavesTheFormUnchanged() {
        var form = RoutingForm(on: true, port: 8787, tokenDir: "")
        form = RoutingPortInput.accept(0, into: form)
        XCTAssertEqual(form.port, 8787)
        form = RoutingPortInput.accept(70_000, into: form)
        XCTAssertEqual(form.port, 8787)
        form = RoutingPortInput.accept(-1, into: form)
        XCTAssertEqual(form.port, 8787)
        form = RoutingPortInput.accept(9000, into: form)
        XCTAssertEqual(form.port, 9000)
    }

    /// Only the four routing tones exist; none maps to a failure look.
    func test_routingTonesNeverReadAsFailed() {
        for tone in [RoutingTone.clear, .held, .attention, .neutral] {
            XCTAssertNotEqual(ToolsSection.tone(tone), .outside)
            XCTAssertNotEqual(ToolsSection.tone(tone), .failed)
            XCTAssertNotEqual(ToolsSection.status(tone), .outside)
        }
        XCTAssertEqual(ToolsSection.tone(.clear), .on)
        XCTAssertEqual(ToolsSection.tone(.attention), .ask)
        XCTAssertEqual(ToolsSection.status(.clear), .on)
        XCTAssertEqual(ToolsSection.status(.attention), .ask)
    }

    private func source() throws -> String {
        try SettingsParityTests.text("Views/Settings/ToolsSection.swift")
    }

    /// The unavailable branch exists, and the refresh hangs on a container
    /// that is present whether or not the core's copy arrived.
    func test_refreshIsOnAnAlwaysPresentContainer() throws {
        let body = try source()
        let container = try XCTUnwrap(body.range(of: "VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {"))
        let gate = try XCTUnwrap(body.range(of: "if let copy = model.routingCopy {"))
        let unavailable = try XCTUnwrap(body.range(of: "} else {"), "no unavailable branch")
        let refresh = try XCTUnwrap(body.range(of: ".onAppear {"))
        XCTAssertLessThan(container.lowerBound, gate.lowerBound)
        XCTAssertLessThan(gate.lowerBound, unavailable.lowerBound)
        XCTAssertLessThan(unavailable.lowerBound, refresh.lowerBound, "the refresh is chained after the container")
        XCTAssertEqual(body.components(separatedBy: ".onAppear {").count - 1, 1)
        let tail = String(body[refresh.lowerBound...])
        XCTAssertTrue(tail.contains("model.discoverRouting()"))
        XCTAssertTrue(tail.contains("model.refreshRoutedTools()"))
    }

    /// The card authors no sentence: no literal Text or Button title.
    func test_theCardAuthorsNoSentence() throws {
        let body = try source()
        for line in body.split(separator: "\n") where !line.trimmingCharacters(in: .whitespaces).hasPrefix("//") {
            XCTAssertFalse(line.contains("Text(\""), "literal Text in: \(line)")
            XCTAssertFalse(line.contains("Button(\""), "literal Button in: \(line)")
        }
    }

    /// The word is the state; the tag's colour never carries it alone.
    func test_toolRowsReadAsWords() throws {
        let body = try source()
        XCTAssertTrue(body.contains("GlassTag(row.word, tone: Self.tone(row.tone))"))
        XCTAssertTrue(body.contains(".accessibilityLabel(\"\\(row.name): \\(row.word)\")"))
    }
}
