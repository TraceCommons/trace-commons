import Foundation
import TCBridge
import TCShellCore
import XCTest

/// `tc_describe_folder` through the bridge: a picked folder is recognised by
/// its layout, and an unrelated one comes back as no rows.
final class DescribeFolderBridgeTests: XCTestCase {
    private func scratch() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-describe-folder-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }

    func testAPickedCodexStoreIsRecognised() throws {
        let dir = try scratch()
        let day = dir.appendingPathComponent("2026/08/20")
        try FileManager.default.createDirectory(at: day, withIntermediateDirectories: true)
        try Data("{}\n".utf8).write(to: day.appendingPathComponent("rollout-a.jsonl"))

        let json = try XCTUnwrap(TCDiscovery.describeFolderJSON(dir.path))
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.source(.codex)])
        XCTAssertEqual(matches.first?.sessionCount, 1)
    }

    /// A flat folder of `.json` files fits both OpenCode and a trajectory
    /// export, and both reach the shell.
    func testAFlatJsonFolderIsTwoMatches() throws {
        let dir = try scratch()
        try Data("{}".utf8).write(to: dir.appendingPathComponent("ses_a.json"))
        try Data("{}".utf8).write(to: dir.appendingPathComponent("ses_b.json"))

        let json = try XCTUnwrap(TCDiscovery.describeFolderJSON(dir.path))
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.source(.opencode), .trajectory])
        XCTAssertEqual(matches.map(\.sessionCount), [2, 2])
    }

    /// A flat folder of `.jsonl` files is a trajectory export only.
    func testAFlatJsonlFolderIsOneTrajectoryMatch() throws {
        let dir = try scratch()
        try Data("{}\n".utf8).write(to: dir.appendingPathComponent("run-1.jsonl"))

        let json = try XCTUnwrap(TCDiscovery.describeFolderJSON(dir.path))
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.trajectory])
    }

    func testAnUnrelatedFolderIsNoRows() throws {
        let dir = try scratch()
        try Data("x".utf8).write(to: dir.appendingPathComponent("notes.txt"))

        let json = try XCTUnwrap(TCDiscovery.describeFolderJSON(dir.path))
        XCTAssertEqual(try FolderMatch.decodeList(from: json), [])
    }
}
