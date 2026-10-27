import Foundation

/// The Devices section's words, decoded from what the Rust exported
/// (`tc_account_devices_copy_json`).
///
/// Every string is a draft awaiting owner approval. None is written in
/// Swift: a payload with a missing or empty field is refused whole. Templates
/// carry one placeholder each, `{device}`, `{code}` or `{when}`, filled by
/// `render(_:_:with:)`.
public struct AccountDevicesCopy: Decodable, Equatable, Sendable {
    public let title: String
    public let body: String
    public let thisDevice: String
    public let unnamed: String
    public let lastSeen: String
    public let linkButton: String
    public let linkAccessibilityName: String
    public let linkBody: String
    public let codeCommand: String
    public let copyButton: String
    public let copied: String
    public let waiting: String
    public let linked: String
    public let expired: String
    public let doneButton: String
    public let unlinkButton: String
    public let unlinkAccessibilityName: String
    public let unlinkConfirm: String
    public let unlinkConfirmButton: String
    public let cancelButton: String
    public let sessionEnded: String
    public let tooMany: String
    public let failed: String

    enum CodingKeys: String, CodingKey {
        case title, body, unnamed, waiting, linked, expired, copied, failed
        case thisDevice = "this_device"
        case lastSeen = "last_seen"
        case linkButton = "link_button"
        case linkAccessibilityName = "link_accessibility_name"
        case linkBody = "link_body"
        case codeCommand = "code_command"
        case copyButton = "copy_button"
        case doneButton = "done_button"
        case unlinkButton = "unlink_button"
        case unlinkAccessibilityName = "unlink_accessibility_name"
        case unlinkConfirm = "unlink_confirm"
        case unlinkConfirmButton = "unlink_confirm_button"
        case cancelButton = "cancel_button"
        case sessionEnded = "session_ended"
        case tooMany = "too_many"
    }

    /// The payload fields this shell decodes, by wire name; compared with the
    /// live export by `TCBridgeTests`.
    public static let consumedFields = [
        "title", "body", "this_device", "unnamed", "last_seen", "link_button",
        "link_accessibility_name", "link_body", "code_command", "copy_button", "copied",
        "waiting", "linked", "expired", "done_button", "unlink_button",
        "unlink_accessibility_name", "unlink_confirm", "unlink_confirm_button",
        "cancel_button", "session_ended", "too_many", "failed",
    ]

    public var sentences: [String] {
        [
            title, body, thisDevice, unnamed, lastSeen, linkButton, linkAccessibilityName,
            linkBody, codeCommand, copyButton, copied, waiting, linked, expired, doneButton,
            unlinkButton, unlinkAccessibilityName, unlinkConfirm, unlinkConfirmButton,
            cancelButton, sessionEnded, tooMany, failed,
        ]
    }

    /// Nil, never a partly-filled value, for an unparseable payload or an
    /// empty field.
    public static func decode(fromJSON json: String) -> AccountDevicesCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(AccountDevicesCopy.self, from: data)
        else { return nil }
        return copy.sentences.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty })
            ? nil : copy
    }

    /// `template` with `{placeholder}` filled in.
    public static func render(_ template: String, _ placeholder: String, with value: String) -> String {
        template.replacingOccurrences(of: "{\(placeholder)}", with: value)
    }
}

/// One device linked to the account, as `account_devices_list` relays it.
/// Strings are carried as sent; dates are the server's RFC 3339 text.
public struct AccountDevice: Decodable, Equatable, Hashable, Identifiable, Sendable {
    public let deviceKeyId: String
    public let label: String?
    public let origin: String?
    public let lastSeenAt: String?
    public let thisDevice: Bool
    /// How many of the device's traces still stand in the commons, as the
    /// server counts them; nil from a server too old to say, never 0.
    public let contributedTraces: Int?
    /// When the newest of those was received (RFC 3339 text).
    public let lastContributedAt: String?

    public var id: String { deviceKeyId }

    enum CodingKeys: String, CodingKey {
        case label, origin
        case deviceKeyId = "device_key_id"
        case lastSeenAt = "last_seen_at"
        case thisDevice = "this_device"
        case contributedTraces = "contributed_traces"
        case lastContributedAt = "last_contributed_at"
    }

    public init(
        deviceKeyId: String, label: String? = nil, origin: String? = nil,
        lastSeenAt: String? = nil, thisDevice: Bool = false,
        contributedTraces: Int? = nil, lastContributedAt: String? = nil
    ) {
        self.deviceKeyId = deviceKeyId
        self.label = label
        self.origin = origin
        self.lastSeenAt = lastSeenAt
        self.thisDevice = thisDevice
        self.contributedTraces = contributedTraces
        self.lastContributedAt = lastContributedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        deviceKeyId = try c.decode(String.self, forKey: .deviceKeyId)
        label = try c.decodeIfPresent(String.self, forKey: .label)
        origin = try c.decodeIfPresent(String.self, forKey: .origin)
        lastSeenAt = try c.decodeIfPresent(String.self, forKey: .lastSeenAt)
        thisDevice = try c.decodeIfPresent(Bool.self, forKey: .thisDevice) ?? false
        contributedTraces = try c.decodeIfPresent(Int.self, forKey: .contributedTraces)
        lastContributedAt = try c.decodeIfPresent(String.self, forKey: .lastContributedAt)
    }
}

/// A minted one-time code. Shown once; never stored by the shell.
public struct DeviceCode: Decodable, Equatable, Sendable {
    public let codeId: String
    public let code: String
    public let expiresAt: String

    enum CodingKeys: String, CodingKey {
        case code
        case codeId = "code_id"
        case expiresAt = "expires_at"
    }

    public init(codeId: String, code: String, expiresAt: String) {
        self.codeId = codeId
        self.code = code
        self.expiresAt = expiresAt
    }

    /// `expiresAt` read as a date, or nil when the server's text is not
    /// RFC 3339 (the miss cap then bounds the wait on its own).
    public var expiryDate: Date? {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return withFraction.date(from: expiresAt) ?? ISO8601DateFormatter().date(from: expiresAt)
    }
}

/// `account_device_code_status`: `pending`, `redeemed` or `expired`.
public struct DeviceCodeStatus: Decodable, Equatable, Sendable {
    public let state: String
    public let deviceKeyId: String?

    enum CodingKeys: String, CodingKey {
        case state
        case deviceKeyId = "device_key_id"
    }

    public init(state: String, deviceKeyId: String? = nil) {
        self.state = state
        self.deviceKeyId = deviceKeyId
    }
}

/// A refusal, routed from the daemon's fixed label.
public enum AccountDevicesRefusal: Equatable, Sendable {
    /// No account session, and the daemon could not renew one.
    case sessionEnded
    /// Too many codes outstanding or minted recently.
    case tooMany
    /// Anything else. Nothing changed.
    case failed

    public static func from(code label: String) -> AccountDevicesRefusal {
        switch label {
        case "account-session-required":
            return .sessionEnded
        case "device_code_too_many", "account_devices_rate_limited":
            return .tooMany
        default:
            return .failed
        }
    }
}

/// Linking a server, one code at a time.
public enum DeviceLinkPhase: Equatable, Sendable {
    case idle
    case minting
    /// The code is on screen and the server has not used it yet.
    case waiting(DeviceCode)
    /// The server used the code.
    case linked(deviceKeyId: String?)
    case expired
    case refused(AccountDevicesRefusal)

    /// Whether a code is on screen, so the shell keeps asking about it.
    public var code: DeviceCode? {
        if case .waiting(let code) = self { return code }
        return nil
    }

    /// How many status asks in a row may go unanswered before the shell
    /// stops asking and says it failed.
    public static let maxMissedAnswers = 5

    /// The phase once the clock reads `now`: a code past its expiry is
    /// expired whether or not the daemon answered.
    public func checked(at now: Date, for codeId: String) -> DeviceLinkPhase {
        guard let code, code.codeId == codeId, let expiry = code.expiryDate, now >= expiry
        else { return self }
        return .expired
    }

    /// The phase after `missed` status asks in a row went unanswered.
    public func afterMissed(_ missed: Int, for codeId: String) -> DeviceLinkPhase {
        guard let code, code.codeId == codeId, missed >= Self.maxMissedAnswers else { return self }
        return .refused(.failed)
    }

    /// The phase after one status answer for the code on screen. An answer
    /// for another code, or for no code, changes nothing.
    public func after(_ status: DeviceCodeStatus, for codeId: String) -> DeviceLinkPhase {
        guard let code, code.codeId == codeId else { return self }
        switch status.state {
        case "redeemed": return .linked(deviceKeyId: status.deviceKeyId)
        case "expired": return .expired
        default: return self
        }
    }
}

/// What the section shows. Pure: every sentence comes from the core's copy
/// and every fact from the daemon.
public struct AccountDevicesSurface: Equatable, Sendable {
    public let copy: AccountDevicesCopy

    public init(copy: AccountDevicesCopy) {
        self.copy = copy
    }

    /// The device's name: its label, or the unnamed line.
    public func name(for device: AccountDevice) -> String {
        let label = device.label?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return label.isEmpty ? copy.unnamed : label
    }

    /// "Last seen ..." with `when` already formatted by the shell, or nil
    /// when the server has never seen the device.
    public func lastSeenLine(for device: AccountDevice, formatted: (String) -> String?) -> String? {
        guard let at = device.lastSeenAt, let when = formatted(at) else { return nil }
        return AccountDevicesCopy.render(copy.lastSeen, "when", with: when)
    }

    /// The command the person runs on the server.
    public func command(for code: DeviceCode) -> String {
        AccountDevicesCopy.render(copy.codeCommand, "code", with: code.code)
    }

    /// Whether this device can be unlinked from here. This device signs
    /// out instead, so it gets no Unlink.
    public func canUnlink(_ device: AccountDevice) -> Bool {
        !device.thisDevice
    }

    public func unlinkConfirm(for device: AccountDevice) -> String {
        AccountDevicesCopy.render(copy.unlinkConfirm, "device", with: name(for: device))
    }

    public func unlinkAccessibilityName(for device: AccountDevice) -> String {
        AccountDevicesCopy.render(copy.unlinkAccessibilityName, "device", with: name(for: device))
    }

    public func refusalLine(_ refusal: AccountDevicesRefusal) -> String {
        switch refusal {
        case .sessionEnded: return copy.sessionEnded
        case .tooMany: return copy.tooMany
        case .failed: return copy.failed
        }
    }

    /// This device first, then the others in the daemon's order.
    public static func ordered(_ devices: [AccountDevice]) -> [AccountDevice] {
        devices.filter(\.thisDevice) + devices.filter { !$0.thisDevice }
    }
}

/// The device IPC calls over any `DaemonTransport`, so the method names and
/// parameters are asserted without a daemon. Calls block: run them off the
/// main actor.
public struct AccountDevicesClient {
    /// A daemon refusal: its fixed label, safe to route on and never shown.
    public struct Failure: Error, Equatable {
        public let label: String
        public var refusal: AccountDevicesRefusal { AccountDevicesRefusal.from(code: label) }
    }

    private let transport: any DaemonTransport

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    public func list() throws -> [AccountDevice] {
        struct Reply: Decodable { let devices: [AccountDevice] }
        return try call("account_devices_list", [:], as: Reply.self).devices
    }

    public func mint(label: String?) throws -> DeviceCode {
        var params: [String: Any] = [:]
        if let label { params["label"] = label }
        return try call("account_device_code_mint", params, as: DeviceCode.self)
    }

    public func status(codeId: String) throws -> DeviceCodeStatus {
        try call("account_device_code_status", ["code_id": codeId], as: DeviceCodeStatus.self)
    }

    public func revoke(deviceKeyId: String) throws {
        _ = try call("account_device_revoke", ["device_key_id": deviceKeyId], as: Ignored.self)
    }

    public func rename(deviceKeyId: String, label: String?) throws {
        var params: [String: Any] = ["device_key_id": deviceKeyId]
        if let label { params["label"] = label }
        _ = try call("account_device_rename", params, as: Ignored.self)
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
