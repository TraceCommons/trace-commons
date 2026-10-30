import Foundation
import XCTest
@testable import TCShellCore

final class SubscriptionUsageWireTests: XCTestCase {
    func testSettingsPreserveLegacyValuesAndOlderDaemonAbsence() throws {
        let decoder = DaemonDataDecoding.decoder()
        let current = try decoder.decode(DaemonData.Settings.self, from: Data(
            #"{"subscription_usage_codex_enabled":true,"subscription_usage_claude_enabled":false}"#.utf8))
        XCTAssertEqual(current.subscriptionUsageCodexEnabled, true)
        XCTAssertEqual(current.subscriptionUsageClaudeEnabled, false)
        let older = try decoder.decode(DaemonData.Settings.self, from: Data("{}".utf8))
        XCTAssertNil(older.subscriptionUsageCodexEnabled)
        XCTAssertNil(older.subscriptionUsageClaudeEnabled)
    }

    func testPartialUsageKeepsUnknownsAndProviderWindowsSeparate() throws {
        let json = #"{"schema_version":"trace_commons.subscription_usage.v1","providers":[{"provider":"claude","enabled":true,"refreshing":false,"logins":[{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","reason":null,"updated_at":"2026-10-26T10:00:00.123Z","stale":false,"banked_resets":null,"windows":[{"lane":"session","label":null,"used_permille":null,"remaining_text":"—","resets_at":null},{"lane":"weekly","label":null,"used_permille":300,"remaining_text":"70%","resets_at":"2026-10-29T10:00:00Z"},{"lane":"model","label":"Fable","used_permille":800,"remaining_text":"20%","resets_at":"2026-10-29T10:00:00Z"}]}]}]}"#
        let result = try DaemonDataDecoding.decoder().decode(DaemonData.SubscriptionUsage.self, from: Data(json.utf8))
        let login = try XCTUnwrap(result.providers.first?.logins.first)
        XCTAssertNotNil(login.updatedAt)
        XCTAssertNil(login.bankedResets)
        XCTAssertNil(login.refreshing)
        XCTAssertNil(login.windows[0].usedPermille)
        XCTAssertNil(login.windows[0].resetsAt)
        XCTAssertEqual(login.windows[1].usedPermille, 300)
        XCTAssertEqual(login.windows[2].label, "Fable")
    }

    func testLiveClientUsesOnlyBoundedUsageAndProviderSettingMethods() async throws {
        let transport = SubscriptionUsageTransport()
        let client = LiveDaemonClient(transport: transport)
        let answer = try await client.subscriptionUsage(refresh: false, interactive: true)
        XCTAssertTrue(answer.providers.isEmpty)
        _ = try await client.subscriptionUsage(refresh: true, interactive: true, provider: "codex")
        try await client.setSubscriptionUsage(provider: "codex", on: true)
        try await client.setSubscriptionUsage(provider: "claude", on: false)
        let calls = transport.recorded()
        XCTAssertEqual(calls.map(\.0), ["subscription_usage", "subscription_usage", "set_settings", "set_settings"])
        let read = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[0].1.utf8)) as? [String: Bool])
        XCTAssertEqual(read, ["refresh": false, "interactive": false])
        let scoped = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[1].1.utf8)) as? [String: Any])
        XCTAssertEqual(scoped["refresh"] as? Bool, true)
        XCTAssertEqual(scoped["interactive"] as? Bool, true)
        XCTAssertEqual(scoped["provider"] as? String, "codex")
        let set = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[2].1.utf8)) as? [String: Bool])
        XCTAssertEqual(set, ["subscription_usage_codex_enabled": true])
        let claude = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(calls[3].1.utf8)) as? [String: Bool])
        XCTAssertEqual(claude, ["subscription_usage_claude_on": false])
        do {
            try await client.setSubscriptionUsage(provider: "unrecognized", on: true)
            XCTFail("only Codex and Claude have a setting")
        } catch let error as DaemonDataError {
            XCTAssertEqual(error, .daemon(code: "bad_params", message: "subscription-provider-invalid"))
        }
        do {
            _ = try await client.subscriptionUsage(refresh: true, interactive: true, provider: "unrecognized")
            XCTFail("unsupported provider must not launch a probe")
        } catch let error as DaemonDataError {
            XCTAssertEqual(error, .daemon(code: "bad_params", message: "subscription-provider-invalid"))
        }
        XCTAssertEqual(transport.recorded().count, 4)
    }

    func testClaudeRefreshSendsExactLoginAndRejectsMissingOrUnboundedSelectors() async throws {
        let transport = SubscriptionUsageTransport()
        let client = LiveDaemonClient(transport: transport)
        _ = try await client.subscriptionUsage(refresh: true, interactive: true, provider: "claude", login: "claude-profile-123")
        let call = try XCTUnwrap(transport.recorded().first)
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(call.1.utf8)) as? [String: Any])
        XCTAssertEqual(params["provider"] as? String, "claude")
        XCTAssertEqual(params["login"] as? String, "claude-profile-123")
        XCTAssertEqual(params["interactive"] as? Bool, true)
        for login in [nil, "", "line\nfeed", String(repeating: "x", count: 257)] as [String?] {
            do {
                _ = try await client.subscriptionUsage(refresh: true, interactive: true, provider: "claude", login: login)
                XCTFail("Claude must have a bounded exact selector")
            } catch let error as DaemonDataError {
                XCTAssertEqual(error, .daemon(code: "bad_params", message: "subscription-login-invalid"))
            }
        }
        XCTAssertEqual(transport.recorded().count, 1)
    }

    func testPerLoginRefreshingIsOptionalAndIndependent() throws {
        let data = Data(#"{"schema_version":"trace_commons.subscription_usage.v1","providers":[{"provider":"claude","enabled":true,"refreshing":true,"logins":[{"id":"one","label":"One","default_login":true,"state":"unknown","stale":false,"windows":[],"refreshing":true},{"id":"two","label":"Two","default_login":false,"state":"unknown","stale":false,"windows":[],"refreshing":false}]}]}"#.utf8)
        let usage = try DaemonDataDecoding.decoder().decode(DaemonData.SubscriptionUsage.self, from: data)
        XCTAssertEqual(usage.providers[0].logins.map(\.refreshing), [true, false])
    }

    func testDurationIsOptionalPositiveAndPreservesNonstandardWindows() throws {
        let decoder = DaemonDataDecoding.decoder()
        let old = try decoder.decode(DaemonData.SubscriptionUsageWindow.self, from: Data(#"{"lane":"weekly"}"#.utf8))
        XCTAssertNil(old.durationMins)
        for minutes in [540, 1_440] {
            let json = "{\"lane\":\"secondary\",\"duration_mins\":\(minutes)}"
            let window = try decoder.decode(DaemonData.SubscriptionUsageWindow.self, from: Data(json.utf8))
            XCTAssertEqual(window.durationMins, minutes)
        }
        for invalid in ["0", "-1", "null", "true", "\"540\"", "18446744073709551615"] {
            let json = "{\"lane\":\"secondary\",\"duration_mins\":\(invalid)}"
            let window = try decoder.decode(DaemonData.SubscriptionUsageWindow.self, from: Data(json.utf8))
            XCTAssertNil(window.durationMins)
        }
    }

    func testReadingSourceIsOptionalAndToleratesOlderDaemons() throws {
        let decoder = DaemonDataDecoding.decoder()
        let cli = Data(#"{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","stale":false,"windows":[],"source":"cli"}"#.utf8)
        XCTAssertEqual(try decoder.decode(DaemonData.SubscriptionUsageLogin.self, from: cli).source, "cli")
        let none = Data(#"{"id":"claude-default","label":"Claude","default_login":true,"state":"unknown","stale":false,"windows":[],"source":null}"#.utf8)
        XCTAssertNil(try decoder.decode(DaemonData.SubscriptionUsageLogin.self, from: none).source)
        let older = Data(#"{"id":"claude-default","label":"Claude","default_login":true,"state":"unknown","stale":false,"windows":[]}"#.utf8)
        XCTAssertNil(try decoder.decode(DaemonData.SubscriptionUsageLogin.self, from: older).source)
        let malformed = Data(#"{"id":"claude-default","label":"Claude","default_login":true,"state":"observed","stale":false,"windows":[],"source":7}"#.utf8)
        XCTAssertNil(try decoder.decode(DaemonData.SubscriptionUsageLogin.self, from: malformed).source)
    }

    func testInvalidOptionalMeasurementDoesNotDiscardValidNeighbors() throws {
        for invalid in ["true", "\"20\"", "-1", "1001", "null"] {
            let data = Data("{\"lane\":\"weekly\",\"label\":null,\"used_permille\":\(invalid),\"remaining_text\":\"do not infer\",\"resets_at\":\"invalid\"}".utf8)
            let window = try DaemonDataDecoding.decoder().decode(DaemonData.SubscriptionUsageWindow.self, from: data)
            XCTAssertNil(window.usedPermille)
            XCTAssertNil(window.resetsAt)
        }
        let data = Data(#"{"id":"codex-default","label":"Codex","default_login":true,"state":"observed","stale":false,"windows":[],"banked_resets":-1}"#.utf8)
        XCTAssertNil(try DaemonDataDecoding.decoder().decode(DaemonData.SubscriptionUsageLogin.self, from: data).bankedResets)
    }
}

private final class SubscriptionUsageTransport: DaemonTransport, @unchecked Sendable {
    private let lock = NSLock()
    private var calls: [(String, String)] = []
    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        calls.append((method, paramsJSON))
        lock.unlock()
        return method == "subscription_usage"
            ? #"{"id":1,"result":{"schema_version":"trace_commons.subscription_usage.v1","providers":[]}}"#
            : #"{"id":1,"result":{}}"#
    }
    func recorded() -> [(String, String)] {
        lock.lock()
        defer { lock.unlock() }
        return calls
    }
}
