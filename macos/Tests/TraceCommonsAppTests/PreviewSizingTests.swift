import AppKit
import SwiftUI
import XCTest
@testable import TraceCommonsApp

final class PreviewSizingTests: XCTestCase {
    @MainActor
    func testPreviewRendersAtMinimumAndExpandedSize() throws {
        _ = NSApplication.shared
        let entry = QueueEntry(entryID: "synthetic", sessionHash: "synthetic", source: "claude_code",
            declaredSource: nil, projectID: "synthetic", projectLabel: "Synthetic project", projectPath: "~/synthetic",
            sessionPath: nil, sizeBytes: 100, discoveredAt: Date(timeIntervalSince1970: 0), state: .pending,
            reasonLabel: nil, attempts: 0, subagentCount: nil, subagentsDropped: nil,
            // No eligibility field: this fixture stands for an invited
            // contributor, whose sheet is the sheet it always was.
            eligibility: nil, eligibilityReason: nil,
            // The same invited contributor still gets a mark. Absent
            // eligibility and present attestation is the ordinary shape
            // of an invited contributor's row, not a special case.
            attestation: "attested", attestationReason: nil, holdsCertificateRaw: nil)
        let summary = PreviewSummary(wouldSendBytes: 80, rawSessionBytes: 100, eventCount: 2,
            openingPrompt: "Review a synthetic session", redactions: [:], redactionsDistinct: [:],
            piiLabelsPresent: [], consentScopes: [], residualRisk: "low")
        let model = AppModel() // No daemon or enrollment starts here.
        let view = PreviewSheet(entry: entry, preloaded: .init(summary: summary,
            transcript: "Synthetic transcript", needle: "", offsets: [])).environmentObject(model)
        for size in [CGSize(width: 760, height: 620), CGSize(width: 1080, height: 820)] {
            let hosting = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
            let bounds = NSRect(origin: .zero, size: size)
            hosting.frame = bounds
            hosting.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
            hosting.cacheDisplay(in: bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(png.count, 1000)
            if let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] {
                try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent("preview-\(Int(size.width)).png"))
            }
        }
    }
}
