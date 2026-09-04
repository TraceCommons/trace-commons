import XCTest

@testable import TraceCommonsApp

/// The daemon gained a project path, a session path, distinct redaction
/// counts, and a project id on history records. Every one of them must be
/// optional in practice: this app is shipped separately from the daemon and
/// routinely runs against an older one.
final class DaemonFieldDecodingTests: XCTestCase {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(type, from: Data(json.utf8))
    }

    func testQueueEntryDecodesProjectAndSessionPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":"~/code/repo/crates/inner",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "~/code/repo")
        XCTAssertEqual(entry.sessionPath, "~/code/repo/crates/inner")
    }

    func testQueueEntryFromAnOlderDaemonHasNoPaths() throws {
        let entry = try decode(QueueEntry.self, """
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """)
        XCTAssertEqual(entry.projectPath, "")
        XCTAssertNil(entry.sessionPath)
    }

    func testPreviewSummaryDecodesDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "redactions_distinct":{"local_path":12},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertEqual(summary.redactions["local_path"], 185)
        XCTAssertEqual(summary.redactionsDistinct["local_path"], 12)
    }

    func testPreviewSummaryFromAnOlderDaemonHasNoDistinctCounts() throws {
        let summary = try decode(PreviewSummary.self, """
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """)
        XCTAssertTrue(summary.redactionsDistinct.isEmpty)
    }

    func testHistoryRecordDecodesProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z","project_id":"proj_abc",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "proj_abc")
    }

    func testHistoryRecordFromBeforeTheUpgradeHasNoProjectID() throws {
        let record = try decode(HistoryRecord.self, """
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """)
        XCTAssertEqual(record.projectID, "")
    }
}
