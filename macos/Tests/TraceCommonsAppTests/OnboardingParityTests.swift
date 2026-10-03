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

    static let steps: [Step] = [
        Step(file: "Views/OnboardingWelcomeView.swift",
             bindings: ["var onGetStarted: () -> Void", "var onWhatGetsRemoved: () -> Void",
                        "Button(OnboardingWelcomeWords.getStarted, action: onGetStarted)",
                        "Button(OnboardingWelcomeWords.whatGetsRemoved, action: onWhatGetsRemoved)"],
             copySources: ["TCOnboardingCopy.load()?.welcomeBody",
                           "Text(OnboardingWelcomeWords.headline)",
                           "GlassTag(OnboardingWelcomeWords.promiseLine1, tone: .accent)",
                           "GlassTag(OnboardingWelcomeWords.promiseLine2, tone: .accent)",
                           "Text(OnboardingWelcomeWords.lede)",
                           "Text(OnboardingWelcomeWords.scrubbing)",
                           "Text(OnboardingWelcomeWords.footer)"],
             guards: [".buttonStyle(GlassButtonStyle(.primary))", ".buttonStyle(GlassButtonStyle(.link))",
                      ".keyboardShortcut(.defaultAction)"]),
    ]

    /// Rows the table must hold; each task that adds a step raises it, so a
    /// dropped row fails here instead of passing silently.
    static let minimumSteps = 1

    func test_theTableKeepsEveryRowAdded() {
        XCTAssertGreaterThanOrEqual(Self.steps.count, Self.minimumSteps)
    }

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
