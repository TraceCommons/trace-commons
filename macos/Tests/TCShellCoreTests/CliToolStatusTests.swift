import XCTest

@testable import TCShellCore

/// The command-line tool's state decodes from the core's object, offers
/// only the actions the core gave a script for, and a cancelled password
/// prompt is told apart from a failure.
final class CliToolStatusTests: XCTestCase {
    private func status(_ state: String, install: String? = "I", uninstall: String? = "U") -> CliToolStatus? {
        var object: [String: Any] = [
            "command_name": "near-ai-staging",
            "link_path": "/usr/local/bin/near-ai-staging",
            "state": state,
        ]
        object["install_script"] = install ?? NSNull()
        object["uninstall_script"] = uninstall ?? NSNull()
        let data = try! JSONSerialization.data(withJSONObject: object)
        return CliToolStatus.decode(fromJSON: String(decoding: data, as: UTF8.self))
    }

    func testEachStateOffersItsActions() throws {
        XCTAssertEqual(try XCTUnwrap(status("not_installed")).actions, [.install])
        XCTAssertEqual(try XCTUnwrap(status("installed")).actions, [.uninstall])
        XCTAssertEqual(try XCTUnwrap(status("stale")).actions, [.updateLink, .uninstall])
        XCTAssertEqual(try XCTUnwrap(status("foreign")).actions, [])
        XCTAssertEqual(try XCTUnwrap(status("unavailable")).actions, [])
        XCTAssertEqual(try XCTUnwrap(status("stale")).primaryAction, .updateLink)
        XCTAssertNil(try XCTUnwrap(status("foreign")).primaryAction)
    }

    /// Each action runs its own script, and only in a state that offers it:
    /// a foreign file is never handed a script even if one arrived.
    func testEachActionRunsItsOwnScript() throws {
        let stale = try XCTUnwrap(status("stale"))
        XCTAssertEqual(stale.script(for: .updateLink), "I")
        XCTAssertEqual(stale.script(for: .uninstall), "U")
        XCTAssertNil(stale.script(for: .install))
        let installed = try XCTUnwrap(status("installed"))
        XCTAssertNil(installed.script(for: .install))
        XCTAssertNil(installed.script(for: .updateLink))
        let foreign = try XCTUnwrap(status("foreign"))
        for action in CliToolStatus.Action.allCases {
            XCTAssertNil(foreign.script(for: action), "\(action)")
        }
    }

    func testAnActionWithoutAScriptIsNotOffered() throws {
        XCTAssertEqual(try XCTUnwrap(status("not_installed", install: nil)).actions, [])
        XCTAssertEqual(try XCTUnwrap(status("stale", install: nil)).actions, [.uninstall])
    }

    func testAnUnknownStateDoesNotDecode() {
        XCTAssertNil(status("maybe"))
        XCTAssertNil(CliToolStatus.decode(fromJSON: nil))
        XCTAssertNil(CliToolStatus.decode(fromJSON: "{}"))
    }

    /// AppleScript's -128 is the cancelled password prompt.
    func testACancelledPromptIsNotAFailure() {
        XCTAssertEqual(CliToolOutcome.classify(exitStatus: 0, standardError: ""), .done)
        XCTAssertEqual(
            CliToolOutcome.classify(exitStatus: 1, standardError: "0:113: execution error: User canceled. (-128)\n"),
            .cancelled)
        XCTAssertEqual(
            CliToolOutcome.classify(exitStatus: 1, standardError: "0:113: execution error: The command exited with a non-zero status. (17)\n"),
            .failed)
        XCTAssertEqual(CliToolOutcome.classify(exitStatus: 1, standardError: ""), .failed)
    }
}
