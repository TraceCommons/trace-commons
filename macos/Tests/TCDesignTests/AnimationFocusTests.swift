import AppKit
import Foundation
import SwiftUI
import XCTest

@testable import TCDesign

/// Animations run at their own rate only while their window is focused
/// (owner, 2026-11-20): a window that can be seen but is not focused draws
/// at most 15 frames a second, and one that cannot be seen draws none
/// (#1485). All of it is decided in `GlassAnimationGate` and applied in
/// `GlassAnimationTimeline`, the one clock every animated surface uses.
final class AnimationFocusTests: XCTestCase {
    // MARK: The rule

    func test_unfocusedRateIsFifteenFramesASecond() {
        XCTAssertEqual(GlassAnimationGate.unfocusedInterval, 1.0 / 15, accuracy: 1e-12)
    }

    /// Focused: the caller's own schedule, untouched, including no cap.
    func test_focusedRunsAtTheCallersRate() {
        for interval in [nil, 1.0 / 60, 1.0 / 30, 1.0 / 15, 1.0 / 5] as [Double?] {
            let schedule = GlassAnimationGate.schedule(minimumInterval: interval, paused: false, presence: .focused)
            XCTAssertEqual(schedule, GlassAnimationSchedule(minimumInterval: interval, paused: false))
        }
        XCTAssertEqual(GlassAnimationGate.schedule(minimumInterval: 1.0 / 30, paused: true, presence: .focused),
                       GlassAnimationSchedule(minimumInterval: 1.0 / 30, paused: true))
    }

    /// Seen but not focused: at most 15 frames a second. A faster request
    /// (or none, the display's rate) is slowed to 15; a slower one is
    /// never sped up.
    func test_unfocusedRunsAtMostFifteenFramesASecond() {
        let cap = GlassAnimationGate.unfocusedInterval
        func interval(_ requested: Double?) -> Double? {
            GlassAnimationGate.schedule(minimumInterval: requested, paused: false, presence: .unfocused).minimumInterval
        }
        XCTAssertEqual(interval(nil), cap)
        XCTAssertEqual(interval(1.0 / 60), cap)
        XCTAssertEqual(interval(1.0 / 30), cap)
        XCTAssertEqual(interval(1.0 / 15), cap)
        XCTAssertEqual(interval(1.0 / 5), 1.0 / 5)
        XCTAssertFalse(GlassAnimationGate.schedule(minimumInterval: 1.0 / 30, paused: false, presence: .unfocused).paused)
        XCTAssertTrue(GlassAnimationGate.schedule(minimumInterval: 1.0 / 30, paused: true, presence: .unfocused).paused)
    }

    /// Cannot be seen: stopped, whatever the caller asked (#1485).
    func test_hiddenIsStopped() {
        for interval in [nil, 1.0 / 60, 1.0 / 5] as [Double?] {
            XCTAssertTrue(GlassAnimationGate.schedule(minimumInterval: interval, paused: false, presence: .hidden).paused)
        }
    }

    /// The rule keeps no history: the first schedule after focus returns
    /// is the caller's full rate, not a ramp back up.
    func test_refocusIsFullRateAtOnce() {
        let sequence: [GlassWindowPresence] = [.focused, .unfocused, .hidden, .unfocused, .focused]
        let intervals = sequence.map {
            GlassAnimationGate.schedule(minimumInterval: 1.0 / 60, paused: false, presence: $0)
        }
        XCTAssertEqual(intervals.last, GlassAnimationSchedule(minimumInterval: 1.0 / 60, paused: false))
        XCTAssertEqual(intervals.first, intervals.last)
    }

    /// Presence from a window's state: hidden whenever it cannot be seen;
    /// otherwise focused when it is the key window, or the main window of
    /// the active app (a sheet or panel over it is key, and draws with the
    /// window's environment).
    func test_presenceFromWindowState() {
        XCTAssertEqual(GlassAnimationGate.presence(visible: false, isKey: true, isMain: true, appActive: true), .hidden)
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: true, isMain: true, appActive: true), .focused)
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: true, isMain: false, appActive: false), .focused,
                       "a key panel of an app that did not activate is focused")
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: false, isMain: true, appActive: true), .focused,
                       "the main window under its own sheet is focused")
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: false, isMain: true, appActive: false), .unfocused)
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: false, isMain: false, appActive: true), .unfocused,
                       "a second window of the active app, behind the focused one")
        XCTAssertEqual(GlassAnimationGate.presence(visible: true, isKey: false, isMain: false, appActive: false), .unfocused)
    }

    /// No window, or one never ordered in, cannot be seen.
    @MainActor
    func test_noWindowOrAWindowOffScreenIsHidden() {
        XCTAssertEqual(GlassAnimationGate.presence(nil), .hidden)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 40, height: 40),
                              styleMask: [.borderless], backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        XCTAssertEqual(GlassAnimationGate.presence(window), .hidden)
    }

    /// With no root keeping it (a preview, a test, an `ImageRenderer`
    /// image), a view is focused and animates at its own rate.
    func test_focusedByDefault() {
        XCTAssertEqual(EnvironmentValues().glassWindowPresence, .focused)
    }

    /// The probe asks again whenever focus can have moved, in either
    /// direction, not only when the window is brought forward.
    @MainActor
    func test_probeWatchesFocusLeavingAndReturning() {
        let names = Set(GlassWindowVisibilityProbe.ProbeView.windowNotifications)
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification,
                     NSWindow.didBecomeMainNotification, NSWindow.didResignMainNotification,
                     NSWindow.didChangeOcclusionStateNotification,
                     NSWindow.didMiniaturizeNotification, NSWindow.didDeminiaturizeNotification] {
            XCTAssertTrue(names.contains(name), "\(name.rawValue)")
        }
        let appNames = Set(GlassWindowVisibilityProbe.ProbeView.appNotifications)
        for name in [NSApplication.didBecomeActiveNotification, NSApplication.didResignActiveNotification,
                     NSApplication.didHideNotification, NSApplication.didUnhideNotification] {
            XCTAssertTrue(appNames.contains(name), "\(name.rawValue)")
        }
    }

    // MARK: Every animated surface uses the one clock

    /// Every Swift file under `Sources/` (both the design kit and the app),
    /// as (path relative to `Sources/`, text).
    private func allSources() throws -> [(String, String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
        // A scan that finds nothing would pass every rule it checks.
        XCTAssertGreaterThan(files.count, 100)
        return try files.map {
            (String($0.path.dropFirst(root.path.count + 1)), try String(contentsOf: $0, encoding: .utf8))
        }
    }

    /// No continuous animation is driven any other way: a `TimelineView`
    /// on the `.animation` schedule appears only inside
    /// `GlassAnimationTimeline`, and no animation repeats forever (SwiftUI
    /// would run it at the display's rate in any window). `.periodic`
    /// timelines (a clock ticking once a second or slower) are not
    /// animations and are not counted.
    func test_noAnimationBypassesTheSharedClock() throws {
        var offenders: [String] = []
        for (path, text) in try allSources() {
            for (number, line) in text.components(separatedBy: "\n").enumerated() {
                let code = line.trimmingCharacters(in: .whitespaces)
                if code.hasPrefix("//") { continue }
                if code.contains("TimelineView(.animation"), !path.hasSuffix("TCDesign/Components/AnimationTimeline.swift") {
                    offenders.append("\(path):\(number + 1)")
                }
                if code.contains(".repeatForever(")
                    || code.contains("CADisplayLink") || code.contains("CVDisplayLink") || code.contains("displayLink(") {
                    offenders.append("\(path):\(number + 1)")
                }
            }
        }
        XCTAssertEqual(offenders, [])
    }

    /// The surfaces named in the decision each draw through the shared
    /// clock: the Traces map (dashes, library, flow motion), the library
    /// station and orb, the Home hero's pixel glyph and field, the
    /// first-run stage and its screens, the spinner and the skeleton.
    func test_namedSurfacesUseTheSharedClock() throws {
        let sources = Dictionary(try allSources(), uniquingKeysWith: { first, _ in first })
        let surfaces: [(String, Int)] = [
            ("TraceCommonsApp/Views/Monitor/FlowMapView.swift", 4),
            ("TCDesign/Components/LibraryStation.swift", 1),
            ("TCDesign/Components/LibraryOrb.swift", 1),
            ("TCDesign/Components/PixelGlyph.swift", 1),
            ("TCDesign/Components/PixelField.swift", 1),
            ("TraceCommonsApp/Views/FirstRun/FirstRunStage.swift", 2),
            ("TraceCommonsApp/Views/FirstRun/WelcomeScreen.swift", 1),
            ("TCDesign/Components/Feedback.swift", 2),
        ]
        for (path, atLeast) in surfaces {
            let text = try XCTUnwrap(sources[path], path)
            let uses = text.components(separatedBy: "GlassAnimationTimeline(").count - 1
            XCTAssertGreaterThanOrEqual(uses, atLeast, path)
        }
    }

    /// The Traces map's dashes and library run on timelines of their own
    /// (#2268), each through the shared clock, so focus applies to each:
    /// focused, dashes at 30 and the library at 15 frames a second; seen
    /// but not focused, both at 15; hidden, both stopped.
    @MainActor
    func test_mapDashAndLibraryTimelinesFollowFocus() throws {
        let sources = Dictionary(try allSources(), uniquingKeysWith: { first, _ in first })
        let map = try XCTUnwrap(sources["TraceCommonsApp/Views/Monitor/FlowMapView.swift"])
        XCTAssertTrue(map.contains("GlassAnimationTimeline(minimumInterval: FlowMapView.dashInterval, paused: !resting.dashes)"))
        XCTAssertTrue(map.contains("GlassAnimationTimeline(minimumInterval: GlassLibraryStation.frameInterval, paused: !resting.turns)"))
        XCTAssertTrue(map.contains("static let dashInterval: Double = 1 / 30"))

        let dashes = 1.0 / 30
        let library = GlassLibraryStation.frameInterval
        XCTAssertEqual(library, 1.0 / 15, accuracy: 1e-12)
        func interval(_ requested: Double, _ presence: GlassWindowPresence) -> GlassAnimationSchedule {
            GlassAnimationGate.schedule(minimumInterval: requested, paused: false, presence: presence)
        }
        XCTAssertEqual(interval(dashes, .focused), GlassAnimationSchedule(minimumInterval: dashes, paused: false))
        XCTAssertEqual(interval(library, .focused), GlassAnimationSchedule(minimumInterval: library, paused: false))
        XCTAssertEqual(interval(dashes, .unfocused).minimumInterval, GlassAnimationGate.unfocusedInterval)
        XCTAssertEqual(interval(library, .unfocused).minimumInterval, GlassAnimationGate.unfocusedInterval)
        XCTAssertTrue(interval(dashes, .hidden).paused)
        XCTAssertTrue(interval(library, .hidden).paused)
    }

    /// The clock reads the window's presence and applies the rule to it,
    /// so every surface above follows focus without code of its own.
    func test_theClockAppliesTheRule() throws {
        let sources = Dictionary(try allSources(), uniquingKeysWith: { first, _ in first })
        let clock = try XCTUnwrap(sources["TCDesign/Components/AnimationTimeline.swift"])
        XCTAssertTrue(clock.contains("@Environment(\\.glassWindowPresence)"))
        XCTAssertTrue(clock.contains("GlassAnimationGate.schedule(minimumInterval: minimumInterval, paused: paused, presence: presence)"))
    }

    // MARK: The skeleton

    /// The skeleton breathes from full opacity down to `skeletonDim` and
    /// back once per `motion.pulse`, on the token curve, now a function of
    /// the time so it can draw through the shared clock.
    @MainActor
    func test_skeletonBreathesOverOnePulse() {
        let pulse = GlassTokens.Motion.pulse
        let dim = GlassTokens.Opacity.skeletonDim
        XCTAssertEqual(GlassSkeleton.opacity(at: 0, reduceMotion: false), 1, accuracy: 1e-9)
        XCTAssertEqual(GlassSkeleton.opacity(at: pulse / 2, reduceMotion: false), dim, accuracy: 1e-6)
        XCTAssertEqual(GlassSkeleton.opacity(at: pulse, reduceMotion: false), 1, accuracy: 1e-6)
        XCTAssertEqual(GlassSkeleton.opacity(at: pulse * 7 + pulse / 2, reduceMotion: false), dim, accuracy: 1e-6)
        let quarter = GlassSkeleton.opacity(at: pulse / 4, reduceMotion: false)
        XCTAssertLessThan(quarter, 1)
        XCTAssertGreaterThan(quarter, dim)
        XCTAssertEqual(GlassSkeleton.opacity(at: pulse / 4, reduceMotion: false),
                       GlassSkeleton.opacity(at: pulse * 3 / 4, reduceMotion: false), accuracy: 1e-9,
                       "the way back is the way down, reversed")
        XCTAssertEqual(GlassSkeleton.opacity(at: pulse / 2, reduceMotion: true), 1)
    }
}
