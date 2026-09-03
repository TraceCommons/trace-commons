import TCBridge
import TCShellCore
import XCTest

/// The routing vocabulary, checked against the real dylib.
///
/// This cannot live in `TCShellCoreTests`: that target deliberately does not
/// link the FFI, and a fixture there would only assert that this file agrees
/// with itself. The properties below are about what the Rust actually
/// exports.
final class RoutingCopyExportTests: XCTestCase {
    private func copy(file: StaticString = #filePath, line: UInt = #line) -> RoutingCopy? {
        guard let json = TCRoutingCopy.copyJSON() else {
            XCTFail("the routing copy export returned nil", file: file, line: line)
            return nil
        }
        guard let copy = RoutingCopy.decode(fromJSON: json) else {
            XCTFail("the payload did not decode: \(json)", file: file, line: line)
            return nil
        }
        return copy
    }

    /// The words this shell will print, pinned as literals.
    ///
    /// The one deliberately hand-written assertion in this file, and the
    /// reason it is here: every other check compares the payload to itself
    /// and would keep passing if all four words were renamed. This is the
    /// tripwire that a word changed at all -- change one in the Rust and this
    /// goes red, which is what proves this shell is reading the shared source
    /// rather than a copy of its own. The GTK and Windows suites make the
    /// same assertion, so a rename turns all three red together.
    func testTheSharedWordsAreTheOnesThisShellReceives() {
        guard let copy = copy() else { return }
        XCTAssertEqual(copy.wordPrivate, "Private")
        XCTAssertEqual(copy.wordDirect, "Sends direct")
        XCTAssertEqual(copy.wordUnknown, "Not known")
        XCTAssertEqual(copy.wordNotUsed, "Not used")
    }

    /// Exactly one word claims privacy, and none denies it.
    ///
    /// "Private" is a substring of "Not private", so a vocabulary carrying
    /// both is one `contains` away from showing the wrong verdict. The
    /// property is asserted here and not inherited from the Rust suite,
    /// because this is the side that would render the wrong one.
    func testOnlyTheWiredWordClaimsPrivacyAndNoneDeniesIt() {
        guard let copy = copy() else { return }
        XCTAssertTrue(copy.wordPrivate.lowercased().contains("privat"))
        for word in [copy.wordDirect, copy.wordUnknown, copy.wordNotUsed] {
            XCTAssertFalse(
                word.lowercased().contains("privat"),
                "a word that denies privacy reintroduces the substring trap: \(word)"
            )
        }
    }

    /// No word contains any other, in either direction, case-insensitively.
    func testNoWordContainsAnotherSoContainsCannotMatchTheWrongOne() {
        guard let copy = copy() else { return }
        let words = copy.words
        for (i, one) in words.enumerated() {
            for (j, other) in words.enumerated() where i != j {
                XCTAssertNotEqual(one, other)
                XCTAssertFalse(
                    one.lowercased().contains(other.lowercased()),
                    "\(other) is a substring of \(one)"
                )
            }
        }
    }

    /// Every field arrives filled. An empty one would render as a blank
    /// beside a tool name rather than as a failure anybody could see.
    func testEveryWordOnTheSurfaceArrivesNonEmpty() {
        guard let copy = copy() else { return }
        let mirror = Mirror(reflecting: copy)
        var checked = 0
        for child in mirror.children {
            guard let text = child.value as? String else { continue }
            XCTAssertFalse(text.isEmpty, "\(child.label ?? "?") arrived empty")
            checked += 1
        }
        XCTAssertEqual(checked, 23, "the payload's field count changed")
    }

    /// The sentences arrive finished. This shell never fills in a hole, so
    /// there is no format marker left in them and no wording of its own
    /// around them.
    func testTheSentencesArriveAssembledAndNotAsTemplates() {
        let named = TCRoutingCopy.tokenLine(path: "/Users/x/.ironwire/control.token")
        XCTAssertEqual(named?.contains("/Users/x/.ironwire/control.token"), true)

        let unnamed = TCRoutingCopy.tokenLine(path: nil)
        XCTAssertNotNil(unnamed)
        XCTAssertEqual(unnamed?.contains("/Users/x"), false)
        XCTAssertNotEqual(named, unnamed)

        let withPort = TCRoutingCopy.unreachableLine(port: 8463)
        XCTAssertEqual(withPort?.contains("8463"), true)
        // No port tried must not become "port 0".
        let noPort = TCRoutingCopy.unreachableLine(port: nil)
        XCTAssertEqual(noPort?.contains("0"), false)

        XCTAssertEqual(TCRoutingCopy.lastChecked(when: "an hour ago"), "Last checked an hour ago")

        for sentence in [named, unnamed, withPort, noPort] {
            guard let sentence else { return XCTFail("a sentence was nil") }
            for marker in ["{}", "%@", "%s", "%d", "{path}", "{port}"] {
                XCTAssertFalse(sentence.contains(marker), "a format marker survived: \(sentence)")
            }
        }
    }

    /// A "last checked" with no time is refused rather than rendered as
    /// "Last checked " with nothing after it.
    func testALastCheckedWithNoTimeIsRefused() {
        XCTAssertNil(TCRoutingCopy.lastChecked(when: ""))
    }
}
