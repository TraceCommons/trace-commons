import XCTest
@testable import TraceCommonsApp

final class ConsentScopesSectionTests: XCTestCase {
    private func scope(_ name: String, alwaysOn: Bool = false, grants: Bool = true) -> ConsentScope {
        ConsentScope(name: name, description: "", alwaysOn: alwaysOn, grantsDataUse: grants)
    }

    /// The list sent is built from what the daemon reports, never from the
    /// ticks: two quick presses cannot race a scope neither touched.
    func test_theTickReadsTheDaemonNeverTheDraft() {
        let options = [scope("debugging_evaluation", alwaysOn: true), scope("benchmark_only"), scope("public_attribution", grants: false)]
        let next = ConsentScopeRows.nextScopes(reported: ["debugging_evaluation", "benchmark_only"],
                                               options: options, toggling: options[2], granted: true)
        XCTAssertEqual(next, ["debugging_evaluation", "benchmark_only", "public_attribution"])
        let off = ConsentScopeRows.nextScopes(reported: ["debugging_evaluation", "benchmark_only"],
                                              options: options, toggling: options[1], granted: false)
        XCTAssertEqual(off, ["debugging_evaluation"])
    }

    /// Always-on is always in the list, whatever the daemon reported.
    func test_alwaysOnIsNeverDropped() {
        let options = [scope("debugging_evaluation", alwaysOn: true), scope("benchmark_only")]
        let next = ConsentScopeRows.nextScopes(reported: [], options: options, toggling: options[1], granted: false)
        XCTAssertEqual(next, ["debugging_evaluation"])
    }

    /// Fail closed: with no daemon answer a row is disabled and reads off.
    func test_unavailableMarkerIsInTheSection() throws {
        let source = try SettingsParityTests.text("Views/Settings/ConsentSection.swift")
        XCTAssertTrue(source.contains("!model.status.loggedIn"))
    }
}
