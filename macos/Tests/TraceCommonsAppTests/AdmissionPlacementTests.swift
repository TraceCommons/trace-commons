import XCTest

@testable import TraceCommonsApp

/// Where the admission-preparation control lives on the preview sheet.
///
/// It used to live inside the sheet's `failure` branch, which is reached by a
/// witness review that failed, a preview that could not be opened, or a
/// summary that could not be decoded. None of those is "this session has no
/// inference evidence" -- a session without evidence previews perfectly well,
/// because it is only a transcript, and the refusal arrives later. So a
/// contributor who wanted evidence had to attempt a review, have it fail, and
/// only then find the control. The GTK shell has always drawn it in the
/// footer, on every preview, and that is the placement all three now share.
///
/// A SwiftUI `body` holding `@State` and an `@EnvironmentObject` cannot be
/// built or reflected outside a running window, so these read the view's own
/// source, as `WitnessBindingTests` and `RoutingBindingTests` do.
private enum PreviewSheetSource {
    /// `.../macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift`
    static let viewPath = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TraceCommonsAppTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .appendingPathComponent("Sources/TraceCommonsApp/Views/PreviewSheet.swift")

    static func footer(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private var footer: some View {", file: file, line: line)
    }

    static func content(file: StaticString = #filePath, line: UInt = #line) -> String? {
        declaration("private var content: some View {", file: file, line: line)
    }

    /// The source between `signature` and the brace that closes it. Braces
    /// inside comments and string literals are not counted; this file carries
    /// a great deal of both.
    static func declaration(
        _ signature: String, file: StaticString = #filePath, line: UInt = #line
    ) -> String? {
        guard let text = try? String(contentsOf: viewPath, encoding: .utf8) else {
            XCTFail("could not read \(viewPath.path)", file: file, line: line)
            return nil
        }
        guard let start = text.range(of: signature) else {
            XCTFail(
                "PreviewSheet.swift no longer declares `\(signature)`", file: file, line: line)
            return nil
        }
        var depth = 1
        var index = start.upperBound
        var inString = false
        var inLineComment = false
        while index < text.endIndex {
            let character = text[index]
            let next = text.index(after: index)
            if inLineComment {
                if character == "\n" { inLineComment = false }
            } else if inString {
                if character == "\\" {
                    index = next < text.endIndex ? text.index(after: next) : text.endIndex
                    continue
                }
                if character == "\"" { inString = false }
            } else if character == "/", next < text.endIndex, text[next] == "/" {
                inLineComment = true
            } else if character == "\"" {
                inString = true
            } else if character == "{" {
                depth += 1
            } else if character == "}" {
                depth -= 1
                if depth == 0 { return String(text[start.upperBound..<index]) }
            }
            index = next
        }
        XCTFail("the body of `\(signature)` is unterminated", file: file, line: line)
        return nil
    }
}

final class AdmissionPlacementTests: XCTestCase {
    /// Drawn with the rest of the footer, so it is on screen for every
    /// preview a contributor opens -- not only the ones that broke.
    func testTheFooterDrawsTheAdmissionPreparationControl() throws {
        let footer = try XCTUnwrap(PreviewSheetSource.footer())
        XCTAssertTrue(
            footer.contains("AdmissionPreparationView"),
            "the footer no longer draws the admission-preparation control")
    }

    /// The other half, and the one that would have caught the defect: it is
    /// not reachable only by way of a preview that failed.
    func testTheFailureBranchNoLongerHidesTheAdmissionPreparationControl() throws {
        let content = try XCTUnwrap(PreviewSheetSource.content())
        XCTAssertFalse(
            content.contains("AdmissionPreparationView"),
            """
            the admission-preparation control is drawn inside `content`, whose \
            only branch that could hold it is the one a failed preview reaches. \
            A contributor who wants evidence would have to fail first to find it.
            """)
    }

    /// Moving it must not have widened who is offered it. The enrolment gate
    /// this shell reads is `admissionEvidenceOffered`; `DaemonFieldDecodingTests`
    /// holds what that answers, and this holds that the footer still asks.
    func testTheMovedControlIsStillGatedOnTheEnrolment() throws {
        let footer = try XCTUnwrap(PreviewSheetSource.footer())
        let gate = try XCTUnwrap(
            footer.range(of: "admissionEvidenceOffered"),
            "the footer draws the control without consulting the enrolment")
        let control = try XCTUnwrap(footer.range(of: "AdmissionPreparationView"))
        XCTAssertTrue(
            gate.upperBound < control.lowerBound,
            "the enrolment is consulted after the control is drawn, which is not a gate")
    }
}
