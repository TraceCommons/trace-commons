import XCTest

@testable import TCShellCore

/// Naming the chips the transcript already draws.
///
/// The marking itself is not new -- `TranscriptMarkerScan` has found these
/// spans and `TranscriptMarkers` has chipped them for some time. What was
/// missing is that every chip drew as the same anonymous block while the
/// tokens carry a label, an ordinal, or a category. These tests are the
/// naming, and the limits on it: a name is never invented, and an ordinal
/// is never invented.
final class RedactionMarksTests: XCTestCase {
    func testABodyWithNoMarkersHasNothingToName() {
        XCTAssertTrue(RedactionMarks.scan("just some ordinary text").isEmpty)
        XCTAssertTrue(RedactionMarks.scan("").isEmpty)
    }

    /// The numbered form is the only one that carries both a label and an
    /// ordinal. `apply_placeholder_regex` mints it for exactly two labels.
    func testANumberedPlaceholderIsNamedAndNumbered() {
        let found = RedactionMarks.scan("ran it in <PRIVATE_LOCAL_PATH_1> today")
        XCTAssertEqual(found.count, 1)
        XCTAssertEqual(found[0].category, "local path")
        XCTAssertEqual(found[0].ordinal, 1)
        XCTAssertEqual(found[0].name, "local path removed")
    }

    /// `[REDACTED:{label}]` is the only fixed token that names its own
    /// category, so it is the only fixed token a chip can name.
    func testALabelledFixedTokenIsNamedFromItsLabel() {
        let found = RedactionMarks.scan("wrote to [REDACTED:person_name] about it")
        XCTAssertEqual(found.count, 1)
        XCTAssertEqual(found[0].category, "person name")
        XCTAssertNil(found[0].ordinal, "a fixed token carries no index")
        XCTAssertEqual(found[0].name, "person name removed")
    }

    /// A bare `[REDACTED]` says that something left and not what. The name
    /// must say exactly that much and no more.
    func testABareFixedTokenSaysOnlyThatSomethingLeft() {
        let found = RedactionMarks.scan("Authorization: [REDACTED]")
        XCTAssertEqual(found.count, 1)
        XCTAssertNil(found[0].category, "a category must never be invented")
        XCTAssertNil(found[0].ordinal)
        XCTAssertEqual(found[0].name, "something removed")
    }

    /// The redactor mints one token per distinct VALUE and reuses it, so the
    /// same ordinal twice means the same original string twice. A reader
    /// cannot see that without scrolling back, which is exactly the kind of
    /// thing a name can carry.
    func testARepeatedPlaceholderSaysItIsTheSameValue() {
        let found = RedactionMarks.scan("<PRIVATE_LOCAL_PATH_1> then <PRIVATE_LOCAL_PATH_1>")
        XCTAssertEqual(found.count, 2)
        XCTAssertFalse(found[0].isRepeat)
        XCTAssertTrue(found[1].isRepeat)
        XCTAssertEqual(found[1].name, "local path removed, the same value as an earlier mark")
    }

    /// Two different ordinals are two different values, however alike they
    /// look on screen.
    func testTwoOrdinalsOfOneLabelAreNotRepeats() {
        let found = RedactionMarks.scan("<PRIVATE_LOCAL_PATH_1> then <PRIVATE_LOCAL_PATH_2>")
        XCTAssertEqual(found.map(\.isRepeat), [false, false])
    }

    /// The one that would be a lie. Two `[REDACTED]` tokens are two
    /// redactions that say nothing about each other -- the fixed form
    /// carries no index precisely because it does not track values -- so
    /// calling the second one a repeat would claim a fact nobody has.
    func testTwoBareFixedTokensAreNeverCalledTheSameValue() {
        let found = RedactionMarks.scan("[REDACTED] and later [REDACTED]")
        XCTAssertEqual(found.map(\.isRepeat), [false, false])
        XCTAssertEqual(found[1].name, "something removed")
    }

    func testTwoIdenticalLabelledTokensAreNotCalledTheSameValueEither() {
        let found = RedactionMarks.scan("[REDACTED:person_name] and [REDACTED:person_name]")
        XCTAssertEqual(found.map(\.isRepeat), [false, false])
    }

    /// Every form in one body, in document order, from the one scan the
    /// chipper already walks -- so a name can never land on a span the chip
    /// did not cover, or miss one it did.
    func testEveryFormIsFoundInDocumentOrder() {
        let body = "<PRIVATE_LOCAL_PATH_1> [REDACTED] [REDACTED:person_name] <PRIVATE_LOCAL_PATH_1>"
        let found = RedactionMarks.scan(body)
        XCTAssertEqual(found.count, 4)
        XCTAssertEqual(
            found.map(\.name),
            [
                "local path removed",
                "something removed",
                "person name removed",
                "local path removed, the same value as an earlier mark",
            ]
        )
        XCTAssertEqual(
            found.map { String(body[$0.range]) },
            [
                "<PRIVATE_LOCAL_PATH_1>",
                "[REDACTED]",
                "[REDACTED:person_name]",
                "<PRIVATE_LOCAL_PATH_1>",
            ]
        )
    }

    /// The scan is `TranscriptMarkerScan`'s, not a second one. This pins
    /// that: naming must cover exactly the spans that get chipped.
    func testTheSpansAreTheOnesTheChipperAlreadyDraws() {
        let body = "a <PRIVATE_LOCAL_PATH_1> b [REDACTED:x] c [REDACTED] d"
        XCTAssertEqual(
            RedactionMarks.scan(body).map(\.range),
            TranscriptMarkerScan.spans(in: body)
        )
    }

    /// What a chunk reads as aloud: the tokens are gibberish spoken
    /// character by character, and this is the whole point of naming them.
    func testSpokenTextReplacesEachTokenWithItsName() {
        XCTAssertEqual(
            RedactionMarks.spoken("ran it in <PRIVATE_LOCAL_PATH_1> and stopped"),
            "ran it in local path removed and stopped"
        )
        XCTAssertEqual(RedactionMarks.spoken("plain text"), "plain text")
    }

    /// `<REDACTED_PRIVATE_KEY>` is a fourth form the redactor emits for PEM
    /// keys. It is classified here so the naming is complete, but it does
    /// NOT reach this type today: `TranscriptMarkerScan`'s pattern matches
    /// `<PRIVATE_...>` and `[REDACTED...]` and this token is neither, so no
    /// shell chips it. Widening that pattern is a three-shell change, not a
    /// macOS one. See the report on this branch.
    func testAPrivateKeyTokenIsClassifiedEvenThoughNoShellChipsItYet() {
        XCTAssertTrue(
            RedactionMarks.scan("<REDACTED_PRIVATE_KEY>").isEmpty,
            "the shared scan does not match this token, so nothing is chipped"
        )
        XCTAssertEqual(RedactionMarks.classify("<REDACTED_PRIVATE_KEY>").category, "private key")
        XCTAssertNil(RedactionMarks.classify("<REDACTED_PRIVATE_KEY>").ordinal)
    }
}
