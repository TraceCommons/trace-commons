import Foundation
@testable import TCBridge
import XCTest

#if DEBUG
@MainActor
final class ComputeResourceBridgeTests: XCTestCase {
    private let healthy = TCComputeResourceReading(power: .ac, lowPowerMode: false, thermal: .nominal, memory: .normal)

    private func local() throws -> TCCompute {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("tc-resource-bridge-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        let worker = root.appendingPathComponent("sleepy-worker")
        // Synthetic lifecycle fixture only: no MLX, pool, or resource load.
        try Data("#!/bin/sh\nexec /bin/sleep 60\n".utf8).write(to: worker)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: worker.path)
        let config: [String: Any] = ["binary": worker.path,
            "expected_sha256": "eb2c0b11d46d6efb3031027345fb05b0e168ed4c2244e3639b302e3e8e1361c9",
            "coordinator": "ws://127.0.0.1:9999", "startup_timeout_secs": 30]
        return try TCCompute(localConfigDirectory: root.path,
            localConfigurationJSON: String(decoding: JSONSerialization.data(withJSONObject: config), as: UTF8.self))
    }

    private func snapshot(_ compute: TCCompute) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(compute.statusJSON().utf8)) as? [String: Any])
    }

    private func waitFor(_ compute: TCCompute, state: String) async throws {
        let deadline = ProcessInfo.processInfo.systemUptime + 3
        while ProcessInfo.processInfo.systemUptime < deadline {
            if try snapshot(compute)["state"] as? String == state { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("compute lifecycle deadline exceeded")
    }

    func testSingleUseTicketAndWakeInvalidationThroughABI() throws {
        let compute = try local()
        defer { _ = try? compute.shutdownJSON(timeoutMilliseconds: 5000); compute.close() }
        let old = try XCTUnwrap(compute.resourceBeginJSON())
        let current = try XCTUnwrap(compute.resourceBeginJSON())
        XCTAssertThrowsError(try compute.resourceSampleJSON(ticket: old, reading: healthy))
        try compute.resourceSampleJSON(ticket: current, reading: healthy)
        XCTAssertThrowsError(try compute.resourceSampleJSON(ticket: current, reading: healthy))
        let preWake = try XCTUnwrap(compute.resourceBeginJSON())
        try compute.resourceWakeJSON()
        XCTAssertThrowsError(try compute.resourceSampleJSON(ticket: preWake, reading: healthy))
        XCTAssertEqual(try snapshot(compute)["worker_stopped"] as? Bool, true)
        XCTAssertEqual(try snapshot(compute)["can_resume"] as? Bool, false)
    }

    func testRealMacReadingReachesRustWithoutLaunching() throws {
        let compute = try local()
        defer { _ = try? compute.shutdownJSON(timeoutMilliseconds: 5000); compute.close() }
        let ticket = try XCTUnwrap(compute.resourceBeginJSON())
        let reading = TCComputeResourceReading.readCurrent()
        try compute.resourceSampleJSON(ticket: ticket, reading: reading)
        let eligible = reading.power == .ac && reading.lowPowerMode == false
            && reading.thermal == .nominal && reading.memory == .normal
        let result = try snapshot(compute)
        XCTAssertEqual(result["can_enable"] as? Bool, eligible)
        XCTAssertEqual(result["worker_stopped"] as? Bool, true)
        XCTAssertEqual(result["consent_granted"] as? Bool, false)
        print("native-resource-gate power=\(reading.power.rawValue) thermal=\(reading.thermal.rawValue) memory=\(reading.memory.rawValue) eligible=\(eligible)")
    }

    func testResourceIngressEscalatesWhileShutdownPinsHandle() async throws {
        let compute = try local()
        defer { _ = try? compute.shutdownJSON(timeoutMilliseconds: 5000); compute.close() }
        _ = try compute.commandJSON(.enable(ramAllowanceGiB: 1))
        XCTAssertEqual(try snapshot(compute)["consent_granted"] as? Bool, false)
        try compute.resourceSampleJSON(ticket: XCTUnwrap(compute.resourceBeginJSON()), reading: healthy)
        _ = try compute.commandJSON(.enable(ramAllowanceGiB: 1))
        try await waitFor(compute, state: "starting")
        // Wait until the fixture's process has actually been created, not just
        // until the actor publishes Starting immediately before preparation.
        try await Task.sleep(for: .milliseconds(200))
        let shutdown = Task.detached { try compute.shutdownJSON(timeoutMilliseconds: 8000) }
        try await waitFor(compute, state: "draining")
        XCTAssertFalse(compute.close())
        let start = ProcessInfo.processInfo.systemUptime
        let critical = TCComputeResourceReading(power: .ac, lowPowerMode: false, thermal: .nominal, memory: .critical)
        try compute.resourceSampleJSON(ticket: XCTUnwrap(compute.resourceBeginJSON()), reading: critical)
        // The old lock-held shutdown would block this call for the normal stop.
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - start, 1.5)
        let stopped = try await shutdown.value
        let evidence = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(stopped.utf8)) as? [String: Any])
        XCTAssertEqual(evidence["worker_stopped"] as? Bool, true)
        XCTAssertEqual(evidence["stop_outcome"] as? String, "forced")
        XCTAssertNotEqual(evidence["drain_outcome"] as? String, "acknowledged")
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - start, 2.8)
        XCTAssertTrue(compute.close())
        XCTAssertNil(compute.resourceBeginJSON())
    }
    func testResourceIngressDoesNotWaitForPinnedCommand() async throws {
        let compute = try local()
        let entered = DispatchSemaphore(value: 0)
        let release = DispatchSemaphore(value: 0)
        defer {
            release.signal()
            _ = try? compute.shutdownJSON(timeoutMilliseconds: 5000)
            compute.close()
        }
        compute.commandWillExecuteForTesting = {
            entered.signal()
            _ = release.wait(timeout: .now() + 5)
        }
        let command = Task.detached { try compute.commandJSON(.pause) }
        let ready = await withCheckedContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(returning: entered.wait(timeout: .now() + 2) == .success)
            }
        }
        XCTAssertTrue(ready)
        let start = ProcessInfo.processInfo.systemUptime
        XCTAssertFalse(compute.close(), "the command must retain its handle")
        let ticket = try XCTUnwrap(compute.resourceBeginJSON())
        try compute.resourceSampleJSON(ticket: ticket, reading: healthy)
        XCTAssertLessThan(ProcessInfo.processInfo.systemUptime - start, 0.5)
        release.signal()
        _ = try await command.value
        // commandJSON acknowledges the queued pause; the Rust actor may still
        // be processing it. Handle lifetime is released before command_pending
        // necessarily clears, and close must refuse that intermediate state.
        let stopped = try await Task.detached {
            try compute.shutdownJSON(timeoutMilliseconds: 5000)
        }.value
        let evidence = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(stopped.utf8)) as? [String: Any])
        XCTAssertEqual(evidence["worker_stopped"] as? Bool, true)
        XCTAssertTrue(compute.close())
    }

}
#endif
