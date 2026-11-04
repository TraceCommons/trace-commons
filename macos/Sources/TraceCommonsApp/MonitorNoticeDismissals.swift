import CryptoKit
import Foundation
import Observation
import TCShellCore

/// Local presentation choices only. Auth, consent and action acknowledgements
/// remain owned by their existing daemon/model paths.
@Observable @MainActor
final class MonitorNoticeDismissals {
    static let shared = MonitorNoticeDismissals()

    enum Kind: String, CaseIterable {
        case queueFull = "queue-full"
        case witness = "witness-saturated"
        case automaticHold = "automatic-contribution-held"

        var closeID: String { "health:" + rawValue }
        var contextMayBeUnknown: Bool { self == .automaticHold }
    }

    struct Scope: Equatable {
        let storageKey: String

        init?(configDirectory: String, channel: String, account: String) {
            guard !configDirectory.isEmpty, !channel.isEmpty, !account.isEmpty else { return nil }
            storageKey = "monitor.noticeDismissals.v1." + MonitorNoticeDismissals.digest([channel, configDirectory, account])
        }
    }

    struct Incident: Equatable {
        let kind: Kind
        let revision: String
        let unboundContext: Bool

        init(kind: Kind, reasons: [String]? = []) {
            self.kind = kind
            let materialReasons = Set((reasons ?? []).filter { !$0.isEmpty }).sorted()
            unboundContext = kind == .automaticHold && materialReasons.isEmpty
            revision = MonitorNoticeDismissals.digest([kind.rawValue] + materialReasons)
        }
    }

    enum Condition: Equatable {
        case active(Incident)
        case clear
        case unknown
    }

    struct Snapshot: Equatable {
        let scope: Scope?
        let conditions: [Kind: Condition]
    }

    @ObservationIgnored private let defaults: UserDefaults
    private var revision = 0

    init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    func isDismissed(_ incident: Incident, scope: Scope?) -> Bool {
        _ = revision
        guard let scope else { return false }
        guard let saved = records(scope)[incident.kind.rawValue] else { return false }
        if incident.kind.contextMayBeUnknown,
           incident.unboundContext || saved == Incident(kind: incident.kind).revision { return true }
        return saved == incident.revision
    }

    func dismiss(_ incident: Incident, scope: Scope?) {
        guard let scope else { return }
        var values = records(scope)
        if incident.unboundContext, values[incident.kind.rawValue] != nil { return }
        values[incident.kind.rawValue] = incident.revision
        write(values, scope: scope)
    }

    func reconcile(_ conditions: [Kind: Condition], scope: Scope?) {
        guard let scope else { return }
        var values = records(scope)
        let previous = values
        for (kind, condition) in conditions {
            switch condition {
            case .clear: values[kind.rawValue] = nil
            case .active(let incident):
                guard !incident.unboundContext else { continue }
                if kind.contextMayBeUnknown, values[kind.rawValue] == Incident(kind: kind).revision {
                    // Bind a dismissal made before its hold reasons were
                    // known; unread context never resets it.
                    values[kind.rawValue] = incident.revision
                } else if let old = values[kind.rawValue], old != incident.revision {
                    values[kind.rawValue] = nil
                }
            case .unknown: break
            }
        }
        if values != previous { write(values, scope: scope) }
    }

    /// Missing, failed and masked observations cannot establish recovery.
    /// Queue depth and the general health slot cannot establish recovery.
    /// Keep a queue dismissal through capacity churn until the admission
    /// backlog empties; missing fields from older daemons remain unknown.
    static func conditions(status: DaemonData.Status?, loaded: Bool) -> [Kind: Condition] {
        var result = Dictionary(uniqueKeysWithValues: Kind.allCases.map { ($0, Condition.unknown) })
        guard loaded, let status else { return result }
        let label = status.health?.lastErrorLabel
        if status.queueAtCapacity == true {
            result[.queueFull] = .active(.init(kind: .queueFull))
        } else if status.queueAtCapacity == false, status.queueCapacityUsed == 0 {
            result[.queueFull] = .clear
        } else if status.queueAtCapacity == nil, label == Kind.queueFull.rawValue {
            result[.queueFull] = .active(.init(kind: .queueFull))
        }
        if let waiting = status.witnessCapacity?.waitingSessions, waiting >= 0 {
            result[.witness] = waiting > 0 ? .active(.init(kind: .witness)) : .clear
        } else if label == Kind.witness.rawValue {
            result[.witness] = .active(.init(kind: .witness))
        }
        if let held = status.automaticContributionHeld?.heldSessions, held >= 0 {
            result[.automaticHold] = held > 0
                ? .active(.init(kind: .automaticHold, reasons: status.automaticContributionHeld?.reasons)) : .clear
        } else if label == Kind.automaticHold.rawValue {
            result[.automaticHold] = .active(.init(kind: .automaticHold))
        }
        return result
    }

    private func records(_ scope: Scope) -> [String: String] {
        defaults.dictionary(forKey: scope.storageKey) as? [String: String] ?? [:]
    }

    private func write(_ values: [String: String], scope: Scope) {
        if values.isEmpty { defaults.removeObject(forKey: scope.storageKey) }
        else { defaults.set(values, forKey: scope.storageKey) }
        revision &+= 1
    }

    private nonisolated static func digest(_ fields: [String]) -> String {
        // Frame fields so different path/account boundaries cannot collide.
        var hash = SHA256()
        for field in fields {
            let bytes = Data(field.utf8)
            hash.update(data: Data("\(bytes.count):".utf8))
            hash.update(data: bytes)
        }
        return hash.finalize().map { String(format: "%02x", $0) }.joined()
    }
}
