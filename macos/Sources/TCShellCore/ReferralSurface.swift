import Foundation

// Referral codes (docs/superpowers/specs/2026-10-29-referral-rewards-design.md,
// sections 5 and 6): the Settings Referrals section, the optional code
// field on the NEAR AI sign-in step, and the own-traces vs referrals split
// on the Insights NEAR AI credit card. Pure: every sentence is the core's
// (`tc_referral_copy_json`, every one a DRAFT for owner review), every fact
// the daemon's. No banner, notification or one-time message is drawn from
// anything here.

/// The referral words, decoded from what the Rust exported
/// (`tc_referral_copy_json`). DRAFT, NEEDS APPROVAL, every one. None is
/// written in Swift: a payload with a missing or empty field is refused
/// whole, so a blank never stands where a sentence belongs. Templates carry
/// `{code}`, `{n}`, `{date}` or `{amount}`, filled by `render(_:_:)`.
public struct ReferralCopy: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let createButton: String
    public let createAccessibilityName: String
    public let limitReached: String
    public let unavailable: String
    public let copyButton: String
    public let copyAccessibilityName: String
    public let copied: String
    public let disableButton: String
    public let disableAccessibilityName: String
    public let disableScope: String
    public let stateDisabled: String
    public let stateSuspended: String
    public let stateVoided: String
    public let codeHidden: String
    public let joinedOne: String
    public let joinedMany: String
    public let peopleJoined: String
    public let credit: String
    public let creditSent: String
    public let enterTitle: String
    public let fieldLabel: String
    public let fieldPlaceholder: String
    public let applyButton: String
    public let applyAccessibilityName: String
    public let enterScope: String
    public let appliedStatus: String
    public let notApplied: String
    public let invalidCode: String
    public let signUpLabel: String
    public let failed: String
    public let sessionEnded: String
    public let insightsOwn: String
    public let insightsReferrals: String
    public let insightsOnHold: String

    enum CodingKeys: String, CodingKey {
        case title, body, copied, credit, failed
        case createButton = "create_button"
        case createAccessibilityName = "create_accessibility_name"
        case limitReached = "limit_reached"
        case unavailable
        case copyButton = "copy_button"
        case copyAccessibilityName = "copy_accessibility_name"
        case disableButton = "disable_button"
        case disableAccessibilityName = "disable_accessibility_name"
        case disableScope = "disable_scope"
        case stateDisabled = "state_disabled"
        case stateSuspended = "state_suspended"
        case stateVoided = "state_voided"
        case codeHidden = "code_hidden"
        case joinedOne = "joined_one"
        case joinedMany = "joined_many"
        case peopleJoined = "people_joined"
        case creditSent = "credit_sent"
        case enterTitle = "enter_title"
        case fieldLabel = "field_label"
        case fieldPlaceholder = "field_placeholder"
        case applyButton = "apply_button"
        case applyAccessibilityName = "apply_accessibility_name"
        case enterScope = "enter_scope"
        case appliedStatus = "applied_status"
        case notApplied = "not_applied"
        case invalidCode = "invalid_code"
        case signUpLabel = "sign_up_label"
        case sessionEnded = "session_ended"
        case insightsOwn = "insights_own"
        case insightsReferrals = "insights_referrals"
        case insightsOnHold = "insights_on_hold"
    }

    /// The payload fields this shell decodes, by wire name; compared with the
    /// live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "create_button", "create_accessibility_name", "limit_reached", "unavailable",
        "copy_button", "copy_accessibility_name", "copied", "disable_button", "disable_accessibility_name",
        "disable_scope", "state_disabled", "state_suspended", "state_voided", "code_hidden", "joined_one",
        "joined_many", "people_joined", "credit", "credit_sent", "enter_title", "field_label",
        "field_placeholder", "apply_button", "apply_accessibility_name", "enter_scope", "applied_status",
        "not_applied", "invalid_code", "sign_up_label", "failed", "session_ended", "insights_own",
        "insights_referrals", "insights_on_hold",
    ]

    public var sentences: [String] {
        [
            title, body, createButton, createAccessibilityName, limitReached, unavailable, copyButton,
            copyAccessibilityName, copied, disableButton, disableAccessibilityName, disableScope,
            stateDisabled, stateSuspended, stateVoided, codeHidden, joinedOne, joinedMany, peopleJoined,
            credit, creditSent, enterTitle, fieldLabel, fieldPlaceholder, applyButton,
            applyAccessibilityName, enterScope, appliedStatus, notApplied, invalidCode, signUpLabel,
            failed, sessionEnded, insightsOwn, insightsReferrals, insightsOnHold,
        ]
    }

    /// Nil, never a partly-filled value, for an unparseable payload or an
    /// empty field.
    public static func decode(fromJSON json: String) -> ReferralCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ReferralCopy.self, from: data)
        else { return nil }
        return copy.sentences.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty })
            ? nil : copy
    }

    /// `template` with each `{name}` in `values` filled.
    public static func render(_ template: String, _ values: [String: String]) -> String {
        values.reduce(template) { text, pair in
            text.replacingOccurrences(of: "{\(pair.key)}", with: pair.value)
        }
    }
}

/// Reads a whole number of nano-USD, defensively. The commons sends payout
/// stream amounts as decimal strings, so a u128 survives JSON; other
/// figures are JSON numbers. A negative, fractional, absent or malformed
/// figure, or one past `UInt64.max`, is nil: unknown, never a zero.
private func nanoUSD<K: CodingKey>(_ c: KeyedDecodingContainer<K>, _ key: K) -> UInt64? {
    if let text = try? c.decodeIfPresent(String.self, forKey: key) {
        // Digits only: `UInt64("+5")` would otherwise read a sign.
        guard !text.isEmpty, text.utf8.allSatisfy({ (0x30...0x39).contains($0) }) else { return nil }
        return UInt64(text)
    }
    if let value = try? c.decodeIfPresent(UInt64.self, forKey: key) { return value }
    return nil
}

/// One code in `referral_codes_list`. `state` is the daemon's label, carried
/// as sent so a state a later daemon adds is not misread as live.
public struct ReferralCodeRow: Decodable, Equatable, Identifiable, Sendable {
    public let codeId: String
    /// `TC-XXXXX-XXXXX`; nil when the commons cannot show it.
    public let code: String?
    public let state: String
    public let refereeCount: UInt64
    public let bonusOwedNanoUSD: UInt64?

    public var id: String { codeId }
    public var isLive: Bool { state == "live" }

    enum CodingKeys: String, CodingKey {
        case codeId = "code_id"
        case code, state
        case refereeCount = "referee_count"
        case bonusOwedNanoUSD = "bonus_owed_nano_usd"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        codeId = try c.decode(String.self, forKey: .codeId)
        code = try c.decodeIfPresent(String.self, forKey: .code)
        state = try c.decode(String.self, forKey: .state)
        refereeCount = (try? c.decodeIfPresent(UInt64.self, forKey: .refereeCount)) ?? 0
        bonusOwedNanoUSD = nanoUSD(c, .bonusOwedNanoUSD)
    }

    public init(codeId: String, code: String?, state: String = "live", refereeCount: UInt64 = 0,
                bonusOwedNanoUSD: UInt64? = 0) {
        self.codeId = codeId
        self.code = code
        self.state = state
        self.refereeCount = refereeCount
        self.bonusOwedNanoUSD = bonusOwedNanoUSD
    }
}

/// `referral_codes_list`.
public struct ReferralCodesListing: Decodable, Equatable, Sendable {
    public let codes: [ReferralCodeRow]
    public let maxLiveCodes: Int
    public let canCreate: Bool
    public let peopleJoined: UInt64
    public let bonusOwedNanoUSD: UInt64?
    public let bonusSentNanoUSD: UInt64?

    enum CodingKeys: String, CodingKey {
        case codes
        case maxLiveCodes = "max_live_codes"
        case canCreate = "can_create"
        case peopleJoined = "people_joined"
        case bonusOwedNanoUSD = "bonus_owed_nano_usd"
        case bonusSentNanoUSD = "bonus_sent_nano_usd"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        codes = try c.decode([ReferralCodeRow].self, forKey: .codes)
        maxLiveCodes = try c.decode(Int.self, forKey: .maxLiveCodes)
        // A daemon that does not say is read as "no": a Create that the
        // commons refuses is worse than one not offered.
        canCreate = (try? c.decodeIfPresent(Bool.self, forKey: .canCreate)) ?? false
        peopleJoined = (try? c.decodeIfPresent(UInt64.self, forKey: .peopleJoined))
            ?? codes.reduce(0) { $0 &+ $1.refereeCount }
        bonusOwedNanoUSD = nanoUSD(c, .bonusOwedNanoUSD)
        bonusSentNanoUSD = nanoUSD(c, .bonusSentNanoUSD)
    }

    public init(codes: [ReferralCodeRow], maxLiveCodes: Int = 5, canCreate: Bool = true,
                peopleJoined: UInt64 = 0, bonusOwedNanoUSD: UInt64? = 0, bonusSentNanoUSD: UInt64? = 0) {
        self.codes = codes
        self.maxLiveCodes = maxLiveCodes
        self.canCreate = canCreate
        self.peopleJoined = peopleJoined
        self.bonusOwedNanoUSD = bonusOwedNanoUSD
        self.bonusSentNanoUSD = bonusSentNanoUSD
    }
}

/// `referral_codes_create`.
public struct ReferralCodeCreated: Decodable, Equatable, Sendable {
    public let codeId: String
    public let code: String

    enum CodingKeys: String, CodingKey {
        case codeId = "code_id"
        case code
    }
}

/// `referral_status`. The referee sees that a code is applied, never whose.
public struct ReferralStatus: Decodable, Equatable, Sendable {
    public let applied: Bool
    /// RFC 3339, as the daemon sent it; the shell formats it.
    public let applyUntil: String?
    public let canApply: Bool

    enum CodingKeys: String, CodingKey {
        case applied
        case applyUntil = "apply_until"
        case canApply = "can_apply"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        applied = try c.decode(Bool.self, forKey: .applied)
        applyUntil = try? c.decodeIfPresent(String.self, forKey: .applyUntil)
        // Never offered on an applied account, whatever was sent.
        canApply = !applied && ((try? c.decodeIfPresent(Bool.self, forKey: .canApply)) ?? false)
    }

    public init(applied: Bool, applyUntil: String? = nil, canApply: Bool = false) {
        self.applied = applied
        self.applyUntil = applyUntil
        self.canApply = !applied && canApply
    }
}

/// Why a referral call did not go through, as the section reacts to it.
public enum ReferralRefusal: Equatable, Sendable {
    /// No Trace Commons account session, and the daemon could not renew one.
    case sessionEnded
    /// The commons refused the code (`referral_not_applied`, whatever the
    /// reason).
    case notApplied
    /// What was typed is not a code (`referral-code-invalid`); nothing was
    /// sent.
    case invalidCode
    /// The commons does not run referrals now (`referral_unavailable`).
    case unavailable
    /// The account has the most live codes it may.
    case limitReached
    /// Anything else. Nothing changed.
    case failed

    /// Routes the daemon's refusal label. Unknown labels are `failed`.
    public static func from(code label: String) -> ReferralRefusal {
        switch label {
        case "account-session-required": return .sessionEnded
        case "referral_not_applied": return .notApplied
        case "referral-code-invalid": return .invalidCode
        case "referral_unavailable": return .unavailable
        case "referral_code_limit_reached": return .limitReached
        default: return .failed
        }
    }
}

/// What the section shows. Pure: every sentence comes from the core's copy.
public struct ReferralSurface: Equatable, Sendable {
    public let copy: ReferralCopy

    public init(copy: ReferralCopy) {
        self.copy = copy
    }

    /// The code as shown, or the core's line for one the commons cannot
    /// show.
    public func shownCode(_ row: ReferralCodeRow) -> String {
        row.code ?? copy.codeHidden
    }

    /// The state beside a code that is not live; nil for a live one. A state
    /// this build does not know reads as off, never as live.
    public func stateLabel(_ row: ReferralCodeRow) -> String? {
        switch row.state {
        case "live": return nil
        case "suspended": return copy.stateSuspended
        case "voided": return copy.stateVoided
        default: return copy.stateDisabled
        }
    }

    public func joinedLine(_ count: UInt64) -> String {
        ReferralCopy.render(count == 1 ? copy.joinedOne : copy.joinedMany, ["n": String(count)])
    }

    /// Copy is offered for a code that can be shown, live or not: someone
    /// who already has an off code may still ask for it.
    public func canCopy(_ row: ReferralCodeRow) -> Bool { row.code != nil }

    /// Turn off is offered for a live code only.
    public func canDisable(_ row: ReferralCodeRow) -> Bool { row.isLive }

    public func copyAccessibilityName(_ row: ReferralCodeRow) -> String {
        ReferralCopy.render(copy.copyAccessibilityName, ["code": shownCode(row)])
    }

    public func disableAccessibilityName(_ row: ReferralCodeRow) -> String {
        ReferralCopy.render(copy.disableAccessibilityName, ["code": shownCode(row)])
    }

    /// Whether the turn-off scope line is drawn: while a live code is listed.
    public func showsDisableScope(_ listing: ReferralCodesListing) -> Bool {
        listing.codes.contains(where: \.isLive)
    }

    /// In place of Create once the account has the most live codes.
    public func limitLine(_ listing: ReferralCodesListing) -> String? {
        listing.canCreate ? nil : ReferralCopy.render(copy.limitReached, ["n": String(listing.maxLiveCodes)])
    }

    /// "Credit from referrals": what was earned, and the part sent when any
    /// was. `money` formats nano-USD (`TCNearAiBalance.amount`, scale 9).
    /// Nil when the daemon did not say, never a zero.
    public func creditLine(_ listing: ReferralCodesListing, money: (UInt64) -> String) -> String? {
        guard let owed = listing.bonusOwedNanoUSD else { return nil }
        let earned = money(owed)
        guard let sent = listing.bonusSentNanoUSD, sent > 0 else { return earned }
        return "\(earned) · \(ReferralCopy.render(copy.creditSent, ["amount": money(sent)]))"
    }

    /// The scope beside the code field. `date` is `apply_until`, formatted.
    public func enterScope(date: String) -> String {
        ReferralCopy.render(copy.enterScope, ["date": date])
    }

    /// Whether the field is drawn: no code applied, and the window open.
    public func showsEnterField(_ status: ReferralStatus?) -> Bool {
        status?.canApply == true
    }

    /// The line for a refusal. `maxLiveCodes` is the listing's limit, which
    /// the limit line names; without it (no listing read, which a Create
    /// cannot outrun) the limit line is not drawn half-filled.
    public func refusalLine(_ refusal: ReferralRefusal, maxLiveCodes: Int? = nil) -> String {
        switch refusal {
        case .sessionEnded: return copy.sessionEnded
        case .notApplied: return copy.notApplied
        case .invalidCode: return copy.invalidCode
        case .unavailable: return copy.unavailable
        case .limitReached:
            guard let maxLiveCodes else { return copy.failed }
            return ReferralCopy.render(copy.limitReached, ["n": String(maxLiveCodes)])
        case .failed: return copy.failed
        }
    }

    /// The field's check before anything is sent: nil for an empty field,
    /// the invalid-code line for text that is not a code. `display` is
    /// `TCConsentCopy.referralCodeDisplay`, the core's own reading.
    public func fieldProblem(_ typed: String, display: (String) -> String?) -> String? {
        let trimmed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return display(trimmed) == nil ? copy.invalidCode : nil
    }
}

/// One stream of a rail's payout status (referral spec section 5, "Own
/// traces vs referrals"): `earned` for the person's own traces, `referral`
/// for the bonus from people who joined with their codes. Nano-USD.
public struct PayoutStreamStatus: Decodable, Equatable, Sendable {
    public let stream: String
    public let owed: UInt64?
    public let sent: UInt64?
    public let waiting: UInt64?
    /// Set while a hold stops the stream being sent.
    public let holdLabel: String?

    enum CodingKeys: String, CodingKey {
        case stream, owed, sent, waiting
        case holdLabel = "hold_label"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        stream = try c.decode(String.self, forKey: .stream)
        owed = nanoUSD(c, .owed)
        sent = nanoUSD(c, .sent)
        waiting = nanoUSD(c, .waiting)
        holdLabel = try? c.decodeIfPresent(String.self, forKey: .holdLabel)
    }

    public init(stream: String, owed: UInt64?, sent: UInt64? = nil, waiting: UInt64? = nil,
                holdLabel: String? = nil) {
        self.stream = stream
        self.owed = owed
        self.sent = sent
        self.waiting = waiting
        self.holdLabel = holdLabel
    }
}

/// The two rows the Insights NEAR AI credit card adds: credit from the
/// person's own traces and credit from referrals. Pure, for tests.
public enum NearAiEarningsSplit {
    public struct Row: Equatable, Sendable, Identifiable {
        /// `earned` or `referral`.
        public let id: String
        public let label: String
        public let value: String
    }

    /// The rows, own traces first; none when the rail reports no streams (a
    /// daemon or commons that predates them), and none for a stream whose
    /// amount is not known. The referral row is drawn only once there is
    /// something in it, so an account that never referred anyone sees only
    /// its own row.
    public static func rows(_ streams: [PayoutStreamStatus]?, copy: ReferralCopy,
                            money: (UInt64) -> String) -> [Row] {
        guard let streams else { return [] }
        func row(_ name: String, _ label: String, skipEmpty: Bool) -> Row? {
            guard let stream = streams.first(where: { $0.stream == name }), let owed = stream.owed
            else { return nil }
            if skipEmpty, owed == 0, stream.holdLabel == nil { return nil }
            let amount = money(owed)
            let value = stream.holdLabel == nil
                ? amount : ReferralCopy.render(copy.insightsOnHold, ["amount": amount])
            return Row(id: name, label: label, value: value)
        }
        // Only beside a referral figure does the split say anything.
        guard let referral = row("referral", copy.insightsReferrals, skipEmpty: true) else { return [] }
        return [row("earned", copy.insightsOwn, skipEmpty: false), referral].compactMap { $0 }
    }
}

/// The five referral IPC calls over any `DaemonTransport`, so the method
/// names and parameters are asserted without a daemon.
public struct ReferralClient {
    /// A daemon refusal: its fixed label, safe to route on and never shown.
    public struct Failure: Error, Equatable {
        public let label: String
        public var refusal: ReferralRefusal { ReferralRefusal.from(code: label) }
    }

    private let transport: any DaemonTransport

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    public func list() throws -> ReferralCodesListing {
        try call("referral_codes_list", [:], as: ReferralCodesListing.self)
    }

    public func create() throws -> ReferralCodeCreated {
        try call("referral_codes_create", [:], as: ReferralCodeCreated.self)
    }

    public func disable(codeId: String) throws {
        _ = try call("referral_codes_disable", ["code_id": codeId], as: Ignored.self)
    }

    /// `code` as typed; the daemon reads it and sends only its canonical
    /// form.
    public func apply(code: String) throws {
        _ = try call("referral_apply", ["code": code], as: Ignored.self)
    }

    public func status() throws -> ReferralStatus {
        try call("referral_status", [:], as: ReferralStatus.self)
    }

    private struct Ignored: Decodable {}

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
