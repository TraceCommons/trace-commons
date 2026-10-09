import Darwin
import Foundation
import TCBridge
import TCShellCore
import XCTest

/// A stand-in for the daemon's Unix socket, so a test can read what
/// `TCDaemon.subscribe` actually puts on the wire. The real daemon keeps its
/// renderer count to itself, and the arbiter publishes `reengage_due` only
/// for idle sessions days old, so neither can be observed from Swift; the
/// `subscribe` request an attached handle sends can.
///
/// It answers each request line with an empty result, and, like the real
/// daemon, pushes a `reengage_due` after a `subscribe` only to a subscriber
/// that declared it.
private final class StandInDaemonSocket: @unchecked Sendable {
    let directory: URL
    private let listener: Int32
    private let lock = NSLock()
    private var subscribeParams: [Data] = []
    private let subscribed = DispatchSemaphore(value: 0)

    /// The frame pushed to a subscriber that declared `reengage_due`.
    static let reengageFrame =
        #"{"event":"reengage_due","data":{"kind":"idle_sessions","title":"T","body":"B","actions":[{"id":"review","label":"R"}]}}"#

    init() throws {
        // Short enough for sockaddr_un on macOS.
        directory = URL(fileURLWithPath: "/private/tmp/tc-sa-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let path = directory.appendingPathComponent("daemon.sock").path
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw POSIXError(.EIO) }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            for (i, b) in bytes.enumerated() where i < raw.count - 1 { raw[i] = b }
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0, listen(fd, 4) == 0 else {
            close(fd)
            throw POSIXError(.EADDRINUSE)
        }
        listener = fd
        Thread.detachNewThread { [self] in serve() }
    }

    /// The `params` of each `subscribe` request received, as JSON objects.
    func subscribeRequests() -> [[String: Any]] {
        lock.lock()
        defer { lock.unlock() }
        return subscribeParams.compactMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
    }

    /// Waits for a `subscribe` request to have been answered.
    func waitForSubscribe(timeout: TimeInterval = 30) -> Bool {
        subscribed.wait(timeout: .now() + timeout) == .success
    }

    func stop() {
        shutdown(listener, SHUT_RDWR)
        close(listener)
        try? FileManager.default.removeItem(at: directory)
    }

    private func serve() {
        while true {
            let connection = accept(listener, nil, nil)
            guard connection >= 0 else { return }
            Thread.detachNewThread { [self] in handle(connection) }
        }
    }

    private func handle(_ connection: Int32) {
        defer { close(connection) }
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(connection, &chunk, chunk.count)
            guard n > 0 else { return }
            buffer.append(contentsOf: chunk[0..<n])
            while let newline = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<newline]
                buffer.removeSubrange(buffer.startIndex...newline)
                guard let request = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: Any],
                      let id = request["id"] as? Int
                else { continue }
                write(connection, #"{"id":\#(id),"result":{}}"#)
                guard request["method"] as? String == "subscribe" else { continue }
                let params = request["params"] as? [String: Any] ?? [:]
                let declared = (params["accepts"] as? [String])?.contains("reengage_due") == true
                lock.lock()
                subscribeParams.append((try? JSONSerialization.data(withJSONObject: params)) ?? Data())
                lock.unlock()
                if declared { write(connection, Self.reengageFrame) }
                subscribed.signal()
            }
        }
    }

    private func write(_ connection: Int32, _ line: String) {
        let bytes = Array((line + "\n").utf8)
        _ = bytes.withUnsafeBufferPointer { Darwin.write(connection, $0.baseAddress, $0.count) }
    }
}

/// `TCDaemon.subscribe(accepts:)` is the only thing that tells the daemon
/// this app can draw a re-engagement notification. Without the declaration
/// the arbiter defers every one as `no_renderer` and the app silently posts
/// none, while every ordinary frame still arrives -- so a test that watches
/// only ordinary frames cannot see it go missing. These read the request
/// itself, through the real dylib, over an attached handle.
final class SubscribeAcceptsDeclarationTests: XCTestCase {
    private func attach(_ socket: StandInDaemonSocket) throws -> TCDaemon {
        let daemon = try TCDaemon(attachingTo: socket.directory.path)
        let owned = AttachedHandle(daemon)
        addTeardownBlock { _ = owned.daemon.shutdown() }
        return daemon
    }

    func testAnAcceptingSubscriberDeclaresReengagementAndReceivesIt() throws {
        let socket = try StandInDaemonSocket()
        defer { socket.stop() }
        let daemon = try attach(socket)
        let frames = Frames()
        let subscription = try XCTUnwrap(daemon.subscribe(accepts: ["reengage_due"]) { frames.add($0) })
        defer { daemon.unsubscribe(subscription) }
        XCTAssertTrue(socket.waitForSubscribe(), "no subscribe request reached the socket")

        let requests = socket.subscribeRequests()
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests.first?["accepts"] as? [String], ["reengage_due"])

        let frame = try XCTUnwrap(frames.next(), "the declared reengage_due did not arrive")
        guard case .reengageDue(let due) = DaemonDataEventParser.parse(frame) else {
            return XCTFail("not a reengage_due: \(frame)")
        }
        XCTAssertEqual(due.kind, "idle_sessions")
    }

    func testAPlainSubscriberDeclaresNothing() throws {
        let socket = try StandInDaemonSocket()
        defer { socket.stop() }
        let daemon = try attach(socket)
        let subscription = try XCTUnwrap(daemon.subscribe { _ in })
        defer { daemon.unsubscribe(subscription) }
        XCTAssertTrue(socket.waitForSubscribe(), "no subscribe request reached the socket")

        let requests = socket.subscribeRequests()
        XCTAssertEqual(requests.count, 1)
        XCTAssertNil(requests.first?["accepts"] as? [String], "a plain subscribe declared \(requests)")
    }
}

/// Carries the handle into the teardown block. `TCDaemon` is safe to use
/// from any thread, and shutdown is idempotent.
private struct AttachedHandle: @unchecked Sendable {
    let daemon: TCDaemon
    init(_ daemon: TCDaemon) { self.daemon = daemon }
}

/// The frames a subscription callback received, from the Rust thread.
private final class Frames: @unchecked Sendable {
    private let lock = NSLock()
    private var received: [String] = []
    private let arrived = DispatchSemaphore(value: 0)

    func add(_ json: String) {
        lock.lock()
        received.append(json)
        lock.unlock()
        arrived.signal()
    }

    /// The next frame, or nil after `timeout` with none.
    func next(timeout: TimeInterval = 10) -> String? {
        guard arrived.wait(timeout: .now() + timeout) == .success else { return nil }
        lock.lock()
        defer { lock.unlock() }
        return received.isEmpty ? nil : received.removeFirst()
    }
}
