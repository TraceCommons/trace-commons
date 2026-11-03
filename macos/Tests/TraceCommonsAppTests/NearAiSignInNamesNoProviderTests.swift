import Foundation
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// "ideally the choosing of login types happens in the browser window rather
/// than with a dropdown menu after sign in with near is clicked" (owner,
/// 2026-11-02). Every NEAR AI Sign in in the app -- Home's Private AI card,
/// the Monitor's NEAR AI notice, the Private AI tab's balance and billing
/// cards, Settings > Private AI, Sign in again, the first run and the legacy
/// move -- is one plain button that starts `near_ai_credential_start` with
/// no `provider`. The daemon's browser page lists the build's logins.
final class NearAiSignInNamesNoProviderTests: XCTestCase {
    private static func source(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

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

    // MARK: - What reaches the daemon

    /// Home's card and the Monitor's notice (`NearAiSignIn.begin`) start
    /// the ceremony with no parameters at all.
    @MainActor
    func test_homeAndTheNoticeStartWithNoProvider() async throws {
        let daemon = NearAiAccountDaemon()
        let (model, _) = model(daemon)
        var opened: [URL] = []
        await NearAiSignIn.run(model, open: { opened.append($0); return true })
        XCTAssertEqual(daemon.startedParamKeys, [[]])
        XCTAssertEqual(opened.count, 1)
        XCTAssertEqual(daemon.cancelledIDs, [], "a browser that opened keeps its ceremony")
    }

    /// Settings' Sign in (`startNearAiCredential`) names no provider.
    @MainActor
    func test_settingsStartsWithNoProvider() async throws {
        let daemon = NearAiAccountDaemon()
        let (model, _) = model(daemon)
        let url = await model.startNearAiCredential()
        XCTAssertNotNil(url)
        XCTAssertEqual(daemon.startedParamKeys, [[]])
    }

    /// The balance and billing cards' Sign in (`signInToNearAI`) names no
    /// provider, and neither does a retry while the browser is out, which
    /// replaces the attempt.
    @MainActor
    func test_theBalanceCardAndItsRetryStartWithNoProvider() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)

        let signIn = Task { await model.signInToNearAI() }
        try await waitFor("browser opened") { opened().count == 1 }
        await model.signInToNearAI()
        XCTAssertEqual(opened().count, 2, "the retry replaced the attempt")

        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertEqual(daemon.startedParamKeys, [[], []])
    }

    /// Sign in again, when NEAR AI refuses the stored login, starts the
    /// browser sign-in with no provider.
    @MainActor
    func test_signInAgainStartsWithNoProvider() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)

        let again = Task { await model.signInAgain() }
        try await waitFor("browser opened") { opened().count == 1 }
        daemon.finishBrowserSignIn()
        await again.value
        XCTAssertEqual(model.accountSession, .signedIn)
        XCTAssertEqual(daemon.startedParamKeys, [[]])
    }

    // MARK: - No shell-side choice of login

    /// The client cannot send a provider: `nearAiCredentialStart` takes
    /// none, and no call site or model method carries one.
    func test_noProviderIsCarriedFromAnyEntryPoint() throws {
        let client = try Self.source("DaemonClient.swift")
        XCTAssertTrue(client.contains("func nearAiCredentialStart() throws -> CredentialAttempt? {"))
        XCTAssertFalse(client.contains("[\"provider\""))

        let model = try Self.source("AppModel.swift")
        XCTAssertFalse(model.contains("nearAiBrowserProvider"))
        for signature in ["func startNearAiCredential() async -> URL?", "func signInAgain() async {",
                          "func signInToNearAI() async {", "func nearAILoginOutcome(fresh: Bool) async -> NearAILoginOutcome {",
                          "private func restartNearAIBrowserSignIn() async {",
                          // The first run and the legacy move go through this.
                          "func nearAILoginOutcome() async -> NearAILoginOutcome { await nearAILoginOutcome(fresh: false) }"] {
            XCTAssertTrue(model.contains(signature), "AppModel.swift lacks \(signature)")
        }
        XCTAssertFalse(model.contains("nearAiCredentialStart(provider:"))

        for file in ["Views/Monitor/HomeViews.swift", "Views/Monitor/MonitorNotices.swift",
                     "Views/NearAiSignInControl.swift", "Views/FundingRow.swift",
                     "Views/CredentialSection.swift", "AppModel.swift", "DaemonClient.swift"] {
            let text = try Self.source(file)
            XCTAssertFalse(text.contains("NearAiSignInChoice"), "\(file) still offers a choice of login")
            // AppModel also carries subscription-usage providers, which
            // are not sign-in logins; its signatures are pinned above.
            if file != "AppModel.swift" {
                XCTAssertFalse(text.contains("provider:"), "\(file) passes a provider")
            }
            for tag in ["near", "github", "google"] {
                XCTAssertFalse(text.contains("provider: \"\(tag)\""), "\(file) names \(tag)")
            }
        }
    }

    /// Each Sign in outside Settings is the one plain control, with no menu.
    func test_everySignInIsOnePlainButton() throws {
        let control = try Self.source("Views/NearAiSignInControl.swift")
        XCTAssertFalse(control.contains("Menu {"), "the sign-in is a menu again")
        XCTAssertTrue(control.contains("let start: () -> Void"))
        XCTAssertTrue(control.contains("Task { await model.signInToNearAI() }"))

        let home = try Self.source("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("NearAiSignInButton("))
        XCTAssertTrue(home.contains("NearAiSignIn.begin(model, openURL: openURL)"))

        let notices = try Self.source("Views/Monitor/MonitorNotices.swift")
        XCTAssertTrue(notices.contains("NearAiSignInButton("))
        XCTAssertTrue(notices.contains("accessibilityName: model.managedText(\"sign_in_near_ai_accessibility\")"))
        XCTAssertTrue(notices.contains("NearAiSignIn.begin(model, openURL: openURL)"))
        XCTAssertTrue(notices.contains("guard let url = await model.startNearAiCredential() else { return }"))

        let funding = try Self.source("Views/FundingRow.swift")
        XCTAssertTrue(funding.contains("NearAiSignInButton("))
        XCTAssertTrue(funding.contains("Task { await model.signInToNearAI() }"))
    }

    /// Settings draws no provider picker and no wallet notice: both are on
    /// the daemon's browser page now.
    func test_settingsDrawsNoPickerAndNoWalletNotice() throws {
        let section = try Self.source("Views/CredentialSection.swift")
        XCTAssertFalse(section.contains("GlassPicker("))
        XCTAssertFalse(section.contains("credentialProviderLabel"))
        XCTAssertFalse(section.contains("credentialWalletNotice"))
        XCTAssertFalse(section.contains("providerOptions"))
        XCTAssertFalse(section.contains("providerSelection"))
        XCTAssertTrue(section.contains("guard let url = await model.startNearAiCredential() else { return }"))
    }
}
