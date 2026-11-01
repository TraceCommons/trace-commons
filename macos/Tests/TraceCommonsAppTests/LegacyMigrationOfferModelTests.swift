import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// A daemon that answers `legacy_invite_migrate` from a script and `status`
/// with whatever the test last set, recording every call.
private final class MigrationDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var recorded: [(String, String)] = []
    private var answers: [String]
    var statusJSON: String

    init(answers: [String], statusJSON: String) {
        self.answers = answers
        self.statusJSON = statusJSON
    }

    var calls: [(String, String)] { lock.lock(); defer { lock.unlock() }; return recorded }

    func call(_ method: String, params: String) -> String {
        lock.lock(); defer { lock.unlock() }
        recorded.append((method, params))
        switch method {
        case "legacy_invite_migrate":
            return answers.isEmpty ? #"{"id":1,"result":{}}"# : answers.removeFirst()
        case "status":
            return #"{"id":1,"result":"# + statusJSON + "}"
        default:
            return #"{"id":1,"error":{"code":"unavailable","message":"not_scripted"}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

private func refusal(_ label: String) -> String {
    #"{"id":1,"error":{"code":"unavailable","message":"\#(label)"}}"#
}

/// The offer to move a legacy invite identity to a NEAR AI account: when
/// Settings and the card above the tabs draw it, that the card's dismiss
/// lasts, and how Move ends.
@MainActor
final class LegacyMigrationOfferModelTests: XCTestCase {
    private func freshTenant(_ prefix: String = "tenant-") -> String {
        let tenant = prefix + UUID().uuidString.lowercased()
        // Resolve the actor-owned key now; teardown owns only its immutable
        // copy and never needs to access this UI-isolated test instance.
        if let key = AppModel.legacyMigrationOfferDismissedKey(tenant) {
            addTeardownBlock { UserDefaults.standard.removeObject(forKey: key) }
        }
        return tenant
    }

    private func statusJSON(tenant: String?, offered: Bool?, notice: String = "null") -> String {
        let tenantField = tenant.map { #","tenant_id":"\#($0)""# } ?? ""
        let migration = offered.map { #","legacy_invite_migration":{"offered":\#($0),"notice":\#(notice)}"# } ?? ""
        return """
            {"schema_version":"v","logged_in":\(tenant != nil),"paused":false,"queue_depth":0,
             "health":{"last_error_label":null,"since":null}\(tenantField)\(migration)}
            """
    }

    private func status(tenant: String?, offered: Bool?, notice: String = "null") throws -> DaemonStatus {
        try JSONDecoder().decode(
            DaemonStatus.self, from: Data(statusJSON(tenant: tenant, offered: offered, notice: notice).utf8))
    }

    // MARK: - Visibility

    func testTheCoresOfferWordsArrive() throws {
        let offer = try XCTUnwrap(AppModel().legacyMigrationOffer)
        XCTAssertFalse(offer.dismiss.isEmpty)
        XCTAssertFalse(offer.later.isEmpty)
        XCTAssertFalse(offer.nearAILoginWaiting.isEmpty)
        XCTAssertFalse(offer.nearAILoginWaiting.lowercased().contains("passkey"))
        XCTAssertFalse(offer.nearAILoginCancel.isEmpty)
    }

    /// An invite-enrolled device the daemon offers the move to: Settings
    /// and the card both show it.
    func testALegacyInviteDeviceIsOfferedTheMove() throws {
        let model = AppModel()
        model.setStatusForTesting(try status(tenant: freshTenant(), offered: true))
        XCTAssertTrue(model.isLegacyMigrationOffered)
        XCTAssertTrue(model.showsLegacyMigrationOfferCard)
        XCTAssertEqual(model.acknowledgeableNoticeCount, 1)
    }

    /// An account-enrolled device (or one already moved), an unenrolled one
    /// and a daemon too old to report the field are offered nothing.
    func testEverythingElseIsOfferedNothing() throws {
        let cases: [(String, DaemonStatus)] = [
            ("account-enrolled", try status(tenant: freshTenant("nearai-"), offered: false)),
            ("already moved", try status(tenant: freshTenant(), offered: false,
                                         notice: #"{"folders_kept":0,"automatic_grant_kept":false}"#)),
            ("unenrolled", try status(tenant: nil, offered: false)),
            ("older daemon", try status(tenant: freshTenant(), offered: nil)),
        ]
        for (name, status) in cases {
            let model = AppModel()
            model.setStatusForTesting(status)
            XCTAssertFalse(model.isLegacyMigrationOffered, name)
            XCTAssertFalse(model.showsLegacyMigrationOfferCard, name)
        }
    }

    /// Nothing is offered before the daemon has answered at all.
    func testNothingIsOfferedBeforeStatusIsRead() {
        let model = AppModel()
        XCTAssertFalse(model.isLegacyMigrationOffered)
        XCTAssertFalse(model.showsLegacyMigrationOfferCard)
    }

    /// The card's dismiss lasts beyond this model (a relaunch) and keeps the
    /// offer in Settings. It is per identity: another invite identity
    /// enrolled on this Mac is offered the card afresh.
    func testADismissedCardStaysDismissedAndSettingsKeepsTheOffer() throws {
        let tenant = freshTenant()
        let model = AppModel()
        model.setStatusForTesting(try status(tenant: tenant, offered: true))
        model.dismissLegacyMigrationOffer()
        XCTAssertFalse(model.showsLegacyMigrationOfferCard)
        XCTAssertTrue(model.isLegacyMigrationOffered)
        XCTAssertEqual(model.acknowledgeableNoticeCount, 0)

        let relaunched = AppModel()
        relaunched.setStatusForTesting(try status(tenant: tenant, offered: true))
        XCTAssertFalse(relaunched.showsLegacyMigrationOfferCard)
        XCTAssertTrue(relaunched.isLegacyMigrationOffered)

        let other = AppModel()
        other.setStatusForTesting(try status(tenant: freshTenant(), offered: true))
        XCTAssertTrue(other.showsLegacyMigrationOfferCard)
    }

    /// The marker never holds the tenant id itself.
    func testTheDismissMarkerIsKeyedByADigest() throws {
        let tenant = freshTenant()
        let key = try XCTUnwrap(AppModel.legacyMigrationOfferDismissedKey(tenant))
        XCTAssertFalse(key.contains(tenant))
        XCTAssertNil(AppModel.legacyMigrationOfferDismissedKey(nil))
        XCTAssertNil(AppModel.legacyMigrationOfferDismissedKey(""))
    }

    /// "Dismiss all" puts the card away as its own Dismiss does.
    func testDismissAllPutsTheCardAway() throws {
        let model = AppModel()
        model.setStatusForTesting(try status(tenant: freshTenant(), offered: true))
        model.acknowledgeAllNotices()
        XCTAssertFalse(model.showsLegacyMigrationOfferCard)
        XCTAssertTrue(model.isLegacyMigrationOffered)
    }

    // MARK: - Move

    /// A move that lands: no invite is sent unasked, status is re-read, and
    /// the daemon's answer -- no offer, the notice -- takes over.
    func testAMoveThatLandsHandsOverToTheNotice() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [#"{"id":1,"result":{"migrated":true}}"#],
            statusJSON: statusJSON(tenant: tenant, offered: false,
                                   notice: #"{"folders_kept":1,"automatic_grant_kept":false}"#))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))

        await model.migrateLegacyInvite(invite: "ignored-until-asked")
        XCTAssertNil(model.legacyMigrationRefusal)
        XCTAssertFalse(model.legacyMigrationStartFailed)
        XCTAssertFalse(model.legacyMigrationBusy)
        let move = try XCTUnwrap(daemon.calls.first { $0.0 == "legacy_invite_migrate" })
        XCTAssertEqual(move.1, "{}", "an invite is sent only once the daemon asks for it")

        try await waitUntil { !model.isLegacyMigrationOffered }
        XCTAssertFalse(model.showsLegacyMigrationOfferCard)
        XCTAssertNotNil(model.legacyMigrationNotice)
    }

    /// A pooled invite cannot be told apart before the commons answers, so
    /// it is offered the move and refused in the core's words. The daemon
    /// then stops offering it (a pooled tenant never links), but the card
    /// and Settings stay drawn, without Move, so the refusal is read rather
    /// than vanishing with the offer.
    func testAPooledInviteIsRefusedInTheCoresWords() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [refusal("legacy_migration_tenant_pooled")],
            statusJSON: statusJSON(tenant: tenant, offered: false))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))

        await model.migrateLegacyInvite()
        XCTAssertEqual(
            model.legacyMigrationRefusal,
            TCCoreCopy.legacyMigrationRefusalLine(label: "legacy_migration_tenant_pooled"))
        XCTAssertFalse(model.legacyMigrationInviteNeeded)
        try await waitUntil { !model.isLegacyMigrationOffered }
        XCTAssertTrue(model.isLegacyMigrationOfferDrawn, "the refusal stays readable")
        XCTAssertTrue(model.showsLegacyMigrationOfferCard)
        XCTAssertFalse(model.offersLegacyMigrationMove, "no Move the daemon no longer offers")

        // Another identity on this Mac does not inherit the held refusal.
        model.setStatusForTesting(try status(tenant: freshTenant(), offered: false))
        XCTAssertFalse(model.isLegacyMigrationOfferDrawn)
        XCTAssertFalse(model.showsLegacyMigrationOfferCard)
    }

    /// `legacy_migration_invite_needed`: the invite is asked for, and the
    /// next Move sends what was pasted, trimmed. A bad paste keeps asking.
    func testAnInviteIsAskedForAndResent() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [
                refusal("legacy_migration_invite_needed"),
                refusal("legacy_migration_invite_invalid"),
                #"{"id":1,"result":{"migrated":true}}"#,
            ],
            statusJSON: statusJSON(tenant: tenant, offered: true))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))

        await model.migrateLegacyInvite()
        XCTAssertTrue(model.legacyMigrationInviteNeeded)
        XCTAssertEqual(
            model.legacyMigrationRefusal,
            TCCoreCopy.legacyMigrationRefusalLine(label: "legacy_migration_invite_needed"))

        await model.migrateLegacyInvite(invite: "  not a code!  ")
        XCTAssertTrue(model.legacyMigrationInviteNeeded, "a refused paste still asks for the invite")

        await model.migrateLegacyInvite(invite: "  SYNTHETICINVITE01 \n")
        XCTAssertFalse(model.legacyMigrationInviteNeeded)
        XCTAssertNil(model.legacyMigrationRefusal)

        let sent = daemon.calls.filter { $0.0 == "legacy_invite_migrate" }.map(\.1)
        XCTAssertEqual(sent.count, 3)
        XCTAssertEqual(sent[0], "{}")
        let last = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(sent[2].utf8)) as? [String: String])
        XCTAssertEqual(last, ["invite": "SYNTHETICINVITE01"])
    }

    // MARK: - The NEAR AI login step

    /// With no session the move leads with the NEAR AI login step: Move runs it
    /// (the browser step the first run uses), then retries the move without
    /// a second press. Nothing says "sign in".
    func testWithNoSessionTheNearAILoginStepRunsAndTheMoveContinues() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [
                refusal("legacy_migration_no_near_ai_session"),
                #"{"id":1,"result":{"migrated":true}}"#,
            ],
            statusJSON: statusJSON(tenant: tenant, offered: false,
                                   notice: #"{"folders_kept":0,"automatic_grant_kept":false}"#))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))
        var steps = 0
        var waitingDuringStep = false
        model.legacyMigrationNearAILoginStep = { [weak model] in
            steps += 1
            waitingDuringStep = model?.legacyMigrationAwaitingNearAILogin ?? false
            return true
        }

        await model.migrateLegacyInvite()
        XCTAssertEqual(steps, 1)
        XCTAssertTrue(waitingDuringStep, "the offer says it waits on the browser while the step runs")
        XCTAssertFalse(model.legacyMigrationAwaitingNearAILogin)
        XCTAssertEqual(daemon.calls.filter { $0.0 == "legacy_invite_migrate" }.count, 2)
        XCTAssertNil(model.legacyMigrationRefusal)
        XCTAssertFalse(model.legacyMigrationNearAILoginFailed)
        try await waitUntil { !model.isLegacyMigrationOffered }
    }

    /// Cancelled while waiting on the NEAR AI login: the move is not retried,
    /// nothing changed, no failure is claimed, and the offer stays.
    func testACancelledNearAILoginStepChangesNothing() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [refusal("legacy_migration_no_near_ai_session")],
            statusJSON: statusJSON(tenant: tenant, offered: true))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))
        var cancelled = 0
        model.legacyMigrationNearAILoginStep = {
            while !Task.isCancelled { try? await Task.sleep(nanoseconds: 5_000_000) }
            return false
        }
        model.legacyMigrationNearAILoginCancel = { cancelled += 1 }

        let move = Task { await model.migrateLegacyInvite() }
        try await waitUntil { model.legacyMigrationAwaitingNearAILogin }
        model.cancelLegacyMigrationNearAILogin()
        await move.value

        XCTAssertEqual(cancelled, 1, "the browser step is ended too")
        XCTAssertEqual(daemon.calls.filter { $0.0 == "legacy_invite_migrate" }.count, 1)
        XCTAssertFalse(model.legacyMigrationAwaitingNearAILogin)
        XCTAssertFalse(model.legacyMigrationBusy)
        XCTAssertFalse(model.legacyMigrationNearAILoginFailed)
        XCTAssertNil(model.legacyMigrationRefusal)
        XCTAssertFalse(model.legacyMigrationStartFailed)
        XCTAssertTrue(model.isLegacyMigrationOffered)
    }

    /// A NEAR AI login step that ends without finishing: the core's line, the
    /// move is not retried, and the offer stays.
    func testAFailedNearAILoginStepSaysTheCoresLine() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [refusal("legacy_migration_no_near_ai_session")],
            statusJSON: statusJSON(tenant: tenant, offered: true))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))
        model.legacyMigrationNearAILoginStep = { false }

        await model.migrateLegacyInvite()
        XCTAssertTrue(model.legacyMigrationNearAILoginFailed)
        XCTAssertNil(model.legacyMigrationRefusal)
        XCTAssertEqual(daemon.calls.filter { $0.0 == "legacy_invite_migrate" }.count, 1)
        XCTAssertTrue(model.isLegacyMigrationOffered)
        let line = try XCTUnwrap(model.legacyMigrationOffer?.nearAILoginFailed)
        XCTAssertFalse(line.isEmpty)
    }

    /// The NEAR AI login step runs once per press: a second "no session" after it
    /// finished is said as the refusal it is, not looped on.
    func testTheNearAILoginStepRunsOncePerPress() async throws {
        let tenant = freshTenant()
        let daemon = MigrationDaemon(
            answers: [
                refusal("legacy_migration_no_near_ai_session"),
                refusal("legacy_migration_no_near_ai_session"),
            ],
            statusJSON: statusJSON(tenant: tenant, offered: true))
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.setStatusForTesting(try status(tenant: tenant, offered: true))
        var steps = 0
        model.legacyMigrationNearAILoginStep = { steps += 1; return true }

        await model.migrateLegacyInvite()
        XCTAssertEqual(steps, 1)
        XCTAssertEqual(daemon.calls.filter { $0.0 == "legacy_invite_migrate" }.count, 2)
        XCTAssertEqual(
            model.legacyMigrationRefusal,
            TCCoreCopy.legacyMigrationRefusalLine(label: "legacy_migration_no_near_ai_session"))
    }

    /// Move is hidden while the invite is asked for and nothing is pasted.
    func testMoveIsHiddenUntilAnInviteIsPastedWhenOneIsAskedFor() {
        XCTAssertTrue(LegacyMigrationMoveControls.canMove(inviteNeeded: false, invite: ""))
        XCTAssertFalse(LegacyMigrationMoveControls.canMove(inviteNeeded: true, invite: "  \n"))
        XCTAssertTrue(LegacyMigrationMoveControls.canMove(inviteNeeded: true, invite: "CODE"))
    }

    private func waitUntil(_ condition: @MainActor () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("condition not met")
    }
}
