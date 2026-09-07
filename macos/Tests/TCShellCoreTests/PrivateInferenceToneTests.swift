import XCTest

@testable import TCShellCore

/// The rule the whole destination hangs on: only `clear` may be painted as
/// working.
///
/// A tab badge and a tray glyph both invite a green dot, and this is the one
/// place a fail-open would be introduced by accident -- `refused` drawn as
/// "on" says a listener is answering calls when it refused to start.
final class PrivateInferenceToneTests: XCTestCase {
    func testOnlyClearReadsAsWorking() {
        XCTAssertTrue(PrivateInferenceTone.clear.readsAsWorking)
        for tone: PrivateInferenceTone in [.neutral, .held, .attention, .refused] {
            XCTAssertFalse(
                tone.readsAsWorking,
                "\(tone) must not be painted as working"
            )
        }
    }

    func testUnknownABIValuesDoNotReadAsWorking() {
        for raw: Int32 in [-1, 99, Int32.max, Int32.min] {
            XCTAssertFalse(
                PrivateInferenceTone.fromABI(raw).readsAsWorking,
                "unknown ABI value \(raw) must not be painted as working"
            )
        }
    }

    /// The ABI value the Rust assigns to `Clear`, and only that one, arrives
    /// as something an indicator may paint. Pinned separately so a renumbered
    /// or cross-wired mapper cannot pass on the enum test alone.
    func testOnlyTheClearABIValueArrivesAsWorking() {
        XCTAssertTrue(PrivateInferenceTone.fromABI(22).readsAsWorking)
        for raw: Int32 in [20, 21, 23, 24] {
            XCTAssertFalse(
                PrivateInferenceTone.fromABI(raw).readsAsWorking,
                "ABI value \(raw) must not be painted as working"
            )
        }
    }
}
