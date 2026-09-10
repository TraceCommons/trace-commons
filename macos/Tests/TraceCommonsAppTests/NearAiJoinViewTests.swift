import XCTest

@testable import TraceCommonsApp

/// Where the join card takes its words and when it offers its control.
///
/// A SwiftUI `body` holding an `@EnvironmentObject` cannot be built outside a
/// running window, so this reads the view's own source, as
/// `WitnessBindingTests` and `RoutingBindingTests` do.
private enum NearAiJoinSource {
    static let path = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp/Views/NearAiJoinView.swift")

    static func text(file: StaticString = #filePath, line: UInt = #line) -> String? {
        guard let raw = try? String(contentsOf: path, encoding: .utf8) else {
            XCTFail("could not read \(path.path)", file: file, line: line)
            return nil
        }
        // Prose stripped: these comments name the control names on purpose,
        // and a guard that failed them would teach the next reader to delete
        // the explanation.
        return raw
            .split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")
    }
}

final class NearAiJoinViewTests: XCTestCase {
    /// Every sentence comes from the shared table; none is spelled here, and
    /// no control name is branched on.
    func testTheCardAsksTheSharedTableAndNamesNoRefusal() throws {
        let source = try XCTUnwrap(NearAiJoinSource.text())
        XCTAssertTrue(source.contains("TCNearAiEnroll.line(label:"))
        XCTAssertTrue(source.contains("TCNearAiEnroll.tone(label:"))
        for label in [
            "near_ai_enroll_no_session",
            "near_ai_enroll_commons_unreachable",
            "near_ai_enroll_commons_unsupported",
            "near_ai_enroll_already_enrolled",
        ] {
            XCTAssertFalse(
                source.contains(label),
                "\(label) is branched on in this shell rather than passed through")
        }
    }

    /// With no sign-in, the step is offered and the control is not.
    ///
    /// A button whose only outcome is `near_ai_enroll_no_session` teaches a
    /// contributor the feature is broken. The daemon deliberately reports
    /// that case rather than blaming the commons; throwing the distinction
    /// away here would undo the fix one layer up.
    func testWithoutASignInTheStepIsOfferedRatherThanAControlThatCanOnlyFail() throws {
        let source = try XCTUnwrap(NearAiJoinSource.text())
        XCTAssertTrue(
            source.contains("} else if signedIn {"),
            "the control is not gated on a sign-in being present")
        XCTAssertTrue(
            source.contains("nearAiEnrollNeedsLogin"),
            "a contributor with no sign-in is shown no step to take")
        XCTAssertTrue(
            source.contains("credentialStatus.sessionState == CredentialSurface.statePresent"),
            "the card must check the retained Cloud session separately from the inference key")
        XCTAssertTrue(source.contains("CredentialSection(copy: copy, requiresSession: true)"))
    }

    /// A success is read from the payload, not assumed from silence.
    func testAJoinIsReadFromTheAnswerRatherThanFromTheAbsenceOfARefusal() throws {
        let source = try XCTUnwrap(NearAiJoinSource.text())
        XCTAssertTrue(
            source.contains("joined = enrollment.enrolled"),
            "the card celebrates a response that never said it enrolled")
    }
}
