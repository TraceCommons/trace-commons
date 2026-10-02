import TCBridge
import TCShellCore
import XCTest

/// K1.1 (#1173): the copy tables that used to be written in this shell, read
/// through the real dylib and decoded the way the views decode them.
///
/// These are the assertions the Swift-authored copy used to carry, moved
/// here so they hold the core's words instead: the ignore and arming
/// confirmations, the scrubbing panel, the surviving-secret line, the extra
/// privacy scan, the quit prompt and the unknown-reach withdrawal prompt.
final class CoreCopyExportTests: XCTestCase {
    // MARK: - Ignore project

    private func ignoreCopy(_ pending: Int) throws -> ProjectIgnoreCopy {
        try XCTUnwrap(ProjectIgnoreCopy.decode(
            fromJSON: TCCoreCopy.projectIgnoreCopyJSON(project: "api", pending: pending)))
    }

    func testTheIgnoreExportCarriesExactlyTheFieldsThisShellDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.projectIgnoreCopyJSON(project: "api", pending: 2))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), ProjectIgnoreCopy.consumedFields.sorted())
    }

    func testTheIgnoreTitleNamesTheProject() throws {
        XCTAssertEqual(try ignoreCopy(1).title, "Ignore api?")
    }

    func testTheIgnoreBodyAgreesInNumber() throws {
        let one = try ignoreCopy(1).body
        XCTAssertTrue(one.contains("1 waiting trace "), one)
        XCTAssertFalse(one.contains("traces"), one)
        let many = try ignoreCopy(12).body
        XCTAssertTrue(many.contains("12 waiting traces"), many)
    }

    /// "Removes 0 waiting traces" would be both wrong and alarming.
    func testNothingWaitingDropsTheRemovalClause() throws {
        let body = try ignoreCopy(0).body
        XCTAssertFalse(body.contains("0"), body)
        XCTAssertFalse(body.lowercased().contains("removes"), body)
        XCTAssertTrue(body.contains("Stops this project being offered."), body)
    }

    func testTheIgnoreBodyAlwaysNamesTheWayBack() throws {
        for n in [0, 1, 7] {
            let body = try ignoreCopy(n).body
            XCTAssertTrue(body.contains("undo this in Settings"), "n=\(n): \(body)")
            XCTAssertTrue(body.contains("Nothing already submitted is affected."), "n=\(n)")
        }
    }

    func testTheReconciliationSpeaksOnlyWhenTheCountMoved() {
        XCTAssertNil(TCCoreCopy.projectIgnoreReconciled(project: "api", promised: 3, purged: 3))
        XCTAssertNil(TCCoreCopy.projectIgnoreReconciled(project: "api", promised: 0, purged: 0))
        XCTAssertEqual(
            TCCoreCopy.projectIgnoreReconciled(project: "api", promised: 3, purged: 5),
            "Ignored api. The queue changed while you were deciding: "
                + "5 waiting traces were removed, not 3."
        )
        XCTAssertEqual(
            TCCoreCopy.projectIgnoreReconciled(project: "api", promised: 3, purged: 1),
            "Ignored api. The queue changed while you were deciding: "
                + "1 waiting trace was removed, not 3."
        )
    }

    // MARK: - Arming

    private func armingCopy(_ count: Int) throws -> ProjectArmingCopy {
        try XCTUnwrap(ProjectArmingCopy.decode(
            fromJSON: TCCoreCopy.armingOfferCopyJSON(project: "api", count: count)))
    }

    /// Evidence first, naming the project and the count, singular for one.
    func testTheEvidenceNamesTheProjectAndTheCount() throws {
        XCTAssertEqual(try armingCopy(5).evidence, "You've contributed from api 5 times.")
        XCTAssertEqual(try armingCopy(1).evidence, "You've contributed from api once.")
    }

    func testTheQuestionNamesTheProject() throws {
        XCTAssertEqual(try armingCopy(5).question, "Contribute from api automatically?")
    }

    /// The buttons carry their actions, and declining is not permanent: the
    /// daemon silences the offer for thirty days, not forever.
    func testTheArmingButtonsCarryTheirActions() throws {
        let copy = try armingCopy(5)
        XCTAssertEqual(copy.confirm, "Turn on automatic contributing")
        XCTAssertEqual(copy.decline, "Not now")
        XCTAssertFalse(copy.decline.lowercased().contains("never"))
        XCTAssertFalse(copy.decline.lowercased().contains("don't ask"))
    }

    /// The confirmation body, whole, as the core holds it for every shell.
    func testTheArmingBodyIsTheAgreedWording() throws {
        XCTAssertEqual(
            try armingCopy(0).body,
            "Sessions from this project will be scrubbed and contributed without asking "
                + "you, from now on. You won't review them first. Sessions already on this Mac "
                + "keep waiting for you to pick them.\n\nNo session is sent until it has been "
                + "quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't "
                + "sent yet goes back to waiting for you, and anything already sent stays sent."
        )
    }

    /// The body states the scrubbing, that review stops, and the way back.
    func testTheArmingBodySaysReviewStopsAndNamesTheWayBack() throws {
        let body = try armingCopy(0).body
        XCTAssertTrue(body.contains("scrubbed"), body)
        XCTAssertTrue(body.contains("without asking you"), body)
        XCTAssertTrue(body.contains("You won't review them first."), body)
        XCTAssertTrue(body.contains("turn this off at any time"), body)
    }

    // MARK: - The scrubbing panel

    private func summary(
        _ occurrences: [String: Int],
        _ distinct: [String: Int] = [:]
    ) throws -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow]) {
        try XCTUnwrap(RedactionSummary.rows(fromJSON: TCCoreCopy.redactionSummaryJSON(
            occurrences: occurrences, distinct: distinct)))
    }

    func testAnEmptyMapProducesNoRows() throws {
        let out = try summary([:])
        XCTAssertTrue(out.removed.isEmpty)
        XCTAssertTrue(out.stillPresent.isEmpty)
    }

    func testOneFamilyBecomesOneRow() throws {
        let out = try summary(["local_path": 185], ["local_path": 12])
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "local_path")
        XCTAssertEqual(out.removed[0].display, "local path")
        XCTAssertEqual(out.removed[0].occurrences, 185)
        XCTAssertEqual(out.removed[0].distinct, 12)
        XCTAssertFalse(out.removed[0].description.isEmpty)
        XCTAssertEqual(out.removed[0].countLine, "185 local path (12 distinct)")
    }

    /// Nine secret patterns are one `secret` row, with the sub-labels on a
    /// detail line, and no distinct figure a secret never has.
    func testSubLabelsCollapseIntoTheirFamily() throws {
        let out = try summary(
            ["secret:contextual_entropy": 3, "secret:pem_private_key": 1, "secret": 2])
        XCTAssertEqual(out.removed.count, 1)
        XCTAssertEqual(out.removed[0].family, "secret")
        XCTAssertEqual(out.removed[0].occurrences, 6)
        XCTAssertEqual(out.removed[0].distinct, 0)
        XCTAssertEqual(out.removed[0].detail, ["contextual entropy", "pem private key"])
        XCTAssertEqual(out.removed[0].countLine, "6 secret")
    }

    /// A secret that was DETECTED AND NOT REMOVED is still present, never
    /// removed, and its site is a schema path reported verbatim.
    func testAResidualSurvivorIsReportedAsStillPresent() throws {
        let out = try summary(
            ["local_path": 3, "residual_secret_at:events.3.tool_result": 1])
        XCTAssertEqual(out.removed.map(\.family), ["local_path"])
        XCTAssertEqual(out.stillPresent.map(\.family), ["residual_secret_at"])
        XCTAssertEqual(out.stillPresent[0].detail, ["events.3.tool_result"])
    }

    /// An unfamiliar family is kept, with a neutral description that does not
    /// pretend to know it.
    func testAnUnknownFamilyIsKeptWithANeutralDescription() throws {
        let out = try summary(["future_category": 4])
        XCTAssertEqual(out.removed.map(\.family), ["future_category"])
        XCTAssertFalse(out.removed[0].description.isEmpty)
        XCTAssertFalse(out.removed[0].description.contains("future"))
    }

    func testRowsAreOrderedByOccurrencesThenFamily() throws {
        let out = try summary(["secret": 3, "local_path": 185, "email": 3])
        XCTAssertEqual(out.removed.map(\.family), ["local_path", "email", "secret"])
    }

    // MARK: - The surviving-secret line

    /// The line never names a number of secrets: it counts detection sites,
    /// and names them.
    func testTheSurvivorLineInflectsAndNamesItsSites() {
        XCTAssertEqual(
            TCCoreCopy.residualSecretLine(count: 1, sites: []),
            "A secret found here is still in what would be sent"
        )
        XCTAssertEqual(
            TCCoreCopy.residualSecretLine(count: 2, sites: []),
            "Secrets found in 2 places are still in what would be sent"
        )
        XCTAssertEqual(
            TCCoreCopy.residualSecretLine(count: 2, sites: ["events.1.x", "events.9.y"]),
            "Secrets found in 2 places are still in what would be sent (events.1.x, events.9.y)"
        )
    }

    // MARK: - The extra privacy scan

    /// Both halves of the disclosure, and no internal name as a headline.
    func testThePrivacyScanCopyKeepsBothHalvesOfTheDisclosure() throws {
        let copy = try XCTUnwrap(PrivacyScanCopy.decode(fromJSON: TCCoreCopy.privacyScanCopyJSON()))
        XCTAssertTrue(copy.disclosure.contains("transmitted to NEAR AI"), copy.disclosure)
        XCTAssertTrue(copy.disclosure.contains("nothing is sent at all"), copy.disclosure)
        XCTAssertFalse(copy.title.contains("NEAR AI"), copy.title)
        XCTAssertFalse(copy.title.contains("PII"), copy.title)
        XCTAssertTrue(copy.localAlways.contains("It runs either way."), copy.localAlways)
    }

    // MARK: - Quit

    /// With no watcher the prompt claims neither that quitting stops one nor
    /// that nothing moves.
    func testTheNoWatcherQuitPromptClaimsNothingItCannotKnow() throws {
        let prompt = try XCTUnwrap(QuitPrompt.decode(
            fromJSON: TCCoreCopy.quitPromptWithoutWatcherJSON()))
        XCTAssertEqual(prompt.role, "unavailable")
        XCTAssertEqual(prompt.title, "Quit Trace Commons?")
        XCTAssertFalse(prompt.body.contains("Quitting stops"), prompt.body)
        XCTAssertFalse(prompt.body.contains("stays waiting"), prompt.body)
    }

    // MARK: - Withdrawal, and the copy no macOS screen renders yet

    func testTheUnknownReachWithdrawalPromptWarnsAboutDistributedCopies() throws {
        let prompt = try XCTUnwrap(TCCoreCopy.withdrawalConfirmationPrompt())
        XCTAssertTrue(prompt.hasPrefix("Withdraw this trace?"), prompt)
        XCTAssertTrue(prompt.contains("cannot be recalled"), prompt)
    }

    /// Exported for the screens macOS does not have yet (the legacy invite
    /// migration panel, the connecting-inference step, the Flow 1 grant
    /// screens), so they arrive with the core's words rather than their own.
    func testTheRemainingTablesCrossAsJSONObjects() throws {
        for (name, json) in [
            ("legacy migration offer", TCCoreCopy.legacyMigrationOfferJSON()),
            ("inference connection", TCCoreCopy.inferenceConnectionCopyJSON()),
        ] {
            let text = try XCTUnwrap(json, name)
            let object = try JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any]
            XCTAssertFalse(object?.isEmpty ?? true, name)
        }
        XCTAssertFalse(
            TCCoreCopy.legacyMigrationRefusalLine(label: "no-such-label")?.isEmpty ?? true)
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-k11-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let grant = try XCTUnwrap(TCCoreCopy.automaticContributionCopyJSON(configDir: dir.path))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(grant.utf8)) as? [String: Any])
        XCTAssertEqual(object["disclosure"] as? String, "patterns_only")
    }
}
