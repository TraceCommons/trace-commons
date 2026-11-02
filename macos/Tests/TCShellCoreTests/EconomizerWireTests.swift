import Foundation
import XCTest
@testable import TCShellCore

/// `economizer_status`, `economizer_set_enabled` and `economizer_restore`
/// as the daemon serves them (`daemon::economizer` in the contributor
/// crate, whose tests pin the same shape).
final class EconomizerWireTests: XCTestCase {
    private func frame(_ result: String) -> String { #"{"id":0,"result":\#(result)}"# }

    static let status = """
    {"enabled":true,"folders_truncated":0,"folders":[
      {"folder":"my-app","harness":"claude","key":"repo-0123abcd","sessions":6,
       "left_out":["CronCreate","TodoWrite"],"given_back":["WebFetch"],
       "told":[{"program":"pytest","prefix":"python -m"}],"forming":2},
      {"folder":"api","harness":"codex","key":"repo-4567ef01","sessions":1,
       "left_out":[],"given_back":[],"told":[],"forming":0}
    ]}
    """

    func testTheStatusDecodesThroughTheLiveClient() async throws {
        let transport = FakeTransport(response: frame(Self.status))
        let answer = try await LiveDaemonClient(transport: transport).economizerStatus()
        XCTAssertEqual(transport.calls.map(\.method), ["economizer_status"])
        XCTAssertTrue(answer.enabled)
        XCTAssertEqual(answer.foldersTruncated, 0)
        XCTAssertEqual(answer.folders.map(\.folder), ["my-app", "api"])
        let first = answer.folders[0]
        XCTAssertEqual(first.harness, "claude")
        XCTAssertEqual(first.id, "repo-0123abcd")
        XCTAssertEqual(first.sessions, 6)
        XCTAssertEqual(first.leftOut, ["CronCreate", "TodoWrite"])
        XCTAssertEqual(first.givenBack, ["WebFetch"])
        XCTAssertEqual(first.told, [.init(program: "pytest", prefix: "python -m")])
        XCTAssertEqual(first.forming, 2)
    }

    func testTheSwitchAndGiveBackSendTheirParameters() async throws {
        let transport = FakeTransport { method, _ in
            method == "economizer_set_enabled" ? #"{"id":0,"result":{"enabled":false}}"#
                : #"{"id":0,"result":{"restored":["TodoWrite"]}}"#
        }
        let client = LiveDaemonClient(transport: transport)
        let switched = try await client.setEconomizer(on: false)
        XCTAssertFalse(switched.enabled)
        let restored = try await client.economizerRestore(harness: "claude", key: "repo-0123abcd")
        XCTAssertEqual(restored.restored, ["TodoWrite"])
        XCTAssertEqual(transport.calls.map(\.method), ["economizer_set_enabled", "economizer_restore"])
        let on = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(transport.calls[0].params.utf8)) as? [String: Any])
        XCTAssertEqual(on["on"] as? Bool, false)
        let give = try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(transport.calls[1].params.utf8)) as? [String: Any])
        XCTAssertEqual(give["tool"] as? String, "claude")
        XCTAssertEqual(give["key"] as? String, "repo-0123abcd")
    }
}
