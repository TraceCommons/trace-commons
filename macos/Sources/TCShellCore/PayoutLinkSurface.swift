import Foundation

/// The NEAR AI payout section's words, decoded from what the Rust exported
/// (`tc_payout_link_copy_json`).
///
/// The owner approved this wording as written on 2026-10-23; the Change
/// button's two strings and `sessionEnded`, added after, are a draft. None is written in
/// Swift: a payload with a missing or empty field is refused whole, so a
/// blank never stands where a consent sentence belongs. Templates carry the
/// one placeholder `{org_name}`, filled by `render(_:orgName:)` with the name
/// the daemon read from NEAR AI.
public struct PayoutLinkCopy: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let scope: String
    public let button: String
    public let secondary: String
    public let accessibilityName: String
    public let unlinkButton: String
    public let unlinkAccessibilityName: String
    public let unlinkBody: String
    public let changeButton: String
    public let changeAccessibilityName: String
    public let statusLinked: String
    public let statusNone: String
    public let statusWaiting: String
    public let statusUnavailable: String
    public let statusMismatch: String
    public let signIn: String
    public let sessionEnded: String
    public let noOrganization: String
    public let changed: String
    public let failed: String

    enum CodingKeys: String, CodingKey {
        case title, body, scope, button, secondary
        case accessibilityName = "accessibility_name"
        case unlinkButton = "unlink_button"
        case unlinkAccessibilityName = "unlink_accessibility_name"
        case unlinkBody = "unlink_body"
        case changeButton = "change_button"
        case changeAccessibilityName = "change_accessibility_name"
        case statusLinked = "status_linked"
        case statusNone = "status_none"
        case statusWaiting = "status_waiting"
        case statusUnavailable = "status_unavailable"
        case statusMismatch = "status_mismatch"
        case signIn = "sign_in"
        case sessionEnded = "session_ended"
        case noOrganization = "no_organization"
        case changed, failed
    }

    /// The payload fields this shell decodes, by wire name; compared with the
    /// live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "scope", "button", "secondary", "accessibility_name",
        "unlink_button", "unlink_accessibility_name", "unlink_body",
        "change_button", "change_accessibility_name", "status_linked", "status_none", "status_waiting", "status_unavailable",
        "status_mismatch", "sign_in", "session_ended", "no_organization", "changed", "failed",
    ]

    public var sentences: [String] {
        [
            title, body, scope, button, secondary, accessibilityName, unlinkButton,
            unlinkAccessibilityName, unlinkBody, changeButton, changeAccessibilityName,
            statusLinked, statusNone, statusWaiting,
            statusUnavailable, statusMismatch, signIn, sessionEnded, noOrganization, changed, failed,
        ]
    }

    /// Nil, never a partly-filled value, for an unparseable payload or an
    /// empty field.
    public static func decode(fromJSON json: String) -> PayoutLinkCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(PayoutLinkCopy.self, from: data)
        else { return nil }
        return copy.sentences.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty })
            ? nil : copy
    }

    /// `template` with `{org_name}` filled in.
    public static func render(_ template: String, orgName: String) -> String {
        template.replacingOccurrences(of: "{org_name}", with: orgName)
    }
}

/// An organization the signed-in NEAR AI user owns and may choose. The name
/// is never sent to the commons; the daemon keeps the linked org's name on
/// this device so status shows it after a restart.
public struct PayoutOrganization: Decodable, Equatable, Hashable, Identifiable, Sendable {
    public let id: String
    public let name: String

    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

/// `payout_link_orgs`: the owned active orgs and the revision over what was
/// shown, which the start must echo.
public struct PayoutOrganizationListing: Decodable, Equatable, Sendable {
    public let organizations: [PayoutOrganization]
    public let orgsRevision: String

    enum CodingKeys: String, CodingKey {
        case organizations
        case orgsRevision = "orgs_revision"
    }

    public init(organizations: [PayoutOrganization], orgsRevision: String) {
        self.organizations = organizations
        self.orgsRevision = orgsRevision
    }
}

/// One rail in `payout_rails_status`. Strings are the daemon's own labels,
/// carried as sent so a state a later daemon adds is not misread.
public struct PayoutRailStatus: Decodable, Equatable, Sendable {
    public let railId: String
    public let mode: String
    public let linked: Bool
    public let state: String?
    public let version: Int64
    public let requestState: String?
    public let unlinkRequested: Bool
    public let recipientUnavailable: Bool
    /// `match`, `mismatch` or `unknown`; nil when the commons reports no ref.
    public let recipientRefCheck: String?
    /// The linked org's name, from the daemon's org list or the name it
    /// saved for this link; nil when it has neither.
    public let organizationName: String?
    /// The locally saved organization ID, only when its recipient ref matches.
    public let organizationID: String?
    /// What each payout stream owes, sent and waits (referral spec section
    /// 5): `earned` for the person's own traces, `referral` for the bonus
    /// from referrals. Nil from a daemon or commons that predates streams,
    /// and for a `streams` this build cannot read, which never fails the
    /// rail.
    public let streams: [PayoutStreamStatus]?

    enum CodingKeys: String, CodingKey {
        case railId = "rail_id"
        case mode, linked, state, version
        case requestState = "request_state"
        case unlinkRequested = "unlink_requested"
        case recipientUnavailable = "recipient_unavailable"
        case recipientRefCheck = "recipient_ref_check"
        case organizationName = "organization_name"
        case organizationID = "organization_id"
        case streams
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        railId = try c.decode(String.self, forKey: .railId)
        mode = try c.decode(String.self, forKey: .mode)
        linked = try c.decode(Bool.self, forKey: .linked)
        state = try c.decodeIfPresent(String.self, forKey: .state)
        version = try c.decode(Int64.self, forKey: .version)
        requestState = try c.decodeIfPresent(String.self, forKey: .requestState)
        unlinkRequested = try c.decodeIfPresent(Bool.self, forKey: .unlinkRequested) ?? false
        recipientUnavailable =
            try c.decodeIfPresent(Bool.self, forKey: .recipientUnavailable) ?? false
        recipientRefCheck = try c.decodeIfPresent(String.self, forKey: .recipientRefCheck)
        organizationName = try c.decodeIfPresent(String.self, forKey: .organizationName)
        organizationID = try c.decodeIfPresent(String.self, forKey: .organizationID)
        streams = try? c.decodeIfPresent([PayoutStreamStatus].self, forKey: .streams)
    }

    public init(
        railId: String = PayoutLinkSurface.nearAiRailId, mode: String = "live", linked: Bool = false,
        state: String? = nil, version: Int64 = 0, requestState: String? = nil,
        unlinkRequested: Bool = false, recipientUnavailable: Bool = false,
        recipientRefCheck: String? = nil, organizationName: String? = nil,
        organizationID: String? = nil, streams: [PayoutStreamStatus]? = nil
    ) {
        self.railId = railId
        self.mode = mode
        self.linked = linked
        self.state = state
        self.version = version
        self.requestState = requestState
        self.unlinkRequested = unlinkRequested
        self.recipientUnavailable = recipientUnavailable
        self.recipientRefCheck = recipientRefCheck
        self.organizationName = organizationName
        self.organizationID = organizationID
        self.streams = streams
    }
}

/// Why a start or confirm did not go through, as the section reacts to it.
public enum PayoutLinkRefusal: Equatable, Sendable {
    /// No NEAR AI session, or it ended: the sign-in line.
    case signIn
    /// No Trace Commons account session, and the daemon could not renew
    /// one: the session-ended line.
    case sessionEnded
    /// The org list or the account's link changed after it was shown: list
    /// again.
    case changed
    /// Anything else. Nothing was saved.
    case failed

    /// Routes the daemon's refusal label. Unknown labels are `failed`, the
    /// plain "nothing changed" line.
    public static func from(code label: String) -> PayoutLinkRefusal {
        switch label {
        case "payout-link-near-ai-sign-in-required", "payout-link-near-ai-session-expired",
            "payout-link-near-ai-session-changed":
            return .signIn
        case "account-session-required":
            return .sessionEnded
        case "payout-link-orgs-changed", "payout-link-changed", "version_conflict",
            "org_not_owned", "payout_link_change_pending":
            return .changed
        default:
            return .failed
        }
    }
}

/// What the section shows for one rail. Pure: every sentence comes from the
/// core's copy and every fact from the daemon's status.
public struct PayoutLinkSurface: Equatable, Sendable {
    public static let nearAiRailId = "near_ai"

    public let copy: PayoutLinkCopy

    public init(copy: PayoutLinkCopy) {
        self.copy = copy
    }

    /// The rail's status when it is enabled (`dry_run` or `live`), else nil:
    /// the section hides itself unless the commons lists `near_ai`.
    public static func nearAiRail(in rails: [PayoutRailStatus]) -> PayoutRailStatus? {
        rails.first { $0.railId == nearAiRailId && ($0.mode == "live" || $0.mode == "dry_run") }
    }

    /// The refusal of a status read the section stays on screen for, with
    /// its line: only a missing account session, which the person can act
    /// on. Any other failure hides the section, so no stale link state is
    /// shown.
    public static func shownStatusRefusal(_ error: any Error) -> PayoutLinkRefusal? {
        let refusal = (error as? PayoutLinkClient.Failure)?.refusal
        return refusal == .sessionEnded ? refusal : nil
    }

    /// The status line, or nil when the line needs an org name this daemon
    /// does not hold in memory (`needsOrgName`). A mismatch is never hidden
    /// behind a friendlier state.
    public func statusLine(for rail: PayoutRailStatus) -> String? {
        if rail.recipientRefCheck == "mismatch" { return copy.statusMismatch }
        if rail.recipientUnavailable { return copy.statusUnavailable }
        guard rail.linked, !rail.unlinkRequested else { return copy.statusNone }
        switch rail.state {
        case "active":
            return named(copy.statusLinked, rail)
        case "waiting":
            return named(copy.statusWaiting, rail)
        default:
            // A closing link, or a state this build does not know: credit
            // is not promised anywhere.
            return copy.statusNone
        }
    }

    private func named(_ template: String, _ rail: PayoutRailStatus) -> String? {
        rail.organizationName.map { PayoutLinkCopy.render(template, orgName: $0) }
    }

    /// Whether the status line is waiting on a name only the org list can
    /// supply.
    public func needsOrgName(for rail: PayoutRailStatus) -> Bool {
        statusLine(for: rail) == nil
    }

    /// Whether the person may choose an organization now. A live link is
    /// changed with `canChange` instead; a mismatch or a gone org is chosen
    /// over directly.
    public func canChoose(for rail: PayoutRailStatus) -> Bool {
        if rail.recipientRefCheck == "mismatch" || rail.recipientUnavailable { return true }
        return !rail.linked || rail.unlinkRequested
    }

    /// Whether stop is offered: an active link nothing is closing, whose
    /// name is known for the button's accessibility name.
    public func canUnlink(for rail: PayoutRailStatus) -> Bool {
        rail.linked && !rail.unlinkRequested && rail.state == "active"
            && rail.organizationName != nil
    }

    /// Whether the direct change is offered: an active link nothing is
    /// closing. A mismatch or a gone org is chosen over with `canChoose`
    /// instead, and a waiting link already has a change in flight. The
    /// change's words name no org, so it does not wait on the name.
    public func canChange(for rail: PayoutRailStatus) -> Bool {
        rail.linked && !rail.unlinkRequested && rail.state == "active"
            && rail.recipientRefCheck != "mismatch" && !rail.recipientUnavailable
    }

    /// Whether the Change button is drawn: on an active link, except while
    /// the change's list or consent is on screen. After a refusal or an
    /// empty list it is back, so the person can list again.
    public func showsChangeButton(for rail: PayoutRailStatus, _ picker: PayoutPicker) -> Bool {
        guard canChange(for: rail) else { return false }
        guard picker.changing else { return true }
        switch picker.phase {
        case .loading, .choosing, .working: return false
        case .idle, .noOrganization, .refused: return true
        }
    }

    /// Whether the org list and consent are drawn: when the person may
    /// choose, or while they are changing an active link.
    public func showsPicker(for rail: PayoutRailStatus, _ picker: PayoutPicker) -> Bool {
        canChoose(for: rail) || picker.changing
    }

    public func unlinkAccessibilityName(for rail: PayoutRailStatus) -> String? {
        rail.organizationName.map {
            PayoutLinkCopy.render(copy.unlinkAccessibilityName, orgName: $0)
        }
    }

    public func refusalLine(_ refusal: PayoutLinkRefusal) -> String {
        switch refusal {
        case .signIn: return copy.signIn
        case .sessionEnded: return copy.sessionEnded
        case .changed: return copy.changed
        case .failed: return copy.failed
        }
    }
}

/// The picker's state. Nothing is ever preselected, however many orgs there
/// are, and choosing needs the person's click after the consent is shown.
public struct PayoutPicker: Equatable, Sendable {
    public enum Phase: Equatable, Sendable {
        case idle
        case loading
        /// The owned active orgs, none selected until the person picks one.
        case choosing
        /// The person owns no active org.
        case noOrganization
        /// A start and confirm are in flight.
        case working
        case refused(PayoutLinkRefusal)
    }

    public private(set) var phase: Phase = .idle
    public private(set) var listing: PayoutOrganizationListing?
    public private(set) var selectedId: String?
    /// The person pressed Change on an active link: the list and consent are
    /// shown over it until they choose, leave, or the list is lost.
    public private(set) var changing = false

    public init() {}

    public var rows: [PayoutOrganization] { listing?.organizations ?? [] }

    public var selected: PayoutOrganization? {
        guard let selectedId else { return nil }
        return rows.first { $0.id == selectedId }
    }

    public mutating func beginLoading() {
        phase = .loading
    }

    /// Change on an active link: list the orgs, then the same consent and
    /// Choose as a first link, sent against the version shown.
    public mutating func beginChange() {
        changing = true
        selectedId = nil
        phase = .loading
    }

    /// A fresh list replaces the old one and clears any selection: a choice
    /// made against a list that has since changed is not carried over.
    public mutating func listed(_ listing: PayoutOrganizationListing) {
        self.listing = listing
        selectedId = nil
        phase = listing.organizations.isEmpty ? .noOrganization : .choosing
    }

    public mutating func select(_ id: String?) {
        guard phase == .choosing, let id, rows.contains(where: { $0.id == id }) else { return }
        selectedId = id
    }

    /// Back to the list without a choice ("Not now"). During a change it
    /// leaves the change: the active link stays as it was.
    public mutating func dismissSelection() {
        selectedId = nil
        if changing {
            changing = false
            listing = nil
            phase = .idle
            return
        }
        if phase == .choosing || phase == .refused(.failed) {
            phase = rows.isEmpty ? .idle : .choosing
        }
    }

    public mutating func beginWorking() {
        guard selected != nil, phase == .choosing else { return }
        phase = .working
    }

    /// A refusal other than a plain failure drops the list: after a changed
    /// list or a lost sign-in the rows on screen are not to be trusted, and
    /// a retry lists again.
    public mutating func refused(_ refusal: PayoutLinkRefusal) {
        selectedId = nil
        if refusal != .failed {
            listing = nil
            changing = false
        }
        phase = .refused(refusal)
    }

    public mutating func finished() {
        selectedId = nil
        changing = false
        listing = nil
        phase = .idle
    }

    /// The consent for the selected org: shown before the button, never after.
    public struct Consent: Equatable, Sendable {
        public let orgName: String
        public let body: String
        public let scope: String
        public let button: String
        public let accessibilityName: String
        public let secondary: String
    }

    public func consent(copy: PayoutLinkCopy) -> Consent? {
        guard let org = selected, phase == .choosing || phase == .working else { return nil }
        return Consent(
            orgName: org.name,
            body: PayoutLinkCopy.render(copy.body, orgName: org.name),
            scope: PayoutLinkCopy.render(copy.scope, orgName: org.name),
            button: copy.button,
            accessibilityName: PayoutLinkCopy.render(copy.accessibilityName, orgName: org.name),
            secondary: copy.secondary)
    }
}

/// The five payout-link IPC calls over any `DaemonTransport`, so the method
/// names and parameters are asserted without a daemon.
public struct PayoutLinkClient {
    /// A daemon refusal: its fixed label, safe to route on and never shown.
    public struct Failure: Error, Equatable {
        public let label: String
        public var refusal: PayoutLinkRefusal { PayoutLinkRefusal.from(code: label) }
    }

    private let transport: any DaemonTransport

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    public func status() throws -> [PayoutRailStatus] {
        struct Reply: Decodable { let rails: [PayoutRailStatus] }
        return try call("payout_rails_status", [:], as: Reply.self).rails
    }

    public func organizations() throws -> PayoutOrganizationListing {
        try call("payout_link_orgs", [:], as: PayoutOrganizationListing.self)
    }

    /// Starts linking; nothing is saved until `confirm`. `expectedVersion` is
    /// the `version` the person was shown, never defaulted.
    public func startLink(
        railId: String, organizationId: String, orgsRevision: String, expectedVersion: Int64
    ) throws {
        _ = try call(
            "payout_link_start",
            [
                "rail_id": railId, "organization_id": organizationId,
                "orgs_revision": orgsRevision, "expected_current_version": expectedVersion,
            ], as: Ignored.self)
    }

    public func startUnlink(railId: String, expectedVersion: Int64) throws {
        _ = try call(
            "payout_link_unlink",
            ["rail_id": railId, "expected_current_version": expectedVersion], as: Ignored.self)
    }

    /// The person's confirmation. Names the action and org that were started.
    public func confirmLink(railId: String, organizationId: String) throws {
        _ = try call(
            "payout_link_confirm",
            [
                "rail_id": railId, "action": "link", "organization_id": organizationId,
                "confirmed": true,
            ], as: Ignored.self)
    }

    public func confirmUnlink(railId: String) throws {
        _ = try call(
            "payout_link_confirm",
            ["rail_id": railId, "action": "unlink", "confirmed": true], as: Ignored.self)
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
