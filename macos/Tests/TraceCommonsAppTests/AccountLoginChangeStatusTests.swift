import XCTest

import TCShellCore
@testable import TraceCommonsApp

/// The change of login's status keys (F10, spec 2.4), as `DaemonStatus`
/// decodes them: each alone, a wrong type read as false, and an older
/// daemon offering nothing.
final class AccountLoginChangeStatusTests: XCTestCase {
    private func status(_ keys: String) throws -> DaemonStatus {
        let json = """
            {"schema_version":"v","logged_in":true,"paused":false,"queue_depth":0,
             "health":{"last_error_label":null,"since":null}\(keys)}
            """
        return try JSONDecoder().decode(DaemonStatus.self, from: Data(json.utf8))
    }

    func testAnOlderDaemonOffersNothing() throws {
        XCTAssertEqual(try status("").loginChange, .none)
        XCTAssertEqual(
            try status(#","login_change_available":"yes","payouts_need_login":1"#).loginChange,
            .none)
    }

    func testTheKeysDecodeEachAlone() throws {
        let wallet = try status(
            #","login_change_available":true,"payouts_need_login":true,"login_change_held":false"#
        ).loginChange
        XCTAssertEqual(
            wallet, AccountLoginChangeStatus(available: true, payoutsNeedLogin: true, held: false))
        XCTAssertTrue(wallet.showsPayoutsLine)
        let held = try status(#","login_change_available":true,"login_change_held":true"#).loginChange
        XCTAssertTrue(held.held)
        XCTAssertFalse(held.payoutsNeedLogin)
    }
}
