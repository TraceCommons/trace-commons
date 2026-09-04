import Foundation
import TCBridge
import XCTest

final class ComputeExportTests: XCTestCase {
    private func directory() throws -> URL {
        let path = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-compute-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: path, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: path) }
        return path
    }

    private func snapshot(_ json: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    func testFreshUnenrolledDirectoryCannotEnableUnavailableWorker() throws {
        let root = try directory()
        let compute = try TCCompute(configDirectory: root.path)
        defer { compute.close() }
        let initial = try snapshot(compute.statusJSON())
        XCTAssertEqual(initial["schema"] as? String, "trace_commons.compute_status.v1")
        XCTAssertEqual(initial["state"] as? String, "disabled")
        XCTAssertEqual(initial["consent_granted"] as? Bool, false)
        XCTAssertEqual(initial["available"] as? Bool, false)
        XCTAssertEqual(initial["can_enable"] as? Bool, false)
        let copy = try XCTUnwrap(initial["copy"] as? [String: String])
        XCTAssertFalse(try XCTUnwrap(copy["destination"]).isEmpty)
        XCTAssertFalse(try XCTUnwrap(copy["allowance_detail"]).isEmpty)

        let refused = try snapshot(compute.commandJSON(.enable(ramAllowanceGiB: 8)))
        XCTAssertEqual(refused["reason"] as? String, "worker-unavailable")
        XCTAssertEqual(refused["consent_granted"] as? Bool, false)
        XCTAssertEqual(refused["available"] as? Bool, false)
        compute.close()
        let reopened = try TCCompute(configDirectory: root.path)
        defer { reopened.close() }
        XCTAssertEqual(try snapshot(reopened.statusJSON())["state"] as? String, "disabled")
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("compute/worker").path))
    }

    func testCloseIsIdempotentAndRefusesFurtherCalls() throws {
        let compute = try TCCompute(configDirectory: directory().path)
        compute.close()
        compute.close()
        XCTAssertThrowsError(try compute.statusJSON()) { error in
            XCTAssertEqual(error as? TCCompute.Failure, .closed)
        }
        XCTAssertThrowsError(try compute.commandJSON(.pause)) { error in
            XCTAssertEqual(error as? TCCompute.Failure, .closed)
        }
    }

    func testPreviouslyGrantedConsentRestoresPausedAndCanBeRevoked() throws {
        let root = try directory()
        let settingsDirectory = root.appendingPathComponent("compute", isDirectory: true)
        try FileManager.default.createDirectory(at: settingsDirectory, withIntermediateDirectories: true)
        // Represents a previous consenting installation, not a shell bypass of
        // the currently unavailable Enable command.
        let fixture = #"{"schema":"trace_commons.compute_settings.v1","consent_granted":true,"ram_allowance_gib":8}"#
        try Data(fixture.utf8).write(to: settingsDirectory.appendingPathComponent("settings.json"))
        let compute = try TCCompute(configDirectory: root.path)
        defer { compute.close() }
        XCTAssertEqual(try snapshot(compute.statusJSON())["state"] as? String, "paused")
        XCTAssertEqual(try snapshot(compute.commandJSON(.resume))["reason"] as? String, "worker-unavailable")
        XCTAssertEqual(try snapshot(compute.commandJSON(.pause))["state"] as? String, "paused")
        compute.close()

        let reopened = try TCCompute(configDirectory: root.path)
        defer { reopened.close() }
        XCTAssertEqual(try snapshot(reopened.statusJSON())["state"] as? String, "paused")
        XCTAssertEqual(try snapshot(reopened.commandJSON(.disable))["consent_granted"] as? Bool, false)
        reopened.close()

        let revoked = try TCCompute(configDirectory: root.path)
        defer { revoked.close() }
        XCTAssertEqual(try snapshot(revoked.statusJSON())["state"] as? String, "disabled")
    }

    func testConcurrentCommandsAndCloseNeverUseAFreedHandle() throws {
        let compute = try TCCompute(configDirectory: directory().path)
        DispatchQueue.concurrentPerform(iterations: 100) { index in
            if index.isMultiple(of: 7) {
                compute.close()
            } else {
                do {
                    _ = try compute.commandJSON(.pause)
                } catch {
                    XCTAssertEqual(error as? TCCompute.Failure, .closed)
                }
            }
        }
        XCTAssertThrowsError(try compute.statusJSON())
    }

    func testNulDirectoryIsRejectedBeforeCTruncation() throws {
        let root = try directory()
        XCTAssertThrowsError(try TCCompute(configDirectory: root.path + "\0suffix")) { error in
            XCTAssertEqual(error as? TCCompute.Failure, .invalidInput)
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.appendingPathComponent("compute").path))
    }
}
