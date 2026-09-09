import XCTest

@testable import TraceCommonsApp

/// Where the certificate-held section takes its words and its membership.
///
/// A SwiftUI `body` holding an `@EnvironmentObject` cannot be built outside a
/// running window, so this reads the view's own source, as
/// `WitnessBindingTests` and `RoutingBindingTests` do.
private enum CertificateSectionSource {
    static let path = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("Sources/TraceCommonsApp/Views/CertificateSection.swift")

    static func text(file: StaticString = #filePath, line: UInt = #line) -> String? {
        guard let raw = try? String(contentsOf: path, encoding: .utf8) else {
            XCTFail("could not read \(path.path)", file: file, line: line)
            return nil
        }
        // Prose stripped: a guard that fails source for naming a sentence in
        // a comment teaches the next reader to delete the comment, and these
        // comments say why the section is drawn when empty.
        return raw
            .split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")
    }
}

final class CertificateSectionTests: XCTestCase {
    /// Every sentence comes across the ABI or from the shared payload.
    func testTheSectionAuthorsNoSentenceOfItsOwn() throws {
        let source = try XCTUnwrap(CertificateSectionSource.text())
        for authored in ["put forward", "signed proof", "witness certificate"] {
            XCTAssertFalse(
                source.contains(authored),
                "\(authored) is written in this shell rather than coming from the shared copy")
        }
        XCTAssertTrue(source.contains("TCCertificate.listTitle"))
        XCTAssertTrue(source.contains("TCCertificate.rowLine"))
        XCTAssertTrue(source.contains("certificateListEmpty"))
    }

    /// The flag reaches the ABI unnegated.
    ///
    /// `admission_evidence_required` is true for a contributor who has NO
    /// invite. A `!` on the way to `TCCertificate` would swap both readings
    /// and compile, telling that contributor their session carries
    /// cryptographic proof when nothing has attested it.
    func testTheEvidenceFlagIsPassedThroughAndNeverNegated() throws {
        let source = try XCTUnwrap(CertificateSectionSource.text())
        XCTAssertTrue(
            source.contains("model.daemonSettings?.admissionEvidenceOffered == true"),
            "the section no longer reads the shared invite status")
        XCTAssertFalse(
            source.contains("!evidenceAdmitted"),
            "the flag is negated on the way to the ABI, which swaps both readings")
        XCTAssertFalse(
            source.contains("evidenceAdmitted: !"),
            "the flag is negated at the call site, which swaps both readings")
    }

    /// Membership is the row's own answer, and the empty state is drawn.
    func testMembershipIsTheRowsAnswerAndEmptyIsSaidOutLoud() throws {
        let source = try XCTUnwrap(CertificateSectionSource.text())
        XCTAssertTrue(
            source.contains("entries.filter(\\.holdsCertificate)"),
            "the section no longer filters on the row's own answer")
        XCTAssertTrue(
            source.contains("held.isEmpty"),
            "the section does not distinguish an empty list from an absent one")
    }
}
