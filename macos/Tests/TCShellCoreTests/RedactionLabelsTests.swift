import XCTest

@testable import TCShellCore

/// `redaction_counts` carries two different kinds of fact under one map, and
/// every shell has been rendering both under the heading "Removed by
/// pattern". These tests are the boundary between them.
final class RedactionLabelsTests: XCTestCase {
    func testAFamilyIsTheLabelBeforeItsColon() {
        XCTAssertEqual(RedactionLabels.family("secret:contextual_entropy"), "secret")
        XCTAssertEqual(RedactionLabels.family("local_path"), "local_path")
        XCTAssertEqual(RedactionLabels.family("residual_secret_at:events.3.correction"), "residual_secret_at")
        XCTAssertEqual(RedactionLabels.family(""), "")
    }

    func testAnOrdinaryLabelIsARemoval() {
        XCTAssertTrue(RedactionLabels.isRemoval("local_path"))
        XCTAssertTrue(RedactionLabels.isRemoval("secret"))
        XCTAssertTrue(RedactionLabels.isRemoval("secret:pem_private_key"))
        XCTAssertTrue(RedactionLabels.isRemoval("privacy_filter:person_name"))
    }

    /// The whole point of the type. `residual_secret_at` counts a secret that
    /// was DETECTED AND LEFT IN, so counting it as removed states the exact
    /// opposite of what happened.
    func testAResidualSurvivorIsNotARemoval() {
        XCTAssertFalse(RedactionLabels.isRemoval("residual_secret_at:events.correction"))
    }

    func testRemovedTotalExcludesSurvivors() {
        let counts = [
            "local_path": 185,
            "secret": 3,
            "residual_secret_at:events.correction": 1,
        ]
        XCTAssertEqual(RedactionLabels.removedTotal(counts), 188)
        XCTAssertEqual(RedactionLabels.removals(counts).count, 2)
        XCTAssertNil(RedactionLabels.removals(counts)["residual_secret_at:events.correction"])
    }

    /// A session that removed nothing and left a secret in reports zero
    /// removals -- which is what puts the card in the tone that asks
    /// somebody to look.
    func testASessionWithOnlyASurvivorRemovedNothing() {
        let counts = ["residual_secret_at:events.x": 1]
        XCTAssertEqual(RedactionLabels.removedTotal(counts), 0)
        XCTAssertTrue(RedactionLabels.removals(counts).isEmpty)
    }

    /// Filtering a survivor out of the figure without showing it anywhere
    /// would trade a wrong statement for silence about a secret still in the
    /// payload. These are the accessors that stop that happening.
    func testSurvivorsAreReportedWithTheirSites() {
        let counts = [
            "local_path": 3,
            "residual_secret_at:events.9.correction": 2,
            "residual_secret_at:events.1.tool_result": 1,
        ]
        XCTAssertEqual(RedactionLabels.survivorTotal(counts), 3)
        XCTAssertEqual(
            RedactionLabels.survivors(counts).map(\.site),
            ["events.1.tool_result", "events.9.correction"]
        )
        XCTAssertEqual(RedactionLabels.survivors(counts).map(\.count), [1, 2])
    }

    func testASessionWithNoSurvivorsHasNoLine() {
        XCTAssertNil(RedactionLabels.survivorLine(["local_path": 3]))
        XCTAssertNil(RedactionLabels.survivorLine([:]))
        XCTAssertEqual(RedactionLabels.survivorTotal(["local_path": 3]), 0)
    }

    func testTheSurvivorLineInflects() {
        XCTAssertEqual(
            RedactionLabels.survivorLine(["residual_secret_at:events.x": 1]),
            "1 secret found here is still in what would be sent"
        )
        XCTAssertEqual(
            RedactionLabels.survivorLine([
                "residual_secret_at:events.x": 1,
                "residual_secret_at:events.y": 1,
            ]),
            "2 secrets found here are still in what would be sent"
        )
    }

    /// A bare `residual_secret_at` with no site still counts. It should never
    /// be minted, but dropping it would be the one failure direction that
    /// matters: silence about a surviving secret.
    func testASurvivorWithNoSiteIsStillCounted() {
        let counts = ["residual_secret_at": 1]
        XCTAssertEqual(RedactionLabels.survivorTotal(counts), 1)
        XCTAssertEqual(RedactionLabels.removedTotal(counts), 0)
        XCTAssertEqual(RedactionLabels.survivors(counts).map(\.site), [""])
    }

    // MARK: - The rendered line

    /// The card's "removed by pattern" figure carries two different numbers
    /// -- how many times a pattern fired, and how many distinct values that
    /// was -- and conflating them overstates or understates the reach of
    /// scrubbing depending on which one you drop.
    func testAnEmptyTallyIsNothingMatched() {
        XCTAssertEqual(RedactionLabels.line(occurrences: [:], distinct: [:]), "nothing matched")
        XCTAssertEqual(RedactionLabels.line(occurrences: [:], distinct: [:]), RedactionLabels.nothingMatched)
    }

    func testLabelsAreHumanReadable() {
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["local_path": 3], distinct: [:]),
            "3 local path"
        )
    }

    func testDistinctCountsAreShownWhenTheyDifferFromOccurrences() {
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["local_path": 185], distinct: ["local_path": 12]),
            "185 local path (12 distinct)"
        )
    }

    func testDistinctIsOmittedWhenEveryOccurrenceIsItsOwnValue() {
        // "3 secret (3 distinct)" is noise: it says the same thing twice.
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["secret": 3], distinct: ["secret": 3]),
            "3 secret"
        )
    }

    func testDistinctIsOmittedWhenTheDaemonDidNotReportIt() {
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["secret": 3], distinct: [:]),
            "3 secret"
        )
    }

    func testBiggestCountLeadsAndTiesBreakOnLabel() {
        let line = RedactionLabels.line(
            occurrences: ["secret": 3, "local_path": 185, "email": 3],
            distinct: [:]
        )
        XCTAssertEqual(line, "185 local path  ·  3 email  ·  3 secret")
    }

    func testADistinctCountAboveItsOccurrenceCountIsIgnored() {
        // Cannot happen from a correct daemon; if it ever does, saying
        // "3 secret (9 distinct)" would be worse than saying nothing.
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["secret": 3], distinct: ["secret": 9]),
            "3 secret"
        )
    }

    /// The line renders under the heading "Removed by pattern", so a
    /// survivor appearing in it states the exact opposite of what happened,
    /// on the screen where someone is deciding whether to send the thing.
    func testAResidualSurvivorIsNotInTheLine() {
        XCTAssertEqual(
            RedactionLabels.line(
                occurrences: ["local_path": 3, "residual_secret_at:events.correction": 1],
                distinct: [:]
            ),
            "3 local path"
        )
    }

    /// A session whose ONLY count is a survivor removed nothing. Saying
    /// "nothing matched" is true, and it is what puts the card in the tone
    /// that asks someone to look.
    func testASessionWithOnlyAResidualMatchedNothingInTheLine() {
        XCTAssertEqual(
            RedactionLabels.line(occurrences: ["residual_secret_at:events.x": 1], distinct: [:]),
            "nothing matched"
        )
    }
}
