import Foundation

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

    static func forLabel(_ label: String) -> HealthCopy {
        switch label {
        case "not-logged-in":
            return HealthCopy(
                title: "Not connected.",
                detail: """
                Sessions are being queued, but nothing can be sent until you \
                reconnect. Nothing has been lost.
                """,
                severity: .actionable,
                actionTitle: "Reconnect"
            )
        case "near-ai-notice-not-acknowledged":
            return HealthCopy(
                title: "One thing to confirm.",
                detail: """
                You chose the extra privacy scan, which sends message text to \
                NEAR AI. Confirm you're OK with that and contributions resume.
                """,
                severity: .actionable,
                actionTitle: "Review and confirm"
            )
        case "privacy-filter-canary-failed":
            return HealthCopy(
                title: "The privacy scan failed its own self-test,",
                detail: """
                so nothing is being sent through it. This is deliberate -- a scan \
                we can't verify doesn't get used.
                """,
                severity: .waiting,
                actionTitle: nil
            )
        case "pii-filter-unavailable":
            return HealthCopy(
                title: "The extra privacy scan isn't reachable.",
                detail: """
                Your traces are waiting rather than going out unscanned. Retrying \
                automatically.
                """,
                severity: .waiting,
                actionTitle: nil
            )
        case "claim-mint-failed", "ingest-unreachable":
            return HealthCopy(
                title: "Can't reach Trace Commons right now.",
                detail: "Your queue is safe; it'll retry on its own.",
                severity: .waiting,
                actionTitle: nil
            )
        case "queue-full":
            return HealthCopy(
                title: "Trace Commons has stopped queuing new sessions",
                detail: """
                -- 500 are already waiting. Review or clear some to start again.
                """,
                severity: .actionable,
                actionTitle: "Review"
            )
        case "daily-cap-reached":
            return HealthCopy(
                title: "Daily limit reached.",
                detail: "The rest goes out tomorrow.",
                severity: .informational,
                actionTitle: nil
            )
        default:
            // An unrecognised label is still a real condition. Say that
            // something is holding contributions rather than inventing a
            // cause, and never render the raw label as an explanation.
            return HealthCopy(
                title: "Contributions are on hold.",
                detail: """
                Something is stopping traces from being sent. Nothing has been \
                lost, and nothing has gone out.
                """,
                severity: .waiting,
                actionTitle: nil
            )
        }
    }
}

/// Plain-English names for the queue states a contributor sees. Four of them
/// mean nothing left the machine, and each says so in words.
enum QueueStateCopy {
    static func sentence(for state: QueueState) -> String {
        switch state {
        case .pending: return "Waiting for your decision. Nothing has been sent."
        case .approved: return "You said yes. Not sent yet."
        case .uploading: return "Being sent now."
        case .uploaded: return "In the commons."
        case .refused: return "The system declined to send this. Nothing was sent."
        case .failed: return "Sending didn't work. Nothing was sent; it will retry."
        case .expired: return "Dropped after waiting too long for a decision. Never sent."
        case .superseded:
            return """
            This session changed after you approved it, so it was not sent. A \
            fresh copy is waiting for a new decision.
            """
        }
    }
}

/// Plain-English reasons for `queue_outcome_counts`. It covers entries that
/// ARE on the queue -- it cannot explain a session the watcher discarded
/// before an entry existed, and this UI does not claim otherwise.
enum OutcomeCopy {
    static func sentence(for label: String) -> String {
        switch label {
        case "dismissed-by-contributor": return "You said no thanks"
        case "expired-without-decision": return "Waited too long without a decision"
        case "session-changed-after-offer": return "Changed after it was offered"
        case "not-logged-in": return "Waiting until you reconnect"
        case "daily-cap-reached": return "Waiting for tomorrow's allowance"
        case "queue-full": return "Queue was full"
        case "ingest-unreachable", "claim-mint-failed": return "Trace Commons was unreachable"
        case "pii-filter-unavailable": return "Waiting for the extra privacy scan"
        case "privacy-filter-canary-failed": return "The privacy scan failed its self-test"
        default: return "Held"
        }
    }
}
