import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// The Models card (model answers spec section 8): its rows in the core's
/// words for every state, from the daemon's recorded answer
/// (`tests/fixtures/insights-analytics/insights_model_answers.json`) and
/// the sample set's hand-written one, and how `InsightsModel` holds the
/// answer.
final class InsightsModelsCardTests: XCTestCase {
    static let fixture = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // TraceCommonsAppTests
        .deletingLastPathComponent()   // Tests
        .deletingLastPathComponent()   // macos
        .deletingLastPathComponent()   // <repo root>
        .appendingPathComponent("tests/fixtures/insights-analytics/insights_model_answers.json")

    static func recordedObject() throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: fixture))
        return try XCTUnwrap(object as? [String: Any])
    }

    static func decode(_ object: [String: Any]) throws -> DaemonData.InsightsModelAnswers {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswers.self, from: data)
    }

    static func recorded() throws -> DaemonData.InsightsModelAnswers { try decode(try recordedObject()) }

    /// The recorded answer with row `index` rewritten by `edit`.
    static func edited(row index: Int, _ edit: ([String: Any]) -> [String: Any]) throws
        -> DaemonData.InsightsModelAnswerRow {
        var object = try recordedObject()
        var rows = try XCTUnwrap(object["rows"] as? [[String: Any]])
        rows[index] = edit(rows[index])
        object["rows"] = rows
        return try XCTUnwrap(try decode(object).rows?[index])
    }

    private func copy() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }

    private func words(_ row: DaemonData.InsightsModelAnswerRow) throws -> InsightsModelsWords.Row {
        InsightsModelsWords.row(row, copy: try copy())
    }

    // MARK: - Rows

    func testEveryRowHasItsNameAndScopedFact() throws {
        let rows = InsightsModelsWords.rows(try Self.recorded(), copy: try copy())
        XCTAssertEqual(rows.map(\.title), [
            "Bug fixing", "Refactoring", "Tests", "Docs cost", "All task types", "Time to finish",
        ])
        XCTAssertEqual(rows.map(\.lead), [
            "Lowest share of edits rejected in your bug-fix traces",
            "Lowest share of edits rejected in your refactoring traces",
            "Lowest share of edits rejected in your test traces",
            "Lowest cost per docs trace, at API prices",
            "Lowest share of edits rejected across your task types, weighted by task mix",
            "Least active time to finish, in your finished traces",
        ])
        for row in rows {
            XCTAssertTrue(row.info.contains(try copy()["analytics_model_labels_note"] ?? "-"), row.id)
        }
    }

    /// Docs cost, answered: the leading model and "Next:", each with its
    /// figure at API prices and its traces, then what Private AI charged,
    /// apart from both figures.
    func testAnAnsweredCostRowNamesTheLeaderThenTheNextAtAPIPrices() throws {
        let row = try words(try XCTUnwrap(try Self.recorded().rows?[3]))
        XCTAssertNil(row.state, "an answered row says no state")
        let leaderCost = InsightsModelsWords.money(1_900)
        let nextCost = InsightsModelsWords.money(85_400)
        XCTAssertTrue(leaderCost.contains("0.0019"), leaderCost)
        XCTAssertEqual(row.models, [
            .init(text: "claude-sonnet-5-5 (\(leaderCost) at API prices, 5 traces)"),
            .init(text: "Next: claude-opus-5-5 (\(nextCost) at API prices, 5 traces)"),
        ])
        let charge = InsightsModelsWords.money(25_000)
        XCTAssertEqual(row.notes, [
            "Charged through Private AI: \(charge) per trace (median), 3 traces \u{b7} Charge not recorded for 1 of them",
        ])
        let info = row.info.joined(separator: "\n")
        XCTAssertTrue(info.contains("At API prices: what the API would charge"), info)
        XCTAssertTrue(info.contains("named first only when its lead holds"), info)
        XCTAssertTrue(info.contains("Left out of this answer:\nRoute doesn't confirm the model: 3"), info)
        XCTAssertFalse(info.contains(": 0"), "a reason with no traces is not listed")
    }

    func testARowShortOfTracesSaysSoAndCountsEachModel() throws {
        let row = try words(try XCTUnwrap(try Self.recorded().rows?[4]))
        XCTAssertEqual(row.state, "Not enough traces yet")
        XCTAssertEqual(row.models.map(\.text), [
            "claude-fixture-model: 1 trace", "claude-opus-5-5: 5 traces", "claude-sonnet-5-5: 8 traces",
        ])
        XCTAssertEqual(row.notes, [], "no basis: the row compares nothing")
        XCTAssertTrue(row.info.last?.contains("Model had too few edits: 13") == true, "\(row.info)")
        // The time row names no edit mode while it compares nothing.
        let finish = try words(try XCTUnwrap(try Self.recorded().rows?[5]))
        XCTAssertEqual(finish.state, "Not enough traces yet")
        XCTAssertEqual(finish.notes, [])
    }

    func testANotPricedRowSaysSoAndNeverShowsZero() throws {
        let raw = try Self.edited(row: 3) { row in
            var row = row
            row["state"] = "not_priced"
            row["leader"] = NSNull(); row["next"] = NSNull(); row["near_ai_charge"] = NSNull()
            return row
        }
        let row = try words(raw)
        XCTAssertEqual(row.state, "Not priced")
        XCTAssertEqual(row.models.map(\.text), ["claude-opus-5-5: 5 traces", "claude-sonnet-5-5: 5 traces"])
        XCTAssertFalse(row.models.contains { $0.text.contains("0.00") })
        XCTAssertEqual(row.notes, [])
    }

    /// No clear difference: both figures, neither first, and why behind
    /// the "i".
    func testNoClearDifferenceShowsBothFiguresWithNeitherFirst() throws {
        let sample = try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "insights_model_answers"))
        let answers = try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswers.self,
                                                              from: Data(sample.utf8))
        let row = try words(try XCTUnwrap(answers.rows?[1]))
        XCTAssertEqual(row.state, "No clear difference")
        XCTAssertEqual(row.models.map(\.text), [
            "claude-opus-4-7 (3%, 9 traces)", "claude-sonnet-4-6 (3.8%, 7 traces)",
        ])
        XCTAssertFalse(row.models.contains { $0.text.hasPrefix("Next:") })
        XCTAssertTrue(row.info.contains("The gap didn't hold when the traces were resampled, so neither model is named first."))
        XCTAssertEqual(row.lead, "Lowest share of edits undone in your refactoring traces")
        XCTAssertEqual(row.notes, ["Undone edits we could see"])
    }

    /// A rejection row on either basis, with interrupts beside each figure
    /// and never added to it; and the time row's figure and edit mode.
    func testRejectionAndTimeRowsCarryTheirBasisInterruptsAndMode() throws {
        let sample = try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "insights_model_answers"))
        let rows = InsightsModelsWords.rows(
            try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswers.self, from: Data(sample.utf8)),
            copy: try copy())
        let bugFix = rows[0]
        XCTAssertNil(bugFix.state)
        XCTAssertEqual(bugFix.lead, "Lowest share of edits undone in your bug-fix traces")
        XCTAssertEqual(bugFix.models, [
            .init(text: "claude-opus-4-7 (4.2%, 31 traces)", detail: "Interrupts per trace: 0.2"),
            .init(text: "Next: claude-sonnet-4-6 (9.1%, 14 traces)", detail: "Interrupts per trace: 0.4"),
        ])
        XCTAssertEqual(bugFix.notes, ["Undone edits we could see"])
        XCTAssertTrue(bugFix.info.first?.hasPrefix("Not every trace records the edits you declined") == true)
        let across = rows[4]
        XCTAssertEqual(across.lead, "Lowest share of edits rejected across your task types, weighted by task mix")
        XCTAssertEqual(across.notes, ["Rejected edits: ones you declined or undid"])
        let finish = rows[5]
        let leaderTime = InsightsModelsWords.duration(312)
        XCTAssertEqual(finish.models.map(\.text), [
            "claude-sonnet-4-6 (\(leaderTime) active \u{b7} 3 turns, 12 traces)",
            "Next: claude-opus-4-7 (\(InsightsModelsWords.duration(498)) active \u{b7} 4 turns, 19 traces)",
        ])
        XCTAssertNil(finish.models.first?.detail, "interrupts sit beside rejection figures only")
        XCTAssertEqual(finish.notes, ["Traces that edited without asking"])
    }

    func testAStateOrFigureThisBuildDoesNotKnowIsTheDash() throws {
        let raw = try Self.edited(row: 3) { row in
            var row = row
            row["state"] = "something_new"
            return row
        }
        let row = try words(raw)
        XCTAssertEqual(row.state, "\u{2014}")
        XCTAssertEqual(row.models, [])
        let odd = DaemonData.InsightsModelFigureValue(kind: "rate_permille")
        XCTAssertEqual(InsightsModelsWords.figure(odd, copy: try copy()), "\u{2014}", "a missing value is not 0")
        let unknownKind = DaemonData.InsightsModelFigureValue(kind: "something_new", value: 3)
        XCTAssertEqual(InsightsModelsWords.figure(unknownKind, copy: try copy()), "\u{2014}")
    }

    /// A charge NEAR AI did not record reads as not recorded, never $0.
    func testAnUnknownChargeIsNotRecordedNeverZero() throws {
        let raw = try Self.edited(row: 3) { row in
            var row = row
            row["near_ai_charge"] = ["basis": "near_ai_ledger", "median": NSNull(),
                                     "routed_sessions": 2, "charge_unknown": 2]
            return row
        }
        XCTAssertEqual(try words(raw).notes, ["Through Private AI: 2 traces, charge not recorded"])
    }

    /// Spec section 8: a scoped fact, never a ranking.
    func testNoLineRanksAModel() throws {
        let sample = try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "insights_model_answers"))
        var rows = InsightsModelsWords.rows(try Self.recorded(), copy: try copy())
        rows += InsightsModelsWords.rows(
            try DaemonDataDecoding.decoder().decode(DaemonData.InsightsModelAnswers.self, from: Data(sample.utf8)),
            copy: try copy())
        for row in rows {
            var lines: [String] = [row.title, row.lead]
            if let state = row.state { lines.append(state) }
            lines += row.models.map(\.text)
            lines += row.models.compactMap(\.detail)
            lines += row.notes
            lines += row.info
            for line in lines {
                XCTAssertFalse(line.isEmpty, "\(row.id): a key with no words")
                for banned in ["best", "better", "fastest", "winner", "benchmark"] {
                    XCTAssertFalse(line.lowercased().contains(banned), "\(row.id): \(line)")
                }
            }
        }
    }

    func testFiguresInWords() {
        XCTAssertEqual(InsightsModelsWords.percent(42), (4.2).formatted(.number.precision(.fractionLength(0...1))))
        XCTAssertEqual(InsightsModelsWords.percent(40), "4")
        XCTAssertEqual(InsightsModelsWords.perSession(180), (0.18).formatted(.number.precision(.fractionLength(0...1))))
        XCTAssertTrue(InsightsModelsWords.money(1_234_560).contains("1.23"), InsightsModelsWords.money(1_234_560))
        XCTAssertTrue(InsightsModelsWords.money(41_200).contains("0.0412"), InsightsModelsWords.money(41_200))
    }

    // MARK: - InsightsModel

    @MainActor
    private func model(_ reader: InsightsModel.ModelAnswersReader?) -> InsightsModel {
        InsightsModel(service: { _ in throw InsightsError.invalidResponse }, modelAnswersReader: reader)
    }

    @MainActor
    func testAnEnabledReadableAnswerIsHeld() async throws {
        let answers = try Self.recorded()
        let model = model({ answers })
        await model.loadModelAnswers()
        XCTAssertEqual(model.modelAnswers, answers)
    }

    @MainActor
    func testOffUnreadableOrFailedReadsDrawNoCard() async throws {
        let decoder = DaemonDataDecoding.decoder()
        let off = try decoder.decode(DaemonData.InsightsModelAnswers.self,
                                     from: Data(#"{"enabled":false,"feed":"counter_pass"}"#.utf8))
        let unreadable = try decoder.decode(
            DaemonData.InsightsModelAnswers.self,
            from: Data(#"{"enabled":true,"feed":"counter_pass","readable":false,"reason":"store_unreadable"}"#.utf8))
        let readers: [InsightsModel.ModelAnswersReader?] = [
            nil,
            { off },
            { unreadable },
            { throw DaemonDataError.daemon(code: "unknown_method", message: "insights_model_answers") },
            { throw DaemonDataError.unreachable },
        ]
        for reader in readers {
            let model = model(reader)
            await model.loadModelAnswers()
            XCTAssertNil(model.modelAnswers)
        }
    }

    @MainActor
    func testALaterFailedReadTakesTheCardAway() async throws {
        let answers = try Self.recorded()
        let reader = FlipModelAnswers(answers: answers)
        let model = model({ try await reader.read() })
        await model.loadModelAnswers()
        XCTAssertNotNil(model.modelAnswers)
        await reader.breakIt()
        await model.loadModelAnswers()
        XCTAssertNil(model.modelAnswers, "an earlier answer is not kept past a failed read")
    }

    /// The window builds its model with the daemon's reader and passes the
    /// answers to the Overview tab, which draws the card.
    func testTheWindowWiresTheCardToTheDaemon() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views")
        let window = try String(contentsOf: root.appendingPathComponent("InsightsView.swift"), encoding: .utf8)
        XCTAssertTrue(window.contains("{ try await client.insightsModelAnswers() }"))
        XCTAssertTrue(window.contains("modelAnswersReader: modelAnswersReader"))
        XCTAssertTrue(window.contains("models: model.modelAnswers,"))
        let overview = try String(contentsOf: root.appendingPathComponent("InsightsOverviewTab.swift"), encoding: .utf8)
        XCTAssertTrue(overview.contains("if let models { InsightsModelsCard(answers: models, copy: copy) }"))
    }
}

private actor FlipModelAnswers {
    let answers: DaemonData.InsightsModelAnswers
    var broken = false
    init(answers: DaemonData.InsightsModelAnswers) { self.answers = answers }
    func breakIt() { broken = true }
    func read() throws -> DaemonData.InsightsModelAnswers {
        if broken { throw DaemonDataError.unreachable }
        return answers
    }
}
