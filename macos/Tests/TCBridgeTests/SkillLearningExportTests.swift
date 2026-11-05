// INTEGRATION: verifies the contributor FFI exports complete skill-learning
// copy and maps daemon labels without exposing raw provider failures.

import Foundation
import TCLocalization
import XCTest

@testable import TCBridge

final class SkillLearningExportTests: XCTestCase {
    func testCompleteCopyPayloadCrossesAsOneObject() throws {
        let json = try XCTUnwrap(TCSkillLearning.copyJSON())
        let data = try XCTUnwrap(json.data(using: .utf8))
        let fields = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: String]
        )

        XCTAssertGreaterThanOrEqual(fields.count, 50)
        XCTAssertEqual(fields["heading"], "Learn from trace")
        XCTAssertTrue(fields.values.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
        XCTAssertEqual(fields["contract_summary_format"], "%1$d tasks across %2$d debugging cases. %3$d requests compare the baseline, a simple instruction, and this candidate. Each request allows %4$d output tokens and %5$d seconds.")
        XCTAssertEqual(fields["review_budget_format"], "%1$d model requests; %2$d output tokens per request; model owner must be %3$@.")
        XCTAssertEqual(fields["model_budget_format"], "Owner %1$@ · %2$d tasks · %3$d output tokens per run")
    }

    func testNamedBudgetPresentationsKeepSentenceStructureAndNativeNumbers() throws {
        let tokens = Localization.current.number(12345)
        XCTAssertEqual(
            try XCTUnwrap(TCSkillLearning.contractSummaryLine(tasks: 3, cases: 4, requests: 9, outputTokens: 12345, seconds: 60)),
            "3 tasks across 4 debugging cases. 9 requests compare the baseline, a simple instruction, and this candidate. Each request allows \(tokens) output tokens and 60 seconds."
        )
        let owner = "owner %1$@ {tasks} {owner}\n\"私有\""
        XCTAssertEqual(
            try XCTUnwrap(TCSkillLearning.reviewBudgetLine(requests: 9, outputTokens: 12345, owner: owner)),
            "9 model requests; \(tokens) output tokens per request; model owner must be \(owner)."
        )
        XCTAssertEqual(
            try XCTUnwrap(TCSkillLearning.modelBudgetLine(owner: owner, tasks: 3, outputTokens: 12345)),
            "Owner \(owner) · 3 tasks · \(tokens) output tokens per run"
        )
    }

    func testNamedBudgetsPreserveSignedIntegerRangeAndEmptyOwner() throws {
        XCTAssertEqual(
            try XCTUnwrap(TCSkillLearning.modelBudgetLine(owner: "", tasks: Int.max, outputTokens: -1)),
            "Owner  · \(Localization.current.number(Int64.max)) tasks · \(Localization.current.number(-1)) output tokens per run"
        )
        XCTAssertNil(LocalizedPresentation.line("skill_learning_contract_summary_line", parameters: ["tasks": 1]))
        XCTAssertNil(LocalizedPresentation.line("skill_learning_model_budget_line", parameters: [
            "owner": "owner", "tasks": "1", "output_tokens": 2,
        ]))
    }

    func testErrorLabelsStayOwnedByTheRustBoundary() throws {
        XCTAssertEqual(
            TCSkillLearning.errorLine(label: "skill-evaluation-did-not-pass"),
            "The skill must pass every applicability check and its repository plans must beat both controls without regressions before installation."
        )
        XCTAssertEqual(
            TCSkillLearning.errorLine(label: "a-later-daemon-label"),
            "Skill workflow could not complete. Retry this step."
        )
    }

    func testDraftValidationUsesRustRulesAndDoesNotEchoContent() throws {
        let input = #"{"name":"Uppercase-Name","description":"Use for generated files.","procedure":"Edit the source."}"#
        let json = try XCTUnwrap(TCSkillLearning.validateDraftJSON(input))
        let data = try XCTUnwrap(json.data(using: .utf8))
        let validation = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )

        XCTAssertEqual(validation["valid"] as? Bool, false)
        XCTAssertEqual(validation["error"] as? String, "skill-name-invalid")
        XCTAssertEqual(validation["name_max_chars"] as? Int, 64)
        XCTAssertNil(validation["name"])
        XCTAssertFalse(json.contains("generated files"))
    }
}
