import XCTest
@testable import TCShellCore

/// SAMPLE organization billing response; never claims this Mac's inference spend.
final class ModelSpendContractTests: XCTestCase {
    private let known = #"{"known":true,"scope":"near_ai_organization","source":"near_ai_usage_by_model","currency":"USD","scale":9,"window_hours":24,"since":"2026-10-01T12:00:00Z","observed_at":"2026-10-02T12:00:00Z","models":[{"model":"SAMPLE-model","billed_nanos":1234500,"billed_micros":1235,"calls":4,"rounding":"nearest_micro_half_up"}]}"#

    func testKnownProviderSpendRetainsOrganizationScopeAndExactNanos() async throws {
        let transport = SpendTransport(known)
        let result = try await LiveDaemonClient(transport: transport).modelSpend()
        XCTAssertTrue(result.known)
        let object = try wire(result)
        XCTAssertEqual(object["scope"] as? String, "near_ai_organization")
        XCTAssertEqual(object["source"] as? String, "near_ai_usage_by_model")
        XCTAssertEqual(object["currency"] as? String, "USD")
        XCTAssertEqual(object["scale"] as? Int, 9)
        XCTAssertEqual(object["window_hours"] as? Int, 24)
        let row = try XCTUnwrap((object["models"] as? [[String: Any]])?.first)
        XCTAssertEqual(row["billed_nanos"] as? Int64, 1234500)
        XCTAssertEqual(row["billed_micros"] as? Int64, 1235)
        XCTAssertEqual(row["calls"] as? Int64, 4)
        XCTAssertEqual(row["rounding"] as? String, "nearest_micro_half_up")
        XCTAssertEqual(transport.method, "model_spend")
        XCTAssertEqual(transport.params, "{}")
    }

    func testKnownWithoutScopeCannotBecomeMachineLocalSpend() throws {
        let old = #"{"known":true,"since":"2026-10-01T12:00:00Z","models":[{"model":"SAMPLE-model","billed_micros":1235}],"reason_label":"legacy"}"#
        XCTAssertThrowsError(try decode(old))
        var object = try json(known)
        object.removeValue(forKey: "scope")
        XCTAssertThrowsError(try decode(serialized(object)))
    }

    func testKnownSpendRequiresCompleteAuthoritativeMetadata() throws {
        for key in ["scope", "source", "currency", "scale", "window_hours", "since", "observed_at"] {
            var object = try json(known)
            object[key] = NSNull()
            XCTAssertThrowsError(try decode(serialized(object)), key)
        }
        for (key, unsafe) in [("scope", "this_mac" as Any), ("source", "registry_price" as Any), ("currency", "points" as Any), ("scale", 6 as Any), ("window_hours", 1 as Any)] {
            var object = try json(known)
            object[key] = unsafe
            XCTAssertThrowsError(try decode(serialized(object)), key)
        }
        var backwards = try json(known)
        backwards["since"] = "2026-10-03T00:00:00Z"
        XCTAssertThrowsError(try decode(serialized(backwards)))
    }

    func testBillingRowsRefuseNegativeOrIncompleteAndIncorrectlyRoundedAmounts() throws {
        for (key, unsafe) in [("billed_nanos", -1 as Any), ("billed_micros", -1 as Any), ("calls", -1 as Any), ("rounding", "floor" as Any), ("billed_micros", 1234 as Any)] {
            var object = try json(known)
            var rows = try XCTUnwrap(object["models"] as? [[String: Any]])
            rows[0][key] = unsafe
            object["models"] = rows
            XCTAssertThrowsError(try decode(serialized(object)), key)
        }
        for key in ["billed_nanos", "billed_micros", "calls", "rounding"] {
            var object = try json(known)
            var rows = try XCTUnwrap(object["models"] as? [[String: Any]])
            rows[0].removeValue(forKey: key)
            object["models"] = rows
            XCTAssertThrowsError(try decode(serialized(object)), key)
        }
    }

    func testUnknownSpendAcceptsOldMetadataAbsenceAndNeverCarriesKnownRows() throws {
        let unknown = #"{"known":false,"since":null,"models":[],"reason_label":"billed-model-spend-unavailable"}"#
        let result = try decode(unknown)
        XCTAssertFalse(result.known)
        XCTAssertTrue(result.models.isEmpty)
        XCTAssertNil(result.since)
        var object = try json(known)
        object["known"] = false
        XCTAssertThrowsError(try decode(serialized(object)), "unknown spend cannot carry cached billed rows")
    }

    func testFullLivePathKeepsLargestExactNanosAndSafeHalfUpRounding() async throws {
        let raw = known.replacingOccurrences(of: "1234500", with: "9223372036854775807")
            .replacingOccurrences(of: "1235", with: "9223372036854776")
        let result = try await LiveDaemonClient(transport: SpendTransport(raw)).modelSpend()
        let row = try XCTUnwrap((try wire(result)["models"] as? [[String: Any]])?.first)
        XCTAssertEqual(row["billed_nanos"] as? Int64, Int64.max)
        XCTAssertEqual(row["billed_micros"] as? Int64, 9223372036854776)
    }

    func testKnownEmptyProviderReportAndMeasuredZeroStayKnown() throws {
        var empty = try json(known)
        empty["models"] = []
        let noUsage = try decode(serialized(empty))
        XCTAssertTrue(noUsage.known)
        XCTAssertTrue(noUsage.models.isEmpty)
        var zero = try json(known)
        zero["models"] = [["model": "SAMPLE-model", "billed_nanos": 0, "billed_micros": 0, "calls": 1, "rounding": "nearest_micro_half_up"]]
        let measured = try decode(serialized(zero))
        XCTAssertTrue(measured.known)
        XCTAssertEqual(measured.models.first?.billedNanos, 0)
        XCTAssertEqual(measured.models.first?.billedMicros, 0)
    }

    private func decode(_ raw: String) throws -> DaemonData.ModelSpend {
        try DaemonDataDecoding.decoder().decode(DaemonData.ModelSpend.self, from: Data(raw.utf8))
    }
    private func json(_ raw: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
    }
    private func wire<T: Encodable>(_ value: T) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any])
    }
    private func serialized(_ object: [String: Any]) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }
}

private final class SpendTransport: DaemonTransport, @unchecked Sendable {
    let response: String
    private(set) var method: String?
    private(set) var params: String?
    init(_ result: String) { response = #"{"id":1,"result":\#(result)}"# }
    func call(_ method: String, params paramsJSON: String) -> String {
        self.method = method
        params = paramsJSON
        return response
    }
}
