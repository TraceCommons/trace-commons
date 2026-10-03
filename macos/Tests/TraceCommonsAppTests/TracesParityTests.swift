#if DEBUG
import XCTest
import TCBridge
import TCDesign
import TCShellCore
@testable import TraceCommonsApp

/// The Traces tab against the legacy queue: each control, confirmation,
/// state and core-copy source the queue had has a glass home. Rows are
/// removed only when the owner retires the control they name.
@MainActor
final class TracesParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    static let words = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())
    static let coreDown = HealthLineCopy.decode(fromJSON: TCCoreCopy.healthCopyJSON(reachable: false, label: nil, maxQueueEntries: nil))

    /// Fail closed: a core that does not answer, a status nobody could read
    /// and a reported label are each a banner; only a read status with
    /// nothing wrong is none.
    func test_anAbsentSignalIsNeverHealthy() throws {
        let words = try XCTUnwrap(Self.words)
        let down = TracesHealth.banners(phase: .failed(.unreachable), status: nil, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(down.map(\.title), [try XCTUnwrap(Self.coreDown).title])
        XCTAssertEqual(down.first?.tone, .outside)
        let unread = TracesHealth.banners(phase: .loaded, status: nil, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(unread.map(\.title), [words.requestFailed])
        XCTAssertTrue(TracesHealth.banners(phase: .loading, status: nil, words: words, coreDown: Self.coreDown).isEmpty)
    }

    func test_aCoreDownLineThatCannotBeReadFallsBackToTheCoresUnreachableWord() throws {
        let words = try XCTUnwrap(Self.words)
        let down = TracesHealth.banners(phase: .failed(.unreachable), status: nil, words: words, coreDown: nil)
        XCTAssertEqual(down.map(\.title), [words.coreUnreachable])
        XCTAssertNil(down.first?.detail)
    }

    func test_aReportedLabelIsTheCoresBanner() throws {
        let words = try XCTUnwrap(Self.words)
        let json = #"{"health":{"last_error_label":"pii-filter-unavailable"}}"#
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
        let banners = TracesHealth.banners(phase: .loaded, status: status, words: words, coreDown: Self.coreDown)
        XCTAssertEqual(banners.map(\.title), [HealthCopy.core(label: "pii-filter-unavailable", maxQueueEntries: nil).title])
        XCTAssertNotEqual(banners.first?.tone, .on)
    }

    func test_aReadStatusWithNothingWrongDrawsNoBanner() throws {
        let words = try XCTUnwrap(Self.words)
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data("{}".utf8))
        XCTAssertTrue(TracesHealth.banners(phase: .loaded, status: status, words: words, coreDown: Self.coreDown).isEmpty)
    }

    /// With no health words at all, a down or unread core still draws the
    /// core's unknown word; it never reads as healthy.
    func test_aDownCoreWithNoHealthWordsStillDrawsABanner() throws {
        let unknown = try XCTUnwrap(TracesHealth.unknownWord)
        XCTAssertFalse(unknown.isEmpty)
        let phases: [TracesStore.Phase] = [.failed(.unreachable), .failed(.notAvailableYet(method: "x")), .loaded]
        for phase in phases {
            let banners = TracesHealth.banners(phase: phase, status: nil, words: nil, coreDown: nil)
            XCTAssertEqual(banners.map(\.title), [unknown])
            XCTAssertEqual(banners.first?.tone, .outside)
        }
    }

    func test_equalTitlesDoNotCollide() {
        let a = TracesHealth.Banner(title: "x", detail: nil, tone: .ask, index: 0)
        let b = TracesHealth.Banner(title: "x", detail: nil, tone: .ask, index: 1)
        XCTAssertNotEqual(a.id, b.id)
    }

    /// Severity per line: budget, witness and gate-held are waiting; the
    /// label line carries the core's own severity.
    func test_eachSafeguardCarriesItsSeverity() throws {
        let json = """
        {"health":{"last_error_label":"pii-filter-unavailable"},
         "daily_budget":{"blocked":true,"blocked_entries":2}}
        """
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
        let lines = TracesStore.safeguards(status)
        let label = HealthCopy.core(label: "pii-filter-unavailable", maxQueueEntries: nil)
        XCTAssertEqual(lines.first?.title, label.title)
        XCTAssertEqual(lines.first?.severity, label.severity)
        XCTAssertEqual(lines.dropFirst().map(\.severity), [.waiting])
        XCTAssertEqual(lines.dropFirst().first?.title, DailyBudgetCopy.title)
    }

    func test_theTreeDrawsTheBannersAboveItself() throws {
        let tree = try Self.text("Views/Monitor/TracesViews.swift")
        let treeView = try XCTUnwrap(tree.range(of: "struct TracesTreeView"))
        let inspector = try XCTUnwrap(tree.range(of: "struct SessionInspectorView"))
        let body = tree[treeView.lowerBound..<inspector.lowerBound]
        XCTAssertTrue(body.contains("TracesHealth.banners("))
        XCTAssertTrue(body.contains("GlassHealthBanner(banner:"))
        XCTAssertFalse(body.contains("ForEach(store.safeguards"), "safeguards are drawn through the fail-closed banners")
        let banner = try Self.text("Views/Monitor/TracesHealth.swift")
        XCTAssertFalse(banner.contains("Button("), "the Traces tab's banners carry no action")
        XCTAssertTrue(banner.contains("TCCoreCopy.healthCopyJSON(reachable: false"))
        XCTAssertTrue(body.contains("coreDown: TracesHealth.coreDownLine)"))
        let banners = try XCTUnwrap(body.range(of: "TracesHealth.banners("))
        let spinner = try XCTUnwrap(body.range(of: "ProgressView()"))
        XCTAssertLessThan(banners.lowerBound, spinner.lowerBound, "the banners precede the spinner and the tree")
    }

    func test_theCaveatAndTheCertificatesAreOnGlass() throws {
        try LegacySymbols.assertClean("Views/ScrubbingCaveat.swift")
        try LegacySymbols.assertClean("Views/CertificateSection.swift")
        XCTAssertEqual(ScrubbingCaveat.status(redactionCount: 0), .ask)
        XCTAssertEqual(ScrubbingCaveat.status(redactionCount: 3), .off)
        let caveat = try Self.text("Views/ScrubbingCaveat.swift")
        let atCommitStart = try XCTUnwrap(caveat.range(of: "struct ScrubbingCaveatAtCommit")).lowerBound
        let atCommit = String(caveat[atCommitStart...])
        for needle in ["GlassStatusLabel(ScrubbingCaveat.canonical, status: .ask)",
                       ".accessibilityLabel(\"Before you contribute. \\(ScrubbingCaveat.canonical)\")"] {
            XCTAssertTrue(atCommit.contains(needle), "ScrubbingCaveatAtCommit lacks \(needle)")
        }
        XCTAssertTrue(caveat.contains("Text(ScrubbingCaveat.canonical)"), "the note draws the canonical sentence")
        let certificates = try Self.text("Views/CertificateSection.swift")
        for needle in ["TCCertificate.listTitle(evidenceAdmitted:", "TCCertificate.rowLine(evidenceAdmitted:",
                       "certificateListEmpty", "admissionEvidenceOffered == true", "\\.holdsCertificate"] {
            XCTAssertTrue(certificates.contains(needle), "CertificateSection.swift lacks \(needle)")
        }
    }
}
#endif
