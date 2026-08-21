import Foundation

/// What `status.daily_budget` says, and the one sentence the shell renders
/// from it.
///
/// Lives here rather than beside `HealthCopy` because it is pure logic with
/// a right answer, and this is the target that has tests. `HealthCopy`'s
/// table is a lookup; this is arithmetic and plural agreement, and both
/// have been wrong in shipped copy before.
///
/// The condition it describes is reported by the daemon *outside* the
/// health slot on purpose. `daily-cap-reached` is last in the daemon's
/// precedence order, so any other condition takes the single
/// `last_error_label` and the cap disappears -- which is exactly how a
/// contributor came to watch approved traces sit still with the app
/// reporting a completely unrelated problem.
public struct DailyBudget: Decodable, Equatable, Sendable {
    public let bytesToday: Int
    public let maxBytesPerDay: Int
    public let bytesRemaining: Int
    public let uploadsToday: Int
    public let maxUploadsPerDay: Int
    public let uploadsRemaining: Int
    /// When the counters zero. The daemon derives this from its own UTC day
    /// bucket, so it is a fact and may be stated.
    public let resetsAt: Date?
    /// Whether at least one approved trace cannot go out before the reset.
    public let blocked: Bool
    public let blockedEntries: Int
    public let blockedBytes: Int

    enum CodingKeys: String, CodingKey {
        case bytesToday = "bytes_today"
        case maxBytesPerDay = "max_bytes_per_day"
        case bytesRemaining = "bytes_remaining"
        case uploadsToday = "uploads_today"
        case maxUploadsPerDay = "max_uploads_per_day"
        case uploadsRemaining = "uploads_remaining"
        case resetsAt = "resets_at"
        case blocked
        case blockedEntries = "blocked_entries"
        case blockedBytes = "blocked_bytes"
    }

    public init(
        bytesToday: Int = 0,
        maxBytesPerDay: Int = 0,
        bytesRemaining: Int = 0,
        uploadsToday: Int = 0,
        maxUploadsPerDay: Int = 0,
        uploadsRemaining: Int = 0,
        resetsAt: Date? = nil,
        blocked: Bool = false,
        blockedEntries: Int = 0,
        blockedBytes: Int = 0
    ) {
        self.bytesToday = bytesToday
        self.maxBytesPerDay = maxBytesPerDay
        self.bytesRemaining = bytesRemaining
        self.uploadsToday = uploadsToday
        self.maxUploadsPerDay = maxUploadsPerDay
        self.uploadsRemaining = uploadsRemaining
        self.resetsAt = resetsAt
        self.blocked = blocked
        self.blockedEntries = blockedEntries
        self.blockedBytes = blockedBytes
    }

    /// A daemon that predates the field, or one that sent nothing.
    public static let unknown = DailyBudget()
}

public enum DailyBudgetCopy {
    public static let title = "Today's upload limit is used up."

    /// The detail line: how many are waiting, and when the limit resets.
    ///
    /// The reset time is stated only when the daemon supplied one. Never
    /// "tomorrow" -- the daemon rolls its counters at UTC midnight, which is
    /// not tomorrow for most of the world, and a shell that guessed would be
    /// wrong for eight hours a day in one direction and sixteen in the other.
    ///
    /// `formatter` is injected so the assertion in the tests does not depend
    /// on the machine's timezone.
    public static func detail(
        blockedEntries: Int,
        resetsAt: Date?,
        formatter: (Date) -> String = defaultTimeFormatter
    ) -> String {
        let waiting: String
        switch blockedEntries {
        case ..<1: waiting = "Approved traces are waiting"
        case 1: waiting = "1 approved trace is waiting"
        default: waiting = "\(blockedEntries) approved traces are waiting"
        }
        guard let resetsAt else {
            return "\(waiting). Nothing has been lost -- they go out when the limit resets."
        }
        return """
            \(waiting). Nothing has been lost -- they go out when the limit \
            resets at \(formatter(resetsAt)).
            """
    }

    public static func defaultTimeFormatter(_ date: Date) -> String {
        let f = DateFormatter()
        f.timeStyle = .short
        f.dateStyle = .none
        return f.string(from: date)
    }
}
