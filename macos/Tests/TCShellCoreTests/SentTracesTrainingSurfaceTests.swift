import XCTest

@testable import TCShellCore

/// Adding Train coding models to sent traces: the pure logic, without the
/// dylib. The words are placeholders ("T:" prefixed) so a test passes only
/// by reading the payload; `TCBridgeTests` checks the live export's keys.
final class SentTracesTrainingSurfaceTests: XCTestCase {
    private static func copyJSON(_ override: [String: String] = [:]) -> String {
        var object = [String: String]()
        for field in SentTracesTrainingCopy.consumedFields {
            object[field] = "T:\(field) {count}"
        }
        for (key, value) in override { object[key] = value }
        let data = try! JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        return String(data: data, encoding: .utf8)!
    }

    private let copy = SentTracesTrainingCopy.decode(fromJSON: copyJSON())!

    func testAPayloadWithAMissingOrBlankFieldIsRefusedWhole() {
        XCTAssertNil(SentTracesTrainingCopy.decode(fromJSON: Self.copyJSON(["pending": "  "])))
        var object = try! JSONSerialization.jsonObject(with: Data(Self.copyJSON().utf8)) as! [String: String]
        object.removeValue(forKey: "not_now_button")
        let data = try! JSONSerialization.data(withJSONObject: object)
        XCTAssertNil(SentTracesTrainingCopy.decode(fromJSON: String(decoding: data, as: UTF8.self)))
        XCTAssertNil(SentTracesTrainingCopy.decode(fromJSON: "not json"))
    }

    func testOneTraceReadsTheOneSentenceAndMoreReadTheCountedOne() {
        XCTAssertEqual(copy.body(count: 1), "T:body_one {count}")
        XCTAssertEqual(copy.body(count: 7), "T:body_other 7")
        XCTAssertEqual(copy.line(count: 1), "T:line_one {count}")
        XCTAssertEqual(copy.line(count: 12), "T:line_other 12")
        XCTAssertEqual(copy.allowAccessibilityName(count: 1), "T:allow_accessibility_name 1")
    }

    func testOnlyATurnOnRaisesTheOffer() {
        let scope = SentTracesTrainingState.scope
        XCTAssertEqual(scope, "model_training")
        XCTAssertTrue(SentTracesTrainingState.turnsOn(before: ["benchmark_only"], after: ["benchmark_only", scope]))
        XCTAssertFalse(SentTracesTrainingState.turnsOn(before: [scope], after: [scope]), "already on")
        XCTAssertFalse(SentTracesTrainingState.turnsOn(before: [scope], after: []), "turned off")
        XCTAssertFalse(SentTracesTrainingState.turnsOn(before: [], after: ["benchmark_only"]), "another scope")
    }

    func testNothingIsDrawnUntilTracesAreEligible() {
        var state = SentTracesTrainingState()
        XCTAssertNil(state.count)
        XCTAssertNil(state.sheet(at: .settings), "unknown raises nothing")
        state.apply(ModelTrainingConsentStatus(eligibleCount: 0, submissionIds: []))
        XCTAssertEqual(state.eligibility, .hidden)
        XCTAssertNil(state.sheet(at: .settings), "none eligible raises nothing")
        state.apply(ModelTrainingConsentStatus(eligibleCount: 3, submissionIds: ["a", "b", "c"]))
        XCTAssertEqual(state.count, 3)
        XCTAssertEqual(state.sheet(at: .settings), SentTracesTrainingSheet(site: .settings, count: 3))
        XCTAssertTrue(state.offers("a"))
        XCTAssertFalse(state.offers("z"), "only the eligible ids are offered")
        state.applyStatusRefusal()
        XCTAssertNil(state.count, "a refused read hides the count")
        XCTAssertFalse(state.offers("a"))
    }

    func testRefusalsRoute() {
        for hidden in ["model-training-not-permitted", "model-training-consent-unavailable", "not-logged-in"] {
            XCTAssertEqual(SentTracesTrainingRefusal.from(code: hidden), .hidden, hidden)
        }
        XCTAssertEqual(SentTracesTrainingRefusal.from(code: "model-training-consent-failed"), .failed)
        XCTAssertEqual(SentTracesTrainingRefusal.from(code: "anything-new"), .failed)
    }

    func testAGrantMarksPendingOnlyWhereCreditFollows() {
        var state = SentTracesTrainingState()
        state.apply(ModelTrainingConsentStatus(eligibleCount: 5, submissionIds: ["a", "b", "c", "d", "e"]))
        state.apply(ModelTrainingGrantAnswer(
            results: [
                ModelTrainingGrantResult(submissionId: "a", outcome: "granted", award: "credited"),
                ModelTrainingGrantResult(submissionId: "b", outcome: "granted", award: "pending"),
                ModelTrainingGrantResult(
                    submissionId: "c", outcome: "granted", award: "none", awardWithheldReason: "below_floor"),
                ModelTrainingGrantResult(submissionId: "d", outcome: "not_eligible", award: "none"),
            ],
            remainingEligible: 1))
        XCTAssertEqual(state.mark(for: "a", credited: false), .pending)
        XCTAssertEqual(state.mark(for: "b", credited: false), .pending)
        XCTAssertNil(state.mark(for: "c", credited: false), "no award follows: no pending mark")
        XCTAssertNil(state.mark(for: "d", credited: false))
        for answered in ["a", "b", "c", "d"] {
            XCTAssertFalse(state.offers(answered), answered)
        }
        XCTAssertTrue(state.offers("e"))
        XCTAssertEqual(state.count, 1, "the count is the server's remaining one")
    }

    func testPendingLastsUntilTheHistoryRowShowsCredit() {
        var state = SentTracesTrainingState()
        state.apply(ModelTrainingConsentStatus(eligibleCount: 1, submissionIds: ["a"]))
        state.apply(ModelTrainingGrantAnswer(
            results: [ModelTrainingGrantResult(submissionId: "a", outcome: "granted", award: "pending")],
            remainingEligible: 0))
        XCTAssertNil(state.count, "nothing left: the line hides")
        XCTAssertFalse(SentTracesTrainingState.credited(final: nil), "not scored yet")
        XCTAssertFalse(SentTracesTrainingState.credited(final: 0), "the withheld award's zero")
        XCTAssertTrue(SentTracesTrainingState.credited(final: 1.5))
        XCTAssertEqual(state.mark(for: "a", credited: false), .pending)
        XCTAssertNil(state.mark(for: "a", credited: true))
    }

    func testARefusedTraceGrantSaysFailedAndIsOfferedAgain() {
        var state = SentTracesTrainingState()
        state.apply(ModelTrainingConsentStatus(eligibleCount: 2, submissionIds: ["a", "b"]))
        state.applyGrantRefusal(.failed, for: ["a"])
        XCTAssertEqual(state.mark(for: "a", credited: false), .failed)
        XCTAssertTrue(state.offers("a"))
        XCTAssertNil(state.mark(for: "b", credited: false))
        state.applyGrantRefusal(.hidden, for: ["b"])
        XCTAssertNil(state.count, "a hiding refusal hides everything")
        XCTAssertFalse(state.offers("a"))
        XCTAssertNil(state.mark(for: "a", credited: false), "a failed mark is drawn only where Allow is")
    }
}

/// The two IPC calls: the method names and parameters the daemon reads.
final class SentTracesTrainingClientTests: XCTestCase {
    private final class Fake: DaemonTransport {
        var calls: [(String, [String: Any])] = []
        var replies: [String]
        init(_ replies: [String]) { self.replies = replies }
        func call(_ method: String, params paramsJSON: String) -> String {
            let params = try! JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as! [String: Any]
            calls.append((method, params))
            return replies.isEmpty ? #"{"id":1,"result":{}}"# : replies.removeFirst()
        }
    }

    func testStatusSendsNothingAndDecodes() throws {
        let fake = Fake([
            #"{"id":1,"result":{"eligible_count":612,"submission_ids":["00000000-0000-4000-8000-000000000001"],"future":1}}"#
        ])
        let status = try SentTracesTrainingClient(transport: fake).status()
        XCTAssertEqual(fake.calls.map(\.0), ["model_training_consent_status"])
        XCTAssertTrue(fake.calls[0].1.isEmpty)
        XCTAssertEqual(status.eligibleCount, 612)
        XCTAssertEqual(status.submissionIds, ["00000000-0000-4000-8000-000000000001"])
    }

    func testGrantAllSendsOnlyAllEligible() throws {
        let fake = Fake([
            #"{"id":1,"result":{"results":[{"submission_id":"a","outcome":"granted","granted_at":"2026-10-28T00:00:00Z","award":"pending"}],"remaining_eligible":0}}"#
        ])
        let answer = try SentTracesTrainingClient(transport: fake).grantAll()
        XCTAssertEqual(fake.calls[0].0, "model_training_consent_grant")
        XCTAssertEqual(fake.calls[0].1.keys.sorted(), ["all_eligible"])
        XCTAssertEqual(fake.calls[0].1["all_eligible"] as? Bool, true)
        XCTAssertEqual(answer.results.first?.grantedAt, "2026-10-28T00:00:00Z")
        XCTAssertEqual(answer.remainingEligible, 0)
    }

    func testGrantByIdSendsOnlyThoseIds() throws {
        let fake = Fake([
            #"{"id":1,"result":{"results":[{"submission_id":"a","outcome":"granted","award":"none","award_withheld_reason":"x"}],"remaining_eligible":4}}"#
        ])
        let answer = try SentTracesTrainingClient(transport: fake).grant(submissionIds: ["a"])
        XCTAssertEqual(fake.calls[0].0, "model_training_consent_grant")
        XCTAssertEqual(fake.calls[0].1.keys.sorted(), ["submission_ids"])
        XCTAssertEqual(fake.calls[0].1["submission_ids"] as? [String], ["a"])
        XCTAssertEqual(answer.results.first?.awardWithheldReason, "x")
        XCTAssertFalse(answer.results[0].creditFollows)
    }

    func testARefusalCarriesItsLabel() {
        let fake = Fake([#"{"id":1,"error":{"code":"unavailable","message":"model-training-not-permitted"}}"#])
        XCTAssertThrowsError(try SentTracesTrainingClient(transport: fake).status()) { error in
            XCTAssertEqual((error as? SentTracesTrainingClient.Failure)?.refusal, .hidden)
        }
        let failing = Fake([#"{"id":1,"error":{"code":"unavailable","message":"model-training-consent-failed"}}"#])
        XCTAssertThrowsError(try SentTracesTrainingClient(transport: failing).grantAll()) { error in
            XCTAssertEqual((error as? SentTracesTrainingClient.Failure)?.refusal, .failed)
        }
    }
}
