import SwiftUI
import XCTest
import TCBridge
import TCDesign
import TCShellCore
@testable import TraceCommonsApp

/// The menu-bar section, and the shortcuts that reach this surface.
final class PrivateInferenceMenuBarTests: XCTestCase {
    /// The menu bar is the surface most likely to be read at a glance and
    /// least likely to be read carefully, so the fail-open matters most
    /// here: none of these states may be drawn the way a working one is.
    func testTheMenuBarPillFollowsToneNotSwitch() {
        for label in ["port_in_use", "start_failed", "crashed", "stopping", "unknown_state", ""] {
            let menu = PrivateInferenceSurface.menuState(
                PrivateInferenceState(label: label, port: nil), calls: Self.coreCalls)
            let state = MenuPanelStatus.privateAI(on: true, menu: menu)
            XCTAssertNotEqual(
                MenuPanelStatus.privateAIDot(state), .on,
                "\(label) is drawn green in the menu bar because the switch is on")
        }
    }

    /// The core's calls, as `AppModel` wires them.
    static let coreCalls = PrivateInferenceCalls(
        stateLine: { TCPrivateInference.stateLine(state: $0) },
        stateTone: { TCPrivateInference.stateTone(state: $0) },
        servingLine: { TCPrivateInference.servingLine(port: $0) },
        shouldOffer: { TCPrivateInference.shouldOffer(answered: $0, on: $1) },
        quitNeedsNotice: { TCPrivateInference.quitNeedsNotice(on: $0, state: $1) },
        menuWord: { TCPrivateInference.menuWord(state: $0) },
        menuTone: { TCPrivateInference.menuTone(state: $0) })

    /// The row is one word, with the dot carrying the difference (owner,
    /// 2026-10-28): every daemon label, from the core's table.
    func testTheRowIsOneWordAndTheDotCarriesTheDifference() {
        let table: [(label: String, word: String, dot: GlassStatus)] = [
            ("running", "On", .on),
            ("running_destination_unknown", "On", .ask),
            ("running_answered_elsewhere", "On", .ask),
            ("running_no_backends", "On", .ask),
            ("running_elsewhere", "On", .ask),
            ("stopping", "Stopping", .off),
            ("off", "Off", .off),
            ("port_in_use", "Not working", .outside),
            ("start_failed", "Not working", .outside),
            ("crashed", "Not working", .outside),
            ("", "Unknown", .off),
            ("a_state_from_a_later_daemon", "Unknown", .off),
        ]
        for row in table {
            let menu = PrivateInferenceSurface.menuState(
                PrivateInferenceState(label: row.label, port: nil), calls: Self.coreCalls)
            let state = MenuPanelStatus.privateAI(on: true, menu: menu)
            XCTAssertEqual(menu?.word, row.word, row.label)
            XCTAssertEqual(MenuPanelStatus.privateAIDot(state), row.dot, row.label)
            // Never the sentence the page shows.
            XCTAssertNotEqual(menu?.word, TCPrivateInference.stateLine(state: row.label), row.label)
        }
    }

    /// The popover's destination-unknown row reads "On" on an amber dot, not
    /// the truncated sentence on a red one.
    func testDestinationUnknownIsOnAmber() {
        let menu = PrivateInferenceSurface.menuState(
            PrivateInferenceState(label: "running_destination_unknown", port: 1), calls: Self.coreCalls)
        XCTAssertEqual(menu?.word, "On")
        XCTAssertEqual(menu?.tone, .attention)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.attention), .ask)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.refused), .outside)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.clear), .on)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.neutral), .off)
        XCTAssertEqual(MenuPanelStatus.privateAIDot(.held), .off)
    }

    /// The Private AI tab's Status row draws the menu bar's word and dot,
    /// with the page's full sentence kept for its help and hint. Signed out
    /// of NEAR AI it read "On. Couldn't tell which account is answering."
    /// on a red dot.
    func testTheTabStatusRowIsTheMenuBarWordAndDot() throws {
        let copy = try XCTUnwrap(PrivateInferenceCopy.decode(fromJSON: TCPrivateInference.copyJSON() ?? ""))
        let labels = ["running", "running_destination_unknown", "running_answered_elsewhere", "running_no_backends",
                      "running_elsewhere", "stopping", "off", "port_in_use", "start_failed", "crashed", "",
                      "a_state_from_a_later_daemon"]
        for label in labels {
            let state = DaemonData.PrivateInferenceState(state: label, port: 1)
            let row = PrivateAIInspectorView.statusRow(state, copy: copy, calls: Self.coreCalls)
            let menu = try XCTUnwrap(PrivateInferenceSurface.menuState(
                PrivateInferenceState(label: label, port: 1), calls: Self.coreCalls), label)
            XCTAssertEqual(row.word, menu.word, label)
            XCTAssertEqual(row.dot, MenuPanelStatus.privateAIDot(menu.tone), label)
            XCTAssertEqual(row.sentence, PrivateInferenceSurface.stateLine(
                PrivateInferenceState(label: label, port: 1), copy: copy, calls: Self.coreCalls), label)
        }
        let elsewhere = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: "running_answered_elsewhere", port: 1), copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(elsewhere.word, "On")
        XCTAssertEqual(elsewhere.dot, .ask, "attention is amber, not red")
        let unknown = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: "running_destination_unknown", port: 1), copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(unknown.dot, .ask, "attention is amber, not red")
        let crashed = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: "crashed", port: nil), copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(crashed.dot, .outside, "refused stays red")
        let running = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: "running", port: 1), copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(running.dot, .on)
        let unreported = PrivateAIInspectorView.statusRow(nil, copy: copy, calls: Self.coreCalls)
        XCTAssertEqual(unreported.dot, .off, "no report is grey, never green")
        // A core that wires no word: the sentence, on grey.
        let noWord = PrivateAIInspectorView.statusRow(
            DaemonData.PrivateInferenceState(state: "running", port: 1), copy: copy, calls: .testing)
        XCTAssertEqual(noWord.word, noWord.sentence)
        XCTAssertEqual(noWord.dot, .off)
    }

    /// A caller that wires no word draws no word, not a guessed one.
    func testNoWordFromTheCoreIsUnknownNotOn() {
        XCTAssertNil(PrivateInferenceSurface.menuState(
            PrivateInferenceState(label: "running", port: 1), calls: .testing))
        XCTAssertEqual(MenuPanelStatus.privateAI(on: true, menu: nil), .unknown)
        XCTAssertEqual(MenuPanelStatus.privateAI(on: false, menu: ("On", .clear)), .off)
        XCTAssertEqual(MenuPanelStatus.privateAI(on: nil, menu: ("On", .clear)), .unknown)
    }

    /// Clicking the row opens the Private AI page in Settings.
    func testTheRowOpensPrivateAIInSettings() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift")
        let source = try String(contentsOf: url, encoding: .utf8)
        XCTAssertTrue(source.contains("PrivateAIStatusLabel { open(.settings(.privateAI)) }"))
        XCTAssertEqual(MonitorDestination.settings(.privateAI).settingsSection, .privateAI)
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
        // Insights is the fourth tab (2026-10-15).
        XCTAssertEqual(tabs, ["1", "2", "3", "4"])
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
        XCTAssertEqual(MonitorCommands.destination(.insights), .insights)
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
