import XCTest
@testable import TCShellCore

/// The nudge switches and one-time offers Settings draws, from
/// `get_settings` and the core's nudge table (`tc_nudge_copy_json`). A
/// switch is drawn only when the daemon reported its value and the core
/// worded it; the held kinds are never drawn.
final class NudgeSettingsTests: XCTestCase {
    /// The fixed strings these rows read, as the core's table carries them.
    static let copy = NudgeCopy(table: [
        "SETTING_SUGGESTIONS": "Show suggestions",
        "SETTING_SUGGESTIONS_HELP": "Worked out here.",
        "SETTING_MARK": "Show a ring",
        "SETTING_MARK_HELP": "Never while waiting.",
        "SETTING_NOTIFY_MASTER": "Notifications from Trace Commons",
        "SETTING_DIGEST": "Waiting and contributed sessions",
        "SETTING_DIGEST_HELP_INTERVAL": "At most one notification every {hours} hours.",
        "SETTING_DIGEST_HELP_EVENING": "At most one each evening.",
        "SETTING_NOTIFY_VERDICTS": "When sessions you sent are judged",
        "SETTING_NOTIFY_IDLE": "When sessions have been idle",
        "SETTING_NOTIFY_IDLE_HELP": "One extra sentence.",
        "SETTING_NOTIFY_RECAP": "A summary of last week",
        "SETTING_NOTIFY_BUDGET_HELP": "At most one a day.",
        "OFFER_NOTIFY_VERDICTS_EXISTING": "Sessions are now judged.",
        "OFFER_NOTIFY_IDLE_EXISTING": "Idle sessions can be a notification.",
        "OFFER_TURN_ON": "Turn on",
        "OFFER_NO_THANKS": "No thanks",
        "LIST_ORDER_SUGGESTED": "Suggested first",
        "LIST_ORDER_QUEUE": "Oldest first",
    ])

    private func settings(_ json: String) throws -> DaemonData.Settings {
        try DaemonDataDecoding.decoder().decode(DaemonData.Settings.self, from: Data(json.utf8))
    }

    private static let full = #"""
        {"suggestions_enabled":true,"menu_bar_mark_enabled":true,"notifications_enabled":true,
         "digest_interval_secs":21600,"digest_schedule":{"mode":"interval"},
         "notify":{"digest":true,"idle_sessions":false,"verdicts_landed":true,"weekly_recap":false,"insights_tip":false}}
        """#

    func testEveryShownSwitchIsDrawnInOrderWithItsHelp() throws {
        let rows = NudgeSettings.rows(
            try settings(Self.full), copy: Self.copy, digestHelp: "At most one notification every 6 hours.")
        XCTAssertEqual(rows.map(\.id), [
            .suggestions, .menuBarMark, .notifications,
            .notify("digest"), .notify("verdicts_landed"), .notify("idle_sessions"),
        ])
        XCTAssertEqual(rows.map(\.label), [
            "Show suggestions", "Show a ring", "Notifications from Trace Commons",
            "Waiting and contributed sessions", "When sessions you sent are judged", "When sessions have been idle",
        ])
        XCTAssertEqual(rows.map(\.help), [
            "Worked out here.", "Never while waiting.", nil,
            "At most one notification every 6 hours.", nil, "One extra sentence.",
        ])
        XCTAssertEqual(rows.map(\.isOn), [true, true, true, true, true, false])
        XCTAssertTrue(rows.allSatisfy(\.enabled))
    }

    /// The weekly recap and the insights tip are held: never a switch,
    /// whatever the daemon reports.
    func testHeldKindsAreNeverDrawn() throws {
        let rows = NudgeSettings.rows(try settings(Self.full), copy: Self.copy, digestHelp: nil)
        XCTAssertFalse(rows.contains { $0.id == .notify("weekly_recap") || $0.id == .notify("insights_tip") })
    }

    /// A value the daemon did not report is unknown: no switch, never off.
    /// A string the core did not word draws no switch either.
    func testUnknownValuesAndMissingWordsDrawNoSwitch() throws {
        let partial = try settings(#"{"suggestions_enabled":false,"notify":{"verdicts_landed":true}}"#)
        XCTAssertEqual(NudgeSettings.rows(partial, copy: Self.copy, digestHelp: nil).map(\.id), [.suggestions, .notify("verdicts_landed")])
        XCTAssertEqual(NudgeSettings.rows(nil, copy: Self.copy, digestHelp: nil), [])
        XCTAssertEqual(NudgeSettings.rows(try settings(Self.full), copy: nil, digestHelp: nil), [])
        let unworded = NudgeCopy(table: ["SETTING_SUGGESTIONS": "Show suggestions"])
        XCTAssertEqual(NudgeSettings.rows(try settings(Self.full), copy: unworded, digestHelp: nil).map(\.id), [.suggestions])
    }

    /// The finer switch follows the broader one: the mark sits under the
    /// suggestions switch, and each kind under the master switch. They keep
    /// their stored value, drawn but not changeable.
    func testFinerSwitchesAreDisabledUnderAnOffBroaderOne() throws {
        let off = try settings(#"""
            {"suggestions_enabled":false,"menu_bar_mark_enabled":true,"notifications_enabled":false,
             "notify":{"digest":true,"idle_sessions":true,"verdicts_landed":true}}
            """#)
        let rows = Dictionary(uniqueKeysWithValues: NudgeSettings.rows(off, copy: Self.copy, digestHelp: nil).map { ($0.id, $0) })
        XCTAssertEqual(rows[.suggestions]?.enabled, true)
        XCTAssertEqual(rows[.menuBarMark]?.enabled, false)
        XCTAssertEqual(rows[.menuBarMark]?.isOn, true)
        XCTAssertEqual(rows[.notifications]?.enabled, true)
        XCTAssertEqual(rows[.notify("digest")]?.enabled, false)
        XCTAssertEqual(rows[.notify("idle_sessions")]?.enabled, false)
    }

    /// The digest's help is the core's finished line
    /// (`tc_nudge_digest_help_json`), drawn exactly as given: this shell
    /// fills no placeholder and picks no plural. No line, no help.
    func testDigestHelpIsTheCoresLineAsGiven() throws {
        func help(_ json: String, _ given: String?) throws -> String? {
            NudgeSettings.rows(try settings(json), copy: Self.copy, digestHelp: given)
                .first { $0.id == .notify("digest") }?.help
        }
        let digest = #"{"digest_interval_secs":3600,"notify":{"digest":true}}"#
        XCTAssertEqual(try help(digest, "The core's line."), "The core's line.")
        XCTAssertNil(try help(digest, nil))
    }

    /// What the core is asked: the schedule and the interval, and nothing
    /// else about the settings; and its answer read back.
    func testTheDigestHelpIsAskedOfTheCore() throws {
        let input = try XCTUnwrap(NudgeSettings.digestHelpInput(try settings(Self.full)))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(input.utf8)) as? [String: Any])
        XCTAssertEqual(Set(object.keys), ["digest_interval_secs", "digest_schedule"])
        XCTAssertEqual(object["digest_interval_secs"] as? Int, 21600)
        XCTAssertEqual((object["digest_schedule"] as? [String: Any])?["mode"] as? String, "interval")
        XCTAssertNil(NudgeSettings.digestHelpInput(nil))
        XCTAssertNil(NudgeSettings.digestHelpInput(try settings("{}")))

        XCTAssertEqual(NudgeSettings.digestHelp(fromJSON: #"{"digest_help":"Line."}"#), "Line.")
        XCTAssertNil(NudgeSettings.digestHelp(fromJSON: "{}"))
        XCTAssertNil(NudgeSettings.digestHelp(fromJSON: #"{"digest_help":"  "}"#))
        XCTAssertNil(NudgeSettings.digestHelp(fromJSON: nil))
    }

    func testThePendingOffersAreDrawnInTheCoresWords() throws {
        let pending = try settings(#"{"verdicts_offer_pending":true,"idle_offer_pending":true}"#)
        XCTAssertEqual(NudgeSettings.offers(pending, copy: Self.copy), [
            .init(kind: "verdicts_landed", text: "Sessions are now judged.", accept: "Turn on", decline: "No thanks"),
            .init(kind: "idle_sessions", text: "Idle sessions can be a notification.", accept: "Turn on", decline: "No thanks"),
        ])
        XCTAssertEqual(NudgeSettings.offers(try settings(#"{"idle_offer_pending":false}"#), copy: Self.copy), [])
        XCTAssertEqual(NudgeSettings.offers(try settings("{}"), copy: Self.copy), [])
        XCTAssertEqual(NudgeSettings.offers(pending, copy: nil), [])
    }

    /// Each kind's offer also has a page of its own: the verdicts offer on
    /// History, the idle one on Traces, where an upgraded install that never
    /// opens Settings still meets it.
    func testEachPageDrawsItsOwnKindsOffer() throws {
        let pending = try settings(#"{"verdicts_offer_pending":true,"idle_offer_pending":true}"#)
        XCTAssertEqual(NudgeSettings.offers(pending, copy: Self.copy, on: .history).map(\.kind), ["verdicts_landed"])
        XCTAssertEqual(NudgeSettings.offers(pending, copy: Self.copy, on: .traces).map(\.kind), ["idle_sessions"])
        let idleOnly = try settings(#"{"idle_offer_pending":true}"#)
        XCTAssertEqual(NudgeSettings.offers(idleOnly, copy: Self.copy, on: .history), [])
        XCTAssertEqual(NudgeSettings.offers(nil, copy: Self.copy, on: .traces), [])
    }

    func testTheFootnoteIsTheCoresBudgetLine() {
        XCTAssertEqual(NudgeSettings.footnote(copy: Self.copy), "At most one a day.")
        XCTAssertNil(NudgeSettings.footnote(copy: nil))
    }

    /// The table decodes from the export's JSON shape; blank strings are
    /// not words.
    func testTheCopyTableDecodesAndBlankIsAbsent() {
        let copy = NudgeCopy.decode(fromJSON: #"{"LIST_ORDER_QUEUE":"Oldest first","OFFER_TURN_ON":"  "}"#)
        XCTAssertEqual(copy?[.listOrderQueue], "Oldest first")
        XCTAssertNil(copy?[.offerTurnOn])
        XCTAssertNil(NudgeCopy.decode(fromJSON: nil))
        XCTAssertNil(NudgeCopy.decode(fromJSON: "[]"))
    }

    // MARK: - Row tags

    func testRowTagsDecodeFromTheCoresAnswer() {
        let tags = NudgeEntryTags.decode(fromJSON: #"""
            {"mission_fit":"Fits a mission","estimate_band":"Estimate: about 2 to 4.5 credit",
             "estimate_tier":"Higher estimate","estimate_explainer":"Made on this device."}
            """#)
        XCTAssertEqual(tags?.missionFit, "Fits a mission")
        XCTAssertEqual(tags?.estimateBand, "Estimate: about 2 to 4.5 credit")
        XCTAssertEqual(tags?.estimateTier, "Higher estimate")
        XCTAssertEqual(tags?.estimateExplainer, "Made on this device.")
        XCTAssertEqual(NudgeEntryTags.decode(fromJSON: "{}"), NudgeEntryTags())
        XCTAssertTrue(NudgeEntryTags().isEmpty)
        XCTAssertNil(NudgeEntryTags.decode(fromJSON: nil))
    }

    /// What is handed to the core is the row's own two fields, as the
    /// daemon sent them; nothing else about the row.
    func testRowTagInputIsOnlyTheRowsMissionFitAndEstimate() throws {
        let json = #"""
            {"pending":[{"entry_id":"e1","source":"codex","project_id":"p","project_label":"Secret project",
              "session_path":"/Users/x/s.jsonl","state":"pending","mission_fit":1,
              "credit_estimate":{"low":2.0,"high":4.5,"tier":"higher","calibration":"c","basis":"published","drawn":true}}]}
            """#
        let entry = try DaemonDataDecoding.decoder().decode(DaemonData.PendingList.self, from: Data(json.utf8)).pending[0]
        let input = try XCTUnwrap(NudgeEntryTags.input(for: entry))
        let object = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(input.utf8)) as? [String: Any])
        XCTAssertEqual(Set(object.keys), ["mission_fit", "credit_estimate"])
        XCTAssertEqual(object["mission_fit"] as? Int, 1)
        let estimate = try XCTUnwrap(object["credit_estimate"] as? [String: Any])
        XCTAssertEqual(estimate["drawn"] as? Bool, true)
        XCTAssertEqual(estimate["low"] as? Double, 2.0)
        XCTAssertFalse(input.contains("Secret project"))
        XCTAssertFalse(input.contains("s.jsonl"))
    }

    /// A row with neither field has nothing to ask the core.
    func testARowWithNothingToTagAsksNothing() throws {
        let json = #"{"pending":[{"entry_id":"e1","source":"codex","project_id":"p","project_label":"P","state":"pending"}]}"#
        let entry = try DaemonDataDecoding.decoder().decode(DaemonData.PendingList.self, from: Data(json.utf8)).pending[0]
        XCTAssertNil(NudgeEntryTags.input(for: entry))
    }
}
