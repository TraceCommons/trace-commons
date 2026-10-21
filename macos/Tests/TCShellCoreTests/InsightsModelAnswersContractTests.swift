import Foundation
import XCTest
@testable import TCShellCore

/// `insights_model_answers` (spec 2026-10-20 section 7): the Models card's
/// six answers over the counter pass. Decoded as the daemon prints them;
/// every field a shape can omit is optional, so the off and unreadable
/// shapes decode too, and only an enabled, readable answer with its rows is
/// drawn.
final class InsightsModelAnswersContractTests: XCTestCase {
    private func frame(_ result: String) -> String { #"{"id":0,"result":\#(result)}"# }

    /// Printed by the daemon's own `model_answers_value` over the
    /// `docs_priced` fixture of `daemon/insights_model_answers.rs` (ten
    /// documentation traces on two priced models), on #1510 at 87b048dad.
    /// Re-print rather than edit: the edited shapes below start from it.
    static let recorded = #"{"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-09-20T12:00:00+00:00","window_start":"2026-06-21T12:00:00+00:00","window_end":"2026-09-20T12:00:00+00:00","pricing":{"basis":"api_list","tables":[{"table_id":"sha256:cc9f2ae4075adad77dc148f24e3d7a4e52b1fbd1452b5be140090a2751e7eecd","version":1}]},"rows":[{"question":"bug_fix_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"refactor_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"tests_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"docs_cost","state":"answered","basis":null,"edit_mode":null,"leader":{"label":"claude-sonnet-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":1900},"interrupts_per_session_permille":null,"near_ai_charge":null},"next":{"label":"claude-opus-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":85400},"interrupts_per_session_permille":null,"near_ai_charge":null},"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",5]],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"rejected_across_tasks","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",5]],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":10,"mix_too_different":0},"near_ai_charge":null},{"question":"finish_time","state":"not_enough_sessions","basis":null,"edit_mode":"prompting","leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":10,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null}]}"#

    /// Printed the same way over the `docs_charged` fixture: `docs_priced`
    /// plus three documentation traces sent through NEAR AI, one of them
    /// with a call the ledger did not charge, at #1510 87b048dad.
    static let recordedCharged = #"{"enabled":true,"feed":"counter_pass","readable":true,"updated_at":"2026-09-20T12:00:00+00:00","window_start":"2026-06-21T12:00:00+00:00","window_end":"2026-09-20T12:00:00+00:00","pricing":{"basis":"api_list","tables":[{"table_id":"sha256:cc9f2ae4075adad77dc148f24e3d7a4e52b1fbd1452b5be140090a2751e7eecd","version":1}]},"rows":[{"question":"bug_fix_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"refactor_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"tests_rejected","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null},{"question":"docs_cost","state":"answered","basis":null,"edit_mode":null,"leader":{"label":"claude-sonnet-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":1900},"interrupts_per_session_permille":null,"near_ai_charge":{"basis":"near_ai_ledger","median":{"kind":"usd_micros","value":25000},"routed_sessions":3,"charge_unknown":1}},"next":{"label":"claude-opus-5-5","sessions":5,"edits":null,"figure":{"kind":"usd_micros","value":85400},"interrupts_per_session_permille":null,"near_ai_charge":null},"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",5]],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":3,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":{"basis":"near_ai_ledger","median":{"kind":"usd_micros","value":25000},"routed_sessions":3,"charge_unknown":1}},{"question":"rejected_across_tasks","state":"not_enough_sessions","basis":null,"edit_mode":null,"leader":null,"next":null,"sessions_by_model":[["claude-opus-5-5",5],["claude-sonnet-5-5",8]],"left_out":{"unknown_facts":0,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":13,"mix_too_different":0},"near_ai_charge":null},{"question":"finish_time","state":"not_enough_sessions","basis":null,"edit_mode":"prompting","leader":null,"next":null,"sessions_by_model":[],"left_out":{"unknown_facts":13,"automated":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0},"near_ai_charge":null}]}"#

    /// One answered rejection row on the undone basis, with interrupts, as
    /// `ModelAnswerRow` serializes (the shape `the_payload_has_the_documented_shape`
    /// pins); the engine needs 30-edit cells to print one.
    static let undoneRow = #"{"question":"bug_fix_rejected","state":"answered","basis":"undone","edit_mode":null,"leader":{"label":"claude-opus-4-7","sessions":31,"edits":412,"figure":{"kind":"rate_permille","value":42},"interrupts_per_session_permille":1500},"next":{"label":"claude-sonnet-4-6","sessions":14,"edits":160,"figure":{"kind":"rate_permille","value":91},"interrupts_per_session_permille":null},"sessions_by_model":[["claude-opus-4-7",31],["claude-sonnet-4-6",14]],"left_out":{"unknown_facts":3,"delegated":1,"capped":0,"routed":0,"reviewer":0,"no_majority":2,"unpriced":0,"too_few_edits":0,"mix_too_different":0}}"#

    /// A finish-time row with no clear difference: both figures are present.
    static let finishRow = #"{"question":"finish_time","state":"no_clear_difference","basis":null,"edit_mode":"not_prompting","leader":{"label":"gpt-5.5","sessions":8,"edits":90,"figure":{"kind":"finish_time","median_active_secs":754,"median_user_turns":4},"interrupts_per_session_permille":null},"next":{"label":"claude-opus-4-7","sessions":12,"edits":200,"figure":{"kind":"finish_time","median_active_secs":801,"median_user_turns":5},"interrupts_per_session_permille":null},"sessions_by_model":[["claude-opus-4-7",12],["gpt-5.5",8]],"left_out":{"unknown_facts":0,"delegated":0,"capped":0,"routed":0,"reviewer":0,"no_majority":0,"unpriced":0,"too_few_edits":0,"mix_too_different":0}}"#

    func testTheRecordedAnswerDecodesThroughTheLiveClient() async throws {
        let transport = FakeTransport(response: frame(Self.recorded))
        let answers = try await LiveDaemonClient(transport: transport).insightsModelAnswers()
        XCTAssertEqual(transport.calls.map(\.method), ["insights_model_answers"])
        XCTAssertEqual(transport.calls.first?.params, "{}", "the method takes no params")
        XCTAssertTrue(answers.showsAnswers)
        XCTAssertEqual(answers.windowStart, "2026-06-21T12:00:00+00:00")
        XCTAssertEqual(answers.windowEnd, "2026-09-20T12:00:00+00:00")
        XCTAssertEqual(answers.pricing?.basis, "api_list")
        XCTAssertEqual(answers.pricing?.tables.map(\.version), [1])
        let rows = try XCTUnwrap(answers.rows)
        XCTAssertEqual(rows.map(\.question), ["bug_fix_rejected", "refactor_rejected", "tests_rejected",
                                              "docs_cost", "rejected_across_tasks", "finish_time"])
        let docs = rows[3]
        XCTAssertEqual(docs.state, "answered")
        XCTAssertEqual(docs.leader?.label, "claude-sonnet-5-5")
        XCTAssertEqual(docs.leader?.figure.usdMicros, 1900)
        XCTAssertNil(docs.leader?.edits, "a cost figure carries no edits, never 0")
        XCTAssertNil(docs.leader?.interruptsPerSessionPermille)
        XCTAssertEqual(docs.next?.figure.usdMicros, 85400)
        XCTAssertEqual(docs.sessionsByModel, [.init(label: "claude-opus-5-5", sessions: 5),
                                              .init(label: "claude-sonnet-5-5", sessions: 5)])
        XCTAssertEqual(rows[4].leftOut.tooFewEdits, 10)
        XCTAssertEqual(rows[5].editMode, "prompting")
        XCTAssertEqual(rows[5].leftOut.unknownFacts, 10)
        XCTAssertEqual(rows[5].leftOut.counts.map(\.reason),
                       ["unknown_facts", "automated", "delegated", "capped", "routed", "reviewer", "no_majority",
                        "unpriced", "too_few_edits", "mix_too_different"],
                       "every reason, in the wire's order")
        XCTAssertEqual(rows[5].leftOut.automated, 0)
        XCTAssertNil(docs.nearAiCharge, "no trace went through NEAR AI: no charge, never a zero")
        XCTAssertNil(docs.leader?.nearAiCharge)
    }

    /// The NEAR AI charge is its own figure beside the API-price one: on the
    /// docs cost row, and on the figure of the model whose traces it covers.
    func testTheNearAIChargeDecodesApartFromTheAPIPrice() throws {
        let answers = try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswers.self,
                                                               from: Data(Self.recordedCharged.utf8))
        let rows = try XCTUnwrap(answers.rows)
        let docs = rows[3]
        let charge = try XCTUnwrap(docs.nearAiCharge)
        XCTAssertEqual(charge.basis, "near_ai_ledger")
        XCTAssertEqual(charge.median?.usdMicros, 25000)
        XCTAssertEqual(charge.routedSessions, 3)
        XCTAssertEqual(charge.chargeUnknown, 1)
        XCTAssertEqual(docs.leader?.nearAiCharge, charge)
        XCTAssertNil(docs.next?.nearAiCharge)
        XCTAssertEqual(docs.leader?.figure.usdMicros, 1900, "the API-price figure is untouched")
        XCTAssertEqual(docs.leftOut.routed, 3)
        for row in rows where row.question != "docs_cost" {
            XCTAssertNil(row.nearAiCharge, row.question)
        }
        let unknown = try DaemonDataDecoding.decoder().decode(
            DaemonData.InsightsModelNearAICharge.self,
            from: Data(#"{"basis":"near_ai_ledger","median":null,"routed_sessions":2,"charge_unknown":2}"#.utf8))
        XCTAssertNil(unknown.median, "an unknown charge is no figure, never 0")
    }

    /// A row from before `left_out.automated` and `near_ai_charge` decodes,
    /// with both unknown.
    func testAnOlderRowWithoutTheNewFieldsDecodes() throws {
        let row = try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswerRow.self,
                                                          from: Data(Self.undoneRow.utf8))
        XCTAssertNil(row.leftOut.automated)
        XCTAssertNil(row.nearAiCharge)
        XCTAssertNil(row.leader?.nearAiCharge)
    }

    func testRateAndFinishFiguresDecode() throws {
        let decoder = DaemonDataDecoding.decoder()
        let undone = try decoder.decode(DaemonData.InsightsModelAnswerRow.self, from: Data(Self.undoneRow.utf8))
        XCTAssertEqual(undone.basis, "undone")
        XCTAssertEqual(undone.leader?.figure.ratePermille, 42)
        XCTAssertEqual(undone.leader?.edits, 412)
        XCTAssertEqual(undone.leader?.interruptsPerSessionPermille, 1500)
        XCTAssertNil(undone.next?.interruptsPerSessionPermille, "unknown interrupts stay unknown, never 0")
        let finish = try decoder.decode(DaemonData.InsightsModelAnswerRow.self, from: Data(Self.finishRow.utf8))
        XCTAssertEqual(finish.state, "no_clear_difference")
        XCTAssertEqual(finish.leader?.figure.kind, "finish_time")
        XCTAssertEqual(finish.leader?.figure.medianActiveSecs, 754)
        XCTAssertEqual(finish.leader?.figure.medianUserTurns, 4)
        XCTAssertNil(finish.leader?.figure.ratePermille)
    }

    func testOffAndUnreadableDecodeAndAreNotShown() throws {
        let decoder = DaemonDataDecoding.decoder()
        let off = try decoder.decode(DaemonData.InsightsModelAnswers.self,
                                     from: Data(#"{"enabled":false,"feed":"counter_pass"}"#.utf8))
        XCTAssertFalse(off.showsAnswers)
        let unreadable = try decoder.decode(
            DaemonData.InsightsModelAnswers.self,
            from: Data(#"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"store_unreadable"}"#.utf8))
        XCTAssertFalse(unreadable.showsAnswers)
        XCTAssertEqual(unreadable.reason, "store_unreadable")
    }

    func testAnOlderDaemonsRefusalReachesTheCaller() async {
        let transport = FakeTransport(response: #"{"id":0,"error":{"code":"unknown_method","message":"unknown-method"}}"#)
        do {
            _ = try await LiveDaemonClient(transport: transport).insightsModelAnswers()
            XCTFail("an older daemon answers unknown_method")
        } catch {
            XCTAssertTrue((error as? DaemonDataError)?.isNotServed == true, "\(error)")
        }
    }

    /// The sample records no counter pass, so it has no answers, as a daemon
    /// that predates the method.
    func testTheSampleClientHasNoAnswers() async {
        do {
            _ = try await SampleDaemonClient(.normalDay).insightsModelAnswers()
            XCTFail("the sample has no answers")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: "insights_model_answers"))
        }
    }
}
