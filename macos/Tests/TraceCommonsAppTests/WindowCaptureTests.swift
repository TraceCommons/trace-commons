@testable import TCShellCore
import XCTest
import AppKit
import TCDesign

@testable import TraceCommonsApp

/// The screenshot hook's whole-window captures from a recorded sample set
/// (slice 1 of `2026-11-16-linux-mac-window-and-map-design.md`), named so
/// `scripts/parity/gallery.py` pairs each with its Linux capture.
@MainActor
final class WindowCaptureTests: XCTestCase {
    /// `gallery.py`'s `NAME`, kept in step by hand: a name the gallery
    /// cannot parse is listed there as unpaired, never shown beside Linux.
    static let galleryName = try! NSRegularExpression(
        pattern: #"^[A-Za-z]+-(home|inference|traces|insights)-main(\+map)?(\+inspector)?-(light|dark)\.png$"#)

    static func matchesGallery(_ name: String) -> Bool {
        galleryName.firstMatch(in: name, range: NSRange(name.startIndex..., in: name)) != nil
    }

    func test_everyTabIsCapturedWithAndWithoutTheMap() {
        let shots = DebugScreenshot.windowShots()
        XCTAssertEqual(Set(shots.map(\.tab)), Set(MonitorWindowView.Tab.allCases))
        for tab in MonitorWindowView.Tab.allCases {
            XCTAssertTrue(shots.contains(.init(tab: tab, showsMap: false, showsInspector: false)), "\(tab)")
            XCTAssertTrue(shots.contains(.init(tab: tab, showsMap: true, showsInspector: false)), "\(tab)")
        }
        XCTAssertEqual(shots.count, Set(shots.map { "\($0.tab)\($0.panes)" }).count, "a shot is repeated")
    }

    func test_theInspectorIsCapturedOnlyWhereTheTabsFirstPageHasOne() {
        let withInspector = DebugScreenshot.windowShots().filter(\.showsInspector)
        XCTAssertEqual(Set(withInspector.map(\.tab)), [.traces])
        XCTAssertEqual(withInspector.map(\.panes).sorted(), ["main+inspector", "main+map+inspector"])
    }

    func test_everyCaptureIsNamedForTheGallery() {
        for shot in DebugScreenshot.windowShots() {
            for appearance in ["light", "dark"] {
                let name = shot.fileName(set: .normalDay, appearance: appearance)
                XCTAssertTrue(Self.matchesGallery(name), name)
                XCTAssertTrue(name.hasPrefix("normalDay-"), name)
            }
        }
        XCTAssertEqual(
            DebugScreenshot.WindowShot(tab: .traces, showsMap: true, showsInspector: true)
                .fileName(set: .heldSessions, appearance: "dark"),
            "heldSessions-traces-main+map+inspector-dark.png")
    }

    func test_aShotIsAsWideAsItsPanes() {
        let main = DebugScreenshot.WindowShot(tab: .home, showsMap: false, showsInspector: false)
        let all = DebugScreenshot.WindowShot(tab: .traces, showsMap: true, showsInspector: true)
        XCTAssertEqual(main.width, GlassPaneLayout.windowWidth(showsMap: false, showsInspector: false))
        XCTAssertEqual(all.width, GlassPaneLayout.windowWidth(showsMap: true, showsInspector: true))
        XCTAssertLessThan(main.width, all.width)
    }

    /// The captures read the sample client alone: no live client, and no
    /// preview, real or recorded (the synthetic-preview rule).
    func test_windowCapturesReadOnlyTheSampleSet() throws {
        let hook = try MonitorNavigationTests.text("DebugScreenshot.swift")
        let start = try XCTUnwrap(hook.range(of: "static func captureWindows("))
        let end = try XCTUnwrap(hook.range(of: "#endif", range: start.upperBound ..< hook.endIndex))
        let body = hook[start.lowerBound ..< end.lowerBound]
        XCTAssertTrue(body.contains("DaemonDataWiring.sample(set)"))
        for live in ["model.daemonData", "model.liveData", "loadCaptureSample", "PreviewSheet(", "openPreview"] {
            XCTAssertFalse(body.contains(live), "a window capture reads \(live)")
        }
        XCTAssertTrue(body.contains(".defaultAppStorage(defaults)"), "the panes would be read from the person's defaults")
        // Debug-only, and false in a release build.
        let flag = try XCTUnwrap(hook.range(of: "static var capturesSampleWindows: Bool {"))
        let flagEnd = try XCTUnwrap(hook.range(of: "#endif", range: flag.upperBound ..< hook.endIndex))
        XCTAssertTrue(hook[flag.lowerBound ..< flagEnd.upperBound].contains("#else\n        false\n        #endif"))
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("if capture != nil { return }"), "a capture would attach the live client")
    }

    /// Every shot renders, at its panes' width, from the recorded set.
    func test_aSampleSetRendersOneImagePerShot() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-window-capture-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        await DebugScreenshot.captureWindows(set: .normalDay, model: AppModel(), to: directory.path)
        XCTAssertFalse(DebugScreenshot.capturesSampleWindows, "the live app would read onboarding as done")
        XCTAssertFalse(GlassMaterial.paintsForCapture, "the live app would draw painted glass")
        let written = try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted()
        let shots = DebugScreenshot.windowShots()
        XCTAssertEqual(written, shots.map { $0.fileName(set: .normalDay, appearance: "light") }.sorted())
        for shot in shots {
            let path = directory.appendingPathComponent(shot.fileName(set: .normalDay, appearance: "light")).path
            let image = try XCTUnwrap(NSImage(contentsOfFile: path).flatMap { $0.representations.first })
            XCTAssertEqual(CGFloat(image.pixelsWide), shot.width * 2, path)
        }
    }
}
