import Foundation

/// `status.witness_capacity`: how many approved sessions are held because
/// the privacy witness is busy, and when the first is tried again.
///
/// Beside `health` rather than in it, for the reason `DailyBudget` is: the
/// health slot holds one label and a higher one can mask
/// `witness-saturated`, while this object always says how many are waiting.
/// A daemon that predates it reports nothing waiting.
public struct WitnessCapacity: Decodable, Equatable, Sendable {
    public let waitingSessions: Int
    /// Rendered by the shell, in local time, beside the notice's
    /// `nextCheck`. Nil when the daemon gave none.
    public let nextRetryAt: Date?

    enum CodingKeys: String, CodingKey {
        case waitingSessions = "waiting_sessions"
        case nextRetryAt = "next_retry_at"
    }

    public init(waitingSessions: Int = 0, nextRetryAt: Date? = nil) {
        self.waitingSessions = waitingSessions
        self.nextRetryAt = nextRetryAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        waitingSessions = try c.decode(Int.self, forKey: .waitingSessions)
        nextRetryAt = try c.decodeIfPresent(Date.self, forKey: .nextRetryAt)
    }

    public static let none = WitnessCapacity()

    public var waiting: Bool { waitingSessions > 0 }

    /// The object handed to the Rust (`TCConsentCopy.witnessCapacityNoticeJSON`),
    /// which words the notice. The count is all the words depend on, so it
    /// is all that crosses.
    public var wireJSON: String {
        let object: [String: Int] = ["waiting_sessions": max(0, waitingSessions)]
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }
}

/// The notice the Rust assembled for sessions waiting on a busy witness.
/// Every property is filled from the payload and none is written in Swift.
public struct WitnessCapacityNotice: Decodable, Equatable, Sendable {
    public let title: String
    /// Why the sessions wait, that nothing is sent meanwhile, and that
    /// nothing was lost -- counted by the Rust.
    public let body: String
    /// The label beside the next retry time.
    public let nextCheck: String

    enum CodingKeys: String, CodingKey {
        case title, body
        case nextCheck = "next_check"
    }

    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by `TCBridgeTests`.
    public static let consumedFields = ["title", "body", "next_check"]

    /// Decode the payload, or nil if it will not parse or a field is empty:
    /// shown whole or not at all.
    public static func decode(fromJSON json: String) -> WitnessCapacityNotice? {
        guard let data = json.data(using: .utf8),
            let notice = try? JSONDecoder().decode(WitnessCapacityNotice.self, from: data)
        else {
            return nil
        }
        if [notice.title, notice.body, notice.nextCheck].contains(where: \.isEmpty) { return nil }
        return notice
    }

    /// The Rust's label and the daemon's time, or nil when the daemon gave
    /// no time -- never one made up here.
    public func nextRetryLine(
        for capacity: WitnessCapacity,
        format: (Date) -> String = { $0.formatted(date: .omitted, time: .shortened) }
    ) -> String? {
        guard let at = capacity.nextRetryAt else { return nil }
        return "\(nextCheck): \(format(at))"
    }
}
