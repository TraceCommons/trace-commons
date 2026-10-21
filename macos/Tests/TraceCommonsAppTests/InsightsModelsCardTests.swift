import Foundation
import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The Patterns "Models" card (spec 2026-10-20 section 8): six questions,
/// each a scoped fact about the contributor's own traces. An answered row
/// names the leading model and the next one with their figures and trace
/// counts; otherwise it says which state it is in. Every row carries the
/// scope on the row itself, so a screenshot keeps it, and reads as one
/// VoiceOver element. An older daemon, counting off or an unreadable store
/// hides the card; it is never drawn greyed.
///
/// The words are stand-ins, so a test proves which key a line reads and
/// what fills its holes, never the core's sentences.
final class InsightsModelsCardTests: XCTestCase {
    static let words: [String: String] = [
        "analytics_unavailable": "DASH",
        "analytics_model_labels_note": "LABELS NOTE",
        "analytics_models_title": "MODELS",
        "analytics_models_scope": "SCOPE",
        "analytics_models_window": "FROM {range}",
        "analytics_models_bug_fix_rejected": "REJECTED BUGFIX",
        "analytics_models_bug_fix_rejected_undone": "UNDONE BUGFIX",
        "analytics_models_refactor_rejected": "REJECTED REFACTOR",
        "analytics_models_refactor_rejected_undone": "UNDONE REFACTOR",
        "analytics_models_tests_rejected": "REJECTED TESTS",
        "analytics_models_tests_rejected_undone": "UNDONE TESTS",
        "analytics_models_docs_cost": "DOCS COST",
        "analytics_models_rejected_across_tasks": "REJECTED ACROSS",
        "analytics_models_rejected_across_tasks_undone": "UNDONE ACROSS",
        "analytics_models_finish_time": "FINISH",
        "analytics_models_leader": "{label} <{x}> {n} TRACES",
        "analytics_models_next": "NEXT {label} <{x}> {n} TRACES",
        "analytics_models_rate_rejected": "{p} PCT REJECTED",
        "analytics_models_rate_undone": "{p} PCT UNDONE",
        "analytics_models_cost": "{x} AT API PRICES",
        "analytics_models_finish": "{span} ACTIVE {q} PROMPTS",
        "analytics_models_interrupts": "{x} INTERRUPTS",
        "analytics_models_not_enough": "NOT ENOUGH",
        "analytics_models_count_one": "{label}: {n} TRACE",
        "analytics_models_count_other": "{label}: {n} TRACES",
        "analytics_models_not_priced": "NOT PRICED",
        "analytics_models_not_priced_line": "NO PRICES",
        "analytics_models_no_clear_difference": "NO CLEAR DIFFERENCE",
        "analytics_models_basis_rejected": "BASIS REJECTED",
        "analytics_models_basis_undone": "BASIS UNDONE",
        "analytics_models_mode_prompting": "MODE ASKED",
        "analytics_models_mode_not_prompting": "MODE NOT ASKED",
        "analytics_models_info_scope": "INFO SCOPE",
        "analytics_models_info_task": "INFO TASK",
        "analytics_models_info_floor": "INFO FLOOR",
        "analytics_models_info_majority": "INFO MAJORITY",
        "analytics_models_info_noise": "INFO NOISE",
        "analytics_models_info_rejected": "INFO REJECTED",
        "analytics_models_info_codex": "INFO CODEX",
        "analytics_models_info_interrupts": "INFO INTERRUPTS",
        "analytics_models_info_cost": "INFO COST",
        "analytics_models_info_time": "INFO TIME",
        "analytics_models_left_out_unknown_facts": "OUT UNKNOWN {n}",
        "analytics_models_left_out_delegated": "OUT DELEGATED {n}",
        "analytics_models_left_out_capped": "OUT CAPPED {n}",
        "analytics_models_left_out_routed": "OUT ROUTED {n}",
        "analytics_models_left_out_reviewer": "OUT REVIEWER {n}",
        "analytics_models_left_out_no_majority": "OUT NO MAJORITY {n}",
        "analytics_models_left_out_unpriced": "OUT UNPRICED {n}",
        "analytics_models_left_out_too_few_edits": "OUT FEW EDITS {n}",
        "analytics_models_left_out_mix_too_different": "OUT MIX {n}",
    ]

    static let zeroLeftOut = #"{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}"#

    /// The daemon's printed reply over its `docs_priced` fixture (see
    /// `InsightsModelAnswersContractTests.recorded`): one answered cost row,
    /// the rest without enough traces.
    static let recorded = #"{"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-09-20T12:00:00+00:00","window_start":"2026-06-21T12:00:00+00:00","window_end":"2026-09-20T12:00:00+00:00","pricing":{"basis":"api_list","tables":[{"table_id":"sha256:cc9f2ae4075adad77dc148f24e3d7a4e52b1fbd1452b5be140090a2751e7eecd","version":1}]},"rows":[{"question":"bug_fix_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}},{"question":"refactor_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}},{"question":"tests_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}},{"question":"docs_cost","state":"answered","basis":null,"edit_mode":null,"leader":{"label":"claude-sonnet-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":1900},"interrupts_per_session_permille":null},"next":{"label":"claude-opus-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":85400},"interrupts_per_session_permille":null},"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",5]],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}},{"question":"rejected_across_tasks","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",5]],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":10,"mix_too_different":0}},{"question":"finish_time","state":"not_enough_sessions","basis":null,"edit_mode":"prompting","leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":10,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}}]}"#

    static let undoneRow = #"{"question":"bug_fix_rejected","state":"answered","basis":"undone","edit_mode":null,"leader":{"label":"claude-opus-4-7","sessions":31,"edits":412,"figure":{"kind":"rate_permille","value":42},"interrupts_per_session_permille":1500},"next":{"label":"claude-sonnet-4-6","sessions":14,"edits":160,"figure":{"kind":"rate_permille","value":91},"interrupts_per_session_permille":null},"sessions_by_model":[["claude-opus-4-7",31],["claude-sonnet-4-6",14]],"left_out":{"unknown_facts":3,"delegated":1,"capped":0,"routed":0,"reviewer":0,"no_majority":2,"unpriced":0,"too_few_edits":0,"mix_too_different":0}}"#

    static let finishRow = #"{"question":"finish_time","state":"no_clear_difference","basis":null,"edit_mode":"not_prompting","leader":{"label":"gpt-5.5","sessions":8,"edits":90,"figure":{"kind":"finish_time","median_active_secs":754,"median_user_turns":4},"interrupts_per_session_permille":null},"next":{"label":"claude-opus-4-7","sessions":12,"edits":200,"figure":{"kind":"finish_time","median_active_secs":801,"median_user_turns":5},"interrupts_per_session_permille":null},"sessions_by_model":[["claude-opus-4-7",12],["gpt-5.5",8]],"left_out":\#(zeroLeftOut)}"#

    static let notPricedRow = #"{"question":"docs_cost","state":"not_priced","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[["model-a",7],["model-b",1]],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":8,"too_few_edits":0,"mix_too_different":0}}"#

    static func row(_ json: String) throws -> DaemonData.InsightsModelAnswerRow {
        try JSONDecoder().decode(DaemonData.InsightsModelAnswerRow.self, from: Data(json.utf8))
    }

    static func answers(_ json: String = recorded) throws -> DaemonData.InsightsModelAnswers {
        try JSONDecoder().decode(DaemonData.InsightsModelAnswers.self, from: Data(json.utf8))
    }

    let words = InsightsModelsCardTests.words

    // MARK: Rows

    func testAnAnsweredRejectionRowNamesTheLeaderAndTheNext() throws {
        let row = try Self.row(Self.undoneRow)
        XCTAssertEqual(InsightsModelsWords.title(row, copy: words), "UNDONE BUGFIX",
                       "the undone basis says undone edits in the question itself")
        XCTAssertEqual(InsightsModelsWords.lines(row, copy: words), [
            "claude-opus-4-7 <4 PCT UNDONE> 31 TRACES \u{b7} 1.5 INTERRUPTS",
            "NEXT claude-sonnet-4-6 <9 PCT UNDONE> 14 TRACES",
        ], "interrupts sit beside the figure, and an unknown count is left off, never 0")
        XCTAssertEqual(InsightsModelsWords.basisLine(row, copy: words), "BASIS UNDONE")
        XCTAssertEqual(InsightsModelsWords.scope(copy: words), "SCOPE")
    }

    func testTheRejectedBasisReadsRejected() throws {
        let row = try Self.row(Self.undoneRow.replacingOccurrences(of: #""basis":"undone""#,
                                                                   with: #""basis":"rejected""#))
        XCTAssertEqual(InsightsModelsWords.title(row, copy: words), "REJECTED BUGFIX")
        XCTAssertEqual(InsightsModelsWords.lines(row, copy: words).last, "NEXT claude-sonnet-4-6 <9 PCT REJECTED> 14 TRACES")
        XCTAssertEqual(InsightsModelsWords.basisLine(row, copy: words), "BASIS REJECTED")
    }

    func testTheCostRowIsMoneyAtAPIPrices() throws {
        let row = try XCTUnwrap(try Self.answers().rows?[3])
        XCTAssertEqual(InsightsModelsWords.title(row, copy: words), "DOCS COST")
        let money = InsightsModelsWords.money(1900)
        XCTAssertTrue(money.contains("0.0019"), money)
        XCTAssertEqual(InsightsModelsWords.lines(row, copy: words), [
            "claude-sonnet-5-5 <\(money) AT API PRICES> 5 TRACES",
            "NEXT claude-opus-5-5 <\(InsightsModelsWords.money(85400)) AT API PRICES> 5 TRACES",
        ])
        XCTAssertNil(InsightsModelsWords.basisLine(row, copy: words))
    }

    func testNoClearDifferenceShowsBothFiguresAndTheMode() throws {
        let row = try Self.row(Self.finishRow)
        XCTAssertEqual(InsightsModelsWords.title(row, copy: words), "FINISH")
        let lines = InsightsModelsWords.lines(row, copy: words)
        XCTAssertEqual(lines.first, "NO CLEAR DIFFERENCE")
        XCTAssertEqual(lines.count, 3)
        XCTAssertTrue(lines[1].hasPrefix("gpt-5.5 <"), lines[1])
        XCTAssertTrue(lines[1].hasSuffix(" ACTIVE 4 PROMPTS> 8 TRACES"), lines[1])
        XCTAssertTrue(lines[2].hasPrefix("claude-opus-4-7 <"), "no \"Next\": neither leads: \(lines[2])")
        XCTAssertEqual(InsightsModelsWords.basisLine(row, copy: words), "MODE NOT ASKED")
    }

    func testNotEnoughTracesShowsEachModelsCount() throws {
        let rows = try XCTUnwrap(try Self.answers().rows)
        XCTAssertEqual(InsightsModelsWords.lines(rows[4], copy: words),
                       ["NOT ENOUGH", "claude-opus-5-5: 5 TRACES", "claude-sonnet-5-5: 5 TRACES"])
        XCTAssertEqual(InsightsModelsWords.lines(rows[0], copy: words), ["NOT ENOUGH"])
        XCTAssertNil(InsightsModelsWords.basisLine(rows[5], copy: words),
                     "a mode is shown only beside figures")
        let one = try Self.row(Self.notPricedRow)
        XCTAssertEqual(InsightsModelsWords.lines(one, copy: words),
                       ["NOT PRICED", "NO PRICES", "model-a: 7 TRACES", "model-b: 1 TRACE"],
                       "not priced is its own state, never not enough traces")
    }

    func testAFigureOfAnUnknownKindIsTheDashNeverZero() throws {
        let row = try Self.row(Self.undoneRow.replacingOccurrences(of: #""kind":"rate_permille","value":42"#,
                                                                   with: #""kind":"later_kind","value":0"#))
        XCTAssertEqual(InsightsModelsWords.lines(row, copy: words).first,
                       "claude-opus-4-7 <DASH> 31 TRACES \u{b7} 1.5 INTERRUPTS")
    }

    // MARK: The (i)

    func testEachRowsInfoSaysHowItIsMeasuredAndWhatWasLeftOut() throws {
        let rejection = try Self.row(Self.undoneRow)
        XCTAssertEqual(InsightsModelsWords.info(rejection, copy: words), [
            "INFO REJECTED", "INFO CODEX", "INFO INTERRUPTS", "INFO FLOOR", "INFO MAJORITY", "INFO NOISE",
            "OUT UNKNOWN 3", "OUT DELEGATED 1", "OUT NO MAJORITY 2",
        ], "only the reasons that left a trace out")
        let docs = try XCTUnwrap(try Self.answers().rows?[3])
        XCTAssertEqual(InsightsModelsWords.info(docs, copy: words),
                       ["INFO COST", "INFO FLOOR", "INFO MAJORITY", "INFO NOISE"])
        let finish = try Self.row(Self.finishRow)
        XCTAssertEqual(InsightsModelsWords.info(finish, copy: words),
                       ["INFO TIME", "INFO FLOOR", "INFO MAJORITY", "INFO NOISE"])
        XCTAssertEqual(InsightsModelsWords.cardInfo(copy: words), ["INFO SCOPE", "LABELS NOTE", "INFO TASK"])
    }

    func testTheWindowLineIsTheAnswersThirteenWeeks() throws {
        let line = try XCTUnwrap(InsightsModelsWords.window(try Self.answers(), copy: words))
        XCTAssertEqual(line, "FROM " + InsightsOverviewWords.weekRange(start: "2026-06-21", end: "2026-09-20"))
    }

    // MARK: VoiceOver

    func testEachRowIsOneSpokenSentence() throws {
        let row = try Self.row(Self.undoneRow)
        XCTAssertEqual(InsightsModelsWords.accessibilityLabel(row, copy: words), [
            "UNDONE BUGFIX",
            "claude-opus-4-7 <4 PCT UNDONE> 31 TRACES \u{b7} 1.5 INTERRUPTS",
            "NEXT claude-sonnet-4-6 <9 PCT UNDONE> 14 TRACES",
            "BASIS UNDONE",
            "SCOPE",
        ].joined(separator: ". "))
    }

    // MARK: Shown or hidden

    @MainActor func testTheAnswersAreShownOnlyWhenEnabledAndReadable() async throws {
        let shown = InsightsModelAnswersModel(reader: { try Self.answers() })
        await shown.load()
        XCTAssertEqual(shown.answers?.rows?.count, 6)

        for json in [#"{"enabled":false,"feed":"counter_pass"}"#,
                     #"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"key_unavailable"}"#] {
            let hidden = InsightsModelAnswersModel(reader: { try Self.answers(json) })
            await hidden.load()
            XCTAssertNil(hidden.answers, json)
        }
    }

    @MainActor func testAnOlderDaemonOrNoDaemonHidesTheCard() async {
        for error in [DaemonDataError.notAvailableYet(method: "insights_model_answers"),
                      DaemonDataError.daemon(code: "unknown_method", message: "unknown-method"),
                      DaemonDataError.unreachable] {
            let model = InsightsModelAnswersModel(reader: { throw error })
            await model.load()
            XCTAssertNil(model.answers, "\(error)")
        }
        let none = InsightsModelAnswersModel(reader: nil)
        await none.load()
        XCTAssertNil(none.answers)
    }

    /// A failed read after a good one takes the card away rather than
    /// keeping figures it can no longer stand behind.
    @MainActor func testAFailedReloadHidesTheCard() async throws {
        let failing = FlagBox()
        let model = InsightsModelAnswersModel(reader: {
            if failing.value { throw DaemonDataError.unreachable }
            return try Self.answers()
        })
        await model.load()
        XCTAssertNotNil(model.answers)
        failing.value = true
        await model.load()
        XCTAssertNil(model.answers)
    }
}

private final class FlagBox: @unchecked Sendable {
    var value = false
}
