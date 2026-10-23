import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Staging builds show where the signed NEAR AI pin set stands, under the
/// Private AI state line, in the core's sentence. Production shows nothing.
/// The dylib under test decides which half of each test is live, so the suite
/// is run against both a default and a staging dylib.
@MainActor
final class PinStatusLineTests: XCTestCase {
    private var isStaging: Bool { TCBuildChannel.windowLabel() != nil }

    private func status(_ pins: String?) throws -> DaemonData.Status {
        let field = pins.map { #","near_ai_pins":"\#($0)""# } ?? ""
        let json = #"{"paused":false\#(field)}"#
        return try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
    }

    func testTheFieldIsDeclaredAndOptional() throws {
        XCTAssertTrue(DaemonData.Status.CodingKeys.allCases.map(\.rawValue).contains("near_ai_pins"))
        XCTAssertNil(try status(nil).nearAiPins)
        XCTAssertEqual(try status("pinned").nearAiPins, "pinned")
    }

    func testAProductionStatusShowsNoLine() async throws {
        // A default daemon never sends the field.
        let store = InferenceStore(client: SampleDaemonClient(.normalDay))
        store.landStatusRead(try status(nil))
        XCTAssertNil(store.pinLine)
        await store.load()
        XCTAssertNil(store.pinLine, "a status without the field draws no line")
        if !isStaging {
            // Even a stray label draws nothing on a default dylib.
            store.landStatusRead(try status("pinned"))
            XCTAssertNil(store.pinLine)
        }
    }

    func testAStagingStatusShowsTheCoresSentence() throws {
        try XCTSkipUnless(isStaging, "needs a staging dylib")
        let store = InferenceStore(client: nil)
        for label in ["pinned", "pins_unavailable", "pins_expired", "pins_unknown_key", "pins_from_a_later_daemon"] {
            store.landStatusRead(try status(label))
            let line = try XCTUnwrap(store.pinLine, label)
            XCTAssertEqual(line, TCBuildChannel.pinStatusLine(label: label))
            XCTAssertFalse(line.contains("pins_"), "the label is never shown: \(line)")
        }
        store.landStatusRead(try status("pinned"))
        XCTAssertNotEqual(store.pinLine, TCBuildChannel.pinStatusLine(label: "pins_expired"))
        // A failed read, or a status that no longer names the field, clears it.
        store.landStatusRead(nil)
        XCTAssertNil(store.pinLine)
    }

    func testTheBridgeAnswersNothingOnProduction() {
        for label in ["pinned", "pins_unavailable", "", "anything"] {
            let line = TCBuildChannel.pinStatusLine(label: label)
            XCTAssertEqual(line == nil, !isStaging, label)
        }
    }

    func testTheCardDrawsTheLineOnlyWhenGiven() throws {
        let source = try String(
            contentsOf: URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .appendingPathComponent("Sources/TraceCommonsApp/Views/PrivateInferenceView.swift"),
            encoding: .utf8)
        XCTAssertTrue(source.contains("if let pinLine {"))
        XCTAssertTrue(source.contains("var pinLine: String?"))
    }
}
