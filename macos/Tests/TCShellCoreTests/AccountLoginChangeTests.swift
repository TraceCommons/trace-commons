import XCTest

@testable import TCShellCore

/// The change of login's pure logic (F10), without the dylib. The words are
/// placeholders ("T:" prefixed) so a test passes only by reading the
/// payload; `TCBridgeTests` checks the live export's keys.
final class AccountLoginChangeSurfaceTests: XCTestCase {
    private static func copyJSON(_ override: [String: String] = [:]) -> String {
        var object = [String: String]()
        for field in AccountChangeCopy.consumedFields {
            object[field] = "T:\(field)"
        }
        object["body"] = "T:body {old_provider}>{new_provider}"
        object["body_old_unknown"] = "T:body_old_unknown >{new_provider}"
        object["body_wallet"] = "T:body_wallet >{new_provider}"
        object["private_ai"] = "T:private_ai {new_provider}"
        object["error_current_login"] = "T:error_current_login {old_provider}"
        object["step_current_accessibility_name"] = "T:a11y_current {old_provider}"
        object["confirm_accessibility_name"] = "T:a11y_confirm {new_provider}"
        object["provider_near"] = "NEAR wallet"
        object["provider_github"] = "GitHub"
        object["provider_google"] = "Google"
        for (key, value) in override { object[key] = value }
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private let copy = AccountChangeCopy.decode(fromJSON: copyJSON())!

    private func state(
        _ step: AccountLoginChangeState.Step, kind: String = "near_ai_login",
        old: String? = "github", new: String? = "google", busy: Bool = false, error: String? = nil
    ) -> AccountLoginChangeState {
        AccountLoginChangeState(
            changeID: "c1", step: step, currentKind: kind, oldProvider: old, newProvider: new,
            busy: busy, error: error)
    }

    func testAPayloadWithAMissingOrBlankFieldIsRefusedWhole() {
        XCTAssertNil(AccountChangeCopy.decode(fromJSON: Self.copyJSON(["title": " "])))
        var object =
            try! JSONSerialization.jsonObject(with: Data(Self.copyJSON().utf8)) as! [String: String]
        object.removeValue(forKey: "mismatch_change")
        let data = try! JSONSerialization.data(withJSONObject: object)
        XCTAssertNil(AccountChangeCopy.decode(fromJSON: String(decoding: data, as: UTF8.self)))
    }

    func testTemplatesNameLoginsByTitleAndNeverByTag() {
        XCTAssertEqual(
            copy.render(copy.body, old: "near", new: "github"), "T:body NEAR wallet>GitHub")
        XCTAssertNil(copy.render(copy.body, old: nil, new: "github"))
        XCTAssertNil(copy.render(copy.body, old: "myspace", new: "github"))
        XCTAssertEqual(copy.render(copy.title), "T:title")
    }

    func testStepOneOfANearAIAccountNamesTheLoginUsedLastTime() {
        let page = AccountLoginChangeSurface.page(state(.current), copy: copy)!
        XCTAssertEqual(page.action, .proveCurrent)
        XCTAssertEqual(page.heading, "T:step_current")
        XCTAssertEqual(page.primary, "T:step_current")
        XCTAssertEqual(page.primaryAccessibilityName, "T:a11y_current GitHub")
        XCTAssertFalse(page.asksWalletAccount)
        XCTAssertEqual(page.lines, [])
        XCTAssertEqual(page.cancel, "T:cancel")
    }

    func testStepOneOfAWalletAccountAsksForTheNameAndTheWallet() {
        let page = AccountLoginChangeSurface.page(state(.current, kind: "wallet", old: nil), copy: copy)!
        XCTAssertTrue(page.asksWalletAccount)
        XCTAssertEqual(page.lines, ["T:step_current_wallet"])
        XCTAssertEqual(page.primaryAccessibilityName, "T:step_current")
    }

    func testStepTwoSignsInWithTheNewAccount() {
        let page = AccountLoginChangeSurface.page(state(.new, new: nil), copy: copy)!
        XCTAssertEqual(page.action, .signInNew)
        XCTAssertEqual(page.primary, "T:step_new")
        XCTAssertEqual(page.primaryAccessibilityName, "T:step_new_accessibility_name")
    }

    func testStepThreeSaysWhatStaysAndWhatTheNewLoginIs() {
        let page = AccountLoginChangeSurface.page(state(.confirm), copy: copy)!
        XCTAssertEqual(page.action, .confirm)
        XCTAssertEqual(page.lines, ["T:body GitHub>Google", "T:private_ai Google"])
        XCTAssertEqual(page.primary, "T:confirm")
        XCTAssertEqual(page.primaryAccessibilityName, "T:a11y_confirm Google")
    }

    func testStepThreeWithAnUnknownOldLoginUsesTheBodyWithoutIt() {
        let page = AccountLoginChangeSurface.page(state(.confirm, old: nil), copy: copy)!
        XCTAssertEqual(page.lines.first, "T:body_old_unknown >Google")
    }

    func testStepThreeOfAWalletAccountUsesTheWalletBody() {
        let page = AccountLoginChangeSurface.page(state(.confirm, kind: "wallet", old: nil), copy: copy)!
        XCTAssertEqual(page.lines.first, "T:body_wallet >Google")
    }

    func testStepThreeWithoutANewLoginItCanNameIsNotDrawn() {
        XCTAssertNil(AccountLoginChangeSurface.page(state(.confirm, new: nil), copy: copy))
        XCTAssertNil(AccountLoginChangeSurface.page(state(.confirm, new: "myspace"), copy: copy))
    }

    func testEachErrorIsItsApprovedSentence() {
        func line(_ error: String, old: String? = "github") -> String? {
            AccountLoginChangeSurface.page(state(.current, old: old, error: error), copy: copy)?.error
        }
        XCTAssertEqual(line("current_login"), "T:error_current_login GitHub")
        XCTAssertEqual(line("current_login", old: nil), "T:error_unavailable")
        XCTAssertEqual(line("wallet"), "T:error_wallet")
        XCTAssertEqual(line("in_use"), "T:error_in_use")
        XCTAssertEqual(line("too_soon"), "T:error_too_soon")
        XCTAssertEqual(line("unavailable"), "T:error_unavailable")
        XCTAssertEqual(line("something_new"), "T:error_unavailable")
        XCTAssertNil(AccountLoginChangeSurface.page(state(.current), copy: copy)?.error)
    }

    func testDoneClosesWithNoConfirmationOfItsOwn() {
        let page = AccountLoginChangeSurface.page(state(.done), copy: copy)!
        XCTAssertEqual(page.action, .close)
        XCTAssertEqual(page.lines, [])
        XCTAssertNil(page.error)
    }

    func testThePayoutsLineShowsOnlyWhereTheChangeIsOffered() {
        XCTAssertTrue(
            AccountLoginChangeStatus(available: true, payoutsNeedLogin: true, held: false)
                .showsPayoutsLine)
        XCTAssertFalse(
            AccountLoginChangeStatus(available: false, payoutsNeedLogin: true, held: false)
                .showsPayoutsLine)
        XCTAssertFalse(AccountLoginChangeStatus.none.showsPayoutsLine)
    }

    func testAStateDecodesFromTheDaemonsAnswer() throws {
        let json = #"{"change_id":"c9","step":"confirm","current_kind":"wallet","old_provider":null,"new_provider":"github","busy":false,"error":null,"browser_url":"http://127.0.0.1:1/x","future":1}"#
        let decoded = try JSONDecoder().decode(AccountLoginChangeState.self, from: Data(json.utf8))
        XCTAssertEqual(decoded.changeID, "c9")
        XCTAssertEqual(decoded.step, .confirm)
        XCTAssertTrue(decoded.isWallet)
        XCTAssertEqual(decoded.newProvider, "github")
        XCTAssertEqual(decoded.browserURL, "http://127.0.0.1:1/x")
    }
}

/// The two IPC methods: the names and parameters the daemon reads.
final class AccountLoginChangeClientTests: XCTestCase {
    private final class Fake: DaemonTransport {
        var calls: [(String, [String: Any])] = []
        var replies: [String]
        init(_ replies: [String]) { self.replies = replies }
        func call(_ method: String, params paramsJSON: String) -> String {
            let params =
                try! JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as! [String: Any]
            calls.append((method, params))
            return replies.isEmpty
                ? #"{"id":1,"result":{"change_id":"c1","step":"current","current_kind":"near_ai_login","busy":false}}"#
                : replies.removeFirst()
        }
    }

    func testEveryStepSendsWhatTheDaemonReads() throws {
        let fake = Fake([])
        let client = AccountLoginChangeClient(transport: fake)
        _ = try client.start()
        _ = try client.proveCurrent(changeID: "c1", walletAccountID: "alice.near")
        _ = try client.proveCurrent(changeID: "c1", walletAccountID: nil)
        _ = try client.signInNew(changeID: "c1")
        _ = try client.confirm(changeID: "c1")
        _ = try client.status(changeID: "c1")
        try client.cancel(changeID: "c1")
        XCTAssertEqual(
            fake.calls.map(\.0),
            Array(repeating: "account_login_change", count: 6) + ["account_login_change_cancel"])
        XCTAssertEqual(
            fake.calls.map { $0.1["step"] as? String },
            ["start", "prove_current", "prove_current", "sign_in_new", "confirm", "status", nil])
        XCTAssertEqual(fake.calls[1].1["wallet_account_id"] as? String, "alice.near")
        XCTAssertNil(fake.calls[2].1["wallet_account_id"])
        XCTAssertEqual(fake.calls[4].1["confirmed"] as? Bool, true)
        XCTAssertNil(fake.calls[3].1["confirmed"], "only Switch confirms")
        XCTAssertEqual(fake.calls[6].1["change_id"] as? String, "c1")
    }

    func testARefusalCarriesItsLabel() {
        let fake = Fake([
            #"{"id":1,"error":{"code":-32001,"message":"account_login_change_not_available"}}"#
        ])
        XCTAssertThrowsError(try AccountLoginChangeClient(transport: fake).start()) { error in
            XCTAssertEqual(
                (error as? AccountLoginChangeClient.Failure)?.label,
                "account_login_change_not_available")
        }
    }
}
