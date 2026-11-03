import os
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class StartRecordingDaemon: DaemonCalling {
    private let recorded = OSAllocatedUnfairLock(initialState: [(String, String)]())

    var calls: [(String, String)] {
        recorded.withLock { $0 }
    }

    func call(_ method: String, params: String) -> String {
        recorded.withLock { $0.append((method, params)) }
        return #"{"id":1,"result":{"attempt_id":"synthetic-attempt","browser_url":"https://cloud.example.invalid/login","status":"waiting_for_browser"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// "In the staging builds the sign in needs to be via Google and not your
/// near wallet" (owner, 2026-10-23). The decision is the core's
/// (`channel::NEAR_AI_SIGN_IN_PROVIDERS`); the app names no provider on the
/// first run's sign-in and draws only what the linked dylib offers. Run
/// against a production dylib these pin production as it was; run against a
/// staging dylib they hold staging to NEAR AI's own login.
final class StagingSignInTests: XCTestCase {
    private static func source(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// The providers the linked core offers: production's NEAR wallet first,
    /// staging's NEAR AI Google login alone (owner: "Google only for now").
    func test_theLinkedCoreOffersOnlyTheNearAiLoginOnStaging() {
        let offered = LinkedChannel.channel.nearAiSignInProviders
        if LinkedChannel.isStaging {
            XCTAssertEqual(offered, ["google"])
            XCTAssertFalse(offered.contains("near"))
            XCTAssertFalse(offered.contains("github"))
        } else {
            XCTAssertEqual(offered, ["near", "github", "google"])
        }
    }

    /// The first run's near.ai login names no provider, and neither does
    /// the Monitor's sign-in notice: the daemon's browser page offers the
    /// channel's logins (owner, 2026-11-02), so the app cannot pick the
    /// wallet on a build that has none. The first-run screens never choose
    /// a provider of their own.
    func test_theFirstRunSignInNamesNoProvider() throws {
        let model = try Self.source("AppModel.swift")
        XCTAssertTrue(
            model.contains("func nearAILoginOutcome() async -> NearAILoginOutcome { await nearAILoginOutcome(fresh: false) }"),
            "the first run's login names a provider")

        let notices = try Self.source("Views/Monitor/MonitorNotices.swift")
        XCTAssertTrue(notices.contains("NearAiSignIn.begin(model, openURL: openURL)"))
        XCTAssertFalse(notices.contains("provider:"))

        let firstRun = GlassSurfaceRulesTests.root.appendingPathComponent("Views/FirstRun")
        for file in try FileManager.default.contentsOfDirectory(atPath: firstRun.path) where file.hasSuffix(".swift") {
            let text = try String(contentsOf: firstRun.appendingPathComponent(file), encoding: .utf8)
            XCTAssertFalse(text.contains("provider:"), "\(file) chooses a sign-in provider")
            XCTAssertFalse(text.lowercased().contains("wallet"), "\(file) offers a wallet")
        }
    }

    /// A start that names no provider sends the daemon no provider at all.
    func test_aStartWithoutAProviderSendsNone() throws {
        let daemon = StartRecordingDaemon()
        _ = try DaemonClient(daemon: daemon).nearAiCredentialStart()
        let start = try XCTUnwrap(daemon.calls.first { $0.0 == CredentialSurface.startMethod })
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(start.1.utf8)) as? [String: Any])
        XCTAssertNil(params["provider"])
        XCTAssertTrue(params.isEmpty, "the start carries no parameters: \(params)")
    }

    /// When a near.ai join is refused, the first run words the daemon's
    /// label with the core's line. On staging none of them offers a wallet;
    /// production's "not offered here" line still does, as it shipped.
    /// Staging's says only Google is supported (approved 2026-10-23).
    @MainActor
    func test_aRefusedJoinOffersNoWalletOnStaging() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        for label in [
            "near_ai_enroll_already_enrolled", "near_ai_enroll_no_session", "near_ai_enroll_endpoint_refused",
            "near_ai_enroll_token_unavailable", "near_ai_enroll_start_failed",
            "near_ai_enroll_commons_unreachable", "near_ai_enroll_commons_unsupported",
            "near_ai_enroll_invalid", "near_ai_enroll_verification_failed", "near_ai_enroll_unavailable",
        ] {
            let line = try XCTUnwrap(
                FoldersScreenLayout.notice(for: .nearAIEnrollFailed(label: label), copy: copy, onboarding: nil))
            let offersWallet = line.lowercased().contains("wallet")
            if LinkedChannel.isStaging || label != "near_ai_enroll_commons_unsupported" {
                XCTAssertFalse(offersWallet, "\(label): \(line)")
            } else {
                XCTAssertTrue(offersWallet, "production's line changed: \(line)")
            }
            if LinkedChannel.isStaging, label == "near_ai_enroll_commons_unsupported" {
                XCTAssertEqual(line, "Only Google sign-in is supported.")
            }
        }
    }

    /// No first-run sentence mentions a wallet, on either channel.
    func test_theFirstRunCopyNamesNoWallet() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        XCTAssertFalse(json.lowercased().contains("wallet"))
    }
}
