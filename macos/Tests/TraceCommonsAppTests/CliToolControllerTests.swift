import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The controller runs only the script the core offered, says nothing when
/// the password prompt is cancelled, and reads the link again afterwards.
/// No test runs `osascript`: the runner is a recording fake.
@MainActor
final class CliToolControllerTests: XCTestCase {
    private final class Recorder: @unchecked Sendable {
        var scripts: [String] = []
    }

    private func status(_ state: String) -> CliToolStatus? {
        CliToolStatus.decode(fromJSON: """
            {"command_name":"near-ai","link_path":"/usr/local/bin/near-ai","state":"\(state)",
             "install_script":"INSTALL","uninstall_script":"UNINSTALL"}
            """)
    }

    private func controller(
        states: [String], outcome: CliToolOutcome, recorder: Recorder
    ) -> CliToolController {
        var remaining = states
        return CliToolController(
            probe: { [self] in
                let next = remaining.count > 1 ? remaining.removeFirst() : remaining[0]
                return status(next)
            },
            runner: { script in
                recorder.scripts.append(script)
                return outcome
            })
    }

    func testInstallRunsTheInstallScriptAndRereads() async {
        let recorder = Recorder()
        let model = controller(states: ["not_installed", "installed"], outcome: .done, recorder: recorder)
        await model.perform(.install)
        XCTAssertEqual(recorder.scripts, ["INSTALL"])
        XCTAssertEqual(model.status?.state, .installed)
        XCTAssertNil(model.failedPath)
        XCTAssertFalse(model.busy)
    }

    func testACancelledPromptShowsNothing() async {
        let recorder = Recorder()
        let model = controller(states: ["installed"], outcome: .cancelled, recorder: recorder)
        await model.perform(.uninstall)
        XCTAssertEqual(recorder.scripts, ["UNINSTALL"])
        XCTAssertEqual(model.status?.state, .installed)
        XCTAssertNil(model.failedPath)
    }

    func testAFailureNamesTheLinkPath() async {
        let recorder = Recorder()
        let model = controller(states: ["not_installed"], outcome: .failed, recorder: recorder)
        await model.perform(.install)
        XCTAssertEqual(model.failedPath, "/usr/local/bin/near-ai")
    }

    /// Nothing runs for an action the state does not offer, or over a
    /// foreign file.
    func testNothingRunsWhenTheActionIsNotOffered() async {
        let recorder = Recorder()
        let installed = controller(states: ["installed"], outcome: .done, recorder: recorder)
        await installed.perform(.install)
        let foreign = controller(states: ["foreign"], outcome: .done, recorder: recorder)
        for action in CliToolStatus.Action.allCases {
            await foreign.perform(action)
        }
        XCTAssertEqual(recorder.scripts, [])
    }
}
