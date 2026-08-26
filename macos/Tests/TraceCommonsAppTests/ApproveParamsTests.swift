import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Covers the one rule the outcome verdict actually turns on: an answer the
/// contributor did not give is an ABSENT `outcome` key, not a null and not
/// an empty string.
///
/// The daemon reads an absent parameter as `TaskSuccess::Unknown` and
/// approves normally, and reads any unrecognised value -- `null` and `""`
/// included -- as `bad_params` / `outcome-invalid`, approving nothing. So a
/// shell that sends a placeholder for "no answer" does not record a blank
/// verdict; it fails the whole approval. `DaemonClient.approveParams` is
/// static precisely so that distinction has somewhere to be asserted
/// without a live socket. See `docs/contributor-daemon-ipc-v1_1.md`, "The
/// `outcome` verdict".
final class ApproveParamsTests: XCTestCase {
    private let entryID = "test-entry-id"
    private let projectID = "proj-1"

    func testAnApproveCallCarriesTheSelectedVerdict() {
        let params = DaemonClient.approveParams(target: .entry(entryID), verdict: .partly)
        XCTAssertEqual(params["entry_id"] as? String, entryID)
        XCTAssertEqual(params["outcome"] as? String, "partly")
    }

    /// The assertion is on the KEY, not on its value: `params["outcome"]`
    /// being nil-ish is not enough, because `NSNull` and `""` would both
    /// pass a value check and both get the approval refused.
    func testAnApproveCallWithNoVerdictOmitsTheParameter() {
        let params = DaemonClient.approveParams(target: .entry(entryID), verdict: nil)
        XCTAssertEqual(params["entry_id"] as? String, entryID)
        XCTAssertFalse(params.keys.contains("outcome"))
    }

    /// And it must survive serialization: the key has to be missing from the
    /// JSON the daemon actually reads, not merely missing from a dictionary
    /// that encodes it as null.
    func testTheOmittedParameterIsAbsentFromTheEncodedJSON() throws {
        let params = DaemonClient.approveParams(target: .entry(entryID), verdict: nil)
        let data = try JSONSerialization.data(withJSONObject: params)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )
        XCTAssertFalse(object.keys.contains("outcome"))
        XCTAssertEqual(String(decoding: data, as: UTF8.self).contains("outcome"), false)
    }

    /// One verdict for a whole group is the point of `Submit all as...`.
    func testABulkApproveCanCarryAVerdict() {
        let params = DaemonClient.approveParams(target: .project(projectID), verdict: .failed)
        XCTAssertEqual(params["project_id"] as? String, projectID)
        XCTAssertEqual(params["outcome"] as? String, "failed")
    }

    /// Plain `Submit all` stays a one-click, unanswered submit.
    func testABulkApproveWithoutAVerdictOmitsTheParameter() {
        let params = DaemonClient.approveParams(target: .project(projectID), verdict: nil)
        XCTAssertEqual(params["project_id"] as? String, projectID)
        XCTAssertFalse(params.keys.contains("outcome"))
    }

    func testTheAllTargetCarriesAVerdictToo() {
        let params = DaemonClient.approveParams(target: .all, verdict: .worked)
        XCTAssertEqual(params["all"] as? Bool, true)
        XCTAssertEqual(params["outcome"] as? String, "worked")
    }

    /// The selectors are mutually exclusive on the wire, so a built call
    /// must never carry two of them.
    func testTheSelectorsAreMutuallyExclusive() {
        let entry = DaemonClient.approveParams(target: .entry(entryID), verdict: .worked)
        XCTAssertFalse(entry.keys.contains("project_id"))
        XCTAssertFalse(entry.keys.contains("all"))

        let project = DaemonClient.approveParams(target: .project(projectID), verdict: nil)
        XCTAssertFalse(project.keys.contains("entry_id"))
        XCTAssertFalse(project.keys.contains("all"))
    }

    /// A written correction rides along with the verdict it was written
    /// under.
    func testAnApproveCallCarriesAWrittenCorrection() {
        let params = DaemonClient.approveParams(
            target: .entry(entryID),
            verdict: .failed,
            correction: "it edited the staging config instead of the local one"
        )
        XCTAssertEqual(params["outcome"] as? String, "failed")
        XCTAssertEqual(
            params["correction"] as? String,
            "it edited the staging config instead of the local one"
        )
    }

    /// An untouched box, and a box holding only whitespace, are the same
    /// thing: no correction. The assertion is on the KEY, for the same
    /// reason the verdict's is -- an empty string would declare
    /// `correction_included` on the envelope for content that is not there.
    func testABlankCorrectionOmitsTheParameter() throws {
        for blank in [nil, "", "   ", "\n\t "] as [String?] {
            let params = DaemonClient.approveParams(
                target: .entry(entryID),
                verdict: .failed,
                correction: blank
            )
            XCTAssertFalse(
                params.keys.contains("correction"),
                "blank correction must send no key at all: \(String(describing: blank))"
            )
            let data = try JSONSerialization.data(withJSONObject: params)
            XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("correction"))
        }
    }

    /// Leading and trailing whitespace goes; the words do not.
    func testACorrectionIsSentTrimmed() {
        let params = DaemonClient.approveParams(
            target: .entry(entryID),
            verdict: .partly,
            correction: "  it stopped halfway  "
        )
        XCTAssertEqual(params["correction"] as? String, "it stopped halfway")
    }

    /// The default keeps every existing call site sending exactly what it
    /// sent before the box existed.
    func testTheCorrectionDefaultsToAbsent() {
        let params = DaemonClient.approveParams(target: .entry(entryID), verdict: .worked)
        XCTAssertFalse(params.keys.contains("correction"))
    }

    /// The wire words are the contract's three, lowercase. The labels the
    /// contributor sees are never what gets sent.
    func testTheWireValuesAreTheContractsThree() {
        XCTAssertEqual(ContributorVerdict.allCases.map(\.rawValue), ["worked", "partly", "failed"])
        XCTAssertEqual(
            ContributorVerdict.allCases.map(\.label),
            [VerdictCopy.worked, VerdictCopy.partly, VerdictCopy.failed]
        )
    }
}
