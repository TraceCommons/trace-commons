import XCTest
@testable import TraceCommonsApp

/// Joining never authorizes sharing (concept rule 2, and the coordinator's
/// "Call ordering"): `enroll` carries no scopes, and a link only fills the
/// field.
final class JoinNeverSharesTests: XCTestCase {
    func test_enrollCarriesNoScopes() throws {
        let source = try OnboardingParityTests.text("Views/OnboardingConnectView.swift")
        XCTAssertTrue(source.contains("model.enroll(invite: link.raw)"))
        XCTAssertFalse(source.contains("scopes:"), "Join must not send scopes; Uses applies them")
    }

    func test_aLinkFillsTheFieldAndStops() throws {
        let source = try OnboardingParityTests.text("Views/OnboardingConnectView.swift")
        XCTAssertTrue(source.contains("pendingInvite.take()"))
        let consume = try XCTUnwrap(source.range(of: "private func consumePendingInvite()"))
        let body = source[consume.lowerBound...].prefix(400)
        XCTAssertTrue(body.contains("resolve()"))
        XCTAssertFalse(body.contains("join("), "a link must never enrol by itself")
    }

    func test_anEmptyInviteParameterIsDropped() {
        XCTAssertNil(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=")!))
        XCTAssertEqual(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE")!),
                       "https://issuer.example/onboard#CODE")
    }
}
