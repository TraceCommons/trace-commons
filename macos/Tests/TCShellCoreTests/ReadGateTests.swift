import XCTest

@testable import TCShellCore

/// What the sheet requires, and what it says, at the moment of consent.
///
/// These are the same assertions the Linux shell holds in
/// `crates/trace-commons-contributor-gtk/src/copy.rs` and the Windows shell
/// in `windows/tests/TraceCommons.Interop.Tests/PreviewTests.cs`. Three
/// shells print one sentence about redaction above one irreversible button,
/// and the only thing that has ever held three languages to one sentence is
/// the same text asserted in each of them -- plus the Rust test that reads
/// this file to prove it.
///
/// The view that draws it cannot be tested at all: `PreviewSheet` is
/// SwiftUI in the app target, which links the FFI dylib, and a SwiftUI
/// view's enabled state has no seam a unit test can reach. That is why the
/// rule and the words live here rather than in the view.
final class ReadGateTests: XCTestCase {
    /// The statement, character for character.
    ///
    /// Written out rather than compared to itself. Changing the sentence
    /// the product asserts about redaction should take two deliberate
    /// edits, not one.
    private let statement = """
        "Exactly what would be sent" is the exact text that would leave this machine. \
        Pattern-based scrubbing may have missed something in it, and nothing here checks \
        that you looked.
        """

    func testTheConsentStatementIsExactlyWhatWasAgreed() {
        XCTAssertEqual(ReadGate.statement, statement)
    }

    func testTheStatementKeepsBothHalvesOfWhatTheCheckboxUsedToSay() {
        // The acknowledgement it replaced made a contributor assert these
        // two things by hand. Neither may quietly drop out of the sentence
        // now that nobody is being asked to tick anything.
        XCTAssertTrue(
            ReadGate.statement.contains("Pattern-based scrubbing may have missed something"))
        XCTAssertTrue(ReadGate.statement.contains("nothing here checks that you looked"))
    }

    func testAPreviewThatHasNotLoadedCannotBeContributed() {
        // The one condition that survived, and the only one that was never
        // friction: an approval covers a pinned envelope, and there is not
        // one yet.
        XCTAssertFalse(ReadGate.canContribute(hasPinnedPreview: false))
        XCTAssertEqual(ReadGate.help(hasPinnedPreview: false), ReadGate.notPinnedHelp)
    }

    func testALoadedPreviewArmsContributeWithNothingElseRequired() {
        // The change this test exists to pin down: no transcript tab, no
        // acknowledgement, no second step. Contribute is live as soon as
        // there is something to contribute.
        XCTAssertTrue(ReadGate.canContribute(hasPinnedPreview: true))
        XCTAssertEqual(ReadGate.help(hasPinnedPreview: true), ReadGate.readyHelp)
    }
}
