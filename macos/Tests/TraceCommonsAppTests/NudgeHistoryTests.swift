import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Answers `status` with verdict news and records every call; anything
/// else is the daemon's refusal, which the stores keep per method.
private final class VerdictNewsTransport: DaemonTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [String] = []
    var calls: [String] { lock.withLock { recorded } }
    var refuse = false

    static let status = #"""
        {"nudge":{"state":"armed","lead":"verdicts_landed","count":3,"accepted":2,"held":1,"final":1,
          "credit_final":2.5,"since":"2026-10-07T09:30:00Z","mark":"news","mark_kinds":["verdicts_landed"],
          "text":{"title":"Since 7 October: 2 accepted, 1 held for privacy review","body":"2.5 credit is now final.",
                  "actions":[{"id":"see_history","label":"See history"}],"panel_row":"2 accepted, 1 held since 7 October"},
          "mark_text":{"accessibility":"New: 2 accepted and 1 held for privacy review.","tooltip":"Verdicts are in."}}}
        """#

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.withLock { recorded.append("\(method) \(paramsJSON)") }
        switch method {
        case "status": return #"{"id":0,"result":\#(Self.status)}"#
        case "nudge_opened" where !refuse: return #"{"id":0,"result":{"opened":true}}"#
        default: return #"{"id":0,"error":{"code":"unavailable","message":"state-write-failed"}}"#
        }
    }
}

/// The verdict news on History: the daemon's card, and See history's
/// acknowledgement.
@MainActor
final class NudgeHistoryTests: XCTestCase {
    func test_theVerdictsCardIsTheDaemonsWords() async {
        let store = HomeStore(client: LiveDaemonClient(transport: VerdictNewsTransport()))
        await store.load()
        let card = store.verdictsCard
        XCTAssertEqual(card?.title, "Since 7 October: 2 accepted, 1 held for privacy review")
        XCTAssertEqual(card?.body, "2.5 credit is now final.")
        XCTAssertEqual(card?.actions, [NudgeSurface.Action(intent: .seeHistory, label: "See history")])
    }

    /// Idle and backlog suggestions are Traces', never History's.
    func test_historyDrawsNoTracesSuggestion() async {
        let store = HomeStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertNotNil(store.status?.nudge?.text)
        XCTAssertNil(store.verdictsCard)
    }

    func test_seeHistoryAcknowledgesTheNews() async {
        let transport = VerdictNewsTransport()
        let store = HomeStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        await store.perform(.seeHistory)
        XCTAssertTrue(transport.calls.contains(#"nudge_opened {"kind":"verdicts_landed"}"#), "\(transport.calls)")
        XCTAssertNil(store.nudgeError)
    }

    func test_aRefusedAcknowledgementIsKept() async {
        let transport = VerdictNewsTransport()
        transport.refuse = true
        let store = HomeStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        await store.perform(.seeHistory)
        XCTAssertEqual(store.nudgeError, .daemon(code: "unavailable", message: "state-write-failed"))
    }
}
