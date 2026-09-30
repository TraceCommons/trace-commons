import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The notice after a legacy invite identity moved to a NEAR AI account is
/// worded across the ABI once per notice the daemon sends, not on every
/// SwiftUI re-render: the model holds the decoded notice and the card only
/// lays it out.
final class LegacyMigrationNoticeModelTests: XCTestCase {
    private func status(_ migration: String?, paused: Bool = false) throws -> DaemonStatus {
        let field = migration.map { #","legacy_invite_migration":\#($0)"# } ?? ""
        let json = """
            {"schema_version":"v","logged_in":true,"paused":\(paused),"queue_depth":0,
             "health":{"last_error_label":null,"since":null}\(field)}
            """
        return try JSONDecoder().decode(DaemonStatus.self, from: Data(json.utf8))
    }

    @MainActor
    func testTheNoticeIsWordedOncePerNoticeNotPerRead() throws {
        var calls = 0
        let model = AppModel()
        model.legacyMigrationWording = { _ in
            calls += 1
            return #"{"title":"T","body":"B","folders":"F","acknowledge":"K"}"#
        }
        XCTAssertNil(model.legacyMigrationNotice)

        let notice = #"{"offered":false,"notice":{"folders_kept":1}}"#
        model.setStatusForTesting(try status(notice))
        XCTAssertEqual(model.legacyMigrationNotice?.title, "T")
        // Reading it again, as each re-render does, words nothing.
        _ = model.legacyMigrationNotice
        _ = model.legacyMigrationNotice
        XCTAssertEqual(calls, 1)

        // A status that differs elsewhere but carries the same notice does
        // not re-word it either.
        model.setStatusForTesting(try status(notice, paused: true))
        XCTAssertTrue(model.status.paused)
        XCTAssertEqual(calls, 1)

        // The daemon clearing it clears the card.
        model.setStatusForTesting(try status(#"{"offered":false,"notice":null}"#))
        XCTAssertNil(model.legacyMigrationNotice)
        XCTAssertEqual(calls, 1)
    }

    /// Through the real ABI, so the default wording is the Rust's.
    @MainActor
    func testTheDefaultWordingIsTheAbis() throws {
        let model = AppModel()
        model.setStatusForTesting(
            try status(#"{"offered":false,"notice":{"folders_kept":1,"automatic_grant_kept":false}}"#))
        let notice = try XCTUnwrap(model.legacyMigrationNotice)
        XCTAssertTrue(notice.title.contains("NEAR AI"))
        XCTAssertFalse(notice.folders.isEmpty)
    }
}
