import Foundation
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Answers each method with a scripted reply, or a transport-shaped error.
private final class ScriptedDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [String: String]
    init(_ answers: [String: String]) { self.answers = answers }
    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        return answers[method] ?? #"{"error":{"code":"unknown_method","message":"unscripted"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// The live `FirstRunDaemon` answers: what `AppModel` tells the runner for
/// each daemon reply.
@MainActor
final class FirstRunDaemonAdaptorTests: XCTestCase {
    private func model(_ answers: [String: String]) -> AppModel {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: ScriptedDaemon(answers)))
        return model
    }

    func test_completeWithNoTenantIsNotReportedAsDone() async {
        let noClient = AppModel()
        let done = await noClient.markComplete()
        XCTAssertFalse(done, "nothing to key the marker to")

        let watching = model(["status": #"{"result":{"logged_in":false,"consent_scopes":[],"health":{}}}"#])
        let watchingDone = await watching.markComplete()
        XCTAssertFalse(watchingDone, "a watch-only Start has no tenant, so it is not marked")
        XCTAssertFalse(watching.isOnboardingComplete)
    }

    func test_completeReadsTheTenantBeforeMarking() async {
        let tenant = "first-run-adaptor-\(UUID().uuidString)"
        let key = "trace_commons.onboarding_complete.\(tenant)"
        defer { UserDefaults.standard.removeObject(forKey: key) }
        // The status a refresh has not delivered yet: enrolled, with a tenant.
        let enrolled = model([
            "status": #"{"result":{"logged_in":true,"tenant_id":"\#(tenant)","consent_scopes":[],"health":{}}}"#,
        ])
        XCTAssertNil(enrolled.status.tenantID)

        let done = await enrolled.markComplete()

        XCTAssertTrue(done)
        XCTAssertTrue(enrolled.isOnboardingComplete)
    }

    func test_aLookupThatDidNotReachTheIssuerIsUnavailableNotDead() async {
        let noClient = await AppModel().lookupInvite("INVITE-1")
        XCTAssertEqual(noClient, .unavailable)

        for reply in [
            #"{"error":{"code":"unavailable","message":"invite-lookup-unavailable"}}"#,
            #"not json"#,
            #"{"id":1}"#,
        ] {
            let answer = await model(["invite_lookup": reply]).lookupInvite("INVITE-1")
            XCTAssertEqual(answer, .unavailable, reply)
        }
    }

    func test_aRefusedInviteIsDeadWithTheDaemonsLabel() async {
        let cases: [(String, String)] = [
            (#"{"result":{"valid":false,"reason_label":"invite-exhausted"}}"#, "invite-exhausted"),
            (#"{"result":{"valid":false}}"#, "invite-invalid"),
            (#"{"error":{"code":"bad_params","message":"invite-invalid"}}"#, "invite-invalid"),
            // Unavailable by code, but permanent for this invite: retrying
            // cannot help, so the person goes back to Join.
            (#"{"error":{"code":"unavailable","message":"invite-host-not-allowed"}}"#, "invite-host-not-allowed"),
        ]
        for (reply, label) in cases {
            let answer = await model(["invite_lookup": reply]).lookupInvite("INVITE-1")
            XCTAssertEqual(answer, .refused(label: label), reply)
        }
    }

    func test_anUnconfirmedGrantCarriesALabelTheCoreKnows() async {
        let noClient = await AppModel().grantAutomatic(witness: nil)
        XCTAssertEqual(noClient, .refused(label: "automatic-grant-unavailable"))

        let ungranted = await model(["grant_automatic": #"{"result":{"granted":false}}"#])
            .grantAutomatic(witness: nil)
        XCTAssertEqual(ungranted, .refused(label: "automatic-grant-unavailable"))

        let refused = await model([
            "grant_automatic": #"{"error":{"code":"unavailable","message":"arming-terms-unavailable"}}"#,
        ]).grantAutomatic(witness: nil)
        XCTAssertEqual(refused, .refused(label: "arming-terms-unavailable"))
    }
}
