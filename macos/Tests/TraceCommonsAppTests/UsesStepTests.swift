import XCTest
@testable import TraceCommonsApp

final class UsesStepTests: XCTestCase {
    /// Nothing optional starts ticked: the only seed of `selected` is the
    /// caller's `initialSelection`, no other default or assignment fills it,
    /// and a fresh run in the coordinator passes an empty one.
    func test_nothingOptionalStartsTicked() throws {
        let source = try OnboardingParityTests.text("Views/ConsentScopesView.swift")
        XCTAssertEqual(source.components(separatedBy: "_selected = State(initialValue: initialSelection)").count - 1, 1)
        XCTAssertEqual(source.components(separatedBy: "@State private var selected: Set<String>\n").count - 1, 1)
        XCTAssertEqual(source.components(separatedBy: " selected = ").count - 1, 0)
        XCTAssertEqual(source.components(separatedBy: "selected.insert(").count - 1, 1)
        XCTAssertEqual(source.components(separatedBy: "var initialSelection: Set<String> = []\n").count - 1, 1)
        let coordinator = try OnboardingParityTests.text("Views/OnboardingCoordinatorView.swift")
        XCTAssertTrue(coordinator.contains("@State private var selectedScopes: Set<String> = []\n"))
        XCTAssertTrue(coordinator.contains("ConsentScopesView(onContinue: advanceFromConsent, initialSelection: selectedScopes)"))
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
        XCTAssertTrue(source.contains("ScrollView {\n            ConsentScopesContent("))
    }
}
