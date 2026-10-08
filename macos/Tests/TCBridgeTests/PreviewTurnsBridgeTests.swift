import TCBridge
import TCShellCore
import XCTest

/// `TCPreviewTurns` reaches `tc_preview_turns_json` on a real daemon. A
/// queued session is not needed to prove the call lands: a bad entry id is
/// the export's own refusal, and an attached handle is refused because the
/// redacted body is an in-process exemption.
final class PreviewTurnsBridgeTests: XCTestCase {
    private let settings = #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#

    private func directory() throws -> URL {
        // Short enough for the daemon's Unix socket path on macOS.
        let directory = URL(fileURLWithPath: "/private/tmp/tc-turns-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    func testABadEntryIsTheExportsRefusal() throws {
        let daemon = try TCDaemon(configDir: try directory().path, settingsJSON: settings)
        defer { daemon.shutdown() }
        XCTAssertEqual(
            daemon.previewTurns(entryID: "not-a-uuid", bodyDigest: "sha256:00"),
            .failure(TCPreviewTurns.Refusal(label: "entry-id-invalid")))
        XCTAssertNil(TCPreviewTurns.turnsJSON(daemon: daemon, entryID: "not-a-uuid", bodyDigest: "sha256:00"))
    }

    func testAnUnknownEntryIsNeverAnIndex() throws {
        let daemon = try TCDaemon(configDir: try directory().path, settingsJSON: settings)
        defer { daemon.shutdown() }
        let unknown = UUID().uuidString.lowercased()
        guard case .failure(let refusal) = daemon.previewTurns(entryID: unknown, bodyDigest: "sha256:00") else {
            return XCTFail("an unknown entry was indexed")
        }
        XCTAssertFalse(refusal.label.isEmpty)
        XCTAssertNil(TCPreviewTurns.turnsJSON(daemon: daemon, entryID: unknown, bodyDigest: "sha256:00"))
    }

    func testAStoppedDaemonIsNoIndex() throws {
        let daemon = try TCDaemon(configDir: try directory().path, settingsJSON: settings)
        _ = daemon.shutdown()
        XCTAssertNil(TCPreviewTurns.turnsJSON(daemon: daemon, entryID: UUID().uuidString, bodyDigest: "sha256:00"))
    }
}
