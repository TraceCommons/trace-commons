import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The get-started card and its sample (product owner, 2026-10-29), and
/// every tab for every kind of user: nothing yet, Claude only, Codex only,
/// Claude and Codex, and with NEAR AI. Each state is the whole Insights
/// window, through its real models, at the wide pane (about 1100pt) and the
/// narrow one (about 760pt).
///
/// Set `INSIGHTS_SHOTS_DIR` to keep the PNGs; CI writes none.
@MainActor
final class InsightsGetStartedRenderTests: XCTestCase {
    nonisolated static let shotsDir = ProcessInfo.processInfo.environment["INSIGHTS_SHOTS_DIR"]

    enum UserState: String, CaseIterable {
        case empty = "no-data", claude = "claude-only", codex = "codex-only", both = "claude-codex"
        case nearAi = "with-near-ai"

        var tools: Set<String> {
            switch self {
            case .empty: []
            case .claude: ["claude_code"]
            case .codex: ["codex"]
            case .both, .nearAi: ["claude_code", "codex"]
            }
        }
    }

    // MARK: The pure rules

    func testTheGuideShowsOnlyWithoutATraceInAnyKeptWeek() throws {
        let empty = try Self.emptyOverview()
        XCTAssertFalse(InsightsEmptyState.weekHasData(nil, history: nil))
        XCTAssertFalse(InsightsEmptyState.weekHasData(empty, history: []))
        XCTAssertTrue(InsightsEmptyState.weekHasData(try InsightsSampleWeek.weekOverview, history: nil))
        let kept = try JSONDecoder().decode([InsightsWeekFigures].self, from: Data(#"""
        [{"week_start":"2026-09-28","comparable":true,"tokens":{},"cache_share_permille":{},"patterns":{},
          "sessions":3,"pattern_counts":{},"reread_files":null,"past_threshold":null}]
        """#.utf8))
        XCTAssertTrue(InsightsEmptyState.weekHasData(empty, history: kept), "a kept week with traces is real data")
    }

    func testBenchmarksNeedAKindOfWorkWithATrace() {
        XCTAssertFalse(InsightsEmptyState.benchmarksHaveData(nil))
        XCTAssertFalse(InsightsEmptyState.benchmarksHaveData(.init(categories: [.init(key: "bug_fix", sessions: 0)])))
        XCTAssertFalse(InsightsEmptyState.benchmarksHaveData(.init(enabled: false, categories: nil)))
        XCTAssertTrue(InsightsEmptyState.benchmarksHaveData(InsightsSample.benchmarks))
    }

    func testTheSampleIsBelievableAndAddsUp() throws {
        let work = InsightsSample.work
        XCTAssertEqual(work.byModel.reduce(0) { $0 + $1.usdMicrosKnown }, work.week.usdMicrosKnown)
        XCTAssertEqual(work.byFolder.reduce(0) { $0 + $1.usdMicrosKnown }, work.week.usdMicrosKnown)
        XCTAssertEqual(work.byDay.reduce(0) { $0 + $1.usdMicrosKnown }, work.week.usdMicrosKnown)
        XCTAssertEqual(work.byDay.count, 7)
        XCTAssertEqual(Set(work.byModel.map(\.tool)), ["claude_code", "codex"])
        for category in InsightsSample.benchmarks.categories ?? [] {
            XCTAssertNotNil(category.quality, category.key)
            XCTAssertNotEqual(category.quality?.leader.label, category.quality?.next.label, category.key)
        }
        let week = try InsightsSample.week()
        XCTAssertTrue(week.showsCounterPass)
        XCTAssertEqual(week.rollup?.sessions.count, 37)
        XCTAssertEqual(week.usageByDay?.count, 7)
        // Goals are /goal goals with each tool's own status, and add up.
        let goals = try XCTUnwrap(week.goals)
        XCTAssertEqual(goals.total, goals.goals.count)
        XCTAssertEqual(goals.met, goals.goals.filter(\.met).count)
        XCTAssertEqual(goals.byTool?.values.reduce(0) { $0 + $1.total }, goals.total)
        for goal in goals.goals {
            XCTAssertTrue(InsightsGoalsWords.statuses.contains(goal.status), goal.status)
            XCTAssertTrue(["claude_code", "codex"].contains(goal.tool), goal.tool)
        }
    }

    func testEveryGetStartedWordIsTheCores() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in ["analytics_start_title", "analytics_start_why", "analytics_start_step_use",
                    "analytics_start_step_use_detail", "analytics_start_step_share",
                    "analytics_start_step_share_detail", "analytics_start_choose",
                    "analytics_start_step_fill", "analytics_start_step_fill_detail",
                    "analytics_sample_banner", "analytics_sample_banner_detail"] {
            XCTAssertFalse((copy[key] ?? "").isEmpty, key)
        }
        XCTAssertTrue(copy["analytics_sample_banner"]?.contains("not yours") == true)
    }

    /// By project draws a tool's column only while some folder has a trace
    /// of it, and NEAR AI's only while it was called.
    func testByProjectHasNoColumnForAToolNotUsed() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let claude = try Self.week(.claude)
        let rows = InsightsProjectRows.rows(claude, copy: copy)
        XCTAssertEqual(InsightsProjectRows.columns(claude, rows: rows),
                       .init(spend: true, claude: true, codex: false, nearAi: false))
        XCTAssertEqual(InsightsDetailsTable.dimensions(claude), [.folder, .model])
        let near = try Self.week(.nearAi)
        XCTAssertEqual(InsightsProjectRows.columns(near, rows: InsightsProjectRows.rows(near, copy: copy)),
                       .init(spend: true, claude: true, codex: true, nearAi: true))
        XCTAssertEqual(InsightsDetailsTable.dimensions(near), [.folder, .model],
                       "By task and By tool are removed (owner, 2026-10-29)")
    }

    func testTilesAndCardsWithNothingKnownAreLeftOut() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let unpriced = DaemonData.InsightsWork(week: .init(sessions: 2, pricedSessions: 0, usdMicrosKnown: 0,
                                                           usdPartial: true))
        XCTAssertFalse(InsightsWorkWords.knowsSpend(unpriced.week))
        XCTAssertFalse(InsightsWorkWords.showsTrends(unpriced, tokens: nil, copy: copy))
        XCTAssertTrue(InsightsWorkWords.showsTrends(InsightsSample.work, tokens: nil, copy: copy))
        let claude = Self.work(["claude_code"])
        XCTAssertEqual(InsightsWorkWords.activeTools(claude).map(\.key), ["claude_code"])
        XCTAssertEqual(InsightsWorkWords.apiPrices(claude, copy: copy), "Claude at API prices")
        XCTAssertEqual(InsightsWorkWords.apiPrices(InsightsSample.work, copy: copy), "Claude and Codex at API prices")
    }

    // MARK: Fixtures

    static func emptyOverview() throws -> InsightsWeekOverview {
        try JSONDecoder().decode(InsightsWeekOverview.self,
                                 from: JSONSerialization.data(withJSONObject: emptyOverviewObject))
    }

    nonisolated static var emptyOverviewObject: [String: Any] {
        var overview = InsightsSampleWeek.overview
        overview["feed"] = "saved"
        overview["sessions"] = 0
        overview["coverage"] = ["known": 0, "partial": 0, "unknown": 0, "reasons": [String: Any]()]
        overview["sources"] = [Any]()
        overview["by_day"] = NSNull()
        overview["codex_interval_tokens"] = NSNull()
        overview["by_model"] = [Any]()
        overview["by_tool"] = [Any]()
        overview["weeks"] = [String]()
        return overview
    }

    nonisolated static var emptyPatternsObject: [String: Any] {
        var patterns = InsightsSampleWeek.patterns
        patterns["feed"] = "saved"
        patterns["sessions"] = 0
        patterns["claude_sessions"] = 0
        patterns["claude_only"] = false
        patterns["coverage"] = ["known": 0, "partial": 0, "unknown": 0, "reasons": [String: Any]()]
        patterns["cards"] = [Any]()
        patterns["reread_files"] = [Any]()
        patterns["weeks"] = [String]()
        return patterns
    }

    /// The sample's work, kept to `tools`.
    static func work(_ tools: Set<String>) -> DaemonData.InsightsWork {
        let all = InsightsSample.work
        let byTool = all.byTool.filter { tools.contains($0.key ?? "") }
        let byModel = all.byModel.filter { tools.contains($0.tool) }
        let usd = byModel.reduce(Int64(0)) { $0 + $1.usdMicrosKnown }
        let share = Double(usd) / Double(all.week.usdMicrosKnown)
        func cents(_ micros: Double) -> Int64 { Int64((micros / 10_000).rounded()) * 10_000 }
        func scaled(_ totals: DaemonData.InsightsWorkTotals?, key: String? = nil) -> DaemonData.InsightsWorkTotals? {
            guard let totals else { return nil }
            return .init(key: totals.key, sessions: max(1, Int((Double(totals.sessions) * share).rounded())),
                         usdMicrosKnown: cents(Double(totals.usdMicrosKnown) * share),
                         cacheSavingsUsdMicrosKnown: Int64((Double(totals.cacheSavingsUsdMicrosKnown) * share).rounded()))
        }
        func days(_ days: [DaemonData.InsightsWorkDay]?) -> [DaemonData.InsightsWorkDay]? {
            days?.map { .init(date: $0.date, usdMicrosKnown: cents(Double($0.usdMicrosKnown) * share)) }
        }
        let week = DaemonData.InsightsWorkTotals(
            sessions: byTool.reduce(0) { $0 + $1.sessions }, usdMicrosKnown: usd,
            cacheSavingsUsdMicrosKnown: Int64((Double(all.week.cacheSavingsUsdMicrosKnown) * share).rounded()))
        return .init(week: week, lastWeek: scaled(all.lastWeek), byTool: byTool, byModel: byModel,
                     byFolder: all.byFolder.compactMap { scaled($0) }, byDay: days(all.byDay) ?? [],
                     lastWeekByDay: days(all.lastWeekByDay))
    }

    /// The sample week as one kind of user would have it.
    static func weekObject(_ state: UserState) -> [String: Any] {
        var object = InsightsSample.weekObject
        let tools = state.tools
        guard tools.count < 2 else {
            if state == .nearAi { object["near_ai_credit"] = nearAiCredit }
            return object
        }
        func keep(_ source: Any?) -> Bool { tools.contains(source as? String ?? "") }
        var rollup = object["rollup"] as? [String: Any] ?? [:]
        rollup["sessions"] = (rollup["sessions"] as? [[String: Any]] ?? []).filter { keep($0["source"]) }
        rollup["sources"] = (rollup["sources"] as? [[String: Any]] ?? []).filter { keep($0["source"]) }
        object["rollup"] = rollup
        var overview = object["overview"] as? [String: Any] ?? [:]
        let sources = (overview["sources"] as? [[String: Any]] ?? []).filter { keep($0["source"]) }
        overview["sources"] = sources
        overview["sessions"] = sources.reduce(0) { $0 + ($1["sessions"] as? Int ?? 0) }
        overview["by_tool"] = (overview["by_tool"] as? [[String: Any]] ?? []).filter { keep($0["source"]) }
        overview["by_model"] = (overview["by_model"] as? [[String: Any]] ?? []).filter { model in
            let label = model["label"] as? String ?? ""
            return tools.contains("codex") ? label.contains("gpt") : !label.contains("gpt")
        }
        if !tools.contains("claude_code") {
            overview["by_day"] = NSNull()
            var patterns = object["patterns"] as? [String: Any] ?? [:]
            patterns["claude_sessions"] = 0
            patterns["claude_only"] = false
            patterns["sessions"] = overview["sessions"]
            patterns["reread_files"] = [Any]()
            object["patterns"] = patterns
        } else {
            overview["codex_interval_tokens"] = NSNull()
            var patterns = object["patterns"] as? [String: Any] ?? [:]
            patterns["claude_only"] = false
            patterns["sessions"] = overview["sessions"]
            object["patterns"] = patterns
        }
        object["overview"] = overview
        object["change_vs_last_week"] = (object["change_vs_last_week"] as? [[String: Any]] ?? [])
            .filter { keep($0["source"]) }
        // Only this user's tools' goals.
        var goals = object["goals"] as? [String: Any] ?? [:]
        let kept = (goals["goals"] as? [[String: Any]] ?? []).filter { keep($0["tool"]) }
        goals["goals"] = kept
        goals["total"] = kept.count
        goals["met"] = kept.filter { $0["met"] as? Bool == true }.count
        goals["by_tool"] = (goals["by_tool"] as? [String: Any] ?? [:]).filter { tools.contains($0.key) }
        object["goals"] = goals
        object["usage_by_day"] = (object["usage_by_day"] as? [[String: Any]] ?? []).map { day -> [String: Any] in
            var day = day
            if !tools.contains("codex") { day["codex"] = 0 }
            if !tools.contains("claude_code") { day["claude_code"] = 0 }
            return day
        }
        return object
    }

    static func week(_ state: UserState) throws -> DaemonData.InsightsWeek {
        let object = weekObject(state)
        // Only the folders this user's traces are in.
        let rows = (object["rollup"] as? [String: Any])?["sessions"] as? [[String: Any]] ?? []
        let folders = Set(rows.compactMap { $0["folder"] as? String })
        let all = work(state.tools)
        let kept = DaemonData.InsightsWork(week: all.week, lastWeek: all.lastWeek, byTool: all.byTool,
                                           byModel: all.byModel,
                                           byFolder: all.byFolder.filter { folders.contains($0.key ?? "") },
                                           byDay: all.byDay, lastWeekByDay: all.lastWeekByDay)
        return try InsightsSample.week(object, work: kept)
    }

    /// NEAR AI calls from this Mac this week, by task, tool and folder.
    static var nearAiCredit: [String: Any] {
        func window(_ usd: Int, _ tokens: Int, _ calls: Int, date: String? = nil) -> [String: Any] {
            var window: [String: Any] = ["from": "", "to": "", "usd_micros": usd, "sessions": max(1, calls / 6),
                                         "calls": calls, "calls_without_charge": 0, "tokens": tokens,
                                         "usd_micros_known": usd, "usd_partial": false, "tokens_known": tokens,
                                         "tokens_partial": false]
            if let date { window["date"] = date }
            return window
        }
        func group(_ key: String, _ usd: Int, _ sessions: Int, _ tokens: Int) -> [String: Any] {
            ["key": key, "usd_micros": usd, "sessions": sessions, "calls": sessions * 6, "calls_without_charge": 0,
             "tokens": tokens]
        }
        var week = window(4_120_000, 1_200_000, 48)
        week["by_task"] = [group("bug_fix", 2_010_000, 3, 610_000), group("docs", 1_240_000, 2, 380_000),
                           group("other", 870_000, 3, 210_000)]
        week["by_tool"] = [group("claude_code", 3_050_000, 6, 900_000), group("codex", 1_070_000, 2, 300_000)]
        week["by_folder"] = [group("trace-commons", 2_300_000, 4, 700_000), group("website", 1_820_000, 4, 500_000)]
        let days = ["2026-10-05", "2026-10-06", "2026-10-07", "2026-10-08", "2026-10-09", "2026-10-10", "2026-10-11"]
        let amounts = [520_000, 880_000, 1_010_000, 640_000, 610_000, 120_000, 340_000]
        return ["basis": "near_ai_ledger", "week": week, "today": window(340_000, 90_000, 4),
                "by_day": zip(days, amounts).map { window($1, $1 / 3, 6, date: $0) },
                "last_31_days": window(15_800_000, 4_900_000, 190)]
    }

    static func benchmarks(_ state: UserState) -> DaemonData.InsightsBenchmarks {
        switch state {
        case .empty:
            return .init(sessions: 0, categories: [])
        case .both, .nearAi:
            return InsightsSample.benchmarks
        case .claude, .codex:
            // One tool: the Claude models still compare with each other;
            // Codex alone has one model, so no kind of work has two.
            let claude = state == .claude
            let categories = (InsightsSample.benchmarks.categories ?? []).map { category in
                func only(_ pick: DaemonData.InsightsBenchmarkPick?) -> DaemonData.InsightsBenchmarkPick? {
                    guard let pick, claude, pick.leader.tool == "claude_code", pick.next.tool == "claude_code"
                    else { return nil }
                    return pick
                }
                let quality = only(category.quality), value = only(category.value), speed = only(category.speed)
                return DaemonData.InsightsBenchmarkCategory(
                    key: category.key, sessions: claude ? category.sessions * 3 / 4 : max(2, category.sessions / 4),
                    quality: quality, value: value, speed: speed,
                    tip: quality == nil ? nil : category.tip)
            }
            return .init(sessions: categories.reduce(0) { $0 + $1.sessions }, categories: categories)
        }
    }

    static func subscriptions(_ state: UserState) -> DaemonData.SubscriptionUsage {
        let all = InsightsSample.subscriptions()
        let providers = state.tools.map { $0 == "claude_code" ? "claude" : $0 }
        return .init(providers: all.providers.filter { providers.contains($0.provider) })
    }

    /// The store's answers for a person with nothing saved: the core's copy,
    /// and empty weeks.
    nonisolated static let emptyStore: InsightsModel.Service = { request in
        let operation = request.operation
        let body: [String: Any]
        switch operation.type {
        case "copy": return try TCInsights.call(request)
        case "week_overview": body = ["type": operation.type, "overview": emptyOverviewObject]
        case "patterns": body = ["type": operation.type, "patterns": emptyPatternsObject]
        case "list": body = ["type": operation.type, "insights": [Any]()]
        case "comparisons":
            body = ["type": operation.type, "comparisons": ["feed": "counter_pass", "week_start": "2026-10-05",
                                                            "goals": [Any](), "lever": NSNull(), "recap": NSNull()]]
        default: throw InsightsError.service("not_answered")
        }
        return try TCInsights.decode(JSONSerialization.data(withJSONObject: body), for: operation)
    }

    // MARK: Shots

    func testEveryTabForEveryKindOfUserRenders() async throws {
        for state in UserState.allCases {
            for tab in InsightsView.shownTabs {
                for width in [1100, 760] as [CGFloat] {
                    let png = try await Self.shot(state, tab: tab, width: width)
                    XCTAssertGreaterThan(png.count, 20_000, "\(state) \(tab) \(width)")
                    if let dir = Self.shotsDir {
                        try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                        let name = "\(state.rawValue)-\(Self.name(tab))-\(Int(width)).png"
                        try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent(name))
                    }
                }
            }
        }
    }

    static func name(_ tab: InsightsTab) -> String {
        switch tab {
        case .overview: "overview"
        case .patterns: "breakdown"
        case .benchmarks: "benchmarks"
        }
    }

    static func shot(_ state: UserState, tab: InsightsTab, width: CGFloat) async throws -> Data {
        let copy = TCInsights.copy()
        let week: DaemonData.InsightsWeek? = state == .empty ? nil : try Self.week(state)
        let reader: InsightsModel.WeekReader = { _ in
            guard let week else { throw DaemonDataError.notAvailableYet(method: "insights_week") }
            return week
        }
        let model = InsightsModel(service: emptyStore, weekReader: reader)
        let overview = InsightsOverviewModel(service: emptyStore)
        let bench = benchmarks(state)
        let benchmarks = InsightsBenchmarksModel(reader: { _, _ in bench })
        let usage = subscriptions(state)
        let subscriptions = SubscriptionUsageModel(reader: { _, _, _, _ in usage })
        let view = InsightsView(storeSelection: .standard, storeCopy: copy, model: model, overviewModel: overview,
                                initialTab: tab, subscriptionModel: subscriptions, benchmarks: benchmarks,
                                openTraces: {})
        let height: CGFloat = width < 900 ? 4_000 : 2_800
        return try await render(AnyView(view), width: width, height: height) {
            model.weekSettled && !overview.busy && overview.shown != nil
                && (tab != .benchmarks || benchmarks.settled)
        }
    }

    /// The view in the main pane at `width`, its insets included, over a
    /// stand-in for the pane's glass, once `ready` and the window's tasks
    /// have run.
    static func render(_ view: AnyView, width: CGFloat, height: CGFloat,
                       ready: () -> Bool) async throws -> Data {
        _ = NSApplication.shared
        let inset = MonitorWindowView.mainInset
        let size = CGSize(width: width, height: height)
        let content = view
            .frame(width: width - 2 * inset, height: height - inset)
            .padding(.horizontal, inset)
            .padding(.top, inset)
            .background {
                ZStack { Color(red: 0.36, green: 0.40, blue: 0.47); GlassTokens.Color.glassVeil.color }
            }
            .environment(\.colorScheme, .light)
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: .aqua)
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        for _ in 0 ..< 300 where !ready() { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertTrue(ready(), "the models did not settle")
        // Let the swap to the sample, and the sample's own read, land.
        for _ in 0 ..< 30 {
            try await Task.sleep(nanoseconds: 10_000_000)
            hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        }
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(cropped(bitmap).representation(using: .png, properties: [:]))
    }

    /// The bitmap without the empty pane under the content, so a shot is
    /// as tall as what it shows.
    static func cropped(_ bitmap: NSBitmapImageRep) -> NSBitmapImageRep {
        guard let data = bitmap.bitmapData, let image = bitmap.cgImage else { return bitmap }
        let (width, height, row, step) = (bitmap.pixelsWide, bitmap.pixelsHigh, bitmap.bytesPerRow,
                                          bitmap.bitsPerPixel / 8)
        func pixel(_ x: Int, _ y: Int) -> (Int, Int, Int) {
            let at = y * row + x * step
            return (Int(data[at]), Int(data[at + 1]), Int(data[at + 2]))
        }
        var last = height - 1
        scan: while last > 0 {
            // The left gutter is always the pane, whatever its shade there.
            let background = pixel(2, last)
            for x in stride(from: 0, to: width, by: 3) {
                let (r, g, b) = pixel(x, last)
                if abs(r - background.0) + abs(g - background.1) + abs(b - background.2) > 12 { break scan }
            }
            last -= 1
        }
        let keep = min(height, last + 48)
        guard keep < height, let crop = image.cropping(to: CGRect(x: 0, y: 0, width: width, height: keep))
        else { return bitmap }
        return NSBitmapImageRep(cgImage: crop)
    }
}
