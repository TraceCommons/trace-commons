import Foundation
import XCTest
import TCShellCore
@testable import TraceCommonsApp

@MainActor
final class MonitorNoticeDismissalTests: XCTestCase {
    private func withStore(_ run: (UserDefaults, MonitorNoticeDismissals.Scope) throws -> Void) rethrows {
        let name = "MonitorNoticeDismissalTests." + UUID().uuidString
        let defaults = UserDefaults(suiteName: name)!
        defer { defaults.removePersistentDomain(forName: name) }
        let scope = MonitorNoticeDismissals.Scope(configDirectory: "/private/synthetic/state", channel: "staging", account: "synthetic-account")!
        try run(defaults, scope)
    }

    func testDismissalSurvivesIdenticalPollsNewWindowsAndRestart() {
        withStore { defaults, scope in
            let firstWindow = MonitorNoticeDismissals(defaults: defaults)
            let incident = MonitorNoticeDismissals.Incident(kind: .queueFull)
            firstWindow.dismiss(incident, scope: scope)
            for _ in 0..<3 { firstWindow.reconcile([.queueFull: .active(incident)], scope: scope) }
            XCTAssertTrue(firstWindow.isDismissed(incident, scope: scope))
            let recreated = MonitorNoticeDismissals(defaults: defaults)
            XCTAssertTrue(recreated.isDismissed(incident, scope: scope), "The same account's dismissal must survive a new window or process")
        }
    }

    func testUnknownReadsRetainDismissalButConfirmedRecoveryStartsANewIncident() {
        withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let incident = MonitorNoticeDismissals.Incident(kind: .queueFull)
            store.dismiss(incident, scope: scope)
            store.reconcile([.queueFull: .unknown], scope: scope)
            XCTAssertTrue(store.isDismissed(incident, scope: scope))
            store.reconcile([.queueFull: .clear], scope: nil)
            XCTAssertTrue(store.isDismissed(incident, scope: scope), "Unknown account context cannot clear a known account's marker")
            store.reconcile([.queueFull: .clear], scope: scope)
            store.reconcile([.queueFull: .active(incident)], scope: scope)
            XCTAssertFalse(store.isDismissed(incident, scope: scope), "A recurrence after confirmed recovery must be shown")
        }
    }

    func testEveryAllowedSafeguardCanBeRememberedAndOnlyItsOwnResolutionClearsIt() {
        withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            for kind in MonitorNoticeDismissals.Kind.allCases { store.dismiss(.init(kind: kind), scope: scope) }
            store.reconcile([.witness: .clear], scope: scope)
            for kind in MonitorNoticeDismissals.Kind.allCases {
                XCTAssertEqual(store.isDismissed(.init(kind: kind), scope: scope), kind != .witness)
            }
        }
    }

    func testChangedAutomaticHoldReasonsShowANewIncident() {
        withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let before = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: ["one"])
            let same = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: ["one", "one"])
            let changed = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: ["two"])
            store.dismiss(before, scope: scope)
            XCTAssertTrue(store.isDismissed(same, scope: scope))
            store.reconcile([.automaticHold: .active(changed)], scope: scope)
            XCTAssertFalse(store.isDismissed(changed, scope: scope))
            XCTAssertFalse(store.isDismissed(before, scope: scope), "A changed incident retires the previous dismissal")
        }
    }

    private func status(_ json: String) throws -> DaemonData.Status {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(DaemonData.Status.self, from: Data(json.utf8))
    }

    func testQueueDismissalSurvivesFailuresMaskingAndLowPendingCountsUntilExplicitHealthyRead() throws {
        try withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let queue = MonitorNoticeDismissals.Incident(kind: .queueFull)
            let full = try status(#"{"health":{"last_error_label":"queue-full"},"queue_depth":500}"#)
            store.dismiss(queue, scope: scope)
            for (read, loaded) in [
                (nil as DaemonData.Status?, false),
                (full, false),
                (try status(#"{"health":{"last_error_label":"privacy-filter-canary-failed"},"queue_depth":0}"#), true),
                (try status(#"{"health":{"last_error_label":"witness-saturated"},"queue_depth":0}"#), true),
                (try status(#"{"queue_depth":0}"#), true),
                (full, true)
            ] {
                store.reconcile(MonitorNoticeDismissals.conditions(status: read, loaded: loaded), scope: scope)
                XCTAssertTrue(store.isDismissed(queue, scope: scope), "An unread or masked health slot is not proof of spare queue capacity")
            }
            let healthy = try status(#"{"health":{"last_error_label":null},"queue_depth":0}"#)
            store.reconcile(MonitorNoticeDismissals.conditions(status: healthy, loaded: true), scope: scope)
            XCTAssertFalse(store.isDismissed(queue, scope: scope))
        }
    }

    func testCountsRetryTimesAndOtherHealthLabelsDoNotReopenIndependentSafeguards() throws {
        try withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let before = try status(#"{"health":{"last_error_label":"queue-full"},"daily_budget":{"blocked":true,"blocked_entries":2},"witness_capacity":{"waiting_sessions":3,"next_retry_at":"2026-10-25T12:00:00Z"},"automatic_contribution_held":{"held_sessions":4,"reasons":["one","two"]}}"#)
            let after = try status(#"{"health":{"last_error_label":"privacy-filter-canary-failed"},"daily_budget":{"blocked":true,"blocked_entries":20},"witness_capacity":{"waiting_sessions":30,"next_retry_at":"2026-10-25T13:00:00Z"},"automatic_contribution_held":{"held_sessions":40,"reasons":["two","one","one"]}}"#)
            let first = MonitorNoticeDismissals.conditions(status: before, loaded: true)
            for condition in first.values {
                if case .active(let incident) = condition { store.dismiss(incident, scope: scope) }
            }
            let latest = MonitorNoticeDismissals.conditions(status: after, loaded: true)
            store.reconcile(latest, scope: scope)
            for kind in [MonitorNoticeDismissals.Kind.dailyCap, .witness, .automaticHold] {
                guard case .active(let incident) = latest[kind] else { return XCTFail("The safeguard must still be active") }
                XCTAssertTrue(store.isDismissed(incident, scope: scope))
            }
            let resolved = try status(#"{"daily_budget":{"blocked":false},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#)
            store.reconcile(MonitorNoticeDismissals.conditions(status: resolved, loaded: true), scope: scope)
            for kind in [MonitorNoticeDismissals.Kind.dailyCap, .witness, .automaticHold] {
                guard case .active(let incident) = latest[kind] else { return XCTFail("Missing incident") }
                XCTAssertFalse(store.isDismissed(incident, scope: scope))
            }
            XCTAssertTrue(store.isDismissed(.init(kind: .queueFull), scope: scope), "Other safeguards resolving cannot clear queue-full")
        }
    }

    func testNewDailyQuotaWindowReappearsAcrossRestartWithoutAnObservedUnblockedRead() throws {
        try withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let first = try status(#"{"daily_budget":{"blocked":true,"resets_at":"2026-10-26T00:00:00Z","blocked_entries":2}}"#)
            guard case .active(let incident) = MonitorNoticeDismissals.conditions(status: first, loaded: true)[.dailyCap] else {
                return XCTFail("Expected a blocked daily quota window")
            }
            store.dismiss(incident, scope: scope)
            let restarted = MonitorNoticeDismissals(defaults: defaults)
            let same = try status(#"{"daily_budget":{"blocked":true,"resets_at":"2026-10-26T01:00:00+01:00","blocked_entries":20}}"#)
            let sameConditions = MonitorNoticeDismissals.conditions(status: same, loaded: true)
            restarted.reconcile(sameConditions, scope: scope)
            guard case .active(let sameIncident) = sameConditions[.dailyCap] else { return XCTFail("Missing same-window incident") }
            XCTAssertTrue(restarted.isDismissed(sameIncident, scope: scope), "Formatting and counts do not change the quota window")
            let unreadWindow = try status(#"{"daily_budget":{"blocked":true}}"#)
            let unknown = MonitorNoticeDismissals.conditions(status: unreadWindow, loaded: true)
            restarted.reconcile(unknown, scope: scope)
            guard case .active(let unknownIncident) = unknown[.dailyCap] else { return XCTFail("Missing unbound incident") }
            XCTAssertTrue(restarted.isDismissed(unknownIncident, scope: scope), "A missing window cannot erase a known dismissal")
            let next = try status(#"{"daily_budget":{"blocked":true,"resets_at":"2026-10-27T00:00:00Z"}}"#)
            let nextConditions = MonitorNoticeDismissals.conditions(status: next, loaded: true)
            restarted.reconcile(nextConditions, scope: scope)
            guard case .active(let nextIncident) = nextConditions[.dailyCap] else { return XCTFail("Missing next-window incident") }
            XCTAssertFalse(restarted.isDismissed(nextIncident, scope: scope), "A new quota window must show without observing the midnight clear")
        }
    }

    func testExplicitIndependentClearsOverrideStaleLabelsAndPreserveUnrelatedHealth() throws {
        for kind in [MonitorNoticeDismissals.Kind.dailyCap, .witness, .automaticHold] {
            let clear = try status(#"{"health":{"last_error_label":"LABEL"},"daily_budget":{"blocked":false},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#.replacingOccurrences(of: "LABEL", with: kind.rawValue))
            XCTAssertFalse(TracesStore.safeguards(clear).contains { $0.condition == kind.rawValue }, "A stale label cannot redraw an explicitly resolved safeguard")
            let unknown = try status(#"{"health":{"last_error_label":"LABEL"}}"#.replacingOccurrences(of: "LABEL", with: kind.rawValue))
            XCTAssertTrue(TracesStore.safeguards(unknown).contains { $0.condition == kind.rawValue }, "An absent independent signal must retain its health fallback")
        }
        let other = try status(#"{"health":{"last_error_label":"privacy-filter-canary-failed"},"daily_budget":{"blocked":false},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#)
        XCTAssertTrue(TracesStore.safeguards(other).contains { $0.condition == "privacy-filter-canary-failed" })
    }

    func testScopeSeparatesAccountsConfigurationsAndChannelsWithoutStoringThem() {
        withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let incident = MonitorNoticeDismissals.Incident(kind: .queueFull)
            store.dismiss(incident, scope: scope)
            for other in [
                MonitorNoticeDismissals.Scope(configDirectory: "/private/synthetic/state", channel: "staging", account: "other-account")!,
                MonitorNoticeDismissals.Scope(configDirectory: "/other/state", channel: "staging", account: "synthetic-account")!,
                MonitorNoticeDismissals.Scope(configDirectory: "/private/synthetic/state", channel: "production", account: "synthetic-account")!
            ] { XCTAssertFalse(store.isDismissed(incident, scope: other)) }
            let stored = String(describing: defaults.dictionaryRepresentation())
            XCTAssertFalse(stored.contains("/private/synthetic/state"))
            XCTAssertFalse(stored.contains("synthetic-account"))
            XCTAssertFalse(stored.contains("Near.ai has stopped"))
            XCTAssertFalse(store.isDismissed(incident, scope: nil))
            XCTAssertNil(MonitorNoticeDismissals.Scope(configDirectory: "", channel: "staging", account: "account"))
            XCTAssertNil(MonitorNoticeDismissals.Scope(configDirectory: "/state", channel: "", account: "account"))
            XCTAssertNil(MonitorNoticeDismissals.Scope(configDirectory: "/state", channel: "staging", account: ""))
        }
    }
}
