import TCBridge
import XCTest

/// The invite's host, read through the real ABI. Only the host crosses;
/// the invite's code never does, and every rejection is the same nil.
final class InviteHostBridgeTests: XCTestCase {
    /// The fixture `tc_invite_issuer_host_returns_the_host_and_nothing_else`
    /// in the ffi crate's `tests/abi.rs` uses.
    private let validInvite = "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6"

    func test_aValidInviteNamesItsHost() throws {
        let host = try XCTUnwrap(TCInvite.issuerHost(validInvite))
        XCTAssertEqual(host, "issuer.tracecommons.ai")
        XCTAssertFalse(host.contains("VQWWPGYSG8Y4LTP6"))
    }

    func test_aMalformedInviteHasNoHost() {
        for bad in [
            "VQWWPGYSG8Y4LTP6",
            "https://issuer.tracecommons.ai/onboard",
            "not a url",
            "",
            // The daemon's `invite_lookup` refuses these, so Join never
            // shows a host for them.
            "http://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6",
            "https://someone@issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6",
        ] {
            XCTAssertNil(TCInvite.issuerHost(bad), bad)
        }
    }

    /// A port is accepted by the daemon too, and the host is shown without it.
    func test_aPortIsAccepted() {
        XCTAssertEqual(
            TCInvite.issuerHost("https://issuer.tracecommons.ai:8443/onboard#VQWWPGYSG8Y4LTP6"),
            "issuer.tracecommons.ai")
    }
}
