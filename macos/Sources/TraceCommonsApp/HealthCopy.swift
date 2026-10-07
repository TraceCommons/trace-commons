import Foundation
import TCShellCore
import TCBridge

/// The health sentences, verbatim from the shared design's failure-state
/// table. Two rules hold across every one of them: never name the mechanism
/// ("privacy filter", "claim", "ingest", "canary" are internal words), and
/// always state the data consequence.
///
/// `status.health.last_error_label` carries ONE label at a time, already
/// resolved by the daemon's precedence order. This table does not
/// reconstruct that order and must not: it renders whichever label arrives.
struct HealthCopy: Equatable {
    enum Severity {
        /// Something the contributor can act on.
        case actionable
        /// Ambient; it clears on its own.
        case waiting
        /// Menu line only.
        case informational
    }

    let title: String
    let detail: String
    let severity: Severity
    /// Present only where there is a real action behind it.
    let actionTitle: String?
    var reviewsQueue = false

    /// The banner for a spent daily budget, built from the numbers the
    /// daemon actually reported.
    ///
    /// Separate from `core(label:maxQueueEntries:)` because the daemon reports this separately:
    /// `daily-cap-reached` is last in the precedence order, so on the
    /// machine this was written for the single health slot was held by
    /// `queue-full` and the real reason nothing was uploading never reached
    /// a screen. A shell that waits for the label to arrive will keep
    /// missing it.
    ///
    /// `.waiting`, not an error: nothing is broken and nothing was lost.
    static func forBudget(_ budget: DailyBudget) -> HealthCopy? {
        guard budget.blocked else { return nil }
        return HealthCopy(
            title: DailyBudgetCopy.title,
            detail: DailyBudgetCopy.detail(
                blockedEntries: budget.blockedEntries,
                resetsAt: budget.resetsAt
            ),
            severity: .waiting,
            actionTitle: nil
        )
    }

    /// The banner for approved sessions held because the privacy witness is
    /// busy, built from `status.witness_capacity` in the Rust's words
    /// (`tc_witness_capacity_notice`): the title, the counted body, and the
    /// next try in local time when the daemon gave one.
    ///
    /// Separate from `core(label:maxQueueEntries:)` for the reason `forBudget` is: the health
    /// slot can be held by a higher label, and the sessions are still
    /// waiting. Nil when nothing is waiting or the notice cannot be read --
    /// the label, if it holds the slot, then falls back to the core's
    /// on-hold line rather than disappearing.
    ///
    /// `.waiting`: nothing is broken, nothing left the machine, and the
    /// daemon retries on its own.
    static func forWitnessCapacity(_ capacity: WitnessCapacity) -> HealthCopy? {
        guard capacity.waiting,
            let json = TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: capacity.wireJSON),
            let notice = WitnessCapacityNotice.decode(fromJSON: json)
        else { return nil }
        let detail = [notice.body, notice.nextRetryLine(for: capacity)]
            .compactMap { $0 }
            .joined(separator: "\n")
        return HealthCopy(
            title: notice.title,
            detail: detail,
            severity: .waiting,
            actionTitle: nil
        )
    }
}

/// The core's words for one health line. In an extension so the struct keeps
/// its memberwise initialiser, which `forBudget` and `forWitnessCapacity` use.
extension HealthCopy {
    init(line: HealthLineCopy) {
        self.init(
            title: line.title,
            detail: line.detail,
            severity: line.severity == .actionable ? .actionable : .waiting,
            actionTitle: line.action,
            reviewsQueue: line.actionKind == .reviewQueue
        )
    }

    /// The core's `on_hold_copy()`, verbatim. Drawn only when
    /// `tc_health_copy_json` returns nothing for a reported label (a caught
    /// panic or an undecodable payload), so a reported condition is never
    /// drawn as healthy.
    static let onHoldFallback = HealthCopy(
        title: "Contributions are on hold.",
        detail: "Something is stopping traces from being sent. Nothing has been lost, and nothing has gone out.",
        severity: .waiting,
        actionTitle: nil
    )

    /// The banner for the one label `status.health.last_error_label` carries,
    /// in the core's words (`tc_health_copy_json`). `maxQueueEntries` is the
    /// daemon's configured queue limit, named in `queue-full`'s count when
    /// known.
    static func core(label: String, maxQueueEntries: Int?) -> HealthCopy {
        guard let line = HealthLineCopy.decode(
            fromJSON: TCCoreCopy.healthCopyJSON(reachable: true, label: label, maxQueueEntries: maxQueueEntries))
        else { return onHoldFallback }
        return HealthCopy(line: line)
    }
}
