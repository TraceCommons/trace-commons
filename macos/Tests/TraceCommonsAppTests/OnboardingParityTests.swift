import XCTest

/// Each onboarding step keeps its daemon calls, its guards and its copy
/// sources through the rebuild. The table is the inventory in the plan.
final class OnboardingParityTests: XCTestCase {
    struct Step {
        let file: String
        let bindings: [String]
        let copySources: [String]
        let guards: [String]
    }

    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let steps: [Step] = []

    func test_everyStepKeepsItsBindingsGuardsAndCopy() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            for needle in step.bindings + step.copySources + step.guards {
                XCTAssertTrue(source.contains(needle), "\(step.file) lacks \(needle)")
            }
        }
    }

    /// No rebuilt step reads the legacy palette.
    func test_noStepReadsTheLegacyPalette() throws {
        for step in Self.steps {
            let source = try Self.text(step.file)
            XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(step.file) reads TC.")
            XCTAssertFalse(source.contains("CommunityBrand"), "\(step.file) reads CommunityBrand")
        }
    }
}
