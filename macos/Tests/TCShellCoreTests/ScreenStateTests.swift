import XCTest

@testable import TCShellCore

/// The stack-wide precedence rule: core down > loading > paused > unknown >
/// ready, and a missing signal is never healthy.
final class ScreenStateTests: XCTestCase {
    func test_precedenceIsCoreDownThenLoadingThenPausedThenUnknown() {
        XCTAssertEqual(ScreenState.resolve(failure: .unreachable, loaded: true, paused: true, known: true), .coreDown)
        XCTAssertEqual(ScreenState.resolve(failure: .unreachable, loaded: false, paused: nil, known: false), .coreDown)
        XCTAssertEqual(ScreenState.resolve(failure: nil, loaded: false, paused: true, known: true), .loading)
        XCTAssertEqual(ScreenState.resolve(failure: nil, loaded: true, paused: true, known: false), .paused)
        XCTAssertEqual(ScreenState.resolve(failure: nil, loaded: true, paused: false, known: false), .unknown)
        XCTAssertEqual(ScreenState.resolve(failure: nil, loaded: true, paused: false, known: true), .ready)
    }

    /// No combination with a missing signal reads as healthy.
    func test_aMissingSignalIsNeverHealthy() {
        for failure in [nil, DaemonDataError.unreachable, .undecodable(method: "x")] {
            for loaded in [true, false] {
                for paused in [nil, true, false] as [Bool?] {
                    for known in [true, false] {
                        let state = ScreenState.resolve(failure: failure, loaded: loaded, paused: paused, known: known)
                        let missing = failure == .unreachable || !loaded || paused == nil || !known
                        if missing { XCTAssertFalse(state.isHealthy, "\(String(describing: failure)) \(loaded) \(String(describing: paused)) \(known)") }
                    }
                }
            }
        }
    }

    /// A refused request is not a core that is down: the screen keeps its
    /// state and says the request failed.
    func test_aRefusedRequestIsNotCoreDown() {
        XCTAssertNotEqual(
            ScreenState.resolve(failure: .daemon(code: "c", message: "m"), loaded: true, paused: false, known: true),
            .coreDown)
    }
}
