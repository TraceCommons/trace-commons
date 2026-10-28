import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class SubscriptionUsageTests: XCTestCase {
    private static let now = Date(timeIntervalSince1970: 1_792_000_000)
    private static func snapshot(enabled: Bool = true, refreshing: Bool = false) -> DaemonData.SubscriptionUsage {
        .init(providers: [.init(provider: "codex", enabled: enabled, refreshing: refreshing,
                               logins: enabled ? [.init(id: "codex-default", label: "Codex", defaultLogin: true,
                                                       state: "observed", windows: [
                                                        .init(lane: "weekly", usedPermille: 300, remainingText: "70%",
                                                              resetsAt: now.addingTimeInterval(3600))], bankedResets: 2)] : [])])
    }
    private func uiCopy() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }

    @MainActor
    private func waitForCalls(_ gate: UsageReaderGate, _ count: Int,
                              file: StaticString = #filePath, line: UInt = #line) async {
        let ready = await gate.waitForCalls(count)
        XCTAssertTrue(ready, file: file, line: line)
    }

    @MainActor
    func testNoReaderAndOlderDaemonNeverSubstituteSampleUsage() async {
        let absent = SubscriptionUsageModel()
        await absent.load(refresh: true, interactive: true)
        XCTAssertFalse(absent.isVisible)
        let old = SubscriptionUsageModel(reader: { _, _, _, _ in
            throw DaemonDataError.daemon(code: "unknown_method", message: "unknown-method")
        })
        await old.load()
        XCTAssertFalse(old.isVisible)
        XCTAssertNil(old.shown)
        XCTAssertNil(old.failureKey)
    }

    @MainActor
    func testRefreshesCoalesceAndOnlyExplicitRefreshIsInteractive() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let first = Task { await model.load() }
        await waitForCalls(gate, 1)
        await model.load(refresh: true, interactive: true)
        await model.load(refresh: true, interactive: true)
        let actual1 = await gate.count()
        XCTAssertEqual(actual1, 1)
        await gate.resolve(0, value: Self.snapshot())
        await first.value
        await waitForCalls(gate, 2)
        let actual3 = await gate.flags(0)
        XCTAssertEqual(actual3, [false, false])
        let actual4 = await gate.flags(1)
        XCTAssertEqual(actual4, [true, true])
        await gate.resolve(1, value: Self.snapshot())
        for _ in 0..<50 where model.isRefreshing { await Task.yield() }
        let actual2 = await gate.count()
        XCTAssertEqual(actual2, 2)
    }

    @MainActor
    func testStoppingHidesUsageAndDiscardsLateRead() async {
        let gate = UsageReaderGate(initial: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        }, writer: { _, _ in })
        await model.load()
        let old = Task { await model.load() }
        await waitForCalls(gate, 2)
        let stop = Task { await model.setEnabled("codex", on: false) }
        await waitForCalls(gate, 3)
        XCTAssertEqual(model.shown?.providers.first?.enabled, false)
        XCTAssertEqual(model.shown?.providers.first?.logins, [])
        await gate.resolve(2, value: Self.snapshot(enabled: false))
        await stop.value
        await gate.resolve(1, value: Self.snapshot())
        await old.value
        XCTAssertEqual(model.shown?.providers.first?.enabled, false)
        XCTAssertEqual(model.shown?.providers.first?.logins, [])
        let actual5 = await gate.flags(2)
        XCTAssertEqual(actual5, [false, false])
    }

    @MainActor
    func testPermissionOnlySavesAndReadsMetadataWithoutRequestingSecrets() async {
        let log = UsageRequestLog()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, _ in
            await log.append("read:\(refresh):\(interactive):\(provider ?? "all")")
            return Self.snapshot()
        }, writer: { provider, on in await log.append("set:\(provider):\(on)") })
        await model.setEnabled("codex", on: true)
        let lines = await log.lines()
        XCTAssertEqual(lines, ["set:codex:true", "read:false:false:all"])
        XCTAssertTrue(model.isVisible)
    }

    private static func claudeSnapshot(codexEnabled: Bool = false, refreshing: Bool = false) -> DaemonData.SubscriptionUsage {
        .init(providers: [
            .init(provider: "codex", enabled: codexEnabled),
            .init(provider: "claude", enabled: true, refreshing: refreshing, logins: [
                .init(id: "claude-default", label: "Default", defaultLogin: true, state: "unknown",
                      reason: "not_checked", refreshing: refreshing),
                .init(id: "claude-work", label: "Work", state: "unknown", reason: "not_checked", refreshing: false)])])
    }

    @MainActor
    func testClaudeStartsOnAndCodexOffDoesNotHideIt() async {
        let log = UsageRequestLog()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
            await log.append("read:\(refresh):\(interactive):\(provider ?? "all"):\(login ?? "all")")
            return Self.claudeSnapshot()
        }, writer: { provider, on in await log.append("set:\(provider):\(on)") })
        await model.load()
        XCTAssertTrue(model.isVisible)
        XCTAssertEqual(model.codexEnabled, false)
        XCTAssertEqual(model.supportedProviders.map(\.provider), ["claude"])
        XCTAssertEqual(model.supportedProviders.first?.logins.map(\.id), ["claude-default", "claude-work"])
        XCTAssertEqual(model.claudeEnabled, true)
        let lines = await log.lines()
        XCTAssertEqual(lines, ["read:false:false:all:all"])
        XCTAssertFalse(model.canRefresh(provider: "claude", login: nil))
        XCTAssertFalse(model.canRefresh(provider: "claude", login: "not-discovered"))
        XCTAssertTrue(model.canRefresh(provider: "claude", login: "claude-work"))
        await model.setEnabled("codex", on: false)
        XCTAssertTrue(model.isVisible)
        XCTAssertEqual(model.supportedProviders.map(\.provider), ["claude"])
    }

    /// 2026-10-27 owner: Claude has its own Settings switch. Off saves the
    /// choice and hides Claude at once, without reading anything.
    @MainActor
    func testClaudeSwitchOffSavesAndHidesClaudeWithoutARead() async {
        let log = UsageRequestLog()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
            await log.append("read:\(refresh):\(interactive):\(provider ?? "all"):\(login ?? "all")")
            return Self.claudeSnapshot()
        }, writer: { provider, on in await log.append("set:\(provider):\(on)") })
        await model.load()
        let stopping = Task { await model.setEnabled("claude", on: false) }
        await stopping.value
        let lines = await log.lines()
        XCTAssertEqual(lines.first, "read:false:false:all:all")
        XCTAssertTrue(lines.contains("set:claude:false"))
        XCTAssertFalse(lines.contains { $0.hasPrefix("read:true") })
    }

    @MainActor
    func testADisabledClaudeIsNotShownEvenWithLogins() async {
        let model = SubscriptionUsageModel(reader: { _, _, _, _ in
            .init(providers: [
                .init(provider: "codex", enabled: false),
                .init(provider: "claude", enabled: false, logins: [
                    .init(id: "claude-default", label: "Default", defaultLogin: true, state: "unknown")])])
        }, writer: { _, _ in })
        await model.load()
        XCTAssertEqual(model.claudeEnabled, false)
        XCTAssertFalse(model.isVisible)
        XCTAssertTrue(model.supportedProviders.isEmpty)
    }

    @MainActor
    func testNoClaudeProfilesDoesNotOfferInstallationOrExposeAnEmptyProvider() async {
        let model = SubscriptionUsageModel(reader: { _, _, _, _ in
            .init(providers: [.init(provider: "codex", enabled: false), .init(provider: "claude", enabled: true)])
        })
        await model.load()
        XCTAssertFalse(model.isVisible)
        XCTAssertTrue(model.supportedProviders.isEmpty)
    }

    @MainActor
    func testExplicitClaudeRefreshTargetsOnlySelectedLoginAndDefaultRefreshRemainsCodex() async {
        let gate = UsageReaderGate(initial: Self.claudeSnapshot(codexEnabled: true))
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
            try await gate.read(refresh: refresh, interactive: interactive, provider: provider, login: login)
        })
        await model.load()
        model.refresh(provider: "claude", login: "claude-work")
        await waitForCalls(gate, 2)
        let selected = await gate.scope(1)
        XCTAssertEqual(selected, ["claude", "claude-work"])
        XCTAssertTrue(model.isRefreshing(provider: "claude", login: "claude-work"))
        XCTAssertFalse(model.isRefreshing(provider: "claude", login: "claude-default"))
        XCTAssertFalse(model.isRefreshing(provider: "codex", login: nil))
        await gate.resolve(1, value: Self.claudeSnapshot(codexEnabled: true))
        for _ in 0..<100 where model.isRefreshing { await Task.yield() }
        model.refresh()
        await waitForCalls(gate, 3)
        let defaultScope = await gate.scope(2)
        XCTAssertEqual(defaultScope, ["codex", nil])
        await gate.resolve(2, value: Self.claudeSnapshot(codexEnabled: true))
    }

    @MainActor
    func testQueuedLoginRefreshNeverWidensIntoOtherLoginReads() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
            try await gate.read(refresh: refresh, interactive: interactive, provider: provider, login: login)
        })
        let first = Task { await model.load() }
        await waitForCalls(gate, 1)
        await model.load(refresh: true, interactive: true, provider: "claude", login: "claude-default")
        await model.load(refresh: true, interactive: false, provider: "claude", login: "claude-work")
        await gate.resolve(0, value: Self.claudeSnapshot())
        await first.value
        await waitForCalls(gate, 2)
        let scope = await gate.scope(1)
        XCTAssertEqual(scope, ["claude", "claude-work"])
        let flags = await gate.flags(1)
        XCTAssertEqual(flags, [true, false])
        await gate.resolve(1, value: Self.claudeSnapshot())
    }

    @MainActor
    func testClaudePendingKeychainReadWakesObserverAndKeepsOnlySelectedSpinner() async {
        let gate = UsageReaderGate(initial: Self.claudeSnapshot())
        let model = SubscriptionUsageModel(reader: { refresh, interactive, provider, login in
            try await gate.read(refresh: refresh, interactive: interactive, provider: provider, login: login)
        }, activePollInterval: .milliseconds(1))
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        for _ in 0..<100 { await Task.yield() }
        model.refresh(provider: "claude", login: "claude-default")
        await waitForCalls(gate, 2)
        await gate.resolve(1, value: Self.claudeSnapshot(refreshing: true))
        await waitForCalls(gate, 3)
        XCTAssertTrue(model.isRefreshing(provider: "claude", login: "claude-default"))
        XCTAssertFalse(model.isRefreshing(provider: "claude", login: "claude-work"))
        XCTAssertFalse(model.canRefresh(provider: "claude", login: "claude-work"))
        let metadataFlags = await gate.flags(2)
        let metadataScope = await gate.scope(2)
        XCTAssertEqual(metadataFlags, [false, false])
        XCTAssertEqual(metadataScope, [nil, nil])
        observing.cancel()
        await gate.resolve(2, value: Self.claudeSnapshot())
        await observing.value
    }

    @MainActor
    func testAllOffHasNoInsightsOfferButRetainsSettingsPermissionValue() async {
        let model = SubscriptionUsageModel(reader: { _, _, _, _ in Self.snapshot(enabled: false) }, writer: { _, _ in })
        await model.load()
        XCTAssertEqual(model.codexEnabled, false)
        XCTAssertTrue(model.canChange)
        XCTAssertFalse(model.isVisible)
        XCTAssertTrue(model.supportedProviders.isEmpty)
        model.refresh()
        for _ in 0..<100 { await Task.yield() }
        XCTAssertFalse(model.isRefreshing)
    }

    @MainActor
    func testExplicitRefreshWakesIdleObserverAndPollsUntilProbeCompletes() async {
        let gate = UsageReaderGate(initial: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        }, activePollInterval: .milliseconds(1))
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        // Allow the initial metadata reply to settle into the 60-second idle wait.
        for _ in 0..<100 { await Task.yield() }
        XCTAssertFalse(model.isRefreshing)
        model.refresh()
        await waitForCalls(gate, 2)
        let refreshFlags = await gate.flags(1)
        XCTAssertEqual(refreshFlags, [true, true])
        await gate.resolve(1, value: Self.snapshot(refreshing: true))
        await waitForCalls(gate, 3)
        let pollingFlags = await gate.flags(2)
        XCTAssertEqual(pollingFlags, [false, false])
        await gate.resolve(2, value: Self.snapshot())
        for _ in 0..<100 { await Task.yield() }
        XCTAssertFalse(model.isRefreshing)
        observing.cancel()
        await observing.value
    }

    @MainActor
    func testSettingsPermissionChangeWakesSharedObserverWithoutAProbe() async {
        let gate = UsageReaderGate(initial: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        }, writer: { _, _ in })
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        for _ in 0..<100 { await Task.yield() }
        let stopping = Task { await model.setEnabled("codex", on: false) }
        await waitForCalls(gate, 2)
        XCTAssertFalse(model.isVisible)
        XCTAssertTrue(model.supportedProviders.isEmpty)
        await gate.resolve(1, value: Self.snapshot(enabled: false))
        await stopping.value
        await waitForCalls(gate, 3)
        for index in 0..<3 {
            let flags = await gate.flags(index)
            XCTAssertEqual(flags, [false, false])
        }
        await gate.resolve(2, value: Self.snapshot(enabled: false))
        observing.cancel()
        await observing.value
        XCTAssertFalse(model.isVisible)
    }

    @MainActor
    func testLeavingOverviewDiscardsAReadThatIgnoresCancellation() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        observing.cancel()
        await gate.resolve(0, value: Self.snapshot())
        await observing.value
        XCTAssertNil(model.shown)
        XCTAssertFalse(model.isRefreshing)
    }

    @MainActor
    func testLeavingOverviewDropsQueuedInteractiveRefresh() async {
        let gate = UsageReaderGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await gate.read(refresh: refresh, interactive: interactive)
        })
        let observing = Task { await model.observe() }
        await waitForCalls(gate, 1)
        await model.load(refresh: true, interactive: true)
        observing.cancel()
        await gate.resolve(0, value: Self.snapshot())
        await observing.value
        for _ in 0..<1_000 { await Task.yield() }
        let count = await gate.count()
        // Resolve any unexpected extra read before asserting, so a regression
        // does not leave a suspended checked continuation behind.
        for index in 1..<max(1, count) { await gate.resolve(index, value: Self.snapshot()) }
        XCTAssertEqual(count, 1)
        XCTAssertNil(model.shown)
    }

    @MainActor
    func testPermissionFinishingAfterNavigationDoesNotPublishOrStartAProbe() async {
        let reader = UsageReaderGate()
        let writer = UsageWriterGate()
        let model = SubscriptionUsageModel(reader: { refresh, interactive, _, _ in
            try await reader.read(refresh: refresh, interactive: interactive)
        }, writer: { _, _ in await writer.write() })
        let observing = Task { await model.observe() }
        await waitForCalls(reader, 1)
        let enabling = Task { await model.setEnabled("codex", on: true) }
        let writerStarted = await writer.waitForCall()
        XCTAssertTrue(writerStarted)
        observing.cancel()
        for _ in 0..<1_000 { await Task.yield() }
        await writer.resolve()
        for _ in 0..<1_000 { await Task.yield() }
        let count = await reader.count()
        for index in 1..<max(1, count) { await reader.resolve(index, value: Self.snapshot()) }
        await enabling.value
        await reader.resolve(0, value: Self.snapshot())
        await observing.value
        XCTAssertEqual(count, 1)
        XCTAssertTrue(model.changingProviders.isEmpty)
        XCTAssertNil(model.shown)
    }

    @MainActor
    func testFailedReadDropsUnmarkedEarlierMeasurements() async {
        let values = UsageFailureReader(value: Self.snapshot())
        let model = SubscriptionUsageModel(reader: { _, _, _, _ in try await values.read() })
        await model.load()
        XCTAssertNotNil(model.shown)
        await model.load()
        XCTAssertNil(model.shown)
        XCTAssertEqual(model.failureKey, "analytics_subscriptions_unavailable")
    }

    func testWordsPreserveUnknownWindowBankedZeroAndScopedFable() throws {
        let copy = try uiCopy()
        let login = DaemonData.SubscriptionUsageLogin(id: "claude-default", label: "Claude", defaultLogin: true,
            state: "observed", updatedAt: Self.now, windows: [
                .init(lane: "session", usedPermille: nil, remainingText: "100%"),
                .init(lane: "weekly", usedPermille: 400, remainingText: "60%", resetsAt: Self.now.addingTimeInterval(60)),
                .init(lane: "model", label: "Fable", usedPermille: 800, remainingText: "20%")], bankedResets: 0)
        let words = InsightsSubscriptionWords.login(login, provider: "claude", now: Self.now, copy: copy)
        XCTAssertEqual(words.windows.map(\.label), ["Session", "Weekly", "Fable"])
        XCTAssertEqual(words.windows.map(\.remaining), ["—", "60% remaining", "20% remaining"])
        XCTAssertEqual(words.windows.map(\.remainingPermille), [nil, 600, 200])
        XCTAssertEqual(words.banked, "0 available")
        XCTAssertNil(words.windows.first?.reset)
        XCTAssertNotNil(words.windows[1].reset)
    }

    func testClaudeCaptureContextIsNotAnErrorOrRetryAndResetExpiryStillApplies() throws {
        let login = DaemonData.SubscriptionUsageLogin(id: "claude-default", label: "Claude", state: "observed",
            reason: "captured_on_refresh", updatedAt: Self.now, stale: true, windows: [
                .init(lane: "weekly", usedPermille: 300, remainingText: "70%", resetsAt: Self.now.addingTimeInterval(60)),
                .init(lane: "session", usedPermille: 100, remainingText: "90%", resetsAt: Self.now)])
        let words = InsightsSubscriptionWords.login(login, provider: "claude", now: Self.now, copy: try uiCopy())
        XCTAssertNil(words.state)
        XCTAssertNil(words.freshness)
        XCTAssertEqual(words.windows.map(\.remaining), ["70% remaining", "—"])
        XCTAssertNotNil(words.checked)
    }

    func testClaudeRecoveryPreservesNativeSessionAndExplainsScopeAndKeychainFailures() throws {
        let copy = try uiCopy()
        for (reason, key) in [
            ("auth_expired", "claude_expired"), ("credentials_expired", "claude_expired"),
            ("auth_missing", "claude_auth_missing"), ("scope_missing", "claude_scope"),
            ("keychain_needs_permission", "claude_keychain_denied"),
            ("credential_read_timeout", "claude_keychain_pending"), ("auth_changed", "claude_auth_changed"),
            ("request_timeout", "request_timeout"), ("rate_limited", "rate_limited")
        ] {
            let login = DaemonData.SubscriptionUsageLogin(id: "claude-default", label: "Claude", state: "unknown", reason: reason)
            let words = InsightsSubscriptionWords.login(login, provider: "claude", now: Self.now, copy: copy)
            XCTAssertEqual(words.state, copy["analytics_subscriptions_" + key])
            XCTAssertFalse(words.state?.isEmpty ?? true)
        }
        let expired = try XCTUnwrap(copy["analytics_subscriptions_claude_expired"])
        XCTAssertTrue(expired.contains("Open Claude Code"))
        XCTAssertTrue(expired.contains("Refresh"))
        XCTAssertFalse(expired.lowercased().contains("sign in"))
    }

    func testPermanentFailuresExplainTheCauseAndNativeBusyHasSpecificRecovery() throws {
        let copy = try uiCopy()
        for reason in ["base_url_refused", "invalid_response", "no_logins", "native_tool_running", "native_storage_unsupported"] {
            let login = DaemonData.SubscriptionUsageLogin(id: "codex-default", label: "Codex", defaultLogin: true,
                state: "unknown", reason: reason)
            let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Self.now, copy: copy)
            XCTAssertEqual(words.state, InsightsOverviewWords.fill(copy["analytics_subscriptions_" + reason] ?? "", ["harness": "Codex"]))
            XCTAssertFalse(words.state?.isEmpty ?? true)
            if reason != "native_tool_running" { XCTAssertFalse(words.state?.contains("Refresh") ?? true) }
            XCTAssertEqual(words.banked, "—")
        }
    }

    func testGenericFailureCopyNamesTheSelectedProvider() throws {
        let copy = try uiCopy()
        for provider in ["claude", "codex"] {
            let name = try XCTUnwrap(copy["analytics_subscriptions_provider_" + provider])
            for reason in ["invalid_response", "no_logins"] {
                let login = DaemonData.SubscriptionUsageLogin(id: provider + "-default", label: name,
                    state: "unknown", reason: reason)
                let words = InsightsSubscriptionWords.login(login, provider: provider, now: Self.now, copy: copy)
                XCTAssertEqual(words.state, InsightsOverviewWords.fill(copy["analytics_subscriptions_" + reason] ?? "", ["harness": name]))
                XCTAssertTrue(words.state?.contains(name) ?? false)
                XCTAssertFalse(words.state?.contains("{harness}") ?? true)
            }
        }
    }

    func testFailedRefreshKeepsCachedAgeAndSpecificRecoveryVisibleTogether() throws {
        let copy = try uiCopy()
        for reason in ["native_tool_running", "invalid_response", "base_url_refused", "native_storage_unsupported"] {
            let login = DaemonData.SubscriptionUsageLogin(id: "codex", label: "Codex", state: "observed", reason: reason,
                stale: true, windows: [.init(lane: "weekly", usedPermille: 300, remainingText: "70%")])
            let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Self.now, copy: copy)
            XCTAssertEqual(words.freshness, "Last reading.")
            XCTAssertEqual(words.state, InsightsOverviewWords.fill(copy["analytics_subscriptions_" + reason] ?? "", ["harness": "Codex"]))
            XCTAssertEqual(words.windows.first?.remaining, "70% remaining")
        }
    }

    func testNonstandardAndUnknownWindowsRemainAccurateAndDistinct() throws {
        let login = DaemonData.SubscriptionUsageLogin(id: "codex-default", label: "Codex", state: "observed", windows: [
            .init(lane: "primary", usedPermille: 200, remainingText: "80%", durationMins: 540),
            .init(lane: "secondary", usedPermille: 300, remainingText: "70%", durationMins: 1_440),
            .init(lane: "secondary", usedPermille: nil),
            .init(lane: "secondary", usedPermille: 500, remainingText: "50%"),
            .init(lane: "session", usedPermille: 0, remainingText: "100%", durationMins: 300),
            .init(lane: "weekly", usedPermille: 1_000, remainingText: "0%", durationMins: 10_080),
            .init(lane: "primary", durationMins: 1),
            .init(lane: "secondary", durationMins: 90)])
        let copy = try uiCopy()
        let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Self.now, copy: copy)
        XCTAssertEqual(words.windows.map(\.label), ["9 hours", "1 day", copy["analytics_subscriptions_window_unknown"]!,
            copy["analytics_subscriptions_window_unknown"]!, "5 hours", "Weekly", "1 minute", "90 minutes"])
        XCTAssertEqual(Set(words.windows.map(\.id)).count, 8)
        XCTAssertEqual(words.windows[2].remaining, "—")
        XCTAssertEqual(words.windows[3].remaining, "50% remaining")
    }

    func testCodexConsentStaysInSettingsAndClaudeDisclosurePrecedesScopedOverviewActions() throws {
        let settings = try SettingsParityTests.text("Views/Settings/SubscriptionUsageSettingsSection.swift")
        // The usage cards replaced the limits card on Overview (product
        // owner, 2026-10-28); each measured login is a card there.
        let overview = try SettingsParityTests.text("Views/InsightsUsageCards.swift")
        let content = try SettingsParityTests.text("Views/Settings/GlassSettingsContent.swift")
        let disclosure = try XCTUnwrap(settings.range(of: "detail(\"analytics_subscriptions_codex_detail\")"))
        let permission = try XCTUnwrap(settings.range(of: "Toggle(text(\"analytics_subscriptions_setting_codex\")"))
        XCTAssertLessThan(disclosure.lowerBound, permission.lowerBound)
        let claudeDisclosure = try XCTUnwrap(settings.range(of: "detail(\"analytics_subscriptions_claude_detail\")"))
        let claudeSwitch = try XCTUnwrap(settings.range(of: "Toggle(text(\"analytics_subscriptions_setting_claude\")"))
        XCTAssertLessThan(claudeDisclosure.lowerBound, claudeSwitch.lowerBound)
        XCTAssertTrue(content.contains("SubscriptionUsageSettingsSection()"))
        XCTAssertTrue(overview.contains("InsightsLimitsRows.rows(subscriptions?.supportedProviders ?? []"))
        XCTAssertFalse(overview.contains("setEnabled"))
        XCTAssertTrue(overview.contains("InsightsInfo(row.account, [text(\"analytics_subscriptions_claude_detail\")])"))
        XCTAssertFalse(overview.contains("Text(text(\"analytics_subscriptions_claude_detail\"))"))
        XCTAssertTrue(overview.contains("subscriptions?.refresh(provider: row.provider, login: row.loginId)"))
        XCTAssertTrue(overview.contains("analytics_subscriptions_refresh_login_accessibility"))
        XCTAssertFalse(overview.contains("model.refresh()"))
        XCTAssertTrue(settings.contains("Toggle(text(\"analytics_subscriptions_setting_claude\")"))
        XCTAssertTrue(settings.contains("model.setEnabled(\"claude\", on: on)"))
        XCTAssertFalse(settings.contains("model.refresh()"))
        XCTAssertFalse(settings.contains(".observe()"))
        XCTAssertTrue(settings.contains("app.subscriptionUsageModel"))
        let monitor = try SettingsParityTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(monitor.contains("subscriptions: model.subscriptionUsageModel"))
    }

    /// 2026-10-27 owner review: the Insights card drops its title and the
    /// Claude paragraphs, keeps the keychain note behind an info button, and
    /// asks people to Refresh. Settings keeps the title on its own card.
    /// The usage cards (product owner, 2026-10-28) keep all of it.
    func testInsightsCardHasNoTitleOrClaudeParagraphsAndAsksForRefresh() throws {
        let overview = try SettingsParityTests.text("Views/InsightsUsageCards.swift")
        let settings = try SettingsParityTests.text("Views/Settings/SubscriptionUsageSettingsSection.swift")
        XCTAssertFalse(overview.contains("analytics_subscriptions_title"))
        XCTAssertFalse(overview.contains("analytics_subscriptions_claude_captured"))
        XCTAssertTrue(overview.contains("Text(text(\"analytics_subscriptions_refresh_hint\"))"))
        // Kristi's #1687 review: no "Refresh..." line without a Refresh button.
        let gate = try XCTUnwrap(overview.range(of: "if let subscriptions, !logins.isEmpty, !subscriptions.isRefreshing {"))
        let hint = try XCTUnwrap(overview.range(of: "Text(text(\"analytics_subscriptions_refresh_hint\"))"))
        XCTAssertLessThan(gate.lowerBound, hint.lowerBound)
        XCTAssertTrue(settings.contains("analytics_subscriptions_title"))
        let copy = try uiCopy()
        XCTAssertEqual(copy["analytics_subscriptions_refresh_hint"], "Refresh to see your latest limits.")
        XCTAssertNil(copy["analytics_subscriptions_claude_captured"])
    }

    func testAnExpiredWindowBecomesUnknownWithoutAFullAllowanceOrNewReset() throws {
        let login = DaemonData.SubscriptionUsageLogin(id: "login", label: "Saved subscription", state: "observed",
            stale: true, windows: [.init(lane: "weekly", usedPermille: 900, remainingText: "10%", resetsAt: Self.now)])
        let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Self.now, copy: try uiCopy())
        XCTAssertEqual(words.windows.first?.remaining, "—")
        XCTAssertNil(words.windows.first?.reset)
        XCTAssertNil(words.windows.first?.remainingPermille)
        XCTAssertEqual(words.banked, "—")
        XCTAssertNotNil(words.freshness)
    }

    @MainActor
    func testSubscriptionCardRendersAtNarrowOverviewWidth() async throws {
        guard let target = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] else {
            throw XCTSkip("Opt-in native render")
        }
        let snapshot = DaemonData.SubscriptionUsage(providers: [
            .init(provider: "claude", enabled: true, logins: [
                .init(id: "claude-default", label: "Claude", defaultLogin: true,
                      state: "observed", updatedAt: Date(), windows: [
                        .init(lane: "session", usedPermille: 300, remainingText: "70%", durationMins: 300),
                        .init(lane: "weekly", usedPermille: 450, remainingText: "55%", durationMins: 10_080),
                        .init(lane: "model", label: "Fable", usedPermille: nil)]),
                .init(id: "claude-work", label: "Work", state: "unknown", reason: "not_checked")]),
            .init(provider: "codex", enabled: true, logins: [.init(id: "codex-default", label: "Codex", defaultLogin: true,
                state: "observed", updatedAt: Date(), windows: [
                    .init(lane: "primary", usedPermille: 300, remainingText: "70%",
                          resetsAt: Date().addingTimeInterval(3_600), durationMins: 540),
                    .init(lane: "secondary", usedPermille: 450, remainingText: "55%",
                          resetsAt: Date().addingTimeInterval(86_400), durationMins: 1_440),
                    .init(lane: "secondary", usedPermille: nil)], bankedResets: nil)])])
        let model = SubscriptionUsageModel(reader: { _, _, _, _ in snapshot }, writer: { _, _ in })
        await model.load()
        let sharedCopy = try uiCopy()
        XCTAssertFalse(sharedCopy["analytics_subscriptions_title"]?.isEmpty ?? true)
        _ = NSApplication.shared
        try render(InsightsUsageCards(subscriptions: model, nearAiCredit: nil, copy: sharedCopy),
                   to: target, name: "insights-subscription-limits.png")
        try render(SubscriptionUsageSettingsCard(model: model, copy: sharedCopy),
                   to: target, name: "settings-subscription-usage.png")
    }

    @MainActor
    private func render(_ content: some View, to target: String, name: String) throws {
        let view = content.padding(16).frame(width: 408).background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: view)
        let size = hosting.fittingSize
        let bounds = NSRect(x: 0, y: 0, width: 408, height: max(600, size.height))
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let representation = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        let directory = URL(fileURLWithPath: target)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try png.write(to: directory.appendingPathComponent(name))
        XCTAssertGreaterThan(png.count, 1000)
    }
}

private actor UsageReaderGate {
    private var requests: [(Bool, Bool, String?, String?)] = []
    private var continuations: [Int: CheckedContinuation<DaemonData.SubscriptionUsage, Error>] = [:]
    private var initial: DaemonData.SubscriptionUsage?
    init(initial: DaemonData.SubscriptionUsage? = nil) { self.initial = initial }
    func read(refresh: Bool, interactive: Bool, provider: String? = nil, login: String? = nil) async throws -> DaemonData.SubscriptionUsage {
        let index = requests.count
        requests.append((refresh, interactive, provider, login))
        if let initial { self.initial = nil; return initial }
        return try await withCheckedThrowingContinuation { continuations[index] = $0 }
    }
    func count() -> Int { requests.count }
    func flags(_ index: Int) -> [Bool] { [requests[index].0, requests[index].1] }
    func scope(_ index: Int) -> [String?] { [requests[index].2, requests[index].3] }
    func waitForCalls(_ count: Int) async -> Bool {
        for _ in 0..<10_000 {
            if requests.count >= count { return true }
            await Task.yield()
        }
        return false
    }
    func resolve(_ index: Int, value: DaemonData.SubscriptionUsage) { continuations.removeValue(forKey: index)?.resume(returning: value) }
}

private actor UsageRequestLog {
    private var values: [String] = []
    func append(_ value: String) { values.append(value) }
    func lines() -> [String] { values }
}

private actor UsageFailureReader {
    private var value: DaemonData.SubscriptionUsage?
    init(value: DaemonData.SubscriptionUsage) { self.value = value }
    func read() throws -> DaemonData.SubscriptionUsage {
        if let value { self.value = nil; return value }
        throw DaemonDataError.unreachable
    }
}

private actor UsageWriterGate {
    private var continuation: CheckedContinuation<Void, Never>?
    private var started = false
    func write() async {
        started = true
        await withCheckedContinuation { continuation = $0 }
    }
    func waitForCall() async -> Bool {
        for _ in 0..<10_000 {
            if started { return true }
            await Task.yield()
        }
        return false
    }
    func resolve() { continuation?.resume(); continuation = nil }
}
