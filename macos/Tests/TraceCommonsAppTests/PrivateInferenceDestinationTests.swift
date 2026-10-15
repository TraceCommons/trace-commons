import SwiftUI
import TCDesign
import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The Private AI destination, and the rule its indicator hangs on.
///
/// The label is never asserted against a spelling here. It comes from the
/// Rust copy payload, and a test that retyped it would be the second place
/// the words live -- which is the thing the copy module exists to prevent.
final class PrivateInferenceDestinationTests: XCTestCase {
    /// The destination's name is the Rust's word, read through the copy
    /// payload -- never retyped in Swift. (The legacy sidebar's row, glyph
    /// and Cmd-N shortcut left with the legacy window, R15; the Monitor's
    /// tabs and `MonitorCommands` are pinned in `PrivateInferenceMenuBarTests`.)
    @MainActor
    func testTheNameComesFromTheCopyPayload() throws {
        let copy = try XCTUnwrap(AppModel().privateInferenceCopy)
        XCTAssertFalse(copy.destination.isEmpty)
        XCTAssertFalse(copy.subtitle.isEmpty)
    }

    /// The switch reports what was asked for; the indicator reports what is
    /// true. A refusal under an on switch must not read as working.
    func testIndicatorDoesNotFollowTheSwitch() {
        let state = PrivateInferenceState(label: "port_in_use", port: nil)
        let tone = PrivateInferenceSurface.tone(state, calls: .testing)
        XCTAssertFalse(tone.readsAsWorking)
        XCTAssertFalse(PrivateInferenceIndicator.readsAsWorking(state, calls: .testing))
    }

    /// Held, attention, refused and anything unknown are drawn differently
    /// from clear. On glass the status is a dot, which is colour alone; the
    /// difference that survives greyscale is the core's state sentence every
    /// `GlassStatusLabel` draws beside it, so this pins only that no other
    /// tone shares clear's status.
    func testEveryNonClearToneIsVisiblyDistinctFromClear() {
        let clear = PrivateInferenceIndicator.status(.clear)
        for tone: PrivateInferenceTone in [.neutral, .held, .attention, .refused] {
            XCTAssertNotEqual(PrivateInferenceIndicator.status(tone), clear, "\(tone) shares clear's status")
        }
        for raw: Int32 in [-1, 0, 99, Int32.max, Int32.min] {
            let tone = PrivateInferenceTone.fromABI(raw)
            XCTAssertFalse(tone.readsAsWorking)
            XCTAssertNotEqual(PrivateInferenceIndicator.status(tone), clear)
        }
    }
}

extension PrivateInferenceCalls {
    /// A daemon that answers every state the way the shared table does, with
    /// no dylib behind it.
    ///
    /// Both branch tables are transcribed from `private_inference_copy.rs`
    /// -- `state_tone` and `quit_needs_notice` -- because a double that
    /// disagrees with the real table leaves every test reasoning about
    /// held-vs-attention or the quit notice reasoning against a table no
    /// contributor will ever meet. `port_in_use` is `Refused` (ABI 24);
    /// `running_elsewhere` is `Held` (21) and not attention, because foreign
    /// ownership is not this app's work to draw attention to -- and for the
    /// same reason it needs no quit notice, while `running` and `stopping`
    /// need one whatever the switch says.
    static let testing = PrivateInferenceCalls(
        stateLine: { $0.isEmpty ? nil : $0 },
        stateTone: { label in
            switch label {
            case "running": return 22
            case "running_no_backends": return 23
            case "running_elsewhere", "stopping": return 21
            case "port_in_use", "start_failed", "crashed": return 24
            default: return 20
            }
        },
        servingLine: { _ in nil },
        shouldOffer: { answered, on in !answered && !on },
        quitNeedsNotice: { on, label in
            switch label {
            case "off", "running_elsewhere": return false
            case "running", "running_no_backends", "stopping": return true
            default: return on
            }
        }
    )
}
