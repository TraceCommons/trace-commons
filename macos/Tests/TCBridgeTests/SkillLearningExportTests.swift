// INTEGRATION: verifies the contributor FFI exports complete skill-learning
// copy and maps daemon labels without exposing raw provider failures.

import Foundation
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
        XCTAssertEqual(fields["heading"], "Learn from session")
        XCTAssertTrue(fields.values.allSatisfy { !$0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty })
    }

    func testErrorLabelsStayOwnedByTheRustBoundary() throws {
        XCTAssertEqual(
            TCSkillLearning.errorLine(label: "skill-evaluation-did-not-pass"),
            "The skill must pass every applicability check and its repository plans must beat both controls without regressions before installation."
        )
        XCTAssertEqual(
            TCSkillLearning.errorLine(label: "a-later-daemon-label"),
            "The skill workflow could not complete. Retry the current step."
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
