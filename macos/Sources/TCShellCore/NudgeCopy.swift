import Foundation

/// The core's nudge table (`tc_nudge_copy_json`): every fixed nudge string,
/// key to text. Sentences with a count in them arrive already composed on
/// `status.nudge`; this table is for the fixed words (Settings rows, offers,
/// the Traces order control and filter, action labels). Every string is
/// DRAFT, NEEDS APPROVAL in the core. A key the table lacks, or holds blank,
/// is absent, and what it would have labelled is not drawn.
public struct NudgeCopy: Equatable, Sendable {
    /// The keys this shell reads.
    public enum Key: String, CaseIterable, Sendable {
        case listOrderSuggested = "LIST_ORDER_SUGGESTED"
        case listOrderQueue = "LIST_ORDER_QUEUE"
        case listOrderLabel = "LIST_ORDER_LABEL"
        case listFilterIdle = "LIST_FILTER_IDLE"
        case listFilterClear = "LIST_FILTER_CLEAR"
        case settingSuggestions = "SETTING_SUGGESTIONS"
        case settingSuggestionsHelp = "SETTING_SUGGESTIONS_HELP"
        case settingMark = "SETTING_MARK"
        case settingMarkHelp = "SETTING_MARK_HELP"
        case settingNotifyMaster = "SETTING_NOTIFY_MASTER"
        case settingDigest = "SETTING_DIGEST"
        case settingNotifyVerdicts = "SETTING_NOTIFY_VERDICTS"
        case settingNotifyIdle = "SETTING_NOTIFY_IDLE"
        case settingNotifyIdleHelp = "SETTING_NOTIFY_IDLE_HELP"
        case settingNotifyBudgetHelp = "SETTING_NOTIFY_BUDGET_HELP"
        case offerNotifyVerdictsExisting = "OFFER_NOTIFY_VERDICTS_EXISTING"
        case offerNotifyIdleExisting = "OFFER_NOTIFY_IDLE_EXISTING"
        case offerTurnOn = "OFFER_TURN_ON"
        case offerNoThanks = "OFFER_NO_THANKS"
        case digestTitle = "DIGEST_TITLE"
        case digestActionReview = "DIGEST_ACTION_REVIEW"
        case digestActionNotNow = "DIGEST_ACTION_NOT_NOW"
        case notifyActionReviewIdle = "NOTIFY_ACTION_REVIEW_IDLE"
        case notifyActionSeeHistory = "NOTIFY_ACTION_SEE_HISTORY"
        case notifyActionNotNow = "NOTIFY_ACTION_NOT_NOW"
    }

    public let table: [String: String]

    public init(table: [String: String]) {
        self.table = table
    }

    /// The table from the export's JSON, or nil when it is not an object of
    /// strings.
    public static func decode(fromJSON json: String?) -> NudgeCopy? {
        guard let json, let table = try? JSONDecoder().decode([String: String].self, from: Data(json.utf8))
        else { return nil }
        return NudgeCopy(table: table)
    }

    /// The text for `key`; nil when absent or blank.
    public subscript(key: Key) -> String? {
        guard let text = table[key.rawValue], !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { return nil }
        return text
    }
}

/// A Traces row's tags, worded by the core from the row's own
/// `mission_fit` and `credit_estimate` (`tc_nudge_entry_tags_json`). Each is
/// present only when there is something true to draw: the mission tag above
/// zero, the estimate only while the daemon says it is drawn.
public struct NudgeEntryTags: Codable, Equatable, Sendable {
    public var missionFit: String?
    public var estimateBand: String?
    public var estimateTier: String?
    /// The estimate's info popover.
    public var estimateExplainer: String?

    public init(
        missionFit: String? = nil, estimateBand: String? = nil, estimateTier: String? = nil,
        estimateExplainer: String? = nil
    ) {
        self.missionFit = missionFit
        self.estimateBand = estimateBand
        self.estimateTier = estimateTier
        self.estimateExplainer = estimateExplainer
    }

    public enum CodingKeys: String, CodingKey {
        case missionFit = "mission_fit"
        case estimateBand = "estimate_band"
        case estimateTier = "estimate_tier"
        case estimateExplainer = "estimate_explainer"
    }

    public var isEmpty: Bool { self == NudgeEntryTags() }

    public static func decode(fromJSON json: String?) -> NudgeEntryTags? {
        guard let json else { return nil }
        return try? JSONDecoder().decode(NudgeEntryTags.self, from: Data(json.utf8))
    }

    /// What the core is asked about a row: its `mission_fit` and
    /// `credit_estimate` as the daemon sent them, and nothing else about
    /// it. Nil when the row carries neither, so nothing is asked.
    public static func input(for entry: DaemonData.QueueEntry) -> String? {
        guard entry.missionFit != nil || entry.creditEstimate != nil else { return nil }
        struct Input: Encodable {
            let missionFit: Int?
            let creditEstimate: DaemonData.CreditEstimate?

            enum CodingKeys: String, CodingKey {
                case missionFit = "mission_fit"
                case creditEstimate = "credit_estimate"
            }
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(Input(missionFit: entry.missionFit, creditEstimate: entry.creditEstimate))
        else { return nil }
        return String(decoding: data, as: UTF8.self)
    }
}

/// The nudge switches and one-time offers Settings draws, from
/// `get_settings` and the core's nudge table.
///
/// A switch is drawn only when the daemon reported its value (unknown is no
/// switch, never off) and the core worded it. The weekly recap and the
/// insights tip are held by owner decision and never drawn.
public enum NudgeSettings {
    public enum Switch: Hashable, Sendable {
        case suggestions
        case menuBarMark
        case notifications
        /// One `notify` kind.
        case notify(String)
    }

    public struct Row: Equatable, Sendable {
        public let id: Switch
        public let label: String
        public let help: String?
        public let isOn: Bool
        /// False under an off broader switch: the mark under suggestions,
        /// each kind under the master. The stored value is still shown.
        public let enabled: Bool
    }

    /// A one-time offer to turn a kind on.
    public struct Offer: Equatable, Sendable {
        public let kind: String
        public let text: String
        public let accept: String
        public let decline: String

        public init(kind: String, text: String, accept: String, decline: String) {
            self.kind = kind
            self.text = text
            self.accept = accept
            self.decline = decline
        }
    }

    /// The `notify` kinds drawn, in order. `weekly_recap` and
    /// `insights_tip` are held and left out.
    public static let shownKinds = ["digest", "verdicts_landed", "idle_sessions"]

    /// The `set_settings` key that clears a kind's offer marker; nil for a
    /// kind with no offer.
    public static func offerMarker(kind: String) -> String? {
        switch kind {
        case "verdicts_landed": "verdicts_offer_pending"
        case "idle_sessions": "idle_offer_pending"
        default: nil
        }
    }

    /// Thrown, sending nothing, for an offer marker that does not exist:
    /// the label the daemon gives an unknown settings key.
    public static let noOfferForKind = DaemonDataError.daemon(code: "bad_params", message: "settings-unknown-field")

    /// `digestHelp` is the core's finished line for the digest switch
    /// (`tc_nudge_digest_help_json`), drawn as given; nil draws none.
    public static func rows(_ settings: DaemonData.Settings?, copy: NudgeCopy?, digestHelp: String?) -> [Row] {
        guard let settings, let copy else { return [] }
        var rows: [Row] = []
        func add(_ id: Switch, _ label: NudgeCopy.Key, help: String?, value: Bool?, enabled: Bool = true) {
            guard let value, let text = copy[label] else { return }
            rows.append(Row(id: id, label: text, help: help, isOn: value, enabled: enabled))
        }
        add(.suggestions, .settingSuggestions, help: copy[.settingSuggestionsHelp], value: settings.suggestionsEnabled)
        add(.menuBarMark, .settingMark, help: copy[.settingMarkHelp], value: settings.menuBarMarkEnabled,
            enabled: settings.suggestionsEnabled == true)
        add(.notifications, .settingNotifyMaster, help: nil, value: settings.notificationsEnabled)
        let master = settings.notificationsEnabled != false
        for kind in shownKinds {
            switch kind {
            case "digest":
                add(.notify(kind), .settingDigest, help: digestHelp,
                    value: settings.notify?.digest, enabled: master)
            case "verdicts_landed":
                add(.notify(kind), .settingNotifyVerdicts, help: nil, value: settings.notify?.verdictsLanded,
                    enabled: master)
            case "idle_sessions":
                add(.notify(kind), .settingNotifyIdle, help: copy[.settingNotifyIdleHelp],
                    value: settings.notify?.idleSessions, enabled: master)
            default:
                break
            }
        }
        return rows
    }

    /// What the core is asked for the digest's help: the schedule and the
    /// interval as the daemon sent them, and nothing else about the
    /// settings. Nil when the settings carry neither, so nothing is asked.
    public static func digestHelpInput(_ settings: DaemonData.Settings?) -> String? {
        guard let settings, settings.digestSchedule != nil || settings.digestIntervalSecs != nil else { return nil }
        struct Input: Encodable {
            let digestIntervalSecs: Int?
            let digestSchedule: DaemonData.DigestSchedule?

            enum CodingKeys: String, CodingKey {
                case digestIntervalSecs = "digest_interval_secs"
                case digestSchedule = "digest_schedule"
            }
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(
            Input(digestIntervalSecs: settings.digestIntervalSecs, digestSchedule: settings.digestSchedule))
        else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    /// The core's digest help from `tc_nudge_digest_help_json`'s answer;
    /// nil when it has none, or the answer cannot be read.
    public static func digestHelp(fromJSON json: String?) -> String? {
        struct Answer: Decodable {
            let digestHelp: String?
            enum CodingKeys: String, CodingKey { case digestHelp = "digest_help" }
        }
        guard let json, let answer = try? JSONDecoder().decode(Answer.self, from: Data(json.utf8)),
              let help = answer.digestHelp, !help.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { return nil }
        return help
    }

    /// The pending offers, verdicts first.
    public static func offers(_ settings: DaemonData.Settings?, copy: NudgeCopy?) -> [Offer] {
        guard let settings, let copy, let accept = copy[.offerTurnOn], let decline = copy[.offerNoThanks]
        else { return [] }
        var offers: [Offer] = []
        if settings.verdictsOfferPending == true, let text = copy[.offerNotifyVerdictsExisting] {
            offers.append(Offer(kind: "verdicts_landed", text: text, accept: accept, decline: decline))
        }
        if settings.idleOfferPending == true, let text = copy[.offerNotifyIdleExisting] {
            offers.append(Offer(kind: "idle_sessions", text: text, accept: accept, decline: decline))
        }
        return offers
    }

    /// The pending offer `place` draws: the verdicts offer on History, the
    /// idle one on Traces. The contract places these one-time offers on the
    /// History and Traces cards, so an upgraded install that never opens
    /// Settings still meets them; Settings draws both as well.
    public static func offers(_ settings: DaemonData.Settings?, copy: NudgeCopy?, on place: NudgeSurface.Place) -> [Offer] {
        let kind = switch place {
        case .history: "verdicts_landed"
        case .traces: "idle_sessions"
        }
        return offers(settings, copy: copy).filter { $0.kind == kind }
    }

    /// The line under the kinds: the caps and quiet hours, in the core's
    /// words.
    public static func footnote(copy: NudgeCopy?) -> String? {
        copy?[.settingNotifyBudgetHelp]
    }

    /// Writes one switch through `client`.
    public static func write(_ id: Switch, on: Bool, through client: any DaemonDataClient) async throws {
        switch id {
        case .suggestions: try await client.setSuggestionsEnabled(on)
        case .menuBarMark: try await client.setMenuBarMarkEnabled(on)
        case .notifications: try await client.setNotificationsEnabled(on)
        case .notify(let kind): try await client.setNotifyKind(kind, on: on)
        }
    }
}
