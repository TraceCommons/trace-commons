import XCTest
@testable import TraceCommonsApp

final class ConsentScopesSectionTests: XCTestCase {
    private func scope(_ name: String, alwaysOn: Bool = false, grants: Bool = true) -> ConsentScope {
        ConsentScope(name: name, title: name, description: "", alwaysOn: alwaysOn, grantsDataUse: grants)
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

    /// Fail closed: with no daemon answer every row reads off and is disabled,
    /// the always-on row included (it keeps its tag, unticked).
    func test_unavailableMeansEveryRowOffAndDisabled() {
        let always = scope("debugging_evaluation", alwaysOn: true)
        let optional = scope("benchmark_only")
        for s in [always, optional] {
            XCTAssertFalse(ConsentScopeRows.isOn(scope: s, granted: ["debugging_evaluation", "benchmark_only"], unavailable: true))
            XCTAssertFalse(ConsentScopeRows.isEnabled(scope: s, busy: false, unavailable: true))
        }
    }

    func test_availableLocksAlwaysOnAndFollowsTheDaemonForTheRest() {
        let always = scope("debugging_evaluation", alwaysOn: true)
        let optional = scope("benchmark_only")
        XCTAssertTrue(ConsentScopeRows.isOn(scope: always, granted: [], unavailable: false))
        XCTAssertFalse(ConsentScopeRows.isEnabled(scope: always, busy: false, unavailable: false))
        XCTAssertTrue(ConsentScopeRows.isOn(scope: optional, granted: ["benchmark_only"], unavailable: false))
        XCTAssertFalse(ConsentScopeRows.isOn(scope: optional, granted: [], unavailable: false))
        XCTAssertTrue(ConsentScopeRows.isEnabled(scope: optional, busy: false, unavailable: false))
        XCTAssertFalse(ConsentScopeRows.isEnabled(scope: optional, busy: true, unavailable: false))
    }
}
