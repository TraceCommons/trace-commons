import XCTest

@testable import TCShellCore

/// The Referrals section's pure logic, without the dylib. The words are
/// placeholders here ("T:" prefixed) so a test passes only by reading the
/// payload; `ConsentCopyBridgeTests` checks the live export's keys.
final class ReferralSurfaceTests: XCTestCase {
    private static func copyJSON(_ override: [String: String] = [:]) -> String {
        var object = [String: String]()
        for field in ReferralCopy.consumedFields { object[field] = "T:\(field)" }
        object["joined_one"] = "T:one {n}"
        object["joined_many"] = "T:many {n}"
        object["limit_reached"] = "T:limit {n}"
        object["copy_accessibility_name"] = "T:copy {code}"
        object["disable_accessibility_name"] = "T:off {code}"
        object["credit_sent"] = "T:sent {amount}"
        object["enter_scope"] = "T:until {date}"
        object["insights_on_hold"] = "T:held {amount}"
        for (key, value) in override { object[key] = value }
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private let copy = ReferralCopy.decode(fromJSON: copyJSON())!
    private var surface: ReferralSurface { ReferralSurface(copy: copy) }
    private static func money(_ nanos: UInt64) -> String { "$\(nanos)" }

    func testCopyDecodesEveryFieldAndRefusesABlank() {
        XCTAssertEqual(copy.sentences.count, ReferralCopy.consumedFields.count)
        XCTAssertEqual(copy.title, "T:title")
        XCTAssertEqual(copy.signUpLabel, "T:sign_up_label")
        XCTAssertNil(ReferralCopy.decode(fromJSON: Self.copyJSON(["title": " "])))
        XCTAssertNil(ReferralCopy.decode(fromJSON: "{}"))
        XCTAssertNil(ReferralCopy.decode(fromJSON: "not json"))
        XCTAssertEqual(ReferralCopy.render("a {code} b {n}", ["code": "TC-1", "n": "2"]), "a TC-1 b 2")
    }

    func testListingDecodesAndReadsAMissingCreateAsNo() throws {
        let json = #"""
        {"codes":[{"code_id":"c1","code":"TC-7K3M9-QX2RD","state":"live","referee_count":2,"bonus_owed_nano_usd":5,"future":1},
                  {"code_id":"c2","code":null,"state":"disabled","referee_count":1}],
         "max_live_codes":5,"live_codes":1,"people_joined":3,"bonus_owed_nano_usd":40,"bonus_sent_nano_usd":10}
        """#
        let listing = try JSONDecoder().decode(ReferralCodesListing.self, from: Data(json.utf8))
        XCTAssertEqual(listing.codes.map(\.codeId), ["c1", "c2"])
        XCTAssertEqual(listing.codes[0].code, "TC-7K3M9-QX2RD")
        XCTAssertNil(listing.codes[1].code)
        XCTAssertNil(listing.codes[1].bonusOwedNanoUSD)
        XCTAssertFalse(listing.canCreate)
        XCTAssertEqual(listing.peopleJoined, 3)
        XCTAssertEqual(listing.bonusOwedNanoUSD, 40)
        XCTAssertEqual(listing.bonusSentNanoUSD, 10)
    }

    func testANegativeAmountIsUnknownNeverZero() throws {
        let json = #"{"codes":[],"max_live_codes":5,"can_create":true,"bonus_owed_nano_usd":-1}"#
        let listing = try JSONDecoder().decode(ReferralCodesListing.self, from: Data(json.utf8))
        XCTAssertNil(listing.bonusOwedNanoUSD)
        XCTAssertNil(surface.creditLine(listing, money: Self.money))
        XCTAssertEqual(listing.peopleJoined, 0)
    }

    func testStatusNeverOffersTheFieldOnAnAppliedAccount() throws {
        let applied = try JSONDecoder().decode(
            ReferralStatus.self, from: Data(#"{"applied":true,"apply_until":null,"can_apply":true}"#.utf8))
        XCTAssertFalse(applied.canApply)
        XCTAssertFalse(surface.showsEnterField(applied))
        let open = try JSONDecoder().decode(
            ReferralStatus.self,
            from: Data(#"{"applied":false,"apply_until":"2026-11-28T00:00:00Z","can_apply":true}"#.utf8))
        XCTAssertTrue(surface.showsEnterField(open))
        XCTAssertEqual(open.applyUntil, "2026-11-28T00:00:00Z")
        let closed = try JSONDecoder().decode(ReferralStatus.self, from: Data(#"{"applied":false}"#.utf8))
        XCTAssertFalse(surface.showsEnterField(closed))
        XCTAssertFalse(surface.showsEnterField(nil))
    }

    func testRefusalLabelsRoute() {
        XCTAssertEqual(ReferralRefusal.from(code: "account-session-required"), .sessionEnded)
        XCTAssertEqual(ReferralRefusal.from(code: "referral_not_applied"), .notApplied)
        XCTAssertEqual(ReferralRefusal.from(code: "referral-code-invalid"), .invalidCode)
        XCTAssertEqual(ReferralRefusal.from(code: "referral_unavailable"), .unavailable)
        XCTAssertEqual(ReferralRefusal.from(code: "referral_code_limit_reached"), .limitReached)
        XCTAssertEqual(ReferralRefusal.from(code: "referral_account_ineligible"), .failed)
        XCTAssertEqual(ReferralRefusal.from(code: "something-new"), .failed)
        XCTAssertEqual(surface.refusalLine(.notApplied), "T:not_applied")
        XCTAssertEqual(surface.refusalLine(.sessionEnded), "T:session_ended")
        XCTAssertEqual(surface.refusalLine(.failed), "T:failed")
    }

    func testCodeRowsOfferCopyAndTurnOffOnlyWhenTheyApply() {
        let live = ReferralCodeRow(codeId: "c1", code: "TC-AAAAA-BBBBB", state: "live", refereeCount: 1)
        let off = ReferralCodeRow(codeId: "c2", code: "TC-CCCCC-DDDDD", state: "disabled", refereeCount: 4)
        let hidden = ReferralCodeRow(codeId: "c3", code: nil, state: "voided")
        let unknown = ReferralCodeRow(codeId: "c4", code: "TC-EEEEE-FFFFF", state: "paused-later")

        XCTAssertNil(surface.stateLabel(live))
        XCTAssertEqual(surface.stateLabel(off), "T:state_disabled")
        XCTAssertEqual(surface.stateLabel(hidden), "T:state_voided")
        XCTAssertEqual(surface.stateLabel(unknown), "T:state_disabled")
        XCTAssertFalse(surface.canDisable(unknown))

        XCTAssertTrue(surface.canCopy(live))
        XCTAssertTrue(surface.canCopy(off))
        XCTAssertFalse(surface.canCopy(hidden))
        XCTAssertTrue(surface.canDisable(live))
        XCTAssertFalse(surface.canDisable(off))
        XCTAssertEqual(surface.shownCode(hidden), "T:code_hidden")
        XCTAssertEqual(surface.copyAccessibilityName(live), "T:copy TC-AAAAA-BBBBB")
        XCTAssertEqual(surface.disableAccessibilityName(live), "T:off TC-AAAAA-BBBBB")
        XCTAssertEqual(surface.joinedLine(1), "T:one 1")
        XCTAssertEqual(surface.joinedLine(0), "T:many 0")
        XCTAssertEqual(surface.joinedLine(4), "T:many 4")
    }

    func testListingLines() {
        let live = ReferralCodeRow(codeId: "c1", code: "TC-AAAAA-BBBBB")
        let off = ReferralCodeRow(codeId: "c2", code: "TC-CCCCC-DDDDD", state: "disabled")
        let open = ReferralCodesListing(codes: [live], canCreate: true, bonusOwedNanoUSD: 7, bonusSentNanoUSD: 0)
        XCTAssertNil(surface.limitLine(open))
        XCTAssertTrue(surface.showsDisableScope(open))
        XCTAssertEqual(surface.creditLine(open, money: Self.money), "$7")

        let full = ReferralCodesListing(codes: [off], maxLiveCodes: 5, canCreate: false,
                                        bonusOwedNanoUSD: 9, bonusSentNanoUSD: 4)
        XCTAssertEqual(surface.limitLine(full), "T:limit 5")
        XCTAssertFalse(surface.showsDisableScope(full))
        XCTAssertEqual(surface.creditLine(full, money: Self.money), "$9 · T:sent $4")
        XCTAssertEqual(surface.enterScope(date: "28 November 2026"), "T:until 28 November 2026")
    }

    func testTheFieldIsCheckedBeforeAnythingIsSent() {
        let display: (String) -> String? = { $0.uppercased().hasPrefix("TC-") ? $0.uppercased() : nil }
        XCTAssertNil(surface.fieldProblem("", display: display))
        XCTAssertNil(surface.fieldProblem("   ", display: display))
        XCTAssertNil(surface.fieldProblem(" tc-7k3m9-qx2rd ", display: display))
        XCTAssertEqual(surface.fieldProblem("hello", display: display), "T:invalid_code")
    }

    func testCopyComesFromCoreKeysOnly() {
        // Every consumed key is read: a key renamed on one side fails here.
        for field in ReferralCopy.consumedFields {
            var object = [String: String]()
            for other in ReferralCopy.consumedFields where other != field { object[other] = "x" }
            let data = try! JSONSerialization.data(withJSONObject: object)
            XCTAssertNil(ReferralCopy.decode(fromJSON: String(decoding: data, as: UTF8.self)), field)
        }
    }
}

/// The payout rail's streams, and the Insights NEAR AI card's split.
final class NearAiEarningsSplitTests: XCTestCase {
    private let copy = ReferralCopy.decode(fromJSON: {
        var object = [String: String]()
        for field in ReferralCopy.consumedFields { object[field] = "T:\(field)" }
        object["insights_on_hold"] = "T:held {amount}"
        let data = try! JSONSerialization.data(withJSONObject: object)
        return String(decoding: data, as: UTF8.self)
    }())!

    private func rows(_ streams: [PayoutStreamStatus]?) -> [NearAiEarningsSplit.Row] {
        NearAiEarningsSplit.rows(streams, copy: copy, money: { "$\($0)" })
    }

    func testNoStreamsDrawNoRows() {
        XCTAssertEqual(rows(nil), [])
        XCTAssertEqual(rows([]), [])
    }

    func testNoReferralFigureDrawsNoSplit() {
        XCTAssertEqual(rows([PayoutStreamStatus(stream: "earned", owed: 50)]), [])
        XCTAssertEqual(rows([PayoutStreamStatus(stream: "earned", owed: 50),
                             PayoutStreamStatus(stream: "referral", owed: 0)]), [])
        XCTAssertEqual(rows([PayoutStreamStatus(stream: "earned", owed: 50),
                             PayoutStreamStatus(stream: "referral", owed: nil)]), [])
    }

    func testOwnTracesFirstThenReferrals() {
        let shown = rows([PayoutStreamStatus(stream: "referral", owed: 7),
                          PayoutStreamStatus(stream: "earned", owed: 50)])
        XCTAssertEqual(shown.map(\.id), ["earned", "referral"])
        XCTAssertEqual(shown.map(\.label), ["T:insights_own", "T:insights_referrals"])
        XCTAssertEqual(shown.map(\.value), ["$50", "$7"])
    }

    func testAHeldStreamSaysSo() {
        let shown = rows([PayoutStreamStatus(stream: "earned", owed: 50),
                          PayoutStreamStatus(stream: "referral", owed: 0, holdLabel: "referral_review")])
        XCTAssertEqual(shown.map(\.value), ["$50", "T:held $0"])
    }

    func testStreamsDecodeDefensivelyOnTheRail() throws {
        func rail(_ streams: String) -> String {
            #"{"rail_id":"near_ai","mode":"live","linked":true,"state":"active","version":1,"request_state":null,"unlink_requested":false,"recipient_unavailable":false,"recipient_ref_check":"match","organization_name":"A","pending_here":null"#
                + streams + "}"
        }
        let present = try JSONDecoder().decode(PayoutRailStatus.self, from: Data(rail(
            #","streams":[{"stream":"earned","owed":10,"sent":4,"waiting":6},{"stream":"referral","owed":3,"sent":0,"waiting":3,"hold_label":"referral_review"}]"#
        ).utf8))
        XCTAssertEqual(present.streams?.map(\.stream), ["earned", "referral"])
        XCTAssertEqual(present.streams?.first?.sent, 4)
        XCTAssertEqual(present.streams?.last?.holdLabel, "referral_review")

        let absent = try JSONDecoder().decode(PayoutRailStatus.self, from: Data(rail("").utf8))
        XCTAssertNil(absent.streams)

        // A malformed value never fails the rail it rides on.
        let malformed = try JSONDecoder().decode(PayoutRailStatus.self, from: Data(rail(#","streams":"soon""#).utf8))
        XCTAssertNil(malformed.streams)
        XCTAssertEqual(malformed.version, 1)

        let negative = try JSONDecoder().decode(PayoutRailStatus.self, from: Data(rail(
            #","streams":[{"stream":"referral","owed":-5}]"#
        ).utf8))
        XCTAssertNil(negative.streams?.first?.owed)
    }

    /// The daemon passes the commons's streams through as they come, and
    /// the commons sends each amount as a decimal string so a u128 survives
    /// JSON. The fixture is the one the daemon's own test pins.
    func testTheDaemonsStreamsDrawTheSplit() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let streams = try String(contentsOf: repository.appendingPathComponent(
            "tests/fixtures/payout/streams.json"), encoding: .utf8)
        let status = try JSONDecoder().decode(PayoutRailStatus.self, from: Data(
            (#"{"rail_id":"near_ai","mode":"live","linked":true,"state":"active","version":1,"request_state":null,"unlink_requested":false,"recipient_unavailable":false,"recipient_ref_check":"match","organization_name":"A","pending_here":null,"streams":"#
                + streams + "}").utf8))
        XCTAssertEqual(status.streams?.map(\.owed), [700, 70])
        XCTAssertEqual(status.streams?.map(\.sent), [500, 0])
        XCTAssertEqual(status.streams?.map(\.waiting), [200, 70])
        XCTAssertEqual(rows(status.streams).map(\.value), ["$700", "T:held $70"])
    }

    /// An amount past what the app can hold, or one that is not a whole
    /// number, is unknown: never zero, and never drawn.
    func testAnAmountTooLargeOrMalformedIsUnknown() throws {
        let big = "340282366920938463463374607431768211455"
        let streams = try JSONDecoder().decode([PayoutStreamStatus].self, from: Data(
            (#"[{"stream":"earned","owed":"50"},{"stream":"referral","owed":""# + big + #"","sent":"1.5","waiting":"-3"}]"#).utf8))
        XCTAssertEqual(streams.first?.owed, 50)
        XCTAssertNil(streams.last?.owed)
        XCTAssertNil(streams.last?.sent)
        XCTAssertNil(streams.last?.waiting)
        XCTAssertEqual(rows(streams), [])
    }
}

/// The five IPC calls: the method names and parameters the daemon reads.
final class ReferralClientTests: XCTestCase {
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

    func testEachCallSendsWhatTheDaemonReads() throws {
        let fake = Fake([
            #"{"id":1,"result":{"codes":[],"max_live_codes":5,"can_create":true,"people_joined":0}}"#,
            #"{"id":1,"result":{"code_id":"11111111-2222-4333-8444-555555555555","code":"TC-7K3M9-QX2RD"}}"#,
            #"{"id":1,"result":{"code_id":"11111111-2222-4333-8444-555555555555","state":"disabled"}}"#,
            #"{"id":1,"result":{"applied":true}}"#,
            #"{"id":1,"result":{"applied":true,"apply_until":null,"can_apply":false}}"#,
        ])
        let client = ReferralClient(transport: fake)
        XCTAssertTrue(try client.list().canCreate)
        XCTAssertEqual(try client.create().code, "TC-7K3M9-QX2RD")
        try client.disable(codeId: "11111111-2222-4333-8444-555555555555")
        try client.apply(code: "tc-7k3m9-qx2rd")
        XCTAssertTrue(try client.status().applied)
        XCTAssertEqual(fake.calls.map(\.0), [
            "referral_codes_list", "referral_codes_create", "referral_codes_disable", "referral_apply",
            "referral_status",
        ])
        XCTAssertEqual(fake.calls[2].1["code_id"] as? String, "11111111-2222-4333-8444-555555555555")
        XCTAssertEqual(fake.calls[3].1["code"] as? String, "tc-7k3m9-qx2rd")
        for (_, params) in fake.calls {
            XCTAssertNil(params["token"])
            XCTAssertNil(params["account_session_token"])
        }
    }

    func testARefusalKeepsTheDaemonsLabel() {
        let fake = Fake([
            #"{"id":1,"error":{"code":"unavailable","message":"referral_not_applied"}}"#,
            #"{"id":1,"error":{"code":"unavailable","message":"account-session-required"}}"#,
            "garbage",
        ])
        let client = ReferralClient(transport: fake)
        XCTAssertThrowsError(try client.apply(code: "TC-7K3M9-QX2RD")) { error in
            XCTAssertEqual((error as? ReferralClient.Failure)?.refusal, .notApplied)
        }
        XCTAssertThrowsError(try client.list()) { error in
            XCTAssertEqual((error as? ReferralClient.Failure)?.refusal, .sessionEnded)
        }
        XCTAssertThrowsError(try client.status()) { error in
            XCTAssertEqual((error as? ReferralClient.Failure)?.label, "unparseable-response")
        }
    }
}
