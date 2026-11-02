import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// #1836 F1: the account section's status, worded across the ABI once per
/// change of the daemon's keys, and absent for a healthy account or an
/// older daemon.
final class AccountAttentionModelTests: XCTestCase {
    private func status(_ keys: String) throws -> DaemonStatus {
        let json = """
            {"schema_version":"v","logged_in":true,"paused":false,"queue_depth":0,
             "health":{"last_error_label":null,"since":null}\(keys)}
            """
        return try JSONDecoder().decode(DaemonStatus.self, from: Data(json.utf8))
    }

    private let unreadable =
        #","account_state":"needs_attention","account_state_reason":"account_record_unreadable","near_ai_login_unconfirmed":true"#

    func testAnOldStatusPayloadDecodesWithNothingToShow() throws {
        let old = try status("")
        XCTAssertEqual(old.accountAttention, .none)
        XCTAssertTrue(old.loggedIn)
        // A wrong type is read as absent, not as a failed status.
        let odd = try status(#","account_state":7,"near_ai_login_unconfirmed":"yes""#)
        XCTAssertNil(odd.accountAttention.state)
        XCTAssertFalse(odd.accountAttention.nearAILoginUnconfirmed)
    }

    func testTheKeysDecode() throws {
        let wire = try status(unreadable).accountAttention
        XCTAssertEqual(wire.state, "needs_attention")
        XCTAssertEqual(wire.reason, "account_record_unreadable")
        XCTAssertTrue(wire.nearAILoginUnconfirmed)
    }

    @MainActor
    func testTheStatusIsWordedOncePerChangeAndClears() throws {
        var calls = 0
        let model = AppModel()
        // Answers as the ABI does: words only for needs_attention.
        model.accountAttentionWording = { wire in
            calls += 1
            guard wire.contains("needs_attention") else { return nil }
            return #"{"title":"T","body":"B","private_ai":"P","action":"A","confirm_title":"CT","confirm_body":"CB","confirm_action":"CA","confirm_cancel":"CC","failed":"F"}"#
        }
        model.setStatusForTesting(try status(unreadable))
        XCTAssertEqual(model.accountAttention?.title, "T")
        model.setStatusForTesting(try status(unreadable))
        XCTAssertEqual(calls, 1)
        model.setStatusForTesting(
            try status(#","account_state":"ok","account_state_reason":null,"near_ai_login_unconfirmed":false"#))
        XCTAssertNil(model.accountAttention)
        XCTAssertEqual(calls, 2)
    }

    /// Through the real ABI: the reason's sentence, then the Private AI line
    /// only while a NEAR AI login is stored, and the button.
    @MainActor
    func testTheBlockShowsTheRustsLinesInOrder() throws {
        let model = AppModel()
        model.setStatusForTesting(try status(unreadable))
        let attention = try XCTUnwrap(model.accountAttention)
        let lines = AccountAttentionBlock.lines(attention)
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines.first, attention.body)
        XCTAssertEqual(lines.last, attention.privateAI)

        model.setStatusForTesting(try status(
            #","account_state":"needs_attention","account_state_reason":"account_address_refused","near_ai_login_unconfirmed":false"#))
        let refused = try XCTUnwrap(model.accountAttention)
        XCTAssertEqual(AccountAttentionBlock.lines(refused), [try XCTUnwrap(refused.body)])
        XCTAssertNotEqual(refused.body, attention.body)
    }

    /// The refusal line shows only while the last attempt failed.
    @MainActor
    func testTheFailureLineFollowsTheLastAttempt() throws {
        let model = AppModel()
        model.setStatusForTesting(try status(unreadable))
        let attention = try XCTUnwrap(model.accountAttention)
        XCTAssertFalse(AccountAttentionBlock.lines(attention).contains(attention.failed))
        XCTAssertEqual(AccountAttentionBlock.lines(attention, failed: true).last, attention.failed)
    }

    /// A refused unenroll is said, and the next attempt clears it; a second
    /// press while one runs does nothing.
    @MainActor
    func testSetUpAgainReportsARefusalAndRunsOnce() async throws {
        let model = AppModel()
        var runs = 0
        var answer = false
        model.setUpAgainUnenroll = {
            runs += 1
            await Task.yield()
            return answer
        }
        let first = await model.setUpAgain()
        XCTAssertFalse(first)
        XCTAssertTrue(model.setUpAgainFailed)
        XCTAssertFalse(model.settingUpAgain)

        answer = true
        async let a = model.setUpAgain()
        async let b = model.setUpAgain()
        let results = await [a, b]
        XCTAssertEqual(results.filter { $0 }.count, 1, "one run, one first-run window")
        XCTAssertEqual(runs, 2)
        XCTAssertFalse(model.setUpAgainFailed)
    }

    @MainActor
    func testAHealthyAccountShowsNoBlock() throws {
        let model = AppModel()
        model.setStatusForTesting(
            try status(#","account_state":"ok","account_state_reason":null,"near_ai_login_unconfirmed":false"#))
        XCTAssertNil(model.accountAttention)
    }
}
