import XCTest
@testable import TraceCommonsApp

final class ComputeSkillsParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Zero and non-numbers are refused; the daemon refuses an allowance of
    /// nothing and a typed word is not an allowance.
    func test_theAllowanceParsesOnlyAPositiveInteger() {
        XCTAssertNil(ComputeAllowance.parse(""))
        XCTAssertNil(ComputeAllowance.parse("0"))
        XCTAssertNil(ComputeAllowance.parse("8 GiB"))
        XCTAssertEqual(ComputeAllowance.parse("8"), 8)
    }

    func test_computeKeepsEveryControlAndCopySource() throws {
        let source = try Self.text("Views/ComputeView.swift")
        for needle in ["ComputeContent(model:", "snapshot.copy.introduction", "snapshot.copy.allowanceLabel",
                       "snapshot.copy.allowanceDetail", "snapshot.copy.resume", "snapshot.copy.pause",
                       "snapshot.copy.disable", "snapshot.copy.enable", "snapshot.canEnable", "snapshot.canResume",
                       "snapshot.canPause", "snapshot.available", "snapshot.consentGranted", "model.controlsBusy",
                       "model.quitWasRefused", "copy.quitRefused", "copy.unavailable", "copy.retry", "model.failureLabel",
                       ".enable(ramAllowanceGiB:", ".perform(.resume)", ".perform(.pause)", ".perform(.disable)",
                       "model.retryOpen()", "ComputeAllowance.parse(", "GlassTextField("] {
            XCTAssertTrue(source.contains(needle), "ComputeView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
    }
}
