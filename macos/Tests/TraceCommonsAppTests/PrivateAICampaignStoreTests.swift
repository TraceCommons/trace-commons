import AppKit
import Foundation
import TCShellCore
import XCTest
@testable import TraceCommonsApp

@MainActor
final class PrivateAICampaignStoreTests: XCTestCase {
    private static let endpoint = URL(string: "https://storage.googleapis.com/test-channel/private-ai-campaign.json")!
    private static let fallbackJSON = #"{"schema_version":1,"id":"compiled","title":"Compiled campaign","body":"Provider-hosted private inference.","action_title":"Learn more","action_url":"https://near.ai/kimi","symbol":"cpu"}"#

    private func campaign(_ id: String, image: Bool = false) throws -> PrivateAICampaign {
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(Self.fallbackJSON.utf8)) as? [String: Any])
        object["id"] = id
        if image { object["image_url"] = Self.endpoint.deletingLastPathComponent().appendingPathComponent("campaign.png").absoluteString }
        return try XCTUnwrap(PrivateAICampaign.decode(
            from: JSONSerialization.data(withJSONObject: object), endpoint: Self.endpoint))
    }

    private func store(fetch: @escaping PrivateAICampaignStore.ManifestLoader,
                       image: @escaping PrivateAICampaignStore.ImageLoader = { _, _ in nil }) -> PrivateAICampaignStore {
        PrivateAICampaignStore(fallbackJSON: Self.fallbackJSON, endpoint: Self.endpoint,
                               fetch: fetch, fetchImage: image)
    }

    func testCompiledFallbackIsPresentBeforeRemoteReadAndSurvivesFailure() async {
        let store = store { _ in throw URLError(.notConnectedToInternet) }
        XCTAssertEqual(store.campaign?.id, "compiled")
        XCTAssertNil(store.image)
        await store.refresh()
        XCTAssertEqual(store.campaign?.id, "compiled")
        XCTAssertNil(store.imageData)
    }

    func testFailedRefreshReplacesPreviousRemoteContentWithCompiledFallback() async throws {
        let remote = try campaign("remote", image: true)
        let gate = Deferred<PrivateAICampaign>()
        let art = try png(width: 1, height: 1)
        let store = store(fetch: { _ in try await gate.load() }, image: { _, _ in art })
        let first = Task { await store.refresh() }
        await gate.waitForRequests(1)
        await gate.finish(1, with: remote)
        await first.value
        XCTAssertEqual(store.campaign?.id, "remote")
        XCTAssertNotNil(store.image)
        let second = Task { await store.refresh() }
        await gate.waitForRequests(2)
        await gate.fail(2)
        await second.value
        XCTAssertEqual(store.campaign?.id, "compiled")
        XCTAssertNil(store.image)
        XCTAssertNil(store.imageData)
    }

    func testMissingEndpointKeepsFallbackWithoutFetching() async {
        let store = PrivateAICampaignStore(fallbackJSON: Self.fallbackJSON, endpoint: nil,
            fetch: { _ in XCTFail("missing endpoint must not fetch"); throw URLError(.badURL) },
            fetchImage: { _, _ in XCTFail("missing endpoint must not fetch art"); return nil })
        await store.refresh()
        XCTAssertEqual(store.campaign?.id, "compiled")
    }

    func testRemoteCampaignUsesItsValidatedActionAndNativeArtWhenImageFails() async throws {
        let remote = try campaign("remote", image: true)
        let store = store(fetch: { _ in remote }, image: { _, _ in throw URLError(.cannotDecodeContentData) })
        await store.refresh()
        XCTAssertEqual(store.campaign, remote)
        XCTAssertEqual(store.campaign?.actionURL, URL(string: "https://near.ai/kimi"))
        XCTAssertNil(store.image)
        XCTAssertNil(store.imageData)
    }

    func testNewerManifestCannotBeOverwrittenByCancelledPredecessor() async throws {
        let gate = Deferred<PrivateAICampaign>()
        let store = store { _ in try await gate.load() }
        let first = Task { await store.refresh() }
        await gate.waitForRequests(1)
        let second = Task { await store.refresh() }
        await gate.waitForRequests(2)
        await gate.finish(2, with: try campaign("new"))
        await second.value
        await gate.finish(1, with: try campaign("old"))
        await first.value
        XCTAssertEqual(store.campaign?.id, "new")
    }

    func testOldImageCannotReplaceNewCampaignArtwork() async throws {
        let manifests = Deferred<PrivateAICampaign>()
        let images = Deferred<Data?>()
        let store = store(fetch: { _ in try await manifests.load() }, image: { _, _ in try await images.load() })
        let first = Task { await store.refresh() }
        await manifests.waitForRequests(1)
        await manifests.finish(1, with: try campaign("old", image: true))
        await images.waitForRequests(1)
        let second = Task { await store.refresh() }
        await manifests.waitForRequests(2)
        await manifests.finish(2, with: try campaign("new"))
        await second.value
        await images.finish(1, with: try png(width: 1, height: 1))
        await first.value
        XCTAssertEqual(store.campaign?.id, "new")
        XCTAssertNil(store.image)
        XCTAssertNil(store.imageData)
    }

    func testCancelledRefreshCannotPublishLateCampaign() async throws {
        let gate = Deferred<PrivateAICampaign>()
        let store = store { _ in try await gate.load() }
        let refresh = Task { await store.refresh() }
        await gate.waitForRequests(1)
        refresh.cancel()
        await gate.finish(1, with: try campaign("late"))
        await refresh.value
        XCTAssertEqual(store.campaign?.id, "compiled")
    }

    func testImageMustDecodeWithinPixelBounds() async throws {
        let remote = try campaign("remote", image: true)
        let good = try png(width: 1, height: 1)
        let oversized = try png(width: 4097, height: 1)
        for data in [Data("invalid image".utf8), oversized, good] {
            let store = store(fetch: { _ in remote }, image: { _, _ in data })
            await store.refresh()
            XCTAssertEqual(store.campaign?.id, "remote")
            XCTAssertEqual(store.image != nil, data == good)
            XCTAssertEqual(store.imageData != nil, data == good)
        }
    }

    private func png(width: Int, height: Int) throws -> Data {
        let bitmap = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
            bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
            colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        bitmap.bitmapData?.initialize(repeating: 0, count: bitmap.bytesPerRow * bitmap.pixelsHigh)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }
}

/// Ignores cancellation deliberately, proving late responses cannot publish.
private actor Deferred<Value: Sendable> {
    private var count = 0
    private var replies: [Int: CheckedContinuation<Value, any Error>] = [:]
    private var waiters: [(Int, CheckedContinuation<Void, Never>)] = []

    func load() async throws -> Value {
        count += 1
        let index = count
        return try await withCheckedThrowingContinuation { continuation in
            replies[index] = continuation
            let ready = waiters.filter { $0.0 <= count }
            waiters.removeAll { $0.0 <= count }
            ready.forEach { $0.1.resume() }
        }
    }

    func waitForRequests(_ expected: Int) async {
        guard count < expected else { return }
        await withCheckedContinuation { waiters.append((expected, $0)) }
    }

    func fail(_ index: Int) {
        replies.removeValue(forKey: index)?.resume(throwing: URLError(.badServerResponse))
    }

    func finish(_ index: Int, with value: Value) {
        replies.removeValue(forKey: index)?.resume(returning: value)
    }
}
