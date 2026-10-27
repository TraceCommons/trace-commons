import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Train coding models, the use that earns credit, is listed first on Uses
/// with "Required to earn credit." under it, and Traces notes, while it is
/// off, that the traces won't earn credit, with a link back to Uses (owner,
/// 2026-10-27).
@MainActor
final class FirstRunCreditNoteTests: XCTestCase {
    private func words() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private let options = [
        ConsentScope(name: "debugging_evaluation", title: "d", description: "d", alwaysOn: true, grantsDataUse: true),
        ConsentScope(name: "benchmark_only", title: "b", description: "b", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "ranking_training", title: "r", description: "r", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "model_training", title: "m", description: "m", alwaysOn: false, grantsDataUse: true),
        ConsentScope(name: "public_attribution", title: "p", description: "p", alwaysOn: false, grantsDataUse: false),
    ]

    private static func source(_ file: String) throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/\(file)")
        return try String(contentsOf: url, encoding: .utf8)
    }

    func testTrainCodingModelsIsListedFirstAndSaysItEarnsCredit() throws {
        let uses = try words().uses
        XCTAssertEqual(uses.creditScope, "model_training")
        XCTAssertEqual(uses.creditRequired, "Required to earn credit.")
        XCTAssertEqual(
            UsesScreenLayout.shownOptionalScopes(options, uses: uses).map(\.name),
            ["model_training", "benchmark_only", "ranking_training"])
        let source = try Self.source("UsesScreen.swift")
        XCTAssertTrue(source.contains("ForEach(UsesScreenLayout.shownOptionalScopes(options, uses: copy.uses))"))
        XCTAssertTrue(source.contains("if option.name == copy.uses.creditScope {"))
        XCTAssertTrue(source.contains("Text(copy.uses.creditRequired)"))
    }

    func testTracesNotesNoCreditOnlyWhileTrainCodingModelsIsOff() throws {
        let copy = try words()
        XCTAssertEqual(
            copy.traces.noCredit,
            "You didn't allow Train coding models, so your traces won't earn credit. To earn credit, go back to Uses and allow it.")
        XCTAssertEqual(copy.traces.backToUses, "Go back to Uses")
        var state = FirstRunState(step: .traces, scopes: ["debugging_evaluation", "benchmark_only"])
        XCTAssertTrue(TracesScreenLayout.showsNoCredit(state, uses: copy.uses))
        state.scopes.insert("model_training")
        XCTAssertFalse(TracesScreenLayout.showsNoCredit(state, uses: copy.uses))

        let source = try Self.source("TracesScreen.swift")
        XCTAssertTrue(source.contains("if TracesScreenLayout.showsNoCredit(runner.state, uses: copy.uses) {"))
        XCTAssertTrue(source.contains(".buttonStyle(GlassButtonStyle(.link))"))
        XCTAssertFalse(source.contains("exclamationmark"), "a note, not a warning")
    }

    func testTheLinkReturnsToUsesWithEveryChoiceKept() {
        var state = FirstRunState(step: .traces, scopes: ["debugging_evaluation", "ranking_training"])
        state = UsesScreenLayout.choosingSharing(state, .automatic)
        state.privateAI = true
        let back = TracesScreenLayout.backToUses(state)
        XCTAssertEqual(back.step, .uses)
        var same = back
        same.step = .traces
        XCTAssertEqual(same, state, "nothing but the step changes")
    }
}
