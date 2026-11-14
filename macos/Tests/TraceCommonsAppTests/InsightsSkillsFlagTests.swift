import AppKit
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class InsightsSkillsFlagTests: XCTestCase {
    private actor Reads {
        private(set) var count = 0
        func read() -> DaemonData.SkillsSnapshot {
            count += 1
            return .init(enabled: true)
        }
    }

    /// The same authoritative typed snapshot injection used by the feature
    /// flag integration tests. Nothing here changes build or daemon defaults.
    private func snapshot(_ enabled: Bool?) throws -> FeatureFlagSnapshot {
        var flags: [String: Any] = [:]
        if let enabled {
            flags["insights_skills"] = ["enabled": enabled, "pending": false,
                "capability_blocked": false, "timing": "live"]
        }
        let object: [String: Any] = ["schema_version": 1, "environment": "testing",
            "profile_digest": String(repeating: "a", count: 64), "instance_id": UUID().uuidString,
            "generation": 1, "sampled_at": "2026-11-05T20:00:00Z", "source": "default",
            "fresh": false, "flags": flags]
        return try JSONDecoder().decode(FeatureFlagSnapshot.self, from: JSONSerialization.data(withJSONObject: object))
    }

    func testMissingFalseAndTrueUseTheAuthoritativeSnapshot() throws {
        let app = AppModel()
        defer { app.shutdown() }
        XCTAssertFalse(app.featureIsEnabled("insights_skills"))
        for enabled in [nil, false, true] as [Bool?] {
            app.featureFlags = try snapshot(enabled)
            XCTAssertEqual(InsightsView.enabledShownTabs(isSkillsEnabled: app.featureIsEnabled("insights_skills")),
                enabled == true ? [.overview, .patterns, .benchmarks, .skills] : [.overview, .patterns, .benchmarks])
        }
    }

    func testMissingOrDisabledFlagNeverStartsSkillsReadsEvenWhenInitiallySelected() async throws {
        for enabled in [nil, false] as [Bool?] {
            let app = AppModel()
            app.featureFlags = try snapshot(enabled)
            let reads = Reads()
            let skills = InsightsSkillsModel(reader: { _ in await reads.read() }, idlePollInterval: .milliseconds(5))
            let hosted = Hosted(app: app, skills: skills)
            await hosted.drain()
            let count = await reads.count
            XCTAssertEqual(count, 0)
            XCTAssertNil(skills.shown)
            await hosted.close()
            app.shutdown()
        }
    }

    func testLiveDisableUnmountsSkillsAndResetsSelectionBeforeReenable() async throws {
        let app = AppModel()
        defer { app.shutdown() }
        app.featureFlags = try snapshot(true)
        let reads = Reads()
        let skills = InsightsSkillsModel(reader: { _ in await reads.read() }, idlePollInterval: .milliseconds(5))
        let hosted = Hosted(app: app, skills: skills)
        for _ in 0 ..< 100 {
            if await reads.count > 0 { break }
            await hosted.drain()
        }
        let enabledReads = await reads.count
        XCTAssertGreaterThan(enabledReads, 0, "The explicitly enabled selected Skills tab must mount")
        app.featureFlags = try snapshot(false)
        await hosted.drain()
        await hosted.drain()
        let disabledReads = await reads.count
        await hosted.drain()
        let stillDisabledReads = await reads.count
        XCTAssertEqual(stillDisabledReads, disabledReads)
        // Reenable without selecting Skills again. A stale .skills selection
        // would remount its task here; the required Overview fallback does not.
        app.featureFlags = try snapshot(true)
        await hosted.drain()
        await hosted.drain()
        let reenabledReads = await reads.count
        XCTAssertEqual(reenabledReads, disabledReads, "Live disable must reset the stored selection to Overview")
        await hosted.close()
    }

    private struct FlagHost: View {
        @ObservedObject var app: AppModel
        let model: InsightsModel
        let skills: InsightsSkillsModel
        var body: some View {
            InsightsView(storeSelection: .standard, storeCopy: TCInsights.copy(), model: model,
                         initialTab: .skills, skills: skills,
                         isSkillsEnabled: app.featureIsEnabled("insights_skills"))
                .environment(\.scenePhase, .active)
        }
    }

    @MainActor private final class Hosted {
        let hosting: NSHostingView<AnyView>
        let window: NSWindow
        init(app: AppModel, skills: InsightsSkillsModel) {
            _ = NSApplication.shared
            let model = InsightsModel(service: InsightsGetStartedRenderTests.emptyStore)
            let bounds = NSRect(x: 0, y: 0, width: 440, height: 900)
            // Constrain SwiftUI as well as AppKit: NSHostingView probes an
            // infinite intrinsic maximum otherwise, unlike the real pane.
            hosting = NSHostingView(rootView: AnyView(FlagHost(app: app, model: model, skills: skills)
                .frame(width: bounds.width, height: bounds.height)))
            hosting.frame = bounds
            window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = hosting
            hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        }
        func drain() async {
            try? await Task.sleep(for: .milliseconds(30))
            hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        }
        func close() async {
            hosting.rootView = AnyView(EmptyView())
            await drain()
            window.contentView = nil
            window.close()
        }
    }
}
