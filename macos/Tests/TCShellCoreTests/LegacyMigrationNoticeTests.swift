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

    // MARK: - The offer

    private func wire(_ json: String) throws -> LegacyMigrationWire {
        try JSONDecoder().decode(LegacyMigrationWire.self, from: Data(json.utf8))
    }

    /// `offered` is the daemon's answer, read as it came: only `true` offers
    /// the move. A missing field, a non-boolean, a non-object and a daemon
    /// too old to know about the move all offer nothing.
    func testOfferedIsReadOnlyWhenTheDaemonSaysTrue() throws {
        XCTAssertTrue(try wire(#"{"offered":true,"notice":null}"#).offered)
        XCTAssertFalse(try wire(#"{"offered":false,"notice":null}"#).offered)
        XCTAssertFalse(try wire(#"{"notice":null}"#).offered)
        XCTAssertFalse(try wire(#"{"offered":"true"}"#).offered)
        XCTAssertFalse(try wire(#"{"offered":1}"#).offered)
        XCTAssertFalse(try wire(#"[]"#).offered)
        XCTAssertFalse(LegacyMigrationWire.none.offered)
        // Offered and the notice are independent fields.
        let both = try wire(#"{"offered":true,"notice":{"folders_kept":0}}"#)
        XCTAssertTrue(both.offered)
        XCTAssertNotNil(both.noticeJSON)
    }

    private let offer = """
    {"title":"Move to your NEAR AI account","body":"b","action":"Move","working":"w",
     "invite_prompt":"p","start_failed":"s","dismiss":"Dismiss","later":"l",
     "passkey_waiting":"pw","passkey_failed":"pf","passkey_cancel":"Cancel",
     "action_name_notice":"Move, card","action_name_settings":"Move, Settings"}
    """

    func testTheOfferDecodesWhole() throws {
        let decoded = try XCTUnwrap(LegacyMigrationOffer.decode(fromJSON: offer))
        XCTAssertEqual(decoded.title, "Move to your NEAR AI account")
        XCTAssertEqual(decoded.action, "Move")
        XCTAssertEqual(decoded.invitePrompt, "p")
        XCTAssertEqual(decoded.startFailed, "s")
        XCTAssertEqual(decoded.dismiss, "Dismiss")
        XCTAssertEqual(decoded.later, "l")
        XCTAssertEqual(decoded.nearAILoginWaiting, "pw")
        XCTAssertEqual(decoded.nearAILoginFailed, "pf")
        XCTAssertEqual(decoded.nearAILoginCancel, "Cancel")
        XCTAssertEqual(decoded.actionNameNotice, "Move, card")
        XCTAssertEqual(decoded.actionNameSettings, "Move, Settings")
    }

    /// The card's Move and the one in Settings look the same and are named
    /// apart for a screen reader, each by the core's words for its place.
    func testEachMoveButtonIsNamedForItsPlace() throws {
        let decoded = try XCTUnwrap(LegacyMigrationOffer.decode(fromJSON: offer))
        XCTAssertEqual(decoded.actionName(for: .notice), "Move, card")
        XCTAssertEqual(decoded.actionName(for: .settings), "Move, Settings")
        XCTAssertNotEqual(decoded.actionName(for: .notice), decoded.actionName(for: .settings))
    }

    /// Shown whole or not at all, as the notice is. A core older than the
    /// card's dismiss gives no offer rather than a card with no way out.
    func testAnOfferMissingAWordDecodesToNothing() {
        for field in LegacyMigrationOffer.consumedFields {
            let emptied = offer.replacingOccurrences(
                of: #""\#(field)":"#, with: #""\#(field)":"","ignored":"#)
            XCTAssertNil(LegacyMigrationOffer.decode(fromJSON: emptied), field)
        }
        let older = offer.replacingOccurrences(of: #","dismiss":"Dismiss","later":"l","#, with: ",")
        XCTAssertNotEqual(older, offer)
        XCTAssertNil(LegacyMigrationOffer.decode(fromJSON: older))
        XCTAssertNil(LegacyMigrationOffer.decode(fromJSON: "null"))
    }
}
