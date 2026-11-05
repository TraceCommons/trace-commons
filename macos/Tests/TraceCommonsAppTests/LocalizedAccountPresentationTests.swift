import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

private struct AccountPresentationDaemon: DaemonCalling {
    let method: String
    let response: String

    func call(_ method: String, params paramsJSON: String) -> String {
        method == self.method ? response : #"{"error":{"code":"unavailable","message":"unexpected_test_method"}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// Exercise the shipping daemon adaptors, not a second presentation path.
final class LocalizedAccountPresentationTests: XCTestCase {
    private func client(_ method: String, result: [String: Any]) throws -> DaemonClient {
        let response = try JSONSerialization.data(withJSONObject: ["id": 1, "result": result])
        return DaemonClient(daemon: AccountPresentationDaemon(
            method: method, response: String(decoding: response, as: UTF8.self)))
    }

    func testBalanceChangeComesFromRawSignedNumbersAndWireScaleNotEnglish() throws {
        for nanos in [Int64(-3_209_000_000), -1, 0, 1, 3_209_000_000] {
            for scale in [UInt8(6), 9, 12] {
                let status = try client("near_ai_balance", result: [
                    "state": "known", "scale": scale, "remaining_nanos": NSNull(),
                    "change_24h_nanos": nanos, "change_24h_text": "not the display source {count}",
                    "change_24h_direction": 0, "received_total_nanos": 12
                ]).nearAiBalance()
                let expected = try XCTUnwrap(TCNearAiBalance.changeLine(nanos: nanos, scale: scale))
                XCTAssertEqual(status.change24hText, expected)
                XCTAssertNotEqual(status.change24hText, "not the display source {count}")
                XCTAssertEqual(status.change24hNanos, nanos)
                XCTAssertEqual(status.scale, scale)
                XCTAssertNil(status.remainingNanos)
                XCTAssertEqual(status.change24hDirection, 0)
                XCTAssertEqual(status.receivedTotalNanos, 12)
            }
        }
    }

    func testUnavailableAndOlderBalancesNeverInventZeroOrHistory() throws {
        let unavailable = try client("near_ai_balance", result: [
            "state": "known", "scale": 9, "change_24h_nanos": NSNull(),
            "change_24h_text": "untrusted stale amount", "change_24h_unavailable_text": "old daemon wording"
        ]).nearAiBalance()
        XCTAssertNil(unavailable.change24hText)
        XCTAssertEqual(unavailable.change24hUnavailableText,
                       try XCTUnwrap(TCNearAiBalance.changeLine(nanos: nil, scale: 9)))
        let older = try client("near_ai_balance", result: ["state": "known", "scale": 9]).nearAiBalance()
        XCTAssertNil(older.change24hText)
        XCTAssertNil(older.change24hUnavailableText)
        let failed = try client("near_ai_balance", result: [
            "state": "unavailable", "scale": 9, "change_24h_nanos": 0,
            "change_24h_text": "stale", "change_24h_unavailable_text": "stale"
        ]).nearAiBalance()
        XCTAssertNil(failed.change24hText)
        XCTAssertNil(failed.change24hUnavailableText)
        let badScale = try client("near_ai_balance", result: [
            "state": "known", "change_24h_nanos": 12, "change_24h_text": "wrong scale"
        ]).nearAiBalance()
        XCTAssertNil(badScale.change24hText)
    }

    private func spendingResult(name: String) throws -> [String: Any] {
        let viewJSON = try XCTUnwrap(TCNearAiBalance.spendOrganizationViewJSON(currentName: name))
        var view = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(viewJSON.utf8)) as? [String: Any])
        // All fields are present so the original fail-closed shape check is
        // still meaningful. None of these completed strings selects copy.
        for key in Array(view.keys) { view[key] = "old wording \(key)" }
        return [
            "state": "listed", "current_organization_id": "selected", "explicit": false,
            "prompt": true, "revoke_pending": true,
            "organizations": [
                ["id": "other", "name": "not selected", "current": true],
                ["id": "selected", "name": name, "current": false]
            ], "view": view
        ]
    }

    func testSpendOrganizationUsesCurrentIDsListedNameAndPreservesFacts() throws {
        let name = "A {organization} {count} %@ <team>"
        let status = try client("near_ai_spend_organizations", result: spendingResult(name: name)).nearAiSpendOrganizations()
        let words = try XCTUnwrap(status.words)
        let expectedJSON = try XCTUnwrap(TCNearAiBalance.spendOrganizationViewJSON(currentName: name))
        let expected = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(expectedJSON.utf8)) as? [String: String])
        XCTAssertEqual(words.promptLine, expected["prompt_line"])
        XCTAssertEqual(words.promptDismissAccessibility, expected["prompt_dismiss_accessibility"])
        XCTAssertTrue(words.promptLine.contains(name))
        XCTAssertEqual(status.organizations.map(\.name), ["not selected", name])
        XCTAssertEqual(status.currentOrganizationID, "selected")
        XCTAssertTrue(status.showsPrompt)
        XCTAssertTrue(status.revokePending)
        XCTAssertFalse(status.explicit)
    }

    func testMissingOrIncompleteSpendingViewDoesNotCreateAPrompt() throws {
        var result = try spendingResult(name: "Selected")
        result.removeValue(forKey: "view")
        XCTAssertNil(try client("near_ai_spend_organizations", result: result).nearAiSpendOrganizations().words)
        result["view"] = ["prompt_line": "incomplete"]
        XCTAssertNil(try client("near_ai_spend_organizations", result: result).nearAiSpendOrganizations().words)
        result = try spendingResult(name: "Selected")
        result["current_organization_id"] = "not-listed"
        let status = try client("near_ai_spend_organizations", result: result).nearAiSpendOrganizations()
        let unnamedJSON = try XCTUnwrap(TCNearAiBalance.spendOrganizationViewJSON(currentName: nil))
        let unnamed = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(unnamedJSON.utf8)) as? [String: String])
        XCTAssertEqual(status.words?.promptLine, unnamed["prompt_line"])
        XCTAssertFalse(status.words?.promptLine.contains("Selected") ?? true)
    }

    func testFundingProseIsLocalizedOnlyAfterDestinationValidation() throws {
        let name = "A {organization} %@ team"
        let url = "\(LinkedChannel.dashboardOrigin)/dashboard/organizations/selected/credits"
        var result: [String: Any] = [
            "state": "ready", "organization_id": "selected", "organization_name": name,
            "connection_revision": String(repeating: "a", count: 64), "browser_url": url,
            "view": ["message": "obsolete prose"]
        ]
        let status = try client("near_ai_funding", result: result).nearAiFunding(expected: nil)
        XCTAssertEqual(status.view.message, try XCTUnwrap(TCNearAiBalance.fundingMessage(state: "ready", organizationName: name)))
        XCTAssertTrue(status.view.message.contains(name))
        XCTAssertEqual(status.destination?.browserURL?.absoluteString, url)
        XCTAssertEqual(status.destination?.organizationID, "selected")
        result["browser_url"] = "https://untrusted.example/credits"
        XCTAssertThrowsError(try client("near_ai_funding", result: result).nearAiFunding(expected: nil))
        result["state"] = "changed"
        let changed = try client("near_ai_funding", result: result).nearAiFunding(expected: nil)
        XCTAssertNil(changed.destination)
        XCTAssertNil(changed.organizationName)
        XCTAssertEqual(changed.view.message, try XCTUnwrap(TCNearAiBalance.fundingMessage(state: "changed", organizationName: nil)))
    }

    func testSubscriptionPercentageUsesRawPermilleAndNativePunctuation() {
        for locale in [Locale(identifier: "en_US"), Locale(identifier: "de_DE"), Locale(identifier: "ar_EG")] {
            for used in [0, 1, 10, 420, 999, 1000] {
                let expected = (Double(1000 - used) / 1000).formatted(
                    .percent.precision(.fractionLength(0...1)).locale(locale))
                XCTAssertEqual(InsightsSubscriptionWords.remainingPercent(usedPermille: used, locale: locale), expected)
            }
        }
        XCTAssertEqual(InsightsSubscriptionWords.remainingPercent(usedPermille: 1, locale: Locale(identifier: "en_US")), "99.9%")
        XCTAssertEqual(InsightsSubscriptionWords.remainingPercent(usedPermille: 420, locale: Locale(identifier: "en_US")), "58%")
        for used in [nil, -1, 1001, Int.max] {
            XCTAssertNil(InsightsSubscriptionWords.remainingPercent(usedPermille: used))
        }
        let login = DaemonData.SubscriptionUsageLogin(id: "fixture", label: "fixture", state: "observed", windows: [
            .init(lane: "session", usedPermille: 1, remainingText: "do not use this string"),
            .init(lane: "weekly", usedPermille: nil, remainingText: "100%")
        ])
        let words = InsightsSubscriptionWords.login(login, provider: "codex", now: Date(), copy: [
            "analytics_subscriptions_remaining": "Remaining {n}", "analytics_unavailable": "unknown"
        ])
        XCTAssertEqual(words.windows[0].remaining, "Remaining \(InsightsSubscriptionWords.remainingPercent(usedPermille: 1)!)")
        XCTAssertEqual(words.windows[1].remaining, "unknown")
    }
}
