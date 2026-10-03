import XCTest
@testable import TraceCommonsApp

final class UsesStepTests: XCTestCase {
    func test_nothingOptionalStartsTicked() {
        XCTAssertTrue(UsesStep.startsUnticked([]))
        XCTAssertFalse(UsesStep.startsUnticked(["benchmark_only"]))
    }

    /// The count includes the always-on permission the upload carries.
    func test_theContinueCountIncludesAlwaysOn() {
        XCTAssertEqual(UsesStep.continueLabel(alwaysOn: 1, selected: 0), ConsentScopesWords.continueWith(1))
        XCTAssertEqual(UsesStep.continueLabel(alwaysOn: 1, selected: 2), ConsentScopesWords.continueWith(3))
    }

    /// Always-on rows are locked and on; they are never a tappable control.
    func test_alwaysOnRowsAreLockedOn() throws {
        let source = try OnboardingParityTests.text("Views/ConsentScopesView.swift")
        XCTAssertTrue(source.contains("isOn: .constant(true)"))
        XCTAssertTrue(source.contains(".disabled(scope.alwaysOn)"))
    }

    /// Until the daemon has listed the scopes nothing reads as working:
    /// Continue is disabled, and a wait indicator stands in for the rows.
    func test_continueIsDisabledUntilTheScopesAreListed() throws {
        let source = try OnboardingParityTests.text("Views/ConsentScopesView.swift")
        XCTAssertTrue(source.contains(".disabled(model.consentScopes.isEmpty)"))
        XCTAssertTrue(source.contains("SettingsAwaiting()"))
    }

    /// The content struct holds no ScrollView; the wrapper scrolls once.
    func test_theStepScrollsOnce() throws {
        let source = try OnboardingParityTests.text("Views/ConsentScopesView.swift")
        XCTAssertEqual(source.components(separatedBy: "ScrollView {").count - 1, 1)
    }
}
