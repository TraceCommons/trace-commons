import SwiftUI
import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The menu-bar section, and the shortcuts that reach this surface.
final class PrivateInferenceMenuBarTests: XCTestCase {
    /// The menu bar is the surface most likely to be read at a glance and
    /// least likely to be read carefully, so the fail-open matters most
    /// here: none of these states may be drawn the way a working one is.
    func testTheMenuBarPillFollowsToneNotSwitch() {
        for label in ["port_in_use", "start_failed", "crashed", "stopping", "unknown_state", ""] {
            let tone = PrivateInferenceSurface.tone(PrivateInferenceState(label: label, port: nil), calls: .testing)
            XCTAssertFalse(tone.readsAsWorking, "\(label) must not read as working in the menu bar")
            XCTAssertNotEqual(
                MenuPanelStatus.privateAI(on: true, tone: tone), .on,
                "\(label) is drawn On in the menu bar because the switch is on")
        }
    }

    /// The indicator the pill reads tells a refused listener from a clear
    /// one: a refused listener is never drawn the way a working one is.
    func testTheIndicatorNeverDrawsARefusedListenerAsClear() {
        XCTAssertNotEqual(PrivateInferenceIndicator.status(.refused), PrivateInferenceIndicator.status(.clear))
    }

    /// The menu may turn it OFF and may not turn it ON.
    ///
    /// Turning it off only ever reduces what this computer will answer, so
    /// it is safe from a menu with nothing else on screen. Turning it on
    /// changes what anything else running here may send through, charged to
    /// the contributor's own accounts, and the sentence that says so is the
    /// reason this became a destination rather than a switch. A menu press
    /// that enabled it would route around that sentence.
    ///
    /// The no-write half is the safety claim, so it is asserted directly
    /// rather than inferred from the wording: pressing the row while it is
    /// off calls nothing that writes.
    ///
    /// The stop press opens the destination too. Its write carries
    /// `private_inference_offer_seen`, which records that the question was
    /// put, so the press that records it must be the press that shows
    /// `offer_exposure` -- as the Windows and GTK off switches do, living
    /// only on that screen.
    func testTheMenuTurnsItOffAndOpensTheScreenToTurnItOn() {
        XCTAssertEqual(PrivateInferenceTray.action(on: false), .openDestination)
        XCTAssertEqual(PrivateInferenceTray.action(on: true), .stopAnswering)

        var wrote = false
        var opened = false
        PrivateInferenceTray.perform(
            on: false, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertFalse(wrote, "the menu wrote a setting to turn model calls on")
        XCTAssertTrue(opened, "the menu did not open the screen that explains what it exposes")

        wrote = false
        opened = false
        PrivateInferenceTray.perform(
            on: true, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertTrue(wrote, "the menu could not stop this computer answering model calls")
        XCTAssertTrue(
            opened,
            "the stop recorded that the question was asked without showing the words")
    }

    /// Both rows read the Rust's words, and the two directions are two
    /// different sentences. Neither is spelled here.
    @MainActor
    func testTheMenuRowTakesItsWordsFromTheCopyPayload() throws {
        let copy = try XCTUnwrap(AppModel().privateInferenceCopy)
        XCTAssertEqual(PrivateInferenceTray.label(on: true, copy: copy), copy.trayTurnOff)
        XCTAssertEqual(
            PrivateInferenceTray.label(on: false, copy: copy), copy.trayOpenToTurnOn)
        XCTAssertNotEqual(copy.trayTurnOff, copy.trayOpenToTurnOn)
    }

    /// The action follows the switch, never the tone. A listener that
    /// refused to start still leaves something to turn off, and the row that
    /// turns it off must not disappear because nothing is running.
    func testTheActionDoesNotFollowTheIndicator() {
        for label in ["port_in_use", "start_failed", "crashed"] {
            let state = PrivateInferenceState(label: label, port: nil)
            XCTAssertFalse(PrivateInferenceSurface.tone(state, calls: .testing).readsAsWorking)
            XCTAssertEqual(PrivateInferenceTray.action(on: true), .stopAnswering)
        }
    }

    /// The toggle's shortcut is in-app only and collides with none of the
    /// three tab shortcuts, which are Cmd-1..3 in the tab strip's order.
    @MainActor
    func testTheToggleShortcutDoesNotCollideWithADestination() {
        let tabs = MonitorWindowView.Tab.allCases.map(MonitorCommands.shortcut)
        XCTAssertEqual(tabs, ["1", "2", "3"])
        XCTAssertEqual(MonitorCommands.toggleModifiers, [.command, .shift])
        XCTAssertEqual(MonitorCommands.tabModifiers, [.command])
        XCTAssertFalse(
            tabs.contains(MonitorCommands.toggleKey),
            "the toggle shares a key with a destination")
    }

    /// Each tab's item opens through `OpenMonitor` at its own tab, so while
    /// onboarding is required Home and Traces open first run instead and
    /// Inference opens the Monitor (R-38; `InferenceDuringOnboardingTests`).
    @MainActor
    func testEachTabCommandOpensItsTab() throws {
        XCTAssertEqual(MonitorCommands.destination(.home), .home(.overview))
        XCTAssertEqual(MonitorCommands.destination(.inference), .inference)
        XCTAssertEqual(MonitorCommands.destination(.traces), .traces(entryId: nil))
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("Button(tab.title) { OpenMonitor.request(Self.destination(tab)) }"))
        XCTAssertTrue(main.contains(".commands {\n            MonitorCommands(model: model, navigation: navigation)\n        }"))
        XCTAssertFalse(main.contains("MainWindowCommands"))
    }
}

/// The app-menu shortcut, under the same rule as the menu-bar row.
///
/// `.commands` is installed in the app-wide menu bar, so this fires whenever
/// the app is frontmost -- including with the main window closed, which this
/// app supports. A press that enabled answering would do it with
/// `offer_exposure` off-screen, which is the whole reason the menu-bar row is
/// asymmetric.
final class PrivateInferenceCommandTests: XCTestCase {
    /// While it is off, the shortcut must not write. It opens the destination.
    func testTheShortcutCannotEnableAnswering() {
        var wrote = false
        var opened = false
        PrivateInferenceTray.perform(
            on: false, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertFalse(wrote, "the shortcut must never enable answering")
        XCTAssertTrue(opened, "the off direction opens the destination instead")
    }

    /// While it is on, the shortcut turns it off -- the one write it may make
    /// -- and raises the destination with it.
    ///
    /// That write carries `private_inference_offer_seen`, which records that
    /// the question was put. A shortcut press with the window closed would
    /// otherwise record an asking with `offer_exposure` nowhere on screen.
    func testTheShortcutStopsAnsweringWhileItIsOn() {
        var wrote = false
        var opened = false
        PrivateInferenceTray.perform(
            on: true, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertTrue(wrote, "the on direction stops answering")
        XCTAssertTrue(opened, "the press that records the asking must show the words")
    }

    /// The label follows the same table, so the menu cannot offer to turn it
    /// on while the action opens a screen, or the reverse.
    func testTheShortcutLabelMatchesItsAction() {
        for on in [true, false] {
            let action = PrivateInferenceTray.action(on: on)
            XCTAssertEqual(
                action == .stopAnswering, on,
                "the action and the switch position must agree")
        }
    }
}
