import XCTest

@testable import TCShellCore

/// The Devices section's pure logic, without the dylib. The words are
/// placeholders ("T:" prefixed) so a test passes only by reading the
/// payload; `TCBridgeTests` checks the live export's keys.
final class AccountDevicesSurfaceTests: XCTestCase {
    private static func copyJSON(_ override: [String: String] = [:]) -> String {
        var object = [String: String]()
        for field in AccountDevicesCopy.consumedFields {
            object[field] = "T:\(field) {device}{code}{when}"
        }
        for (key, value) in override { object[key] = value }
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private let surface = AccountDevicesSurface(
        copy: AccountDevicesCopy.decode(fromJSON: copyJSON())!)

    func testAPayloadWithAMissingOrBlankFieldIsRefusedWhole() {
        XCTAssertNil(AccountDevicesCopy.decode(fromJSON: Self.copyJSON(["title": "  "])))
        var object = try! JSONSerialization.jsonObject(with: Data(Self.copyJSON().utf8)) as! [String: String]
        object.removeValue(forKey: "unlink_confirm")
        let data = try! JSONSerialization.data(withJSONObject: object)
        XCTAssertNil(AccountDevicesCopy.decode(fromJSON: String(decoding: data, as: UTF8.self)))
    }

    func testANameFallsBackToTheUnnamedLine() {
        XCTAssertEqual(surface.name(for: AccountDevice(deviceKeyId: "a", label: "box")), "box")
        XCTAssertEqual(
            surface.name(for: AccountDevice(deviceKeyId: "a", label: " ")),
            surface.copy.unnamed)
        XCTAssertEqual(surface.name(for: AccountDevice(deviceKeyId: "a")), surface.copy.unnamed)
    }

    func testTheCommandCarriesTheCode() {
        let code = DeviceCode(codeId: "c1", code: "tcd1_dA.s", expiresAt: "x")
        XCTAssertEqual(surface.command(for: code), "T:code_command {device}tcd1_dA.s{when}")
    }

    func testThisDeviceIsListedFirstAndCannotBeUnlinked() {
        let other = AccountDevice(deviceKeyId: "b")
        let mine = AccountDevice(deviceKeyId: "a", thisDevice: true)
        XCTAssertEqual(AccountDevicesSurface.ordered([other, mine]), [mine, other])
        XCTAssertFalse(surface.canUnlink(mine))
        XCTAssertTrue(surface.canUnlink(other))
        XCTAssertEqual(
            surface.unlinkConfirm(for: AccountDevice(deviceKeyId: "b", label: "box")),
            "T:unlink_confirm box{code}{when}")
    }

    func testLastSeenIsShownOnlyWhenTheServerSawTheDevice() {
        XCTAssertNil(surface.lastSeenLine(for: AccountDevice(deviceKeyId: "a"), formatted: { $0 }))
        XCTAssertEqual(
            surface.lastSeenLine(
                for: AccountDevice(deviceKeyId: "a", lastSeenAt: "2026-10-27"), formatted: { _ in "today" }),
            "T:last_seen {device}{code}today")
    }

    func testTheLinkPhaseFollowsOnlyItsOwnCode() {
        let code = DeviceCode(codeId: "c1", code: "tcd1_dA.s", expiresAt: "x")
        let waiting = DeviceLinkPhase.waiting(code)
        XCTAssertEqual(waiting.after(DeviceCodeStatus(state: "pending"), for: "c1"), waiting)
        XCTAssertEqual(waiting.after(DeviceCodeStatus(state: "redeemed", deviceKeyId: "d"), for: "c2"), waiting)
        XCTAssertEqual(
            waiting.after(DeviceCodeStatus(state: "redeemed", deviceKeyId: "d"), for: "c1"),
            .linked(deviceKeyId: "d"))
        XCTAssertEqual(waiting.after(DeviceCodeStatus(state: "expired"), for: "c1"), .expired)
        XCTAssertEqual(DeviceLinkPhase.idle.after(DeviceCodeStatus(state: "expired"), for: "c1"), .idle)
    }

    func testTheLinkPhaseEndsAtTheCodesExpiry() {
        let code = DeviceCode(codeId: "c1", code: "tcd1_dA.s", expiresAt: "2026-10-27T00:10:00Z")
        let waiting = DeviceLinkPhase.waiting(code)
        let expiry = code.expiryDate!
        XCTAssertEqual(waiting.checked(at: expiry.addingTimeInterval(-1), for: "c1"), waiting)
        XCTAssertEqual(waiting.checked(at: expiry, for: "c1"), .expired)
        XCTAssertEqual(waiting.checked(at: expiry, for: "c2"), waiting)
        let fractional = DeviceCode(codeId: "c1", code: "x", expiresAt: "2026-10-27T00:10:00.250Z")
        XCTAssertNotNil(fractional.expiryDate)
        let unreadable = DeviceLinkPhase.waiting(DeviceCode(codeId: "c1", code: "x", expiresAt: "soon"))
        XCTAssertEqual(unreadable.checked(at: .distantFuture, for: "c1"), unreadable)
    }

    func testTheLinkPhaseStopsAfterARunOfMissedAnswers() {
        let waiting = DeviceLinkPhase.waiting(DeviceCode(codeId: "c1", code: "x", expiresAt: "soon"))
        let cap = DeviceLinkPhase.maxMissedAnswers
        XCTAssertEqual(waiting.afterMissed(cap - 1, for: "c1"), waiting)
        XCTAssertEqual(waiting.afterMissed(cap, for: "c1"), .refused(.failed))
        XCTAssertEqual(waiting.afterMissed(cap, for: "c2"), waiting)
    }

    func testRefusalsRoute() {
        XCTAssertEqual(AccountDevicesRefusal.from(code: "account-session-required"), .sessionEnded)
        XCTAssertEqual(AccountDevicesRefusal.from(code: "device_code_too_many"), .tooMany)
        XCTAssertEqual(AccountDevicesRefusal.from(code: "anything-new"), .failed)
        XCTAssertEqual(surface.refusalLine(.tooMany), surface.copy.tooMany)
    }
}

/// The device IPC calls: the method names and parameters the daemon reads.
final class AccountDevicesClientTests: XCTestCase {
    private final class Fake: DaemonTransport {
        var calls: [(String, [String: Any])] = []
        var replies: [String]
        init(_ replies: [String]) { self.replies = replies }
        func call(_ method: String, params paramsJSON: String) -> String {
            let params = try! JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as! [String: Any]
            calls.append((method, params))
            return replies.isEmpty ? #"{"id":1,"result":{}}"# : replies.removeFirst()
        }
    }

    func testListDecodesAndToleratesUnknownFields() throws {
        let fake = Fake([
            #"{"id":1,"result":{"devices":[{"device_key_id":"sha256:aa","public_key":"k","label":"laptop","origin":"near_ai","created_at":"2026-10-27T00:00:00Z","this_device":true,"future":1},{"device_key_id":"sha256:bb","public_key":"k","origin":"near_ai","created_at":"2026-10-27T00:00:00Z","last_seen_at":"2026-10-27T01:00:00Z","this_device":false}]}}"#
        ])
        let devices = try AccountDevicesClient(transport: fake).list()
        XCTAssertEqual(fake.calls[0].0, "account_devices_list")
        XCTAssertEqual(devices.count, 2)
        XCTAssertEqual(devices[0].label, "laptop")
        XCTAssertTrue(devices[0].thisDevice)
        XCTAssertEqual(devices[1].lastSeenAt, "2026-10-27T01:00:00Z")
    }

    /// What each device has contributed, as the server counts it: carried
    /// when sent, and unknown (nil), never zero, from a server too old to
    /// send it.
    func testListCarriesEachDevicesContributions() throws {
        let fake = Fake([
            #"{"id":1,"result":{"devices":[{"device_key_id":"sha256:aa","public_key":"k","origin":"near_ai","created_at":"2026-10-27T00:00:00Z","this_device":false,"contributed_traces":12,"last_contributed_at":"2026-10-27T02:00:00Z"},{"device_key_id":"sha256:bb","public_key":"k","origin":"near_ai","created_at":"2026-10-27T00:00:00Z","this_device":false,"contributed_traces":0},{"device_key_id":"sha256:cc","public_key":"k","origin":"near_ai","created_at":"2026-10-27T00:00:00Z","this_device":false}]}}"#
        ])
        let devices = try AccountDevicesClient(transport: fake).list()
        XCTAssertEqual(devices.map(\.contributedTraces), [12, 0, nil])
        XCTAssertEqual(devices.map(\.lastContributedAt), ["2026-10-27T02:00:00Z", nil, nil])
    }

    func testMintStatusAndRevokeSendWhatTheDaemonReads() throws {
        let fake = Fake([
            #"{"id":1,"result":{"code_id":"c1","code":"tcd1_dA.s","expires_at":"2026-10-27T00:10:00Z"}}"#,
            #"{"id":1,"result":{"state":"redeemed","device_key_id":"sha256:cc"}}"#,
            #"{"id":1,"result":{"revoked":true}}"#,
        ])
        let client = AccountDevicesClient(transport: fake)
        let code = try client.mint(label: "build box")
        XCTAssertEqual(code.code, "tcd1_dA.s")
        let status = try client.status(codeId: code.codeId)
        XCTAssertEqual(status, DeviceCodeStatus(state: "redeemed", deviceKeyId: "sha256:cc"))
        try client.revoke(deviceKeyId: "sha256:cc")
        XCTAssertEqual(
            fake.calls.map(\.0),
            ["account_device_code_mint", "account_device_code_status", "account_device_revoke"])
        XCTAssertEqual(fake.calls[0].1["label"] as? String, "build box")
        XCTAssertEqual(fake.calls[1].1["code_id"] as? String, "c1")
        XCTAssertEqual(fake.calls[2].1["device_key_id"] as? String, "sha256:cc")
    }

    func testAMintWithoutALabelSendsNone() throws {
        let fake = Fake([#"{"id":1,"result":{"code_id":"c1","code":"tcd1_dA.s","expires_at":"x"}}"#])
        _ = try AccountDevicesClient(transport: fake).mint(label: nil)
        XCTAssertNil(fake.calls[0].1["label"])
    }

    func testARefusalCarriesItsLabel() {
        let fake = Fake([#"{"id":1,"error":{"code":"unavailable","message":"device_code_too_many"}}"#])
        XCTAssertThrowsError(try AccountDevicesClient(transport: fake).mint(label: nil)) { error in
            XCTAssertEqual((error as? AccountDevicesClient.Failure)?.refusal, .tooMany)
        }
    }
}
