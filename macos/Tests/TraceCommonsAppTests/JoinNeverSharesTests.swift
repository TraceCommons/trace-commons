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
        // The function body: from its signature to the next declaration.
        let rest = source[consume.upperBound...]
        let end = try XCTUnwrap(rest.range(of: "private var header"))
        let body = rest[..<end.lowerBound]
        XCTAssertTrue(body.contains("resolve()"))
        XCTAssertFalse(body.contains("join("), "a link must never enrol by itself")
        XCTAssertFalse(body.contains("enroll"), "a link must never enrol by itself")
    }

    /// The wallet card's lifecycle hooks hang off an always-present
    /// container, not a Group whose only content is conditional on state
    /// the `.task` itself sets.
    func test_walletLifecycleHooksSitOnAnAlwaysPresentContainer() throws {
        let source = try OnboardingParityTests.text("Views/NearAccountConnectView.swift")
        XCTAssertFalse(source.contains("Group {"))
        XCTAssertTrue(source.contains("VStack(alignment: .leading, spacing: 0) {\n            if let copy = model.witnessCopy?.wallet"))
        XCTAssertTrue(source.contains("        }\n        .task {"))
        XCTAssertTrue(source.contains(".onChange(of: busy)"))
        XCTAssertTrue(source.contains(".onDisappear {"))
    }

    func test_anEmptyInviteParameterIsDropped() {
        XCTAssertNil(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=")!))
        XCTAssertEqual(DeepLink.inviteURL(from: URL(string: "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE")!),
                       "https://issuer.example/onboard#CODE")
    }
}
