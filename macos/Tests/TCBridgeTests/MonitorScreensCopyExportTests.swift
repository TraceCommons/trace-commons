import TCBridge
import TCShellCore
import XCTest

/// The monitor screens' words cross the ABI whole, and this shell decodes
/// exactly the fields the core exports.
final class MonitorScreensCopyExportTests: XCTestCase {
    func testTheExportCarriesExactlyTheFieldsThisShellDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.monitorScreensCopyJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), MonitorScreensCopy.consumedFields.sorted())
        XCTAssertNotNil(MonitorScreensCopy.decode(fromJSON: json))
        // And each nested table's fields, so a word added in the core is
        // one this shell decodes.
        for (table, fields) in MonitorScreensCopy.consumedTables {
            let nested = try XCTUnwrap(object[table] as? [String: Any], table)
            XCTAssertEqual(nested.keys.sorted(), fields.sorted(), table)
        }
    }

    /// Ron's safeguards panel labels (#1241).
    func testTheSafeguardsLabelsDecode() throws {
        let copy = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        XCTAssertEqual(copy.safeguards.heading, "Contribution safeguards")
        XCTAssertEqual(copy.safeguards.rowsUnavailable, "{count} rows unavailable")
        XCTAssertTrue(copy.safeguards.capacityUnreadable.contains("could not read how many"))
        XCTAssertEqual(copy.safeguards.remaining, "{uploads} uploads left \u{00B7} {megabytes} MB left")
        XCTAssertEqual(copy.safeguards.heldByLimitOne, "1 queued session held by limit")
    }

    /// The screens and the Traces tab say the same thing when the core does
    /// not answer, and never the error's fixed label.
    func testErrorsAreSaidInTheCoresWords() throws {
        let screens = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        let traces = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(screens.line(for: .unreachable), traces.coreUnreachable)
        XCTAssertNotEqual(screens.line(for: .unreachable), DaemonDataError.unreachable.description)
        XCTAssertEqual(screens.line(for: .undecodable(method: "x")), traces.requestFailed)
    }
}
