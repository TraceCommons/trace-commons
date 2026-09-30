import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Records what the model actually put on the wire.
///
/// Locked for `CredentialCancelTests.RecordingDaemon`'s reason: one refresh
/// fans out into several `Task.detached` calls, and an unsynchronised append
/// under that corrupts the buffer's refcount and takes the whole test process
/// down with SIGSEGV after every test has reported passing.
private final class RecordingDaemon: DaemonCalling {
    private let lock = NSLock()
    private var recorded: [String] = []
    private var responseValue = #"{"id":1,"result":{}}"#

    var methods: [String] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    var response: String {
        get {
            lock.lock()
            defer { lock.unlock() }
            return responseValue
        }
        set {
            lock.lock()
            defer { lock.unlock() }
            responseValue = newValue
        }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(method)
        return responseValue
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

/// The balance row as the model really wires it.
///
/// `BalanceExportTests` builds its own `BalanceCalls` from `TCNearAiBalance`,
/// which proves the ABI answers correctly but CANNOT prove the model installs
/// those particular functions. A `balanceCalls` that had been given
/// `TCNearAiCredential.action` -- the neighbouring table, the same signature,
/// the same enum -- would satisfy every other test in this repo and would put
/// a Forget button beside somebody's money.
final class BalanceBindingTests: XCTestCase {
    @MainActor
    func testTheModelSeedsAnUnreportedBalanceRatherThanAnEmptyOne() {
        let model = AppModel()
        XCTAssertEqual(model.balanceStatus.state, "")
        XCTAssertNil(model.balanceStatus.remainingNanos, "nothing has been read yet")
        // Unreported, not "no sign-in kept here": before the first poll this
        // shell has read nothing, and either seed that named a state would be
        // a claim about somebody's account made before anything was asked.
        XCTAssertEqual(model.balanceStatus, BalanceStatus.unreported)
    }

    /// The model's own closures, against the real Rust.
    ///
    /// The states are asserted through `model.balanceCalls` specifically --
    /// not through a locally built struct -- so a mis-wired table is caught.
    @MainActor
    func testTheModelsOwnTablesAnswerTheBalanceContract() throws {
        let calls = AppModel().balanceCalls

        // OBTAIN on exactly the two states that answer it.
        for state in ["no_session", "session_expired"] {
            XCTAssertEqual(
                BalanceSurface.action(BalanceStatus(state: state), calls: calls), .obtain,
                "\(state) offers a sign-in")
        }
        for state in ["known", "no_organization", "unavailable", "", "a_state_from_2027"] {
            XCTAssertEqual(
                BalanceSurface.action(BalanceStatus(state: state), calls: calls),
                CredentialAction.none,
                "\(state) offers nothing")
        }

        // The money formatter is the balance one, and it keeps null apart
        // from zero.
        let zero = BalanceStatus(state: "known", remainingNanos: 0, scale: 9)
        let null = BalanceStatus(state: "known", remainingNanos: nil, scale: 9)
        let payload = try XCTUnwrap(
            PrivateInferenceCopy.decode(fromJSON: try XCTUnwrap(TCPrivateInference.copyJSON())))
        XCTAssertEqual(
            BalanceSurface.remaining(zero, copy: payload, calls: calls), .figure("$0.00"))
        XCTAssertEqual(
            BalanceSurface.remaining(null, copy: payload, calls: calls),
            .sentence(payload.balanceNoRemaining))

        // Only a successful read is settled, and no amount moves that.
        XCTAssertEqual(BalanceSurface.tone(zero, calls: calls), .clear)
        XCTAssertEqual(BalanceSurface.tone(null, calls: calls), .clear)
        XCTAssertNotEqual(
            BalanceSurface.tone(BalanceStatus(state: "unavailable"), calls: calls), .clear)
    }

    /// Reading the key reads the balance too.
    ///
    /// They are one card and one session: signing in, signing in again and
    /// forgetting all move both halves, and a card whose halves disagreed
    /// about whether a session exists would be worse than either alone.
    @MainActor
    func testRefreshingTheCredentialAlsoRefreshesTheBalance() async throws {
        let daemon = RecordingDaemon()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        model.refreshNearAiCredential()

        var lastCount = daemon.methods.count
        var quietTicks = 0
        for _ in 0..<200 {
            try await Task.sleep(nanoseconds: 10_000_000)
            let count = daemon.methods.count
            if count == lastCount {
                quietTicks += 1
                if quietTicks >= 3 { break }
            } else {
                lastCount = count
                quietTicks = 0
            }
        }

        XCTAssertTrue(
            daemon.methods.contains(BalanceSurface.statusMethod),
            "the balance was never asked for: \(daemon.methods)")
        XCTAssertTrue(daemon.methods.contains(CredentialSurface.statusMethod))
    }
}
