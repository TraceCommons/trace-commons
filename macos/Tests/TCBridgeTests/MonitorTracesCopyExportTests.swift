import TCBridge
import TCShellCore
import XCTest

/// The monitor's Traces words come from the core, read through the real
/// dylib and decoded the way the Traces tab decodes them.
final class MonitorTracesCopyExportTests: XCTestCase {
    func testTheExportCarriesExactlyTheFieldsThisShellDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.monitorTracesCopyJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), MonitorTracesCopy.consumedFields.sorted())
        XCTAssertNotNil(MonitorTracesCopy.decode(fromJSON: json))
    }

    /// Keep and its undo are Customize's words, so the two surfaces agree.
    func testKeepIsTheCustomizeLabel() throws {
        let copy = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(copy.keep, "Keep on this Mac")
    }

    /// A core that does not answer, and a request that failed, each get the
    /// core's line, never the error's fixed label.
    func testErrorsAreSaidInTheCoresWords() throws {
        let copy = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(copy.line(for: .unreachable), copy.coreUnreachable)
        XCTAssertEqual(copy.line(for: .daemon(code: "x", message: "y")), copy.requestFailed)
        XCTAssertNotEqual(copy.line(for: .unreachable), DaemonDataError.unreachable.description)
    }

    /// The badge's text equivalent: unknown is never zero, zero says nothing.
    func testTheBadgeTextNeverReadsUnknownAsZero() throws {
        let unknown = try XCTUnwrap(TCCoreCopy.decisionsOwedText(nil))
        XCTAssertFalse(unknown.isEmpty)
        XCTAssertEqual(TCCoreCopy.decisionsOwedText(0), "")
        let three = try XCTUnwrap(TCCoreCopy.decisionsOwedText(3))
        XCTAssertTrue(three.contains("3"), three)
        XCTAssertNotEqual(unknown, three)
    }

    func testAnEmptyFieldDecodesToNothing() throws {
        let json = try XCTUnwrap(TCCoreCopy.monitorTracesCopyJSON())
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        object["contribute"] = ""
        let broken = String(data: try JSONSerialization.data(withJSONObject: object), encoding: .utf8)
        XCTAssertNil(MonitorTracesCopy.decode(fromJSON: broken))
    }
}
