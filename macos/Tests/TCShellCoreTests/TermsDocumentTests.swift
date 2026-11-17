import Foundation
import TCShellCore
import XCTest

/// The terms document decodes exactly the shape the core checked
/// (`trace_commons.terms.v1`), and a block kind this build cannot draw
/// fails the whole decode rather than drawing part of the terms.
final class TermsDocumentTests: XCTestCase {
    /// The protocol crate's fixture: one of each block kind.
    static let fixture = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TCShellCoreTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .deletingLastPathComponent()  // repo
        .appendingPathComponent("crates/trace-commons-protocol/tests/fixtures/terms-v1.json")

    func testDecodesEveryBlockKind() throws {
        let data = try Data(contentsOf: Self.fixture)
        let document = try XCTUnwrap(TermsDocument.decode(data))
        XCTAssertEqual(document.version, "2026-08-22-draft")
        XCTAssertEqual(document.effective, "2026-08-22")
        XCTAssertFalse(document.inEffect)
        XCTAssertEqual(document.title, "Terms and conditions")
        XCTAssertEqual(document.blocks.count, 5)
        XCTAssertEqual(document.blocks[0], .heading("1. Who we are"))
        guard case .paragraph(let text) = document.blocks[1] else {
            return XCTFail("second block is a paragraph")
        }
        XCTAssertTrue(text.contains("mailto:"))
        XCTAssertEqual(
            document.blocks[2], .list(["First item with `code`", "Second item with *emphasis*"]))
        guard case .table(let header, let rows) = document.blocks[3] else {
            return XCTFail("fourth block is a table")
        }
        XCTAssertEqual(header, ["Scope", "What it permits"])
        XCTAssertEqual(rows.count, 2)
        XCTAssertEqual(rows[0][0], "`debugging_evaluation`")
        XCTAssertEqual(document.blocks[4], .callout("These terms are a draft."))
    }

    func testUnknownBlockKindFailsDecode() throws {
        let json = #"""
        {"schema_version":"trace_commons.terms.v1","version":"v1","effective":"2026-08-22",
         "in_effect":true,"title":"T","blocks":[{"kind":"heading","text":"A"},{"kind":"image","src":"x"}]}
        """#
        XCTAssertNil(TermsDocument.decode(Data(json.utf8)))
    }

    /// The C ABI's wrapper: the document plus the hash of the bytes served.
    func testFetchedTermsDecodesTheExportShape() throws {
        let document = try String(contentsOf: Self.fixture, encoding: .utf8)
        let hash = String(repeating: "ab", count: 32)
        let json = #"{"document":"# + document + #","content_sha256":""# + hash + #""}"#
        let fetched = try XCTUnwrap(FetchedTerms.decode(json))
        XCTAssertEqual(fetched.contentSHA256, hash)
        XCTAssertEqual(fetched.document.version, "2026-08-22-draft")
    }

    /// `terms_accept`'s params: the version and hash read, and the time in
    /// RFC 3339 UTC, which the daemon parses.
    func testAcceptedTermsParams() throws {
        let at = Date(timeIntervalSince1970: 1_762_689_600)  // 2025-11-09T12:00:00Z
        let terms = AcceptedTerms(version: "v2", contentSHA256: String(repeating: "0", count: 64), acceptedAt: at)
        let params = terms.ipcParams
        XCTAssertEqual(params["version"] as? String, "v2")
        XCTAssertEqual(params["content_sha256"] as? String, String(repeating: "0", count: 64))
        XCTAssertEqual(params["accepted_at"] as? String, "2025-11-09T12:00:00Z")
        XCTAssertEqual(params.count, 3)
    }

    /// `status.terms_hold` decodes on the menu panel's status, and an older
    /// daemon's status, without it, reads as unknown.
    func testDataStatusDecodesTermsHold() throws {
        let held = try JSONDecoder().decode(
            DaemonData.Status.self,
            from: Data(#"{"terms_hold":true,"terms_version_owed":"v2","paused":false}"#.utf8))
        XCTAssertEqual(held.termsHold, true)
        XCTAssertEqual(held.termsVersionOwed, "v2")
        let old = try JSONDecoder().decode(DaemonData.Status.self, from: Data(#"{"paused":false}"#.utf8))
        XCTAssertNil(old.termsHold)
        XCTAssertNil(old.termsVersionOwed)
    }
}
