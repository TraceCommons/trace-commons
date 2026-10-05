import Combine
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
        XCTAssertFalse(watchingDone, "no tenant, so the tenant's marker is not written (watching only has its own)")
        XCTAssertFalse(watching.isOnboardingComplete)
    }

    /// A watch-only Start has no tenant, so its marker is keyed by the
    /// config directory the daemon runs from. It finishes the first run
    /// only while the daemon holds no enrolment: an enrolled person must
    /// still confirm on Start, whatever an earlier watch-only run wrote.
    func test_aWatchOnlyCompleteIsKeyedByTheConfigDirectory() async {
        let directory = "/tmp/first-run-adaptor-\(UUID().uuidString)"
        let watching = model(["status": #"{"result":{"logged_in":false,"consent_scopes":[],"health":{}}}"#])
        watching.setStartupForTesting(.running)
        defer { watching.clearWatchOnlyMarkerForTesting() }

        let noDirectory = await watching.markWatchOnlyComplete()
        XCTAssertFalse(noDirectory, "nothing to key the marker to")
        XCTAssertTrue(watching.requiresOnboarding)

        watching.setConfigDirectoryForTesting(directory)
        var notifications = 0
        let observing = watching.objectWillChange.sink { _ in notifications += 1 }
        let done = await watching.markWatchOnlyComplete()
        observing.cancel()
        XCTAssertGreaterThan(notifications, 0, "the hosts re-read requiresOnboarding only when told")
        XCTAssertTrue(done)
        XCTAssertFalse(watching.requiresOnboarding)
        XCTAssertTrue(watching.traceNavigationReady, "a finished watch-only run shows the content header")

        let tenant = "first-run-adaptor-\(UUID().uuidString)"
        watching.setStatusForTesting(DaemonStatus(
            schemaVersion: "1.1", loggedIn: true, tenantID: tenant, consentScopes: [], paused: false,
            queueDepth: 0, nextDigestAt: nil, health: DaemonHealth(lastErrorLabel: nil, since: nil)))
        XCTAssertTrue(watching.requiresOnboarding, "an enrolment needs its own Start")
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
