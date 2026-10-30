import Foundation
import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Answers from a script and records every call with its parameters.
private final class RecordingDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private let answers: [String: String]
    private(set) var calls: [(method: String, params: [String: Any])] = []
    init(_ answers: [String: String]) { self.answers = answers }
    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        let params = (try? JSONSerialization.jsonObject(with: Data(paramsJSON.utf8))) as? [String: Any] ?? [:]
        calls.append((method, params))
        return answers[method] ?? #"{"error":{"code":"unknown_method","message":"unscripted"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

/// Settings' sharing choice and the calls behind it (#1753 A2; owner,
/// 2026-10-30): Ask each time is a default a folder's own rule beats, never
/// a hidden override, and Automatic keeps or clears the folders set to Ask.
final class SharingChoiceTests: XCTestCase {
    private func grant(_ json: String) throws -> AutomaticGrant {
        try DaemonClient(daemon: RecordingDaemon(["automatic_grant": #"{"result":"# + json + "}"]))
            .automaticGrant()
    }

    func testWhatIsInForceIsReadFromTheGrant() throws {
        XCTAssertNil(SharingChoice.current(nil), "nothing picked until it is read")
        XCTAssertEqual(SharingChoice.current(try grant(#"{"granted":false}"#)), .askMe)
        XCTAssertEqual(
            SharingChoice.current(try grant(#"{"granted":true,"include_existing":true}"#)), .automatic)
        XCTAssertEqual(
            SharingChoice.current(try grant(#"{"granted":true,"include_existing":false}"#)), .askMe,
            "a from-now grant leaves folders asking; choosing Automatic widens it")
    }

    func testAutomaticAsksAboutTheFoldersSetToAskFirst() throws {
        let rows = #"""
        {"result":{"projects":[
          {"project_id":"a","project_label":"a","mode":"notify_only","folder_mode":"notify_only","configured":true,"is_unresolved_bucket":false},
          {"project_id":"b","project_label":"b","mode":"notify_only","folder_mode":"notify_only","configured":false,"is_unresolved_bucket":false},
          {"project_id":"c","project_label":"c","mode":"auto_upload","folder_mode":"auto_upload","configured":true,"is_unresolved_bucket":false},
          {"project_id":"d","project_label":"d","mode":"ignore","folder_mode":"ignore","configured":true,"is_unresolved_bucket":false},
          {"project_id":"u","project_label":"u","mode":"notify_only","folder_mode":"notify_only","configured":true,"is_unresolved_bucket":true}
        ]}}
        """#
        let projects = try DaemonClient(daemon: RecordingDaemon(["list_projects": rows])).listProjects()
        XCTAssertEqual(SharingChoice.askingFolders(projects), 1, "only a folder with its own Ask")
        XCTAssertEqual(SharingChoice.firstStep(asking: 1), .keepAsks(count: 1))
        XCTAssertEqual(SharingChoice.firstStep(asking: 0), .disclose(keepFolderAsks: true))
    }

    /// The bug: Ask each time set a global Ask override nothing could
    /// clear, so a folder set to Automatic later never sent. Now it only
    /// withdraws the grant and returns the folders it armed to Ask.
    func testAskEachTimeSetsNoOverride() throws {
        let daemon = RecordingDaemon(["withdraw_automatic_grant": #"{"result":{"withdrawn":true}}"#])
        XCTAssertTrue(try DaemonClient(daemon: daemon).setAskEachTime())
        XCTAssertEqual(daemon.calls.map(\.method), ["withdraw_automatic_grant"])
        XCTAssertEqual(daemon.calls[0].params["return_folders_to_ask"] as? Bool, true)
    }

    func testAutomaticSaysWhetherToKeepTheFoldersSetToAsk() throws {
        let answer = #"{"result":{"granted":true,"include_existing":true}}"#
        for keep in [true, false] {
            let daemon = RecordingDaemon(["grant_automatic_including_existing": answer])
            _ = try DaemonClient(daemon: daemon).grantAutomatic(witnessSigningAddress: nil, keepFolderAsks: keep)
            XCTAssertEqual(daemon.calls.first?.params["keep_folder_asks"] as? Bool, keep)
        }
        let daemon = RecordingDaemon(["grant_automatic_including_existing": answer])
        _ = try DaemonClient(daemon: daemon).grantAutomatic(witnessSigningAddress: nil)
        XCTAssertEqual(daemon.calls.first?.params["keep_folder_asks"] as? Bool, true,
                       "setup keeps a folder's own Ask")
    }

    private func folder(_ id: String, _ mode: ProjectMode?, offers: [ProjectMode] = [.ask, .autoUpload, .ignore])
        -> TracesTree.FolderNode
    {
        TracesTree.FolderNode(id: id, label: id, mode: mode, offerableModes: offers, sessions: [])
    }

    /// The Traces tab (owner, 2026-10-30): a switch on each folder beside
    /// Submit, on at Automatic and off at Ask; none at Never, which stays
    /// in the row menu.
    func testEachFolderRowSwitchesBetweenAutomaticAndAsk() {
        XCTAssertEqual(TracesTreeView.automaticSwitch(folder("a", .autoUpload)), true)
        XCTAssertEqual(TracesTreeView.automaticSwitch(folder("b", .ask)), false)
        XCTAssertNil(TracesTreeView.automaticSwitch(folder("c", .ignore)))
        XCTAssertNil(TracesTreeView.automaticSwitch(folder("d", nil)))
        XCTAssertNil(TracesTreeView.automaticSwitch(folder("e", .ask, offers: [.ask, .ignore])),
                     "no switch to a mode the core does not offer")
    }

    func testTheTabSwitchCountsTheFoldersThatSendOnTheirOwn() {
        let folders = [folder("a", .autoUpload), folder("b", .ask), folder("c", .ignore), folder("d", .autoUpload)]
        XCTAssertEqual(SharingChoice.automaticCount(folders, words: "{count} of {total}"), "2 of 3",
                       "Never is left out of the total")
    }

    func testTheCardIsAtTheTopOfProjects() throws {
        let source = try InsightsGlassConventionsTests.text("Views/Settings/ProjectsSection.swift")
        XCTAssertTrue(source.contains("SharingChoiceCard()\n            GlassEyebrowCard(SettingsWords.projects)"))
    }
}
