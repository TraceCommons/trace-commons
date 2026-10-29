import AppKit
import SwiftUI
import TCBridge
import TCDesign
import XCTest
@testable import TraceCommonsApp

/// Direction B (2026-10-15): the Insights tabs are laid out for the main
/// pane at the window's default width, 440pt, which leaves 408pt inside the
/// pane's insets. This renders each tab there from the sample week
/// (`InsightsSampleWeek`, the one `TRACE_COMMONS_INSIGHTS_SAMPLE=1` shows in a
/// dev build) through the real models, so the layout can be looked at
/// without a store.
///
/// Set `TC_INSIGHTS_RENDER_DIR` to keep the PNGs, and `TC_GLASS_THEME=flat`
/// to draw them in the theme the app ships with (a test run is classic).
final class InsightsPaneWidthRenderTests: XCTestCase {
    /// `GlassPaneLayout.mainWidth` less the main pane's insets on each side.
    static var paneContentWidth: CGFloat {
        GlassPaneLayout.mainWidth - 2 * MonitorWindowView.mainInset
    }

    func test_thePaneContentWidthIs408AtTheDefaultWindow() {
        XCTAssertEqual(Self.paneContentWidth, 408)
    }

    @MainActor
    func test_everyTabRendersAtThePaneWidth() async throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let service: InsightsModel.Service = { request in
            guard let answer = try InsightsSampleWeek.response(request.operation) else {
                throw InsightsError.invalidResponse
            }
            return answer
        }

        let overview = InsightsOverviewModel(service: service)
        let patterns = InsightsPatternsModel(service: service)
        let sessions = InsightsSessionsModel(service: service)
        let comparisons = InsightsComparisonsModel(service: service)
        let week = try InsightsSampleWeek.weekOverview
        let pattern = try InsightsSampleWeek.weekPatterns
        overview.open(); patterns.open(); sessions.open()
        overview.showCounter(week)
        patterns.showCounter(pattern)
        sessions.sync(snapshotIDs: ["snap-1"])
        comparisons.load(counterWeeks: [], weekStart: week.week_start, recapCardEnabled: true)
        try await settle { !overview.busy && !patterns.busy && !sessions.busy && !comparisons.busy }
        XCTAssertNotNil(comparisons.comparisons?.recap)
        XCTAssertNotNil(sessions.drill)

        let tabs: [(String, CGFloat, AnyView)] = [
            ("overview", 1860, AnyView(InsightsOverviewTab(
                model: overview, comparisons: comparisons, copy: copy, snapshots: []))),
            ("patterns", 1900, AnyView(InsightsPatternsTab(model: patterns, comparisons: comparisons, copy: copy))),
            ("sessions", 1000, AnyView(InsightsSessionsTab(model: sessions, snapshots: [], copy: copy))),
        ]
        for (name, height, view) in tabs {
            for dark in [true, false] {
                let png = try Self.render(view, height: height, dark: dark)
                XCTAssertGreaterThan(png.count, 20_000, name)
                if let dir = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_DIR"] {
                    try png.write(to: URL(fileURLWithPath: dir)
                        .appendingPathComponent("insights-\(name)-\(dark ? "dark" : "light").png"))
                }
            }
        }
    }

    @MainActor
    private func settle(_ done: () -> Bool) async throws {
        for _ in 0 ..< 200 where !done() { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertTrue(done(), "the models did not settle")
    }

    /// The tab on a stand-in for the pane's glass at the pane's content width.
    @MainActor
    static func render(_ view: AnyView, height: CGFloat, dark: Bool) throws -> Data {
        _ = NSApplication.shared
        let size = CGSize(width: paneContentWidth + 2 * MonitorWindowView.mainInset, height: height)
        let content = view
            .frame(width: paneContentWidth, height: height)
            .padding(.horizontal, MonitorWindowView.mainInset)
            .background {
                // Glass over a mid-grey desktop: the veil the app paints over
                // its material, so cards and plots read as they do on screen.
                ZStack { Color(red: 0.36, green: 0.40, blue: 0.47); GlassTokens.Color.glassVeil.color }
            }
            .environment(\.colorScheme, dark ? .dark : .light)
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }
}
