import XCTest

@testable import TCShellCore

/// The certificate-held list's two readings, on this shell.
///
/// The list is driven by the queue entry's `holds_certificate`, which is
/// true after either witness route. Which sentence a row gets is decided by
/// whether the contributor holds an invite -- the same status this shell
/// already reads for the eligibility surface -- and both sentences come
/// across the ABI. Neither is authored here.
///
/// The wording is pinned in the Rust; what is pinned here is that this shell
/// receives both, that they are different sentences, and that the reading it
/// picks follows the invite status rather than the attestation mark.
final class CertificateListCopyTests: XCTestCase {
    private func decoded() throws -> PrivateInferenceCopy {
        try JSONDecoder().decode(
            PrivateInferenceCopy.self, from: Data(PrivateInferenceCopyFixture.complete.utf8))
    }

    func testBothReadingsArriveAndDiffer() throws {
        let copy = try decoded()
        XCTAssertFalse(copy.certificateRowCandidate.isEmpty)
        XCTAssertFalse(copy.certificateRowAttested.isEmpty)
        XCTAssertFalse(copy.certificateListCandidate.isEmpty)
        XCTAssertFalse(copy.certificateListAttested.isEmpty)
        XCTAssertFalse(copy.certificateListEmpty.isEmpty)
        XCTAssertNotEqual(copy.certificateRowCandidate, copy.certificateRowAttested)
        XCTAssertNotEqual(copy.certificateListCandidate, copy.certificateListAttested)
    }
}
