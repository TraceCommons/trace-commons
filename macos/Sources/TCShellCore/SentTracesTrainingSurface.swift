import Foundation

/// The words for adding Train coding models to traces already sent,
/// decoded from what the Rust exported (`tc_sent_traces_training_copy_json`).
///
/// Every string is a draft awaiting owner approval. None is written in
/// Swift: a payload with a missing or empty field is refused whole. The
/// `*_other` templates and `allow_accessibility_name` carry `{count}`,
/// filled by `body(count:)`, `line(count:)` and
/// `allowAccessibilityName(count:)`.
public struct SentTracesTrainingCopy: Decodable, Equatable, Sendable {
    public let title: String
    public let bodyOne: String
    public let bodyOther: String
    public let scope: String
    public let allowButton: String
    public let allowAccessibilityName: String
    public let notNowButton: String
    public let notNowAccessibilityName: String
    public let lineOne: String
    public let lineOther: String
    public let reviewButton: String
    public let reviewAccessibilityName: String
    public let traceLine: String
    public let traceButton: String
    public let traceAccessibilityName: String
    public let pending: String
    public let failed: String

    enum CodingKeys: String, CodingKey {
        case title, scope, pending, failed
        case bodyOne = "body_one"
        case bodyOther = "body_other"
        case allowButton = "allow_button"
        case allowAccessibilityName = "allow_accessibility_name"
        case notNowButton = "not_now_button"
        case notNowAccessibilityName = "not_now_accessibility_name"
        case lineOne = "line_one"
        case lineOther = "line_other"
        case reviewButton = "review_button"
        case reviewAccessibilityName = "review_accessibility_name"
        case traceLine = "trace_line"
        case traceButton = "trace_button"
        case traceAccessibilityName = "trace_accessibility_name"
    }

    /// The payload fields this shell decodes, by wire name; compared with the
    /// live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body_one", "body_other", "scope", "allow_button", "allow_accessibility_name",
        "not_now_button", "not_now_accessibility_name", "line_one", "line_other", "review_button",
        "review_accessibility_name", "trace_line", "trace_button", "trace_accessibility_name",
        "pending", "failed",
    ]

    public var sentences: [String] {
        [
            title, bodyOne, bodyOther, scope, allowButton, allowAccessibilityName, notNowButton,
            notNowAccessibilityName, lineOne, lineOther, reviewButton, reviewAccessibilityName,
            traceLine, traceButton, traceAccessibilityName, pending, failed,
        ]
    }

    /// Nil, never a partly-filled value, for an unparseable payload or an
    /// empty field.
    public static func decode(fromJSON json: String) -> SentTracesTrainingCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(SentTracesTrainingCopy.self, from: data)
        else { return nil }
        return copy.sentences.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty })
            ? nil : copy
    }

    /// `template` with `{count}` filled in.
    public static func render(_ template: String, count: UInt64) -> String {
        template.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// The sheet's body: the one-trace sentence for 1, the counted one
    /// otherwise.
    public func body(count: UInt64) -> String {
        count == 1 ? bodyOne : Self.render(bodyOther, count: count)
    }

    /// The Insights card's quiet line, chosen the same way.
    public func line(count: UInt64) -> String {
        count == 1 ? lineOne : Self.render(lineOther, count: count)
    }

    public func allowAccessibilityName(count: UInt64) -> String {
        Self.render(allowAccessibilityName, count: count)
    }
}

/// `model_training_consent_status`: every eligible trace counted, and the
/// oldest 500 of their ids.
public struct ModelTrainingConsentStatus: Decodable, Equatable, Sendable {
    public let eligibleCount: UInt64
    public let submissionIds: [String]

    enum CodingKeys: String, CodingKey {
        case eligibleCount = "eligible_count"
        case submissionIds = "submission_ids"
    }

    public init(eligibleCount: UInt64, submissionIds: [String]) {
        self.eligibleCount = eligibleCount
        self.submissionIds = submissionIds
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        eligibleCount = try c.decode(UInt64.self, forKey: .eligibleCount)
        submissionIds = try c.decodeIfPresent([String].self, forKey: .submissionIds) ?? []
    }
}

/// One trace in a `model_training_consent_grant` answer, as the server
/// said it. `outcome` is `granted`, `already_granted` or `not_eligible`;
/// `award` is `credited`, `pending` or `none`.
public struct ModelTrainingGrantResult: Decodable, Equatable, Sendable {
    public let submissionId: String
    public let outcome: String
    public let grantedAt: String?
    public let award: String
    public let awardWithheldReason: String?

    enum CodingKeys: String, CodingKey {
        case outcome, award
        case submissionId = "submission_id"
        case grantedAt = "granted_at"
        case awardWithheldReason = "award_withheld_reason"
    }

    public init(
        submissionId: String, outcome: String, grantedAt: String? = nil, award: String,
        awardWithheldReason: String? = nil
    ) {
        self.submissionId = submissionId
        self.outcome = outcome
        self.grantedAt = grantedAt
        self.award = award
        self.awardWithheldReason = awardWithheldReason
    }

    /// Whether a credit follows from this grant: the use is on the trace
    /// and the server wrote the award or will.
    public var creditFollows: Bool {
        (outcome == "granted" || outcome == "already_granted") && (award == "pending" || award == "credited")
    }
}

/// `model_training_consent_grant`'s answer.
public struct ModelTrainingGrantAnswer: Decodable, Equatable, Sendable {
    public let results: [ModelTrainingGrantResult]
    public let remainingEligible: UInt64

    enum CodingKeys: String, CodingKey {
        case results
        case remainingEligible = "remaining_eligible"
    }

    public init(results: [ModelTrainingGrantResult], remainingEligible: UInt64) {
        self.results = results
        self.remainingEligible = remainingEligible
    }
}

/// A refusal, routed from the daemon's fixed label.
public enum SentTracesTrainingRefusal: Equatable, Sendable {
    /// The grant ceiling does not include model training, the server cannot
    /// record a grant, or nobody is enrolled: every affordance hides.
    case hidden
    /// Anything else. Nothing changed.
    case failed

    public static func from(code label: String) -> SentTracesTrainingRefusal {
        switch label {
        case "model-training-not-permitted", "model-training-consent-unavailable", "not-logged-in":
            return .hidden
        default:
            return .failed
        }
    }
}

/// A trace's mark after the person allowed the use on it.
public enum SentTraceTrainingMark: Equatable, Sendable {
    /// Allowed, and its credit has not shown on its History row yet.
    case pending
    /// The grant did not go through; the trace is offered again.
    case failed
}

/// Which window raised the sheet: it is drawn there and nowhere else.
public enum SentTracesTrainingSite: Equatable, Sendable {
    /// Settings, where Train coding models was turned on.
    case settings
    /// The Monitor: the Insights line's Review, and a turn-on from the
    /// first run, which hands off to the Monitor when it finishes.
    case monitor
}

/// The sheet while it is up: where, and the count it was raised for.
public struct SentTracesTrainingSheet: Equatable, Sendable {
    public let site: SentTracesTrainingSite
    public let count: UInt64

    public init(site: SentTracesTrainingSite, count: UInt64) {
        self.site = site
        self.count = count
    }
}

/// What the shell knows about sent traces without Train coding models, and
/// the marks it drew after a grant. Pure: every fact is the daemon's.
public struct SentTracesTrainingState: Equatable, Sendable {
    /// The consent scope and allowed use this is about.
    public static let scope = "model_training"

    public enum Eligibility: Equatable, Sendable {
        /// Not asked yet: nothing is drawn.
        case unknown
        /// Refused, or nothing eligible: nothing is drawn.
        case hidden
        /// `count` traces are eligible; `ids` are the oldest 500 of them.
        case eligible(count: UInt64, ids: Set<String>)
    }

    public private(set) var eligibility: Eligibility = .unknown
    public private(set) var marks: [String: SentTraceTrainingMark] = [:]

    public init() {}

    /// Whether this write turns Train coding models on: absent from what the
    /// daemon last reported, present in what is now sent.
    public static func turnsOn(before: [String], after: [String]) -> Bool {
        !before.contains(scope) && after.contains(scope)
    }

    /// The count the Insights line and the sheet say, or nil when nothing
    /// is to be drawn.
    public var count: UInt64? {
        if case .eligible(let count, _) = eligibility, count > 0 { return count }
        return nil
    }

    /// The sheet a turn-on raises, from the status just read: only while
    /// traces are eligible.
    public func sheet(at site: SentTracesTrainingSite) -> SentTracesTrainingSheet? {
        count.map { SentTracesTrainingSheet(site: site, count: $0) }
    }

    /// Whether the inspector offers the use on this trace: it is among the
    /// eligible ids and no grant for it is waiting on its credit.
    public func offers(_ submissionId: String) -> Bool {
        guard case .eligible(_, let ids) = eligibility, ids.contains(submissionId) else { return false }
        return marks[submissionId] != .pending
    }

    /// The mark this trace draws: pending until `credited` (its History
    /// row shows credit), failed until it is offered again and taken.
    public func mark(for submissionId: String, credited: Bool) -> SentTraceTrainingMark? {
        switch marks[submissionId] {
        case .pending: return credited ? nil : .pending
        case .failed: return offers(submissionId) ? .failed : nil
        case nil: return nil
        }
    }

    /// Whether a History row reports this trace's credit: final credit
    /// above zero. The row draws its credit from the final figure alone
    /// (`HomeFormat.credit`), zero included, and a scored trace whose award
    /// was withheld reads zero there, so only a figure above zero ends the
    /// pending mark.
    public static func credited(final: Double?) -> Bool {
        (final ?? 0) > 0
    }

    public mutating func apply(_ status: ModelTrainingConsentStatus) {
        eligibility = status.eligibleCount == 0
            ? .hidden : .eligible(count: status.eligibleCount, ids: Set(status.submissionIds))
    }

    /// A status read was refused: nothing is drawn, whatever the label.
    public mutating func applyStatusRefusal() {
        eligibility = .hidden
    }

    /// A grant answered. A trace whose credit follows is marked pending;
    /// every answered trace leaves the eligible set; the count is the
    /// server's remaining one.
    public mutating func apply(_ answer: ModelTrainingGrantAnswer) {
        var ids: Set<String> = []
        if case .eligible(_, let current) = eligibility { ids = current }
        for result in answer.results {
            ids.remove(result.submissionId)
            marks[result.submissionId] = result.creditFollows ? .pending : nil
        }
        eligibility = answer.remainingEligible == 0
            ? .hidden : .eligible(count: answer.remainingEligible, ids: ids)
    }

    /// A grant for these traces was refused.
    public mutating func applyGrantRefusal(_ refusal: SentTracesTrainingRefusal, for submissionIds: [String]) {
        switch refusal {
        case .hidden:
            eligibility = .hidden
        case .failed:
            for id in submissionIds { marks[id] = .failed }
        }
    }
}

/// The two IPC calls over any `DaemonTransport`, so the method names and
/// parameters are asserted without a daemon. Calls block: run them off the
/// main actor.
public struct SentTracesTrainingClient {
    /// A daemon refusal: its fixed label, safe to route on and never shown.
    public struct Failure: Error, Equatable {
        public let label: String
        public var refusal: SentTracesTrainingRefusal { SentTracesTrainingRefusal.from(code: label) }
    }

    private let transport: any DaemonTransport

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    public func status() throws -> ModelTrainingConsentStatus {
        try call("model_training_consent_status", [:], as: ModelTrainingConsentStatus.self)
    }

    /// Every eligible trace. Sent only after the person chose Allow.
    public func grantAll() throws -> ModelTrainingGrantAnswer {
        try call("model_training_consent_grant", ["all_eligible": true], as: ModelTrainingGrantAnswer.self)
    }

    /// These traces only. Sent only after the person chose Allow on them.
    public func grant(submissionIds: [String]) throws -> ModelTrainingGrantAnswer {
        try call(
            "model_training_consent_grant", ["submission_ids": submissionIds], as: ModelTrainingGrantAnswer.self)
    }

    private func call<T: Decodable>(
        _ method: String, _ params: [String: Any], as type: T.Type
    ) throws -> T {
        let data = try JSONSerialization.data(withJSONObject: params)
        let frame = transport.call(method, params: String(decoding: data, as: UTF8.self))
        guard let bytes = frame.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        else { throw Failure(label: "unparseable-response") }
        if let error = object["error"] as? [String: Any] {
            throw Failure(label: error["message"] as? String ?? "unavailable")
        }
        guard let result = object["result"],
            let resultBytes = try? JSONSerialization.data(
                withJSONObject: result, options: [.fragmentsAllowed]),
            let decoded = try? JSONDecoder().decode(T.self, from: resultBytes)
        else { throw Failure(label: "unparseable-response") }
        return decoded
    }
}
