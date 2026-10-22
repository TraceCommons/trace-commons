import XCTest

@testable import TraceCommonsApp

/// Staging design 4.5: each channel answers only its own URL scheme, which
/// it takes from the dylib, so a staging app never picks up a production
/// invite link and a production app never picks up a staging one.
final class DeepLinkTests: XCTestCase {
    private let invite = "https%3A%2F%2Fissuer.example.invalid%2Fonboard"

    private func link(_ scheme: String) throws -> URL {
        try XCTUnwrap(URL(string: "\(scheme)://enroll?invite=\(invite)"))
    }

    func testTheChannelsOwnSchemeCarriesTheInvite() throws {
        XCTAssertEqual(
            DeepLink.inviteURL(from: try link("tracecommons"), scheme: "tracecommons"),
            "https://issuer.example.invalid/onboard")
        XCTAssertEqual(
            DeepLink.inviteURL(from: try link("tracecommons-staging"), scheme: "tracecommons-staging"),
            "https://issuer.example.invalid/onboard")
    }

    func testAStagingChannelRefusesAProductionLink() throws {
        XCTAssertNil(DeepLink.inviteURL(from: try link("tracecommons"), scheme: "tracecommons-staging"))
    }

    func testAProductionChannelRefusesAStagingLink() throws {
        XCTAssertNil(DeepLink.inviteURL(from: try link("tracecommons-staging"), scheme: "tracecommons"))
    }

    func testNoAgreedChannelRefusesEveryLink() throws {
        XCTAssertNil(DeepLink.inviteURL(from: try link("tracecommons"), scheme: nil))
        XCTAssertNil(DeepLink.inviteURL(from: try link("tracecommons"), scheme: ""))
    }

    func testAnEmptyInviteIsDropped() throws {
        let url = try XCTUnwrap(URL(string: "tracecommons://enroll?invite="))
        XCTAssertNil(DeepLink.inviteURL(from: url, scheme: "tracecommons"))
    }
}
