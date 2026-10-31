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

    func testQueueDismissalSurvivesHealthResetsCapacityChurnAndRestartUntilQueueEmpties() throws {
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
                (try status(#"{"health":{"last_error_label":null},"queue_depth":0}"#), true),
                (try status(#"{"health":{"last_error_label":null},"queue_at_capacity":true,"queue_capacity_used":500}"#), true),
                (try status(#"{"health":{"last_error_label":null},"queue_at_capacity":false,"queue_capacity_used":499}"#), true),
                (try status(#"{"queue_at_capacity":false}"#), true),
                (try status(#"{"queue_at_capacity":false,"queue_capacity_used":-1}"#), true),
                (full, true)
            ] {
                store.reconcile(MonitorNoticeDismissals.conditions(status: read, loaded: loaded), scope: scope)
                XCTAssertTrue(store.isDismissed(queue, scope: scope), "The same backlog must remain dismissed across health resets and capacity churn")
            }
            let restarted = MonitorNoticeDismissals(defaults: defaults)
            XCTAssertTrue(restarted.isDismissed(queue, scope: scope))
            let empty = try status(#"{"health":{"last_error_label":null},"queue_at_capacity":false,"queue_capacity_used":0}"#)
            restarted.reconcile(MonitorNoticeDismissals.conditions(status: empty, loaded: true), scope: scope)
            XCTAssertFalse(restarted.isDismissed(queue, scope: scope))
            restarted.reconcile(MonitorNoticeDismissals.conditions(status: full, loaded: true), scope: scope)
            XCTAssertFalse(restarted.isDismissed(queue, scope: scope), "A new backlog after the queue empties may show its warning")
        }
    }

    func testQueueConditionUsesCapacityEvidenceWithLegacyFallback() throws {
        let incident = MonitorNoticeDismissals.Incident(kind: .queueFull)
        let cases: [(String, MonitorNoticeDismissals.Condition)] = [
            (#"{"queue_at_capacity":true,"health":{"last_error_label":null}}"#, .active(incident)),
            (#"{"queue_at_capacity":false,"queue_capacity_used":0,"health":{"last_error_label":"queue-full"}}"#, .clear),
            (#"{"queue_at_capacity":false,"queue_capacity_used":499,"health":{"last_error_label":"queue-full"}}"#, .unknown),
            (#"{"health":{"last_error_label":"queue-full"}}"#, .active(incident)),
            (#"{"health":{"last_error_label":null}}"#, .unknown),
            (#"{"queue_at_capacity":false}"#, .unknown),
            (#"{"queue_at_capacity":false,"queue_capacity_used":-1}"#, .unknown)
        ]
        for (json, expected) in cases {
            XCTAssertEqual(MonitorNoticeDismissals.conditions(status: try status(json), loaded: true)[.queueFull], expected, json)
        }
    }

    func testCountsRetryTimesAndOtherHealthLabelsDoNotReopenIndependentSafeguards() throws {
        try withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let before = try status(#"{"health":{"last_error_label":"queue-full"},"witness_capacity":{"waiting_sessions":3,"next_retry_at":"2026-10-25T12:00:00Z"},"automatic_contribution_held":{"held_sessions":4,"reasons":["one","two"]}}"#)
            let after = try status(#"{"health":{"last_error_label":"privacy-filter-canary-failed"},"witness_capacity":{"waiting_sessions":30,"next_retry_at":"2026-10-25T13:00:00Z"},"automatic_contribution_held":{"held_sessions":40,"reasons":["two","one","one"]}}"#)
            let first = MonitorNoticeDismissals.conditions(status: before, loaded: true)
            for condition in first.values {
                if case .active(let incident) = condition { store.dismiss(incident, scope: scope) }
            }
            let latest = MonitorNoticeDismissals.conditions(status: after, loaded: true)
            store.reconcile(latest, scope: scope)
            for kind in [MonitorNoticeDismissals.Kind.witness, .automaticHold] {
                guard case .active(let incident) = latest[kind] else { return XCTFail("The safeguard must still be active") }
                XCTAssertTrue(store.isDismissed(incident, scope: scope))
            }
            let resolved = try status(#"{"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#)
            store.reconcile(MonitorNoticeDismissals.conditions(status: resolved, loaded: true), scope: scope)
            for kind in [MonitorNoticeDismissals.Kind.witness, .automaticHold] {
                guard case .active(let incident) = latest[kind] else { return XCTFail("Missing incident") }
                XCTAssertFalse(store.isDismissed(incident, scope: scope))
            }
            XCTAssertTrue(store.isDismissed(.init(kind: .queueFull), scope: scope), "Other safeguards resolving cannot clear queue-full")
        }
    }

    /// The daily upload limit is retired: a blocked `daily_budget` from an
    /// older daemon opens no safeguard and no dismissable notice.
    func testTheRetiredDailyBudgetIsNotASafeguard() throws {
        XCTAssertNil(MonitorNoticeDismissals.Kind(rawValue: "daily-cap-reached"))
        let blocked = try status(#"{"daily_budget":{"blocked":true,"blocked_entries":2},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#)
        let conditions = MonitorNoticeDismissals.conditions(status: blocked, loaded: true)
        XCTAssertFalse(conditions.values.contains { if case .active = $0 { return true } else { return false } })
        XCTAssertFalse(TracesStore.safeguards(blocked).contains { $0.condition == "daily-cap-reached" })
    }

    func testExplicitIndependentClearsOverrideStaleLabelsAndPreserveUnrelatedHealth() throws {
        for kind in [MonitorNoticeDismissals.Kind.witness, .automaticHold] {
            let clear = try status(#"{"health":{"last_error_label":"LABEL"},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#.replacingOccurrences(of: "LABEL", with: kind.rawValue))
            XCTAssertFalse(TracesStore.safeguards(clear).contains { $0.condition == kind.rawValue }, "A stale label cannot redraw an explicitly resolved safeguard")
            let unknown = try status(#"{"health":{"last_error_label":"LABEL"}}"#.replacingOccurrences(of: "LABEL", with: kind.rawValue))
            XCTAssertTrue(TracesStore.safeguards(unknown).contains { $0.condition == kind.rawValue }, "An absent independent signal must retain its health fallback")
        }
        let other = try status(#"{"health":{"last_error_label":"privacy-filter-canary-failed"},"witness_capacity":{"waiting_sessions":0},"automatic_contribution_held":{"held_sessions":0}}"#)
        XCTAssertTrue(TracesStore.safeguards(other).contains { $0.condition == "privacy-filter-canary-failed" })
    }

    func testMissingOrEmptyHoldReasonsRetainTheKnownIncidentAndBindAnUnboundDismissal() throws {
        try withStore { defaults, scope in
            let store = MonitorNoticeDismissals(defaults: defaults)
            let known = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: ["one"])
            store.dismiss(known, scope: scope)
            for json in [
                #"{"automatic_contribution_held":{"held_sessions":4}}"#,
                #"{"automatic_contribution_held":{"held_sessions":40,"reasons":[]}}"#
            ] {
                let observed = MonitorNoticeDismissals.conditions(status: try status(json), loaded: true)
                store.reconcile(observed, scope: scope)
                guard case .active(let incident) = observed[.automaticHold] else { return XCTFail("Missing active hold") }
                XCTAssertTrue(store.isDismissed(incident, scope: scope))
                XCTAssertTrue(store.isDismissed(known, scope: scope), "Unread reasons must retain the previous known identity")
            }
            store.reconcile([.automaticHold: .clear], scope: scope)
            let unbound = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: nil)
            store.dismiss(unbound, scope: scope)
            store.reconcile([.automaticHold: .active(known)], scope: scope)
            XCTAssertTrue(store.isDismissed(known, scope: scope), "The first known reason set binds an earlier dismissal")
            let changed = MonitorNoticeDismissals.Incident(kind: .automaticHold, reasons: ["two"])
            store.reconcile([.automaticHold: .active(changed)], scope: scope)
            XCTAssertFalse(store.isDismissed(changed, scope: scope), "A known material change still starts a new incident")
        }
    }

    func testScopeTransitionsDiscardOnlyUnscopedSafeguardCloses() {
        let first = MonitorNoticeDismissals.Scope(configDirectory: "/state", channel: "staging", account: "first")!
        let second = MonitorNoticeDismissals.Scope(configDirectory: "/state", channel: "staging", account: "second")!
        let other: Set<String> = ["health:Other warning", "arming-offer:project", "action-notice"]
        let closed = other.union(MonitorNoticeDismissals.Kind.allCases.map(\.closeID))
        XCTAssertEqual(MonitorNotices.closedAfterScopeChange(closed, from: nil, to: first), other,
                       "A close before identity was known must not hide the identified account's safeguard")
        XCTAssertEqual(MonitorNotices.closedAfterScopeChange(closed, from: first, to: second), other)
        XCTAssertEqual(MonitorNotices.closedAfterScopeChange(closed, from: first, to: first), closed)
        XCTAssertEqual(MonitorNotices.closedAfterScopeChange(closed, from: nil, to: nil), closed)
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
