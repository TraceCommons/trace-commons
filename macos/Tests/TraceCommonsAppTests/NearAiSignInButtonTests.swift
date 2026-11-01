import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The Private AI tab's Sign in button (owner, 2026-10-24: "I would have
/// expected to see a signin button here that would take me to browser").
/// The balance card and the billing card run the one flow Settings'
/// "Sign in again" runs; these tests hold the offer and the flow.
final class NearAiSignInButtonTests: XCTestCase {
    // MARK: - When the button is offered

    func test_theBalanceCardOffersSignInWhenTheLoginIsNotUsable() {
        XCTAssertTrue(NearAiSignInOffer.balanceOffers(state: "session_expired", renewalFailure: nil))
        XCTAssertTrue(NearAiSignInOffer.balanceOffers(state: "no_session", renewalFailure: nil))
        XCTAssertTrue(
            NearAiSignInOffer.balanceOffers(state: "known", renewalFailure: "near_ai_sign_in_required"),
            "the daemon's renewal says NEAR AI refused the login")
    }

    func test_theBalanceCardOffersNothingWhenTheLoginIsHealthy() {
        XCTAssertFalse(NearAiSignInOffer.balanceOffers(state: "known", renewalFailure: nil))
        XCTAssertFalse(NearAiSignInOffer.balanceOffers(state: "unavailable", renewalFailure: nil))
        XCTAssertFalse(NearAiSignInOffer.balanceOffers(state: "unreported", renewalFailure: nil))
        XCTAssertFalse(
            NearAiSignInOffer.balanceOffers(state: "known", renewalFailure: "near_ai_unreachable"),
            "NEAR AI being unreachable is not a sign-in problem")
    }

    func test_theBillingCardOffersSignInWhenSignedOut() {
        XCTAssertTrue(NearAiSignInOffer.billingOffers(state: "no_session"))
        XCTAssertTrue(NearAiSignInOffer.billingOffers(state: "session_expired"))
        for state in ["ready", "unavailable", "changed", "no_organization", "no_inference_key", nil] {
            XCTAssertFalse(NearAiSignInOffer.billingOffers(state: state), "\(state ?? "nil")")
        }
    }

    func test_aFailedSignInKeepsRetryAvailableWhenTheNextReadIsUnavailable() {
        XCTAssertTrue(NearAiSignInOffer.balanceOffers(state: "unavailable", renewalFailure: nil, failed: true))
        XCTAssertTrue(NearAiSignInOffer.billingOffers(state: "unavailable", failed: true))
        XCTAssertTrue(NearAiSignInOffer.billingOffers(state: nil, failed: true))
        XCTAssertFalse(NearAiSignInOffer.balanceOffers(state: "unavailable", renewalFailure: nil, failed: false))
        XCTAssertFalse(NearAiSignInOffer.billingOffers(state: "unavailable", failed: false))
    }

    // MARK: - The flow

    @MainActor
    private func model(_ daemon: NearAiAccountDaemon) -> (AppModel, () -> [URL]) {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let lock = NSLock()
        var urls: [URL] = []
        model.openBrowser = { url in lock.withLock { urls.append(url) } }
        return (model, { lock.withLock { urls } })
    }

    @MainActor
    private func waitFor(_ what: String, _ condition: () -> Bool) async throws {
        let deadline = Date().addingTimeInterval(5)
        while !condition(), Date() < deadline { try await Task.sleep(for: .milliseconds(20)) }
        XCTAssertTrue(condition(), what)
    }

    /// A Commons session does not prove the separate NEAR AI login works.
    @MainActor
    func test_expiredNearAiLoginOpensBrowserEvenWithAValidAccountSession() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.signedIn = true
        daemon.balanceState = "session_expired"
        let (model, opened) = model(daemon)
        model.refreshNearAiBalance(fresh: true)
        try await waitFor("expired NEAR AI balance read") { model.balanceStatus.state == "session_expired" }
        let generation = model.accountSessionGeneration

        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("browser opened despite valid Commons session") { !opened().isEmpty }
        guard !opened().isEmpty else {
            signIn.cancel()
            await signIn.value
            return
        }
        XCTAssertEqual(daemon.count("near_ai_credential_start"), 1)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 0, "refresh NEAR AI before reusing the Commons session")
        try await Task.sleep(for: .milliseconds(1_200))
        XCTAssertTrue(model.nearAiSigningIn, "the stored session must not finish a fresh browser attempt")
        XCTAssertEqual(model.accountSessionGeneration, generation)

        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 1)
        XCTAssertEqual(model.accountSessionGeneration, generation + 1)
    }

    /// Refused login: the browser opens, the wait is on that attempt, the
    /// commons session is re-established, and the balance is read again.
    @MainActor
    func test_signInOpensTheBrowserWaitsThenReestablishesTheSessionAndRefreshes() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)
        let generation = model.accountSessionGeneration

        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("browser opened") { !opened().isEmpty }
        XCTAssertEqual(opened().map(\.host), ["cloud.example.invalid"])
        XCTAssertTrue(model.nearAiSigningIn, "progress shows while the browser is open")
        XCTAssertFalse(model.nearAiSignInFailed)
        daemon.finishBrowserSignIn()
        await signIn.value

        XCTAssertFalse(model.nearAiSigningIn)
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertEqual(daemon.count("near_ai_credential_start"), 1)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 1)
        XCTAssertEqual(daemon.count("account_sign_in"), 0, "never account_sign_in for a NEAR AI account")
        XCTAssertEqual(model.accountSessionGeneration, generation + 1, "billing and payout read again")
        try await waitFor("balance read again") { daemon.count("near_ai_balance") > 0 }
    }

    /// A browser sign-in that ends without a login says so and stops.
    @MainActor
    func test_aSignInThatDoesNotFinishShowsAFailureAndCanBeRetried() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        daemon.attemptFails = true
        let (model, opened) = model(daemon)

        await model.signInToNearAI()

        XCTAssertTrue(model.nearAiSignInFailed)
        XCTAssertFalse(model.nearAiSigningIn)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 0, "do not renew Commons without a refreshed NEAR AI login")

        daemon.attemptFails = false
        daemon.loginRefused = false
        let retry = Task { await model.signInToNearAI() }
        try await waitFor("retry opens a fresh browser") { opened().count == 2 }
        daemon.finishBrowserSignIn()
        await retry.value
        XCTAssertFalse(model.nearAiSignInFailed, "a new attempt clears the failure line")
        XCTAssertEqual(model.accountSession, .signedIn)
    }

    /// An account that is not a NEAR AI account (no `sign_in`) whose NEAR
    /// AI login is refused has nothing to renew at the commons: only the
    /// NEAR AI browser sign-in runs.
    @MainActor
    func test_aNonNearAiAccountSignsInToNearAiOnly() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.signInPath = nil
        let (model, opened) = model(daemon)

        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("browser opened") { !opened().isEmpty }
        daemon.finishBrowserSignIn()
        await signIn.value

        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 0)
        XCTAssertEqual(daemon.count("account_sign_in"), 0)
        try await waitFor("balance read again") { daemon.count("near_ai_balance") > 0 }
    }

    @MainActor
    func test_retryCancelsTheNamedAttemptAndOpensAFreshBrowserSession() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }

        await model.signInToNearAI()

        XCTAssertEqual(opened().count, 2)
        XCTAssertEqual(daemon.cancelledIDs, ["synthetic-attempt-1"])
        XCTAssertEqual(Set(opened()).count, 2, "retry must get a new URL")
        XCTAssertTrue(model.nearAiSigningIn)
        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertFalse(model.nearAiSignInFailed)
    }

    @MainActor
    func test_failedCancellationKeepsTheOriginalWaitAndDoesNotOpenAnotherBrowser() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        daemon.cancelFails = true
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }
        await model.signInToNearAI()
        XCTAssertEqual(daemon.cancelledIDs, ["synthetic-attempt-1"])
        XCTAssertEqual(opened().count, 1)
        XCTAssertTrue(model.nearAiSigningIn)
        XCTAssertNotNil(model.lastActionError)
        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertEqual(model.accountSession, .signedIn)
    }

    @MainActor
    func test_repeatedRetryWhileCancellationIsPendingStartsOnlyOneReplacement() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        daemon.cancelDelay = 0.2
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }
        let retry = Task { await model.signInToNearAI() }
        try await waitFor("cancel requested") { daemon.count("near_ai_credential_cancel") == 1 }
        await model.signInToNearAI()
        await retry.value
        XCTAssertEqual(opened().count, 2)
        XCTAssertEqual(daemon.count("near_ai_credential_cancel"), 1)
        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertEqual(model.accountSessionGeneration, 1)
    }

    @MainActor
    func test_accountRetryDiscardsAnOldPollThatCompletesAfterTheReplacementOpened() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        daemon.holdNextSuccessfulStatus()
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToAccount() }
        try await waitFor("old status held") { daemon.statusHeld }
        await model.signInToAccount()
        XCTAssertEqual(opened().count, 2)
        daemon.releaseStatus.signal()
        try await Task.sleep(for: .milliseconds(1_200))
        XCTAssertTrue(model.accountSigningIn, "the old success cannot finish the replacement")
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 1)
        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertEqual(model.accountSession, .signedIn)
    }

    @MainActor
    func test_nonNearAiAccountsCanRestartTheNearAiBrowserWait() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.signInPath = nil
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }
        await model.signInToNearAI()
        XCTAssertEqual(opened().count, 2)
        XCTAssertEqual(daemon.cancelledIDs, ["synthetic-attempt-1"])
        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSessionGeneration, 1)
        XCTAssertEqual(daemon.count("near_ai_account_sign_in"), 0)
    }

    @MainActor
    func test_replacementStartFailureEndsTheWaitAndAllowsAnotherSignIn() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)
        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }
        daemon.startFails = true
        await model.signInToNearAI()
        // Ensure the old implementation can fail without waiting ten minutes.
        if daemon.cancelledIDs.isEmpty { daemon.finishBrowserSignIn() }
        await signIn.value
        XCTAssertEqual(opened().count, 1)
        XCTAssertTrue(model.nearAiSignInFailed)
        XCTAssertFalse(model.nearAiSigningIn)
        daemon.startFails = false
        let next = Task { await model.signInToNearAI() }
        try await waitFor("another browser opened") { opened().count == 2 }
        daemon.finishBrowserSignIn()
        await next.value
        XCTAssertFalse(model.nearAiSignInFailed)
    }

    @MainActor
    func test_aRestartCannotReplaceANewerFlowAfterItsOriginalWaitWasCancelled() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        daemon.cancelDelay = 0.3
        let (model, opened) = model(daemon)
        let first = Task { await model.signInToNearAI() }
        try await waitFor("first browser opened") { opened().count == 1 }
        let retry = Task { await model.signInToNearAI() }
        try await waitFor("cancel requested") { daemon.count("near_ai_credential_cancel") == 1 }
        first.cancel()
        await first.value
        let next = Task { await model.signInToNearAI() }
        try await waitFor("new flow browser opened") { opened().count == 2 }
        await retry.value
        XCTAssertEqual(opened().count, 2, "the old restart cannot open a third browser")
        XCTAssertTrue(model.nearAiSigningIn)
        daemon.finishBrowserSignIn()
        await next.value
        XCTAssertFalse(model.nearAiSignInFailed)
    }

    // MARK: - The cards

    private func source(_ path: String) throws -> String {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("Sources/TraceCommonsApp")
        return try String(contentsOf: root.appendingPathComponent(path), encoding: .utf8)
    }

    /// Both cards draw the one control, under the core's words.
    func test_bothCardsDrawTheSharedSignInControl() throws {
        let account = try source("Views/Monitor/InferenceAccount.swift")
        XCTAssertTrue(account.contains("NearAiSignInControl(copy: copy)"))
        let funding = try source("Views/FundingRow.swift")
        XCTAssertTrue(funding.contains("NearAiSignInOffer.billingOffers"))
        XCTAssertTrue(funding.contains("model.signInToNearAI()"))
        let control = try source("Views/NearAiSignInControl.swift")
        XCTAssertTrue(control.contains("copy.credentialObtaining"), "the core's waiting line")
        XCTAssertTrue(control.contains("copy.credentialFailed"), "the core's failure line")
        XCTAssertTrue(control.contains("signInToNearAI()"))
    }
}
