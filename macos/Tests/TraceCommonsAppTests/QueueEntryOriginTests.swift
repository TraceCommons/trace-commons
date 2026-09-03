import XCTest

@testable import TraceCommonsApp

/// The queue names what a trace came FROM, not how it is stored.
///
/// An imported Antigravity conversation is staged as a trajectory file and
/// read by the `trajectory` adapter, so the adapter name alone labelled it
/// "Letta trajectory" -- the storage format, and not the word the
/// contributor typed to collect it.
final class QueueEntryOriginTests: XCTestCase {
    /// The app's own decoder, not a fresh one: a bare `JSONDecoder` rejects
    /// the daemon's timestamps, and a test that configured its own would be
    /// asserting against a decoder the app never uses.
    private func entry(from json: String) throws -> QueueEntry {
        try DaemonDecoding.decoder().decode(QueueEntry.self, from: Data(json.utf8))
    }

    /// Decoded from JSON rather than constructed, because the field has to
    /// survive the wire: the daemon is the only thing that ever supplies it.
    private func wire(source: String, declared: String?) -> String {
        let declaredLine = declared.map { "\"declared_source\": \"\($0)\"," } ?? ""
        return """
            {
              "entry_id": "e1",
              "session_hash": "sha256:aa",
              "source": "\(source)",
              \(declaredLine)
              "project_id": "p1",
              "project_label": "demo",
              "size_bytes": 10,
              "discovered_at": "2026-09-02T00:00:00Z",
              "state": "pending",
              "attempts": 0
            }
            """
    }

    func testAnImportedConversationIsNamedByWhatItDeclares() throws {
        let e = try entry(from: wire(source: "trajectory", declared: "antigravity"))
        XCTAssertEqual(e.declaredSource, "antigravity")
        XCTAssertEqual(
            e.agentName, "Antigravity",
            "an imported conversation must not be shown as the file format it is stored in"
        )
    }

    /// A native session declares nothing and still reads correctly, which is
    /// what keeps this from being a one-source special case.
    func testASessionThatDeclaresNothingFallsBackToItsAdapter() throws {
        let e = try entry(from: wire(source: "claude-code", declared: nil))
        XCTAssertNil(e.declaredSource)
        XCTAssertEqual(e.agentName, "Claude Code")
    }

    /// An unrecognised declaration is untrusted text out of a file. It must
    /// not reach the screen; the adapter is rendered instead.
    func testAnUnknownDeclarationDoesNotReachTheScreen() throws {
        let e = try entry(from: wire(source: "trajectory", declared: "not-a-known-slug"))
        XCTAssertEqual(e.agentName, "Trajectory")
    }
}
