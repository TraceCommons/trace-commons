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
