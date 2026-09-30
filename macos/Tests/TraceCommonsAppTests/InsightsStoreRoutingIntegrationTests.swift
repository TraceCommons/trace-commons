import AppKit
import Foundation
import SwiftUI
import Vision
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class InsightsStoreRoutingIntegrationTests: XCTestCase {
    /// Pinned raster scale for the rendered-copy assertions below.
    private static let renderScale = 2

    func testOCRNormalizationChangesOnlyCaseAndWhitespace() {
        XCTAssertEqual(Self.normalizedOCR("  Outcome\tIs\n unassessed.  "), "outcome is unassessed.")
        XCTAssertNotEqual(Self.normalizedOCR("Outcome is assessed."), "outcome is unassessed.")
    }

    /// The captured schema-10 store claims the exact-categorical estimator, and
    /// the frozen protocol sets `qualified_for_saved_specifications: false`.
    /// The store refuses it by name, so the shell must surface an error and an
    /// empty list rather than a simultaneous-coverage screen the user granted
    /// themselves by editing a local file.
    @MainActor
    func testActualQualifiedClaimingStoreIsRefusedByRoutedModel() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = root.appendingPathComponent("supported-store")
        try FileManager.default.createDirectory(at: store, withIntermediateDirectories: true,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent(
                "crates/trace-commons-contributor/fixtures/insights/comparison-estimator/schema2-supported-store/index.json")
        try FileManager.default.copyItem(at: fixture, to: store.appendingPathComponent("index.json"))

        let router = InsightsServiceRouter(selection: .custom(store.path))
        do {
            _ = try await router.call(.init(operation: .init("comparison_list_specs")))
            XCTFail("the store must refuse a specification claiming an unqualified estimator")
        } catch {
            XCTAssertEqual(error as? InsightsError,
                           .service("insights-comparison-estimator-not-qualified"), "\(error)")
        }

        let model = ComparisonSpecificationsModel(service: { try await router.call($0) })
        model.open(); try await settle(model)
        XCTAssertTrue(model.specifications.isEmpty)
        XCTAssertNil(model.selected)
        XCTAssertNil(model.result)
        XCTAssertEqual(model.error, "comparison_specification_error")
    }

    @MainActor
    func testActualSelectedStoreRendersLocationAndPopulatedComparisonSections() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("selected-store")
        let seeded = try seedStore(store, marker: "visible", includeComparisonData: true)
        let task = try XCTUnwrap(TCInsights.call(.init(
            storeDirectory: store.path, operation: .init("comparison_task_list"))).tasks?.first)
        XCTAssertTrue(task.stale_reasons.contains(.attributionPendingQualification))
        let copy = try XCTUnwrap(TCInsights.copy())
        _ = NSApplication.shared
        let selection = InsightsStoreSelection.custom(store.path)
        // The view's own models, held here so the capture waits for what the
        // screen shows to be loaded -- a timer raced the store reads on a
        // loaded machine.
        let router = InsightsServiceRouter(selection: selection)
        let service: InsightsModel.Service = { try await router.call($0) }
        let insights = InsightsModel(service: service)
        let tasks = ComparisonTasksModel(service: service)
        let specifications = ComparisonSpecificationsModel(service: service)
        let size = CGSize(width: 1_180, height: 3_200)
        let content = InsightsView(storeSelection: selection, storeCopy: copy, model: insights,
                                   comparisonModel: tasks, specificationModel: specifications)
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = hosting
        // Pinned light appearance. The raster otherwise follows the host's
        // system appearance, and dark-mode pixels (light text on a near-black
        // window background) are a different OCR input on a developer
        // machine than on the runner.
        window.appearance = NSAppearance(named: .aqua)
        defer { window.close() }
        // Wait for the models to be populated with this store's data, not for
        // a timer: the view's `onAppear` opens them, so layout is driven until
        // every section this test reads has its data.
        let taskID = try XCTUnwrap(seeded.taskID)
        let specificationID = try XCTUnwrap(seeded.specificationID)
        var populated = false
        for _ in 0..<1_000 {
            hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
            populated = !insights.busy && !insights.episodeBusy && !insights.loadingSummary
                && !tasks.busy && !specifications.busy
                && insights.summary != nil && insights.snapshots.map(\.id) == [seeded.snapshotID]
                && tasks.tasks.map(\.id) == [taskID]
                && specifications.specifications.map(\.id) == [specificationID]
            if populated { break }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(populated, "the Insights models never loaded the selected store: "
            + "\(insights.error ?? "-") \(tasks.error ?? "-") \(specifications.error ?? "-")")
        // The data is in the models; lay out and draw it once more before the
        // capture so the frame reflects it.
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        await Task.yield()
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        // Rasterize at a pinned 2x rather than at the host's backing scale.
        // `bitmapImageRepForCachingDisplay` follows the display, so this render
        // was 2x on a Retina developer machine and 1x on the CI runner, where
        // the recognizer read the tail of this view's copy as "unassessec".
        // The mismatch is a property of the raster, not of the view under test,
        // and a fixed scale makes the same pixels available everywhere.
        let bitmap = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: Int(size.width) * Self.renderScale,
            pixelsHigh: Int(size.height) * Self.renderScale, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        bitmap.size = size
        hosting.cacheDisplay(in: bounds, to: bitmap)
        XCTAssertEqual(bitmap.pixelsWide, Int(size.width) * Self.renderScale)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 20_000)
        if let path = ProcessInfo.processInfo.environment["TC_INSIGHTS_STORE_RENDER"] {
            try png.write(to: URL(fileURLWithPath: path))
        }
        XCTAssertNotNil(seeded.taskID); XCTAssertNotNil(seeded.specificationID)
        // The location line is asserted on the view's own string. It is a
        // caption-sized absolute temp path of ~100 characters, and OCR reads
        // it inconsistently ("nsights store:/var/tolders/..."), which says
        // nothing about whether the view named the store. What the pixels
        // must still show is that a line naming this store is at the top.
        XCTAssertEqual(InsightsView.storeLocationLine(selection, copy: copy),
                       (copy["insights_store_title"] ?? "") + ": " + store.path)
        XCTAssertEqual(copy["insights_store_title"], "Insights store")
        let topText = try recognizedText(bitmap, region: .init(x: 0, y: 0.94, width: 1, height: 0.06))
        XCTAssertTrue(topText.contains("selected-store"), topText)
        let text = try recognizedText(bitmap)
        let normalizedText = Self.normalizedOCR(text)
        XCTAssertTrue(normalizedText.contains(Self.normalizedOCR("Whole saved snapshot members")), text)
        XCTAssertTrue(normalizedText.contains(Self.normalizedOCR("Outcome is unassessed")), text)
        XCTAssertTrue(text.contains("model-a") && text.contains("model-b"), text)
    }

    @MainActor
    func testActualServiceKeepsAllModelsAndDrilldownsInSelectedStore() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let selectedStore = root.appendingPathComponent("selected")
        let otherStore = root.appendingPathComponent("other")
        let selected = try seedStore(selectedStore, marker: "selected", includeComparisonData: true)
        let other = try seedStore(otherStore, marker: "other", includeComparisonData: false)
        let directTasks = try XCTUnwrap(TCInsights.call(.init(
            storeDirectory: selectedStore.path, operation: .init("comparison_task_list"))).tasks)
        XCTAssertEqual(directTasks.count, 1)
        try directTasks.forEach { try $0.validateSupportedSchema() }
        let directSpecifications = try XCTUnwrap(TCInsights.call(.init(
            storeDirectory: selectedStore.path, operation: .init("comparison_list_specs"))).specifications)
        XCTAssertEqual(directSpecifications.count, 1)
        try directSpecifications.forEach { try $0.validateStructure() }

        let router = InsightsServiceRouter(selection: .custom(selectedStore.path))
        let service: InsightsModel.Service = { try await router.call($0) }
        let insights = InsightsModel(service: service)
        let tasks = ComparisonTasksModel(service: service)
        let specifications = ComparisonSpecificationsModel(service: service)
        insights.open(); tasks.open(); specifications.open()
        try await settle(insights, tasks, specifications)

        XCTAssertEqual(insights.snapshots.map(\.id), [selected.snapshotID])
        XCTAssertEqual(insights.episodes.map(\.id), [try XCTUnwrap(selected.episodeID)])
        XCTAssertEqual(tasks.tasks.map(\.id), [try XCTUnwrap(selected.taskID)], tasks.error ?? "no task error")
        XCTAssertEqual(specifications.specifications.map(\.id), [try XCTUnwrap(selected.specificationID)],
                       specifications.error ?? "no specification error")
        insights.explain(selected.snapshotID)
        try await settle(insights, tasks, specifications)
        XCTAssertEqual(insights.selected?.id, selected.snapshotID, insights.error ?? "no Insights error")
        insights.openEpisode(try XCTUnwrap(selected.episodeID))
        try await settle(insights, tasks, specifications)
        XCTAssertEqual(insights.episodeDetail?.episode.id, selected.episodeID,
                       insights.episodeError ?? "no episode error")
        tasks.select(try XCTUnwrap(selected.taskID))
        try await settle(insights, tasks, specifications)
        XCTAssertEqual(tasks.detail?.id, selected.taskID, tasks.error ?? "no task error")
        specifications.select(try XCTUnwrap(selected.specificationID))
        try await settle(insights, tasks, specifications)
        XCTAssertEqual(specifications.selected?.id, selected.specificationID,
                       specifications.error ?? "no specification error")

        let otherList = try TCInsights.call(.init(storeDirectory: otherStore.path, operation: .init("list")))
        XCTAssertEqual(try XCTUnwrap(otherList.insights).map(\.id), [other.snapshotID])
        XCTAssertNotEqual(other.snapshotID, selected.snapshotID)
        insights.close(); tasks.close(); specifications.close()
    }

    func testActualReadOnlyCallsLeaveEmptySelectedStoreUntouched() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let emptyStore = root.appendingPathComponent("empty")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let router = InsightsServiceRouter(selection: .custom(emptyStore.path))
        for type in ["list", "episode_list", "comparison_task_list", "comparison_list_specs"] {
            _ = try await router.call(.init(operation: .init(type)))
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: emptyStore.path))
    }

    @MainActor
    private func settle(_ insights: InsightsModel, _ tasks: ComparisonTasksModel,
                        _ specifications: ComparisonSpecificationsModel) async throws {
        for _ in 0..<500 {
            if !insights.busy, !insights.episodeBusy, !tasks.busy, !specifications.busy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("routed Insights models did not settle")
    }

    @MainActor
    private func settle(_ model: ComparisonSpecificationsModel) async throws {
        for _ in 0..<500 {
            if !model.busy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("comparison specification model did not settle")
    }

    private func seedStore(_ store: URL, marker: String, includeComparisonData: Bool) throws -> SeededStore {
        let source = store.deletingLastPathComponent().appendingPathComponent("\(marker).json")
        let fixture = "[{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"\(marker)\"}," +
            "{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"\(marker)\"}]"
        try Data(fixture.utf8).write(to: source)
        let analyzed = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("analyze", source: "trajectory", file: source.path, save: true)))
        let snapshotID = try XCTUnwrap(analyzed.insight?.id)
        guard includeComparisonData else { return .init(snapshotID: snapshotID) }
        let episodeResponse = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("episode_create", snapshotIDs: [snapshotID])))
        let episodeID = try XCTUnwrap(episodeResponse.episode?.id)
        let taskResponse = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("comparison_task_create", episodeIDs: [episodeID])))
        let taskID = try XCTUnwrap(taskResponse.task?.id)
        let input = ComparisonSpecificationDraftInput(
            evidenceCutoff: "2026-09-12T00:00:00Z", cohortLabels: ["model-a", "model-b"],
            dateStart: "2026-09-01", dateEnd: "2026-09-11",
            stratum: .init(projectID: "30c18c96-6093-49f5-bb6f-6092ef0630b9", language: "swift",
                           configurationFingerprint: String(repeating: "a", count: 64)))
        let specResponse = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("comparison_save_spec", input: input)))
        return .init(snapshotID: snapshotID, episodeID: episodeID, taskID: taskID,
                     specificationID: try XCTUnwrap(specResponse.specification?.id))
    }

    /// OCR of `region` (normalized, bottom-left origin), read in bands.
    ///
    /// One request over the whole 2360x6400 raster returned a single line on
    /// macOS 26/27 -- the recognizer scales its input to a fixed working size,
    /// and at this aspect ratio caption text falls below what it can read. So
    /// the region is read as overlapping horizontal bands no taller than a
    /// screenful; the overlap means a line cut by one band boundary is whole in
    /// the next band.
    private func recognizedText(_ bitmap: NSBitmapImageRep, region: CGRect = .init(x: 0, y: 0, width: 1, height: 1))
        throws -> String {
        let band = 0.08, overlap = 0.02
        var readings: [String] = []
        var bottom = region.minY
        repeat {
            let top = min(bottom + band, region.maxY)
            readings.append(try recognizedBand(
                bitmap, region: CGRect(x: region.minX, y: bottom, width: region.width, height: top - bottom)))
            if top >= region.maxY { break }
            bottom = top - overlap
        } while true
        return readings.joined(separator: "\n")
    }

    private func recognizedBand(_ bitmap: NSBitmapImageRep, region: CGRect) throws -> String {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        request.regionOfInterest = region
        let handler = VNImageRequestHandler(cgImage: try XCTUnwrap(bitmap.cgImage), options: [:])
        try handler.perform([request])
        // Vision ranks several readings per observation. One misread character
        // in the top reading -- the CI runner read this view's "unassessed" as
        // "unassessec" -- must not decide whether the copy rendered, so the
        // alternates are searched too. Competing readings of one line are
        // separated by a non-whitespace marker, which normalization keeps, so
        // no asserted phrase can match across two of them.
        return (request.results ?? [])
            .map { $0.topCandidates(3).map(\.string).joined(separator: " | ") }
            .joined(separator: "\n")
    }

    private static func normalizedOCR(_ text: String) -> String {
        text.split(whereSeparator: \.isWhitespace).joined(separator: " ").lowercased()
    }
}

private struct SeededStore {
    let snapshotID: String
    var episodeID: String?
    var taskID: String?
    var specificationID: String?
}
