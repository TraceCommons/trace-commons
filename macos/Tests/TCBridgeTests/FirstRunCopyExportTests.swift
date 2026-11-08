import TCBridge
import TCShellCore
import XCTest

/// The first-run table decodes from the real export, so a key renamed in the
/// core fails here rather than as a blank first run.
final class FirstRunCopyExportTests: XCTestCase {
    func testTheExportDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        let copy = try XCTUnwrap(FirstRunCopy.decode(json))
        XCTAssertEqual(copy.frame.quickSetup, "Quick setup")
        XCTAssertEqual(copy.frame.back, "Back")
        // The animated first run's own groups decode from the same table.
        XCTAssertEqual(copy.welcome.title, "Welcome to Near AI Gateway")
        XCTAssertEqual(copy.frame.stepTraces, "Traces")
        XCTAssertEqual(copy.traces.foundCredit, "We have detected some traces that can earn you credit.")
        XCTAssertTrue(copy.privateAi.canConnect.contains("{total}"))
    }

    /// One trace reads "1 trace", never "1 traces".
    func testASingleTraceIsSingular() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        XCTAssertEqual(copy.frame.sessionCountLine(1), "1 trace")
        XCTAssertEqual(copy.frame.sessionCountLine(0), "0 traces")
        XCTAssertEqual(copy.frame.sessionCountLine(2), "2 traces")
    }

    /// Folders' sign-in line is the core's existing Private AI one, read
    /// through the first-run table rather than a second bridge call. It is
    /// no longer the inference step's, which offers a Skip Join does not
    /// have (tester report, 2026-11-04).
    func testTheSignInLineIsPrivateAIs() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let privateAI = try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: try XCTUnwrap(TCPrivateInference.copyJSON())))
        XCTAssertEqual(copy.folders.signInFailed, privateAI.credentialFailed)
        XCTAssertFalse(copy.folders.signInFailed.lowercased().contains("skip"))
        XCTAssertNotEqual(copy.folders.lookupUnavailable, copy.join.inviteError)
        XCTAssertNotEqual(copy.uses.completeFailed, copy.uses.sharingRefused)
    }

    func testATableWithAnEmptyStringIsRefused() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        let blanked = json.replacingOccurrences(of: "\"Quick setup\"", with: "\"\"")
        XCTAssertNotEqual(blanked, json)
        XCTAssertNil(FirstRunCopy.decode(blanked))
    }
    func testWelcomeLegalPlaceholderDecodesPublishedLinks() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let legal = try XCTUnwrap(copy.welcome.legal)
        XCTAssertEqual(legal.accept, "I accept the Terms of Service")
        XCTAssertEqual(legal.termsUrl.absoluteString, "https://\(LinkedChannel.siteHost)/about/tnc/")
        XCTAssertEqual(legal.privacyUrl.absoluteString, "https://\(LinkedChannel.siteHost)/about/privacy/")
    }

    func testOlderWelcomeExportsRemainCompatible() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        var tree = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        var welcome = try XCTUnwrap(tree["welcome"] as? [String: Any])
        welcome.removeValue(forKey: "legal")
        tree["welcome"] = welcome
        let old = String(decoding: try JSONSerialization.data(withJSONObject: tree), as: UTF8.self)
        let copy = try XCTUnwrap(FirstRunCopy.decode(old))
        XCTAssertNil(copy.welcome.legal)
        XCTAssertEqual(copy.welcome.start, "Get started")
    }

}
