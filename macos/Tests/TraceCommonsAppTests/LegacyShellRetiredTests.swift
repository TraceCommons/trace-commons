import XCTest
@testable import TraceCommonsApp

/// R15 of #1173: the glass windows are the release default and the legacy
/// shell is gone. What survives of the legacy files is their words tables,
/// which the glass screens read.
@MainActor
final class LegacyShellRetiredTests: XCTestCase {
    static let root = GlassSurfaceRulesTests.root

    func test_theLegacyWindowsAreGone() throws {
        for file in ["Views/MainWindowView.swift", "Views/BrandMark.swift", "Views/CommunityBrand.swift",
                     "Views/ActionMessageBanner.swift", "Views/CreditRecordView.swift",
                     "Views/PrivateInferenceActivationView.swift"] {
            XCTAssertFalse(FileManager.default.fileExists(atPath: Self.root.appendingPathComponent(file).path), "\(file) still exists")
        }
        for file in ["Views/QueueView.swift", "Views/QueueFolderRow.swift", "Views/HistoryView.swift"] {
            let source = try MonitorNavigationTests.text(file)
            XCTAssertNil(source.range(of: #"struct \w+: View"#, options: .regularExpression), "\(file) still draws a view")
        }
        XCTAssertTrue(try MonitorNavigationTests.text("Views/QueueView.swift").contains("enum QueueLegacyWords"))
        XCTAssertTrue(try MonitorNavigationTests.text("Views/HistoryView.swift").contains("enum HistoryLegacyWords"))
        // R-37: the held sentence is the core's; the Swift one left the shell.
        XCTAssertFalse(try MonitorNavigationTests.text("Views/HistoryView.swift").contains("enum HistoryCopy"))
        XCTAssertFalse(try MonitorNavigationTests.text("Views/PrivateInferenceView.swift").contains("struct PrivateInferenceContent"))
    }

    /// The glass windows are the release default: no DEBUG gate and no env
    /// flag stands between a release build and the Monitor, Settings or the
    /// first-run window. The menu preview and sample data stay debug-only.
    func test_theGlassWindowsHaveNoGate() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        for flag in ["TRACE_COMMONS_GLASS_MENU", "TRACE_COMMONS_MONITOR\"", "TRACE_COMMONS_FIRST_RUN", "WindowID.main", "MainWindowView("] {
            XCTAssertFalse(main.contains(flag), "\(flag) survives")
        }
        for scene in ["MonitorWindowView(", "MonitorSettingsWindow(", "FirstRunWindowView(", "MenuBarGlassPanel(store: menuPanel)"] {
            let at = try XCTUnwrap(main.range(of: scene)).lowerBound
            let before = main[..<at]
            let opens = before.components(separatedBy: "#if DEBUG").count - 1
            let closes = before.components(separatedBy: "#endif").count - 1
            XCTAssertEqual(opens, closes, "\(scene) is inside #if DEBUG")
        }
        XCTAssertFalse(main.contains("#else"), "a release build has no branch of its own")
        XCTAssertTrue(main.contains("TRACE_COMMONS_MENU_PREVIEW"))
        // The menu preview window and its env read stay inside #if DEBUG
        // (D-18): more gates open than closed before each.
        for debugOnly in ["MenuBarPreviewWindow(", "\"TRACE_COMMONS_MENU_PREVIEW\""] {
            let at = try XCTUnwrap(main.range(of: debugOnly)).lowerBound
            let before = main[..<at]
            let opens = before.components(separatedBy: "#if DEBUG").count - 1
            let closes = before.components(separatedBy: "#endif").count - 1
            XCTAssertEqual(opens, closes + 1, "\(debugOnly) is outside #if DEBUG")
        }
        for file in ["Views/MonitorWindowView.swift", "Views/Monitor/TracesViews.swift", "Views/Monitor/HomeViews.swift",
                     "Views/Monitor/InferenceViews.swift", "Views/Monitor/FirstRunViews.swift", "Views/Monitor/MenuBarGlassPanel.swift",
                     "Views/Monitor/MissionsViews.swift", "Views/Monitor/FlowMapView.swift", "Views/Monitor/FlowMapScene.swift",
                     "Views/Monitor/HomeStore.swift", "Views/Monitor/InferenceStore.swift", "Views/Monitor/MenuPanelStore.swift",
                     "Views/Monitor/TracesHealth.swift", "Views/Monitor/TracesOffers.swift",
                     "Views/Monitor/InferenceAccount.swift", "Views/Monitor/HistoryInspector.swift",
                     "Views/Monitor/TracesTree.swift", "Views/Monitor/TracesStore.swift"] {
            XCTAssertFalse(try MonitorNavigationTests.text(file).hasPrefix("#if DEBUG"), "\(file) is debug-only")
        }
        // The menu-bar preview window stays debug-only (D-18).
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("#if DEBUG\n/// The menu-bar item and the popover under it, in a window"))
    }

    /// Sample data never reaches a release build.
    func test_sampleDataStaysDebugOnly() throws {
        let shellCore = Self.root.deletingLastPathComponent().appendingPathComponent("TCShellCore/DataContract")
        for file in ["SampleDaemonClient.swift", "SampleDaemonData.swift"] {
            let source = try String(contentsOf: shellCore.appendingPathComponent(file), encoding: .utf8)
            XCTAssertTrue(source.hasPrefix("#if DEBUG"), "\(file) must stay debug-only")
        }
        let wiring = try MonitorNavigationTests.text("DaemonDataWiring.swift")
        XCTAssertTrue(wiring.contains("#if DEBUG\n    /// Sample data"))
    }

    /// The shipped hooks scripts and CI rely on are still read.
    func test_theScriptHooksSurvive() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains("TRACE_COMMONS_SHOW_WINDOW"))
        XCTAssertTrue(main.contains("TRACE_COMMONS_APPEARANCE"))
        XCTAssertTrue(main.contains("DebugScreenshot.scheduleIfRequested(model: model)"))
        XCTAssertTrue(main.contains("SelfTest.runIfRequested(model: model)"))
        XCTAssertTrue(main.contains("UpdateController.shared.start()"))
        XCTAssertTrue(main.contains("Notifier.shared.configure()"))
        let demo = try String(contentsOf: Self.root.deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("scripts/run-demo.sh"), encoding: .utf8)
        XCTAssertFalse(demo.contains("TRACE_COMMONS_MONITOR"))
        XCTAssertTrue(demo.contains("TRACE_COMMONS_SHOW_WINDOW"))
    }

    /// The app's tint is the glass token, not the legacy accent (R-7).
    func test_noAppFileTintsWithTheLegacyAccent() throws {
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertFalse(main.contains("TC."))
        XCTAssertTrue(main.contains(".tint(GlassTokens.Color.purpleSoft.color)"))
    }

    /// The legacy sidebar's navigation is gone; the class is the services
    /// gate and the holder of a pending destination (ruling R-33).
    func test_theLegacySectionIsGone() throws {
        let navigation = try MonitorNavigationTests.text("MainWindowNavigation.swift")
        for remnant in ["var section", "legacySection", "displaysInsights", "displaysCompute", "MainWindowView"] {
            XCTAssertFalse(navigation.contains(remnant), "\(remnant) survives")
        }
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertFalse(panel.contains("MainWindowNavigation"), "the panel opens through OpenMonitor")
    }
}
