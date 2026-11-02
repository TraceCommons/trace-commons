import Foundation
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// "We are going back to supporting all the near.ai cloud sign in
/// mechanisms. Github, google, or near wallet." (owner, 2026-11-01). Every
/// Sign in outside Settings -- Home's Private AI card, the Monitor's NEAR AI
/// notice, the Private AI tab's balance and billing cards -- offers each
/// login the linked core lists, in its order and with its titles, and starts
/// the one chosen. A build that offers one login keeps a plain button.
final class NearAiSignInChoiceTests: XCTestCase {
    private static func source(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    // MARK: - What is offered

    @MainActor
    func test_theChoiceOffersEveryLoginTheCoreLists() throws {
        let copy = try XCTUnwrap(AppModel().privateInferenceCopy)
        let offered = LinkedChannel.channel.nearAiSignInProviders
        switch NearAiSignInChoice.of(copy) {
        case .menu(let options):
            XCTAssertGreaterThan(offered.count, 1)
            XCTAssertEqual(options.map(\.tag), offered, "the core's logins, in its order")
            XCTAssertEqual(options, CredentialSection.providerOptions(copy), "Settings' titles")
            if !LinkedChannel.isStaging {
                XCTAssertEqual(options.map(\.tag), ["near", "github", "google"])
                XCTAssertEqual(options.map(\.title), [
                    copy.credentialProviderNear, copy.credentialProviderGithub, copy.credentialProviderGoogle,
                ])
            }
        case .button(let provider):
            XCTAssertEqual(offered.count, 1, "a plain button only when the build offers one login")
            XCTAssertEqual(provider, offered.first)
        }
        if LinkedChannel.isStaging {
            XCTAssertEqual(NearAiSignInChoice.of(copy), .button(provider: "google"))
        }
    }

    func test_oneLoginIsAPlainButtonAndSeveralAreAMenu() {
        let near = CredentialSection.ProviderOption(tag: "near", title: "NEAR wallet")
        let github = CredentialSection.ProviderOption(tag: "github", title: "GitHub")
        XCTAssertEqual(NearAiSignInChoice.of([github]), .button(provider: "github"))
        XCTAssertEqual(NearAiSignInChoice.of([near, github]), .menu([near, github]))
        XCTAssertEqual(NearAiSignInChoice.of([]), .button(provider: nil), "none known: the daemon's default")
        XCTAssertEqual(NearAiSignInChoice.of(nil as PrivateInferenceCopy?), .button(provider: nil))
    }

    // MARK: - The chosen login is the one started

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

    /// Home's card and the Monitor's notice start the login chosen.
    @MainActor
    func test_homeAndTheNoticeStartTheChosenLogin() async throws {
        let daemon = NearAiAccountDaemon()
        let (model, _) = model(daemon)
        var opened: [URL] = []
        await NearAiSignIn.run(model, provider: "github", open: { opened.append($0); return true })
        XCTAssertEqual(daemon.startedProviders, ["github"])
        XCTAssertEqual(opened.count, 1)
        XCTAssertEqual(daemon.cancelledIDs, [], "a browser that opened keeps its ceremony")
    }

    /// The balance and billing cards' Sign in starts the login chosen, and
    /// a retry that names none starts that same login again.
    @MainActor
    func test_theBalanceCardStartsTheChosenLoginAndARetryKeepsIt() async throws {
        let daemon = NearAiAccountDaemon()
        daemon.loginRefused = true
        let (model, opened) = model(daemon)

        let signIn = Task { await model.signInToNearAI(provider: "google") }
        try await waitFor("browser opened") { opened().count == 1 }
        XCTAssertEqual(daemon.startedProviders, ["google"])

        await model.signInToNearAI(provider: nil)
        XCTAssertEqual(opened().count, 2)
        XCTAssertEqual(daemon.startedProviders, ["google", "google"], "a retry naming none keeps the login")

        await model.signInToNearAI(provider: "github")
        XCTAssertEqual(opened().count, 3)
        XCTAssertEqual(daemon.startedProviders, ["google", "google", "github"], "a retry naming one switches")

        daemon.finishBrowserSignIn()
        await signIn.value
        XCTAssertFalse(model.nearAiSignInFailed)
        XCTAssertEqual(model.accountSession, .signedIn)
    }

    // MARK: - Every entry point draws the choice

    func test_everySignInOutsideSettingsDrawsTheChoice() throws {
        let home = try Self.source("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("choice: NearAiSignInChoice.of(model.privateInferenceCopy)"))
        XCTAssertTrue(home.contains("NearAiSignInButton("))
        XCTAssertTrue(home.contains("NearAiSignIn.begin(model, provider: provider, openURL: openURL)"))

        let notices = try Self.source("Views/Monitor/MonitorNotices.swift")
        XCTAssertTrue(notices.contains("NearAiSignInButton("))
        XCTAssertTrue(notices.contains("choice: NearAiSignInChoice.of(model.privateInferenceCopy)"))
        XCTAssertTrue(notices.contains("accessibilityName: model.managedText(\"sign_in_near_ai_accessibility\")"))
        XCTAssertTrue(notices.contains("guard let url = await model.startNearAiCredential(provider: provider)"))

        let control = try Self.source("Views/NearAiSignInControl.swift")
        XCTAssertTrue(control.contains("choice: NearAiSignInChoice.of(copy)"))
        let funding = try Self.source("Views/FundingRow.swift")
        XCTAssertTrue(funding.contains("choice: NearAiSignInChoice.of(copy)"))

        // No sign-in starts the wallet, or any login, by name.
        for file in ["Views/Monitor/HomeViews.swift", "Views/Monitor/MonitorNotices.swift",
                     "Views/NearAiSignInControl.swift", "Views/FundingRow.swift"] {
            let text = try Self.source(file)
            for tag in ["near", "github", "google"] {
                XCTAssertFalse(text.contains("provider: \"\(tag)\""), "\(file) names \(tag)")
            }
        }
        // The menu's items are Settings' list, never one of its own.
        XCTAssertTrue(control.contains("CredentialSection.providerOptions($0)"))
    }
}
