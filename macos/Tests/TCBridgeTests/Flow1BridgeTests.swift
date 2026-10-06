import TCBridge
import TCShellCore
import XCTest

/// The Flow 1 grant request crosses the real ABI: the core decides whether
/// the grant may be asked for, and which witness to name when it is. The
/// shell encodes its progress and decodes the answer, nothing more.
final class Flow1BridgeTests: XCTestCase {
    /// Every step done except the one under test.
    private func progress(
        scopesSaved: [String]? = ["code"],
        witnessShown: String? = nil
    ) -> Flow1Progress {
        Flow1Progress(
            connected: true,
            scopesSaved: scopesSaved,
            path: .automatic,
            scrubDisclosureSeen: true,
            witnessDisclosureSeen: true,
            witnessShown: witnessShown
        )
    }

    private func request(_ progress: Flow1Progress) throws -> Flow1GrantRequest {
        let progressJSON = try XCTUnwrap(progress.jsonString())
        let json = try XCTUnwrap(TCFlow1.grantRequestJSON(progressJSON: progressJSON))
        return try XCTUnwrap(Flow1GrantRequest.decode(fromJSON: json))
    }

    func test_aGrantRequestIsNotReadyUntilScopesAreSaved() throws {
        for unsaved: [String]? in [nil, []] {
            let answer = try request(progress(scopesSaved: unsaved))
            XCTAssertFalse(answer.ready)
            XCTAssertEqual(answer.blockers, ["scope"])
            XCTAssertNil(answer.witnessSigningAddress)
        }
    }

    func test_aReadyRequestNamesTheWitnessShown() throws {
        let answer = try request(progress(witnessShown: "witness-address"))
        XCTAssertTrue(answer.ready)
        XCTAssertEqual(answer.blockers, [])
        XCTAssertEqual(answer.witnessSigningAddress, "witness-address")
    }

    func test_theAskFirstPathBlocksTheGrant() throws {
        var asked = progress()
        asked.path = .askFirst
        let answer = try request(asked)
        XCTAssertFalse(answer.ready)
        XCTAssertEqual(answer.blockers, ["path"])
    }

    func test_progressEncodesTheCoresKeys() throws {
        let json = try XCTUnwrap(progress(witnessShown: nil).jsonString())
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(
            Set(object.keys),
            [
                "connected", "scopes_saved", "path", "scrub_disclosure_seen",
                "witness_disclosure_seen", "witness_shown",
            ])
        XCTAssertEqual(object["path"] as? String, "automatic")
        XCTAssertTrue(object["witness_shown"] is NSNull)
    }

    func test_unreadableProgressHasNoAnswer() {
        XCTAssertNil(TCFlow1.grantRequestJSON(progressJSON: "not json"))
    }
}
