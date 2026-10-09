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

    /// The edge rail's words: a singular apart, and each link's accessible
    /// name says where it goes.
    func testTheEdgeRailWordsDecode() throws {
        let copy = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        let rail = copy.edgeRail
        XCTAssertEqual(rail.onThisMac(count: 1), rail.onThisMacLineOne)
        XCTAssertTrue(rail.onThisMac(count: 3).hasPrefix("3 "))
        XCTAssertEqual(rail.inTheLibrary(count: 1), rail.inTheLibraryLineOne)
        XCTAssertTrue(rail.inTheLibrary(count: 12).hasPrefix("12 "))
        let link = rail.open(section: copy.settingsNav.tools)
        XCTAssertEqual(link.text, "Open " + copy.settingsNav.tools)
        XCTAssertTrue(link.label.hasPrefix(link.text))
        XCTAssertTrue(link.label.hasSuffix("Trace Commons"))
    }

    /// Ron's #1146 Settings modal over the Monitor (#1241 Task 10): its
    /// title, subtitle, section list name and close button, from the core.
    func testTheSettingsModalWordsDecode() throws {
        let copy = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        XCTAssertEqual(copy.settingsTitle, "Settings")
        XCTAssertNotEqual(copy.settingsTitle, copy.settings)
        XCTAssertTrue(copy.settingsSubtitle.hasPrefix("What this machine watches"))
        XCTAssertFalse(copy.settingsSections.isEmpty)
        XCTAssertFalse(copy.close.isEmpty)
    }

    /// History's refresh and sign-in words: Ron's #1146 controls, in his
    /// words (owner ruling, 2026-10-06).
    func testTheHistoryActionsDecode() throws {
        let copy = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON()))
        XCTAssertEqual(copy.historyActions.requestRefresh, "Refresh")
        XCTAssertEqual(copy.historyActions.signInToWithdraw, "Sign in")
        XCTAssertTrue(copy.historyActions.refreshFailed.hasPrefix("Could not ask for updates"))
    }

    /// Ron's #1146 toolbar, Home and History words.
    func testTheShellWordsDecode() throws {
        let shell = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())).shell
        XCTAssertEqual(shell.watching(tools: 1), "Watching 1 tool")
        XCTAssertEqual(shell.watching(tools: 3), "Watching 3 tools")
        XCTAssertEqual(shell.waiting(0, secondLook: 2), "Nothing waiting for you")
        XCTAssertEqual(shell.waiting(1, secondLook: 0), "1 session waiting for you")
        XCTAssertEqual(shell.waiting(4, secondLook: 2), "4 sessions waiting for you \u{00B7} 2 worth a second look")
        XCTAssertEqual(shell.focusTip(tool: nil, focused: false), "Select a tool, project or session first")
        XCTAssertEqual(shell.focusTip(tool: "Codex", focused: false), "Show Codex in the map")
        XCTAssertEqual(shell.focusTip(tool: "Codex", focused: true), "Back to the whole map")
        XCTAssertEqual(shell.showMap, "Show the flow map")
        XCTAssertEqual(shell.hideInspector, "Hide the inspector")
    }

    /// Ron's #1146 Home and History structure words (glass parity): his
    /// headings verbatim, a singular of its own, and his holes filled.
    func testTheHomeAndHistoryWordsDecode() throws {
        let words = try XCTUnwrap(MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())).homeHistory
        XCTAssertEqual(words.draftsTag, "Drafts")
        XCTAssertEqual(words.noMissionDrafts, "No mission drafts on this machine.")
        XCTAssertEqual(words.sources(1), "1 source")
        XCTAssertEqual(words.sources(3), "3 sources")
        XCTAssertEqual(words.contributionHistory, "Contribution history")
        XCTAssertEqual(words.records(1), "1 record")
        XCTAssertEqual(words.records(2), "2 records")
        XCTAssertEqual(words.held(1), "1 held for privacy review")
        XCTAssertEqual(words.held(4), "4 held for privacy review")
        XCTAssertEqual(words.status("In the commons"), "Status: In the commons")
        XCTAssertEqual(words.credit("3.5"), "credit 3.5")
        XCTAssertEqual(words.accepted(window: "Last 30 days"), "Accepted \u{00B7} Last 30 days")
        XCTAssertEqual(words.filterLabel, "Filter history")
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
