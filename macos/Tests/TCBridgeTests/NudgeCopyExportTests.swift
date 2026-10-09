import TCBridge
import TCShellCore
import XCTest

/// The nudge table and a Traces row's tags, read through the real dylib
/// (`tc_nudge_copy_json`, `tc_nudge_entry_tags_json`) and decoded the way
/// the views decode them.
final class NudgeCopyExportTests: XCTestCase {
    /// Every key this shell reads is in the core's table, with words.
    func testEveryKeyThisShellReadsIsInTheCoresTable() throws {
        let copy = try XCTUnwrap(NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON()))
        for key in NudgeCopy.Key.allCases {
            XCTAssertNotNil(copy[key], key.rawValue)
        }
    }

    /// The Settings rows over the real table: every shown switch worded,
    /// the interval help filled, and no placeholder left in what is drawn.
    func testSettingsRowsOverTheRealTableLeaveNoPlaceholder() throws {
        let copy = try XCTUnwrap(NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON()))
        let json = #"""
            {"suggestions_enabled":true,"menu_bar_mark_enabled":true,"notifications_enabled":true,
             "digest_interval_secs":21600,"notify":{"digest":true,"idle_sessions":true,"verdicts_landed":false},
             "verdicts_offer_pending":true,"idle_offer_pending":true}
            """#
        let settings = try JSONDecoder().decode(DaemonData.Settings.self, from: Data(json.utf8))
        let rows = NudgeSettings.rows(settings, copy: copy)
        XCTAssertEqual(rows.count, 6)
        let offers = NudgeSettings.offers(settings, copy: copy)
        XCTAssertEqual(offers.map(\.kind), ["verdicts_landed", "idle_sessions"])
        let drawn = rows.flatMap { [$0.label, $0.help ?? ""] }
            + offers.flatMap { [$0.text, $0.accept, $0.decline] }
            + [NudgeSettings.footnote(copy: copy) ?? ""]
        for text in drawn {
            XCTAssertFalse(text.contains("{"), text)
        }
        XCTAssertTrue(drawn.contains { $0.contains("6 hours") })
    }

    func testARowsTagsAreTheCoresWords() throws {
        let json = #"""
            {"pending":[{"entry_id":"e1","source":"codex","project_id":"p","project_label":"P","state":"pending",
              "mission_fit":1,
              "credit_estimate":{"low":2.0,"high":4.5,"tier":"higher","calibration":"c","basis":"published","drawn":true}}]}
            """#
        struct List: Decodable { let pending: [DaemonData.QueueEntry] }
        let entry = try JSONDecoder().decode(List.self, from: Data(json.utf8)).pending[0]
        let tags = try XCTUnwrap(NudgeEntryTags.decode(
            fromJSON: TCCoreCopy.nudgeEntryTagsJSON(entryJSON: NudgeEntryTags.input(for: entry))))
        XCTAssertEqual(tags.missionFit, "Fits a mission")
        XCTAssertEqual(tags.estimateBand, "Estimate: about 2 to 4.5 credit")
        XCTAssertEqual(tags.estimateTier, "Higher estimate")
        XCTAssertNotNil(tags.estimateExplainer)
    }

    /// An estimate the daemon does not draw, and no input at all, answer
    /// no tags.
    func testAnUndrawnEstimateAndNoInputAnswerNoTags() throws {
        let undrawn = #"{"credit_estimate":{"low":1.0,"high":3.0,"calibration":"c","basis":"built_in","drawn":false}}"#
        XCTAssertEqual(NudgeEntryTags.decode(fromJSON: TCCoreCopy.nudgeEntryTagsJSON(entryJSON: undrawn)), NudgeEntryTags())
        XCTAssertEqual(NudgeEntryTags.decode(fromJSON: TCCoreCopy.nudgeEntryTagsJSON(entryJSON: nil)), NudgeEntryTags())
    }
}
