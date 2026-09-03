import XCTest

@testable import TCShellCore

/// The redactor leaves a typed placeholder where it removed a value, and
/// those tokens are already in the body the ABI hands us -- rendered, until
/// now, as ordinary transcript text. Finding them is what lets the preview
/// say WHERE something was cut, which is more than a category count can.
final class RedactionPlaceholdersTests: XCTestCase {
    func testABodyWithNoPlaceholdersScansToNothing() {
        XCTAssertTrue(RedactionPlaceholders.scan("just some ordinary text").isEmpty)
        XCTAssertTrue(RedactionPlaceholders.scan("").isEmpty)
    }

    func testASinglePlaceholderIsFound() {
        let body = "ran the build in <PRIVATE_LOCAL_PATH_1> and stopped"
        let found = RedactionPlaceholders.scan(body)
        XCTAssertEqual(found.count, 1)
        XCTAssertEqual(found[0].label, "LOCAL_PATH")
        XCTAssertEqual(found[0].ordinal, 1)
        XCTAssertEqual(String(body[found[0].range]), "<PRIVATE_LOCAL_PATH_1>")
    }

    func testTheDisplayNameIsHumanReadable() {
        let found = RedactionPlaceholders.scan("<PRIVATE_CONTEXTUAL_ENTROPY_2>")
        XCTAssertEqual(found[0].display, "contextual entropy")
    }

    func testMultiplePlaceholdersAreFoundInOrder() {
        let body = "<PRIVATE_SECRET_1> then <PRIVATE_LOCAL_PATH_3> then <PRIVATE_SECRET_1>"
        let found = RedactionPlaceholders.scan(body)
        XCTAssertEqual(found.map(\.label), ["SECRET", "LOCAL_PATH", "SECRET"])
        XCTAssertEqual(found.map(\.ordinal), [1, 3, 1])
    }

    func testALabelContainingDigitsIsParsedCorrectly() {
        // The ordinal is the LAST underscore-delimited run of digits, so a
        // label that itself ends in a number must not steal it.
        let found = RedactionPlaceholders.scan("<PRIVATE_SHA256_KEY_7>")
        XCTAssertEqual(found[0].label, "SHA256_KEY")
        XCTAssertEqual(found[0].ordinal, 7)
    }

    func testTextThatMerelyLooksLikeAPlaceholderIsIgnored() {
        XCTAssertTrue(RedactionPlaceholders.scan("<PRIVATE>").isEmpty)
        XCTAssertTrue(RedactionPlaceholders.scan("<PRIVATE_LOCAL_PATH_>").isEmpty)
        XCTAssertTrue(RedactionPlaceholders.scan("<private_local_path_1>").isEmpty)
        XCTAssertTrue(RedactionPlaceholders.scan("PRIVATE_LOCAL_PATH_1").isEmpty)
    }

    func testScanningCarriesNoMatchedContent() {
        // The placeholder IS the content here -- the original value is gone
        // by construction. This asserts the type exposes nothing else.
        let found = RedactionPlaceholders.scan("<PRIVATE_SECRET_1>")
        XCTAssertEqual(found[0].display, "secret")
        XCTAssertEqual(found[0].ordinal, 1)
    }
}
