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
        let candidates = try SourceCandidate.decodeList(from: json)
        XCTAssertEqual(candidates.map(\.source), [.codex])
        XCTAssertEqual(candidates.first?.sessionCount, 1)
    }

    func testAnUnrelatedFolderIsNoRows() throws {
        let dir = try scratch()
        try Data("x".utf8).write(to: dir.appendingPathComponent("notes.txt"))

        let json = try XCTUnwrap(TCDiscovery.describeFolderJSON(dir.path))
        XCTAssertEqual(try SourceCandidate.decodeList(from: json), [])
    }
}
