import Darwin
import XCTest
@testable import TraceCommonsApp

final class TerminationSignalTests: XCTestCase {
    /// SIGTERM reaches the handler instead of ending the process. The signal
    /// goes to this test process, which survives only if the handler is in
    /// place.
    @MainActor
    func testSigtermRunsTheHandlerAndDoesNotEndTheProcess() async {
        let signal = TerminationSignal()
        let delivered = expectation(description: "SIGTERM delivered to the handler")
        signal.install { delivered.fulfill() }
        XCTAssertEqual(kill(getpid(), SIGTERM), 0)
        await fulfillment(of: [delivered], timeout: 5)
    }

    /// The disposition is a caught handler, never `SIG_IGN`: an ignored
    /// SIGTERM would be inherited across exec by every child the app starts.
    @MainActor
    func testTheDispositionIsCaughtNotIgnored() {
        let signal = TerminationSignal()
        signal.install {}
        var current = sigaction()
        XCTAssertEqual(sigaction(SIGTERM, nil, &current), 0)
        let handler = unsafeBitCast(current.__sigaction_u.__sa_handler, to: Int.self)
        XCTAssertNotEqual(handler, 0, "SIG_DFL would end the process")
        XCTAssertNotEqual(handler, 1, "SIG_IGN would be inherited by children")
    }
}
