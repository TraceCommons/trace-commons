import TCShellCore
import XCTest

/// The notice after a legacy invite identity moved to a NEAR AI account:
/// `status.legacy_invite_migration.notice` off the wire, and the words the
/// Rust assembles for it.
final class LegacyMigrationNoticeTests: XCTestCase {
    /// The notice object goes back to the core exactly as it came, so the
    /// core -- not this shell -- reads `folders_kept` to choose the words.
    func testTheNoticeObjectRoundTripsToTheCore() throws {
        let wire = try JSONDecoder().decode(
            LegacyMigrationWire.self,
            from: Data(#"{"offered":false,"notice":{"folders_kept":2,"automatic_grant_kept":false}}"#.utf8))
        let json = try XCTUnwrap(wire.noticeJSON)
        let passed = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? NSDictionary
        XCTAssertEqual(passed, ["folders_kept": 2, "automatic_grant_kept": false] as NSDictionary)
    }

    /// No notice, and a daemon too old to know about the move, both show
    /// nothing.
    func testNoNoticeShowsNothing() throws {
        let none = try JSONDecoder().decode(
            LegacyMigrationWire.self, from: Data(#"{"offered":true,"notice":null}"#.utf8))
        XCTAssertNil(none.noticeJSON)
        XCTAssertNil(LegacyMigrationWire.none.noticeJSON)
    }

    private let notice = """
    {"title":"Your contributions now go under your NEAR AI account","body":"b",
     "folders":"f","acknowledge":"Got it"}
    """

    func testTheNoticeDecodesWhole() throws {
        let decoded = try XCTUnwrap(LegacyMigrationNotice.decode(fromJSON: notice))
        XCTAssertEqual(decoded.title, "Your contributions now go under your NEAR AI account")
        XCTAssertEqual(decoded.folders, "f")
        XCTAssertEqual(decoded.acknowledge, "Got it")
    }

    /// Shown whole or not at all: no sentence is written in this shell.
    func testAMissingOrEmptySentenceDecodesToNothing() {
        for field in LegacyMigrationNotice.consumedFields {
            let emptied = notice.replacingOccurrences(
                of: #""\#(field)":"#, with: #""\#(field)":"","ignored":"#)
            XCTAssertNil(LegacyMigrationNotice.decode(fromJSON: emptied), field)
        }
        XCTAssertNil(LegacyMigrationNotice.decode(fromJSON: "null"))
        XCTAssertNil(LegacyMigrationNotice.decode(fromJSON: "not json"))
    }
}
