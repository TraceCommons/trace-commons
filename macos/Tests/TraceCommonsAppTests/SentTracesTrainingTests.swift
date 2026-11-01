import Foundation
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Adding Train coding models to sent traces in the Mac app: the sheet's
/// two equal choices with nothing preselected, where the sheet, the
/// Insights line and the inspector's Allow are drawn, and what raises the
/// sheet. The pure logic is `TCShellCoreTests.SentTracesTrainingSurfaceTests`.
@MainActor
final class SentTracesTrainingTests: XCTestCase {
    private static func source(_ rel: String) throws -> String {
        try MonitorNavigationTests.text(rel)
    }

    private static let copy: SentTracesTrainingCopy = {
        var object = [String: String]()
        for field in SentTracesTrainingCopy.consumedFields { object[field] = "T:\(field) {count}" }
        let data = try! JSONSerialization.data(withJSONObject: object)
        return SentTracesTrainingCopy.decode(fromJSON: String(decoding: data, as: UTF8.self))!
    }()

    // MARK: The sheet's choices

    /// Not now and Allow carry equal weight: the same role (so the same
    /// kind and size in the modal's bar), neither the default nor
    /// prominent, so neither is drawn as the primary or taken by Return.
    func test_theTwoChoicesShareOneStyleAndNeitherIsTheDefault() {
        let actions = SentTracesTrainingSheetView.actions(Self.copy, count: 4, busy: false, notNow: {}, allow: {})
        XCTAssertEqual(actions.map(\.title), ["T:not_now_button {count}", "T:allow_button {count}"])
        XCTAssertEqual(Set(actions.map(\.role)), [.standard])
        for action in actions {
            XCTAssertFalse(action.isDefault, action.title)
            XCTAssertFalse(action.isProminent, action.title)
            XCTAssertTrue(action.isEnabled, action.title)
        }
        XCTAssertEqual(actions[0].accessibilityName, "T:not_now_accessibility_name {count}")
        XCTAssertEqual(actions[1].accessibilityName, "T:allow_accessibility_name 4")
    }

    /// While the grant runs neither choice can be taken again.
    func test_bothChoicesWaitForAGrantInFlight() {
        let actions = SentTracesTrainingSheetView.actions(Self.copy, count: 4, busy: true, notNow: {}, allow: {})
        XCTAssertTrue(actions.allSatisfy { !$0.isEnabled })
    }

    /// Read from the source: no choice is made the default, prominent or
    /// bound to a key, and the sheet's words are the core's.
    func test_theSheetPreselectsNothing() throws {
        let views = try Self.source("Views/SentTracesTrainingViews.swift")
        for forbidden in ["isDefault: true", "isProminent: true", ".keyboardShortcut(", ".defaultAction", "role: .cancel",
                          "role: .destructive"] {
            XCTAssertFalse(views.contains(forbidden), forbidden)
        }
        XCTAssertTrue(views.contains("GlassModal(\n            title: copy.title"))
        XCTAssertTrue(views.contains("copy.body(count: sheet.count)"))
        XCTAssertTrue(views.contains("Text(copy.scope)"))
    }

    /// Only the person's choice sends a grant: the model's two grant calls
    /// sit in Allow's two handlers, and nothing else in the app calls them.
    func test_onlyAllowSendsAGrant() throws {
        let model = try Self.source("SentTracesTrainingModel.swift")
        let allowAll = try XCTUnwrap(model.range(of: "func allowAll()"))
        let allowOne = try XCTUnwrap(model.range(of: "func allow(_ submissionId: String)"))
        let grantAll = try XCTUnwrap(model.range(of: ".grantAll()"))
        let grantIds = try XCTUnwrap(model.range(of: ".grant(submissionIds:"))
        XCTAssertTrue(allowAll.upperBound < grantAll.lowerBound && grantAll.lowerBound < allowOne.lowerBound)
        XCTAssertTrue(allowOne.upperBound < grantIds.lowerBound)
        XCTAssertEqual(model.components(separatedBy: ".grantAll()").count, 2)
        XCTAssertEqual(model.components(separatedBy: ".grant(submissionIds:").count, 2)
        for other in ["AppModel.swift", "Views/SentTracesTrainingViews.swift", "Views/Monitor/HomeViews.swift",
                      "Views/Monitor/HistoryInspector.swift"] {
            let text = try Self.source(other)
            XCTAssertFalse(text.contains(".grantAll()"), other)
            XCTAssertFalse(text.contains(".grant(submissionIds:"), other)
        }
    }

    /// A fresh model raises nothing: no sheet before the daemon counted
    /// eligible traces, and Review with nothing counted raises nothing.
    func test_nothingIsRaisedBeforeTheDaemonCounts() {
        let app = AppModel()
        XCTAssertNil(app.sentTracesTraining.sheet)
        app.sentTracesTraining.review()
        XCTAssertNil(app.sentTracesTraining.sheet)
        app.sentTracesTraining.notNow()
        XCTAssertNil(app.sentTracesTraining.sheet)
        XCTAssertFalse(app.sentTracesTraining.busy)
    }

    // MARK: Where it is drawn and what raises it

    /// Turning Train coding models on in Settings raises the sheet in the
    /// Settings window; the first run's Start raises it on the Monitor;
    /// both only after the daemon confirmed the write.
    func test_aConfirmedTurnOnRaisesTheSheet() throws {
        let app = try Self.source("AppModel.swift")
        XCTAssertTrue(app.contains("offerSentTracesTraining(outcome, before: before, at: .settings)"))
        XCTAssertTrue(app.contains("offerSentTracesTraining(outcome, before: before, at: .monitor)"))
        XCTAssertTrue(app.contains("guard case .succeeded(let confirmed) = outcome,"))
        XCTAssertTrue(app.contains("SentTracesTrainingState.turnsOn(before: before, after: confirmed)"))
    }

    /// Each window that can be asked for the sheet presents it inside its
    /// modal host, at its own site.
    func test_theSheetIsPresentedInsideEachWindowsHost() throws {
        for (file, site) in [("Views/MonitorWindowView.swift", ".monitor"), ("TraceCommonsAppMain.swift", ".settings")] {
            let text = try Self.source(file)
            let presenter = try XCTUnwrap(text.range(of: "SentTracesTrainingPresenter(site: \(site))"), file)
            let host = try XCTUnwrap(
                text.range(of: ".glassModalHost()", range: presenter.upperBound..<text.endIndex), file)
            XCTAssertTrue(presenter.upperBound < host.lowerBound, file)
        }
    }

    /// The quiet line sits under Home's Insights card, re-read with Home's
    /// History; the inspector offers Allow on the opened sent trace.
    func test_theLineAndTheInspectorAreWired() throws {
        let home = try Self.source("Views/Monitor/HomeViews.swift")
        let insights = try XCTUnwrap(home.range(of: "HomeInsightsVisual(glance: freshGlance, words: words)"))
        let line = try XCTUnwrap(
            home.range(of: "SentTracesTrainingLine(model: model.sentTracesTraining, history: store.history)"))
        XCTAssertTrue(insights.upperBound < line.lowerBound)
        let inspector = try Self.source("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SentTraceTrainingSection(model: model.sentTracesTraining, row: row)"))
        let views = try Self.source("Views/SentTracesTrainingViews.swift")
        XCTAssertTrue(views.contains(".accessibilityLabel(copy.reviewAccessibilityName)"))
        XCTAssertTrue(views.contains(".accessibilityLabel(copy.traceAccessibilityName)"))
        XCTAssertTrue(views.contains("Task { await model.allow(id) }"))
    }
}
