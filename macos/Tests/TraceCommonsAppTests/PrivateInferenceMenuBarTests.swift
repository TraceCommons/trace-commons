import SwiftUI
import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// The menu-bar section, and the shortcuts that reach this surface.
final class PrivateInferenceMenuBarTests: XCTestCase {
    /// The menu bar is the surface most likely to be read at a glance and
    /// least likely to be read carefully, so the fail-open matters most
    /// here: none of these states may be drawn the way a working one is.
    func testMenuBarGlyphFollowsToneNotSwitch() {
        let working = MenuBarContent.privateInferenceSymbol(
            PrivateInferenceState(label: "running", port: 8080), calls: .testing)
        for label in ["port_in_use", "start_failed", "crashed", "stopping", "unknown_state", ""] {
            let state = PrivateInferenceState(label: label, port: nil)
            XCTAssertFalse(
                PrivateInferenceSurface.tone(state, calls: .testing).readsAsWorking,
                "\(label) must not read as working in the menu bar")
            XCTAssertNotEqual(
                MenuBarContent.privateInferenceSymbol(state, calls: .testing), working,
                "\(label) is drawn with the working glyph in the menu bar")
        }
    }

    /// The symbol is a function of the reported state alone. The menu bar is
    /// handed no switch to read, so it cannot accidentally read one.
    func testTheMenuBarSymbolIsAFunctionOfTheReportedStateAlone() {
        let refused = PrivateInferenceState(label: "port_in_use", port: nil)
        XCTAssertEqual(
            MenuBarContent.privateInferenceSymbol(refused, calls: .testing),
            PrivateInferenceIndicator.palette(.refused).symbol)
        XCTAssertEqual(
            MenuBarContent.privateInferenceSymbol(
                PrivateInferenceState(label: "running", port: 8080), calls: .testing),
            PrivateInferenceIndicator.palette(.clear).symbol)
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
        XCTAssertEqual(MenuBarContent.privateInferenceTrayAction(on: false), .openDestination)
        XCTAssertEqual(MenuBarContent.privateInferenceTrayAction(on: true), .stopAnswering)

        var wrote = false
        var opened = false
        MenuBarContent.performPrivateInferenceTray(
            on: false, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertFalse(wrote, "the menu wrote a setting to turn model calls on")
        XCTAssertTrue(opened, "the menu did not open the screen that explains what it exposes")

        wrote = false
        opened = false
        MenuBarContent.performPrivateInferenceTray(
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
        XCTAssertEqual(MenuBarContent.privateInferenceTrayLabel(on: true, copy: copy), copy.trayTurnOff)
        XCTAssertEqual(
            MenuBarContent.privateInferenceTrayLabel(on: false, copy: copy), copy.trayOpenToTurnOn)
        XCTAssertNotEqual(copy.trayTurnOff, copy.trayOpenToTurnOn)
    }

    /// The action follows the switch, never the tone. A listener that
    /// refused to start still leaves something to turn off, and the row that
    /// turns it off must not disappear because nothing is running.
    func testTheActionDoesNotFollowTheIndicator() {
        for label in ["port_in_use", "start_failed", "crashed"] {
            let state = PrivateInferenceState(label: label, port: nil)
            XCTAssertFalse(PrivateInferenceSurface.tone(state, calls: .testing).readsAsWorking)
            XCTAssertEqual(MenuBarContent.privateInferenceTrayAction(on: true), .stopAnswering)
        }
    }

    /// The toggle's shortcut is in-app only and collides with none of the
    /// five destination shortcuts.
    @MainActor
    func testTheToggleShortcutDoesNotCollideWithADestination() {
        let destinations = MainWindowView.Section.allCases.compactMap(\.shortcut)
        XCTAssertEqual(MainWindowCommands.toggleModifiers, [.command, .shift])
        XCTAssertEqual(MainWindowCommands.destinationModifiers, [.command])
        XCTAssertFalse(
            destinations.contains(MainWindowCommands.toggleKey),
            "the toggle shares a key with a destination")
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
        MenuBarContent.performPrivateInferenceTray(
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
        MenuBarContent.performPrivateInferenceTray(
            on: true, turnOff: { wrote = true }, open: { opened = true })
        XCTAssertTrue(wrote, "the on direction stops answering")
        XCTAssertTrue(opened, "the press that records the asking must show the words")
    }

    /// The label follows the same table, so the menu cannot offer to turn it
    /// on while the action opens a screen, or the reverse.
    func testTheShortcutLabelMatchesItsAction() {
        for on in [true, false] {
            let action = MenuBarContent.privateInferenceTrayAction(on: on)
            XCTAssertEqual(
                action == .stopAnswering, on,
                "the action and the switch position must agree")
        }
    }
}
