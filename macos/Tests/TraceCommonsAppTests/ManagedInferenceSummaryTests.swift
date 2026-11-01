import Foundation
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class ManagedInferenceSummaryTests: XCTestCase {
    private final class Transport: DaemonTransport {
        var reply: String
        var callsReadable = false
        init(_ reply: String) { self.reply = reply }
        func call(_ method: String, params paramsJSON: String) -> String {
            if method == "inference_summary" { return reply }
            if method == "inference_calls" {
                return #"{"id":0,"result":{"readable":\#(callsReadable),"calls":[],"next_cursor":null}}"#
            }
            return #"{"id":0,"error":{"code":"unknown_method","message":"unknown_method"}}"#
        }
    }

    private func reply(calls: [UInt64] = [3, 4, 5], verified: [UInt64] = [1, 2, 3],
                       readable: Bool = true, enabled: Bool = true) -> String {
        let routes = zip(calls, verified).map { calls, verified in
            #"{"calls":\#(calls),"priced_calls":0,"cost_usd":0,"proof":{"verified":\#(verified),"gateway_only":10,"unattested":11,"pending":12,"unavailable":13,"failed":14,"outside":15,"unrecorded":16}}"#
        }
        return #"{"id":0,"result":{"readable":\#(readable),"window_hours":24,"summary":{"enabled":\#(enabled),"receipts":true,"since":"2026-10-01T00:00:00Z","groups":[],"routed":\#(routes[0]),"outside":\#(routes[1]),"unknown":\#(routes[2])}}}"#
    }

    func testManagedSummaryDoesNotRequireReadableSharedCalls() async {
        let store = InferenceStore(client: LiveDaemonClient(transport: Transport(reply())))
        await store.load()
        XCTAssertEqual(store.calls?.readable, false)
        XCTAssertEqual(HomePrivateAI.totals(store).calls, 12)
        XCTAssertEqual(HomePrivateAI.totals(store).verified, 6)
    }

    func testObservedZeroAndLargestExactIntRemainKnown() async {
        for count in [UInt64(0), UInt64(Int.max)] {
            let store = InferenceStore(client: LiveDaemonClient(transport: Transport(
                reply(calls: [count, 0, 0], verified: [0, 0, 0]))))
            await store.load()
            XCTAssertEqual(HomePrivateAI.totals(store).calls, Int(exactly: count))
            XCTAssertEqual(HomePrivateAI.totals(store).verified, 0)
            store.attach(nil)
            XCTAssertNil(store.networkSummary)
            XCTAssertNil(HomePrivateAI.totals(store).calls)
        }
    }

    func testDisabledUnreadableAndMissingAreUnknown() async {
        for response in [reply(readable: false), reply(enabled: false),
                         #"{"id":0,"result":{"readable":true,"summary":null}}"#] {
            let store = InferenceStore(client: LiveDaemonClient(transport: Transport(response)))
            await store.load()
            XCTAssertNil(HomePrivateAI.totals(store).calls)
            XCTAssertNil(HomePrivateAI.totals(store).verified)
        }
    }

    func testOverflowRemainsUnknownWithoutLosingIndependentCount() async {
        for counts in [[UInt64.max, 1, 0], [UInt64(Int.max), 1, 0]] {
            let store = InferenceStore(client: LiveDaemonClient(transport: Transport(reply(calls: counts))))
            await store.load()
            XCTAssertNil(HomePrivateAI.totals(store).calls)
            XCTAssertEqual(HomePrivateAI.totals(store).verified, 6)
        }
        let store = InferenceStore(client: LiveDaemonClient(transport: Transport(reply(verified: [.max, 1, 0]))))
        await store.load()
        XCTAssertEqual(HomePrivateAI.totals(store).calls, 12)
        XCTAssertNil(HomePrivateAI.totals(store).verified)
    }

    func testOlderDaemonNeverSubstitutesSharedPageZeroForManagedTotals() async {
        let transport = Transport(#"{"id":0,"error":{"code":"unknown_method","message":"unknown_method"}}"#)
        transport.callsReadable = true
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.calls?.readable, true)
        XCTAssertNil(store.failures["inference_summary"])
        XCTAssertNil(HomePrivateAI.totals(store).calls)
        XCTAssertNil(HomePrivateAI.totals(store).verified)
    }

    private func pricingSummary(
        calls: [UInt64] = [3, 4, 5], pricedCalls: [UInt64?] = [3, 4, 5],
        costs: [Double?] = [0.125, 0.25, 0.625], readable: Bool = true, enabled: Bool = true
    ) throws -> DaemonData.NetworkInferenceSummary {
        let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(
            reply(calls: calls, readable: readable, enabled: enabled).utf8)) as? [String: Any])
        var result = try XCTUnwrap(frame["result"] as? [String: Any])
        var summary = try XCTUnwrap(result["summary"] as? [String: Any])
        for (index, route) in ["routed", "outside", "unknown"].enumerated() {
            var total = try XCTUnwrap(summary[route] as? [String: Any])
            total["priced_calls"] = pricedCalls[index]
            total["cost_usd"] = costs[index]
            summary[route] = total
        }
        result["summary"] = summary
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(DaemonData.NetworkInferenceSummary.self,
                                  from: JSONSerialization.data(withJSONObject: result))
    }

    func testNetworkHeaderPricingRequiresCoverageOfItsManagedCalls() async throws {
        let transport = Transport(reply())
        transport.callsReadable = true
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        await store.load()
        XCTAssertEqual(store.calls?.calls.count, 0)
        let totals = InferenceTabView.networkHeaderTotals(store.networkSummary)
        XCTAssertEqual(totals.calls, 12)
        XCTAssertEqual(totals.verified, 6)
        XCTAssertEqual(totals.priced, "—", "the empty shared page cannot price managed calls")
    }

    func testNetworkHeaderSumsCompletePricingAcrossEveryRoute() throws {
        let totals = InferenceTabView.networkHeaderTotals(try pricingSummary())
        XCTAssertEqual(totals.calls, 12)
        XCTAssertEqual(totals.verified, 6)
        XCTAssertEqual(totals.priced, InferenceTabView.money(1_000_000))
        let zero = try pricingSummary(costs: [0, 0, 0])
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(zero).priced, InferenceTabView.money(0))
        let empty = try pricingSummary(calls: [0, 0, 0], pricedCalls: [0, 0, 0], costs: [0, 0, 0])
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(empty).priced, InferenceTabView.money(0))
    }

    func testNetworkHeaderMissingAndPartialPricingStayUnknown() throws {
        for summary in [
            try pricingSummary(pricedCalls: [3, 3, 5]),
            try pricingSummary(pricedCalls: [3, 4, 6]),
            try pricingSummary(pricedCalls: [nil, 4, 5]),
            try pricingSummary(costs: [0.125, nil, 0.625]),
            try pricingSummary(costs: [-0.125, 0.25, 0.625]),
            try pricingSummary(costs: [Double.greatestFiniteMagnitude, Double.greatestFiniteMagnitude, 0]),
            try pricingSummary(calls: [.max, 1, 0], pricedCalls: [.max, 1, 0], costs: [0, 0, 0]),
            try pricingSummary(calls: [UInt64(Int.max), 1, 0],
                               pricedCalls: [UInt64(Int.max), 1, 0], costs: [0, 0, 0])
        ] {
            XCTAssertEqual(InferenceTabView.networkHeaderTotals(summary).priced, "—")
        }
        let missing = try pricingSummary(pricedCalls: [nil, 4, 5], costs: [nil, 0.25, 0.625])
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(missing).calls, 12,
                       "missing prices must not discard call counts")
    }

    func testNetworkHeaderRejectsNonfiniteDecodedCosts() throws {
        for value in ["NaN", "Infinity", "-Infinity"] {
            let frame = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply().utf8)) as? [String: Any])
            var result = try XCTUnwrap(frame["result"] as? [String: Any])
            var summary = try XCTUnwrap(result["summary"] as? [String: Any])
            var routed = try XCTUnwrap(summary["routed"] as? [String: Any])
            routed["priced_calls"] = 3
            routed["cost_usd"] = value
            summary["routed"] = routed
            result["summary"] = summary
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            decoder.nonConformingFloatDecodingStrategy = .convertFromString(
                positiveInfinity: "Infinity", negativeInfinity: "-Infinity", nan: "NaN")
            let decoded = try decoder.decode(DaemonData.NetworkInferenceSummary.self,
                                             from: JSONSerialization.data(withJSONObject: result))
            XCTAssertEqual(InferenceTabView.networkHeaderTotals(decoded).priced, "—")
        }
    }

    func testNetworkHeaderRejectsUnavailablePricing() throws {
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(nil).priced, "—")
        let noSummary = try JSONDecoder().decode(DaemonData.NetworkInferenceSummary.self,
            from: Data(#"{"readable":true,"summary":null}"#.utf8))
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(noSummary).priced, "—")
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(try pricingSummary(readable: false)).priced, "—")
        XCTAssertEqual(InferenceTabView.networkHeaderTotals(try pricingSummary(enabled: false)).priced, "—")
    }

    func testRefreshClearsStaleSummaryIncludingUnsupportedDaemon() async {
        let transport = Transport(reply())
        let store = InferenceStore(client: LiveDaemonClient(transport: transport))
        for failed in [#"{"id":0,"error":{"code":"unknown_method","message":"unknown_method"}}"#,
                       #"{"id":0,"error":{"code":"internal","message":"unavailable"}}"#,
                       reply(readable: false)] {
            transport.reply = reply()
            await store.load()
            XCTAssertEqual(HomePrivateAI.totals(store).calls, 12)
            transport.reply = failed
            await store.load()
            XCTAssertNil(store.networkSummary)
            if failed.contains("unknown_method") { XCTAssertNil(store.failures["inference_summary"]) }
            XCTAssertNil(HomePrivateAI.totals(store).calls)
            XCTAssertNil(HomePrivateAI.totals(store).verified)
        }
    }
}
