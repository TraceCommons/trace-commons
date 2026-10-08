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
        // And each nested table's fields, so a word added in the core is
        // one this shell decodes.
        for (table, fields) in MonitorTracesCopy.consumedTables {
            let nested = try XCTUnwrap(object[table] as? [String: Any], table)
            XCTAssertEqual(nested.keys.sorted(), fields.sorted(), table)
        }
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

    /// Ron's #1146 inspector words (#1241), nested by the part of the
    /// inspector that shows them, cross whole.
    func testRonsInspectorWordsDecode() throws {
        let copy = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(copy.tree.submitCount, "Submit \u{00B7} {count}")
        XCTAssertEqual(copy.tree.dismissSessionTitle, "Dismiss this session?")
        // #1273 review: the folder tile's mark and the bucket's note are the
        // core's, the note keeping "You'll always be asked".
        XCTAssertEqual(copy.tree.folderMark, "dir")
        XCTAssertEqual(copy.tree.unresolvedBucketNote,
                       "These sessions cannot be contributed automatically. You'll always be asked.")
        XCTAssertEqual(copy.counts.sessionsWaiting, "{count} sessions waiting")
        XCTAssertEqual(copy.inspector.noRule, "This folder has no rule of its own yet.")
        XCTAssertEqual(copy.inspector.applyOutcome, "Apply one outcome to {count} eligible sessions.")
        XCTAssertEqual(copy.summaryPanel.statistics, "Statistics")
        XCTAssertEqual(copy.sessionReview.heading, "What would leave this computer")
        XCTAssertEqual(copy.lookInside.witnessConfirmLine, "I understand and want to send this session for review.")
        XCTAssertEqual(copy.undo.within, "Undo within {seconds}s before upload starts.")
        XCTAssertEqual(copy.optionalAutomation, "OPTIONAL AUTOMATION")
        XCTAssertEqual(copy.dismissAction, "Dismiss")
        // Native's "Not this one" is untouched beside Ron's Dismiss.
        XCTAssertNotEqual(copy.dismiss, copy.dismissAction)
    }

    /// An empty word anywhere in a nested table fails the whole decode.
    func testAnEmptyNestedWordDecodesToNothing() throws {
        let json = try XCTUnwrap(TCCoreCopy.monitorTracesCopyJSON())
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        var lookInside = try XCTUnwrap(object["look_inside"] as? [String: Any])
        lookInside["turn_index"] = ""
        object["look_inside"] = lookInside
        let broken = String(data: try JSONSerialization.data(withJSONObject: object), encoding: .utf8)
        XCTAssertNil(MonitorTracesCopy.decode(fromJSON: broken))
    }

    func testAnEmptyFieldDecodesToNothing() throws {
        let json = try XCTUnwrap(TCCoreCopy.monitorTracesCopyJSON())
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        object["contribute"] = ""
        let broken = String(data: try JSONSerialization.data(withJSONObject: object), encoding: .utf8)
        XCTAssertNil(MonitorTracesCopy.decode(fromJSON: broken))
    }
}
