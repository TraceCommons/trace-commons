import TCBridge
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
        let refused = try status(#","login_mismatch":true"#).loginChange
        XCTAssertTrue(refused.mismatch)
        XCTAssertFalse(refused.offersChange)
    }

    /// `account_signed_in` (the account section after Sign out): read as
    /// given, and nil when absent (an account that never signs in, or an
    /// older daemon) or of the wrong type, never a guess.
    func testTheAccountSessionKeyIsReadOnlyAsGiven() throws {
        XCTAssertNil(try status("").accountSignedIn)
        XCTAssertEqual(try status(#","account_signed_in":true"#).accountSignedIn, true)
        XCTAssertEqual(try status(#","account_signed_in":false"#).accountSignedIn, false)
        XCTAssertNil(try status(#","account_signed_in":"no""#).accountSignedIn)
        XCTAssertEqual(try status(#","account_login":"github""#).accountLogin, "github")
    }

    /// A sign-in refused as another account says so in the approved words,
    /// not the generic failure line.
    func testARefusedSignInSaysWhyNotJustFailed() throws {
        let copy = try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
        XCTAssertEqual(
            NearAiSignInOffer.progressLine(copy, signingIn: false, failed: true, mismatch: "M"), "M")
        XCTAssertEqual(
            NearAiSignInOffer.progressLine(copy, signingIn: false, failed: true), copy.credentialFailed)
        XCTAssertEqual(
            NearAiSignInOffer.progressLine(copy, signingIn: true, failed: true, mismatch: "M"),
            copy.credentialObtaining)
    }
}
