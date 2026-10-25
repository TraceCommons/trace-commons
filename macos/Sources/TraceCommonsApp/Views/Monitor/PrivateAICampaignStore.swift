import AppKit
import Foundation
import ImageIO
import Observation
import TCBridge
import TCShellCore

/// A native promotion whose compiled copy is available before any network read.
/// Fetch failures remain the compiled campaign; artwork failures use native art.
@MainActor
@Observable
final class PrivateAICampaignStore {
    typealias ManifestLoader = @Sendable (URL) async throws -> PrivateAICampaign
    typealias ImageLoader = @Sendable (PrivateAICampaign, URL) async throws -> Data?

    private(set) var campaign: PrivateAICampaign?
    private(set) var image: NSImage?
    private(set) var imageData: Data?

    @ObservationIgnored private let fallback: PrivateAICampaign?
    @ObservationIgnored private let endpoint: URL?
    @ObservationIgnored private let fetch: ManifestLoader
    @ObservationIgnored private let fetchImage: ImageLoader
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var requestTask: Task<Void, Never>?

    convenience init() {
        let client = PrivateAICampaignClient()
        self.init(
            fallbackJSON: TCCoreCopy.privateAICampaignJSON(),
            endpoint: (Bundle.main.object(forInfoDictionaryKey: "TCPrivateAICampaignURL") as? String)
                .flatMap(URL.init(string:)),
            fetch: { try await client.fetch(from: $0) },
            fetchImage: { try await client.fetchImage(for: $0, from: $1) })
    }

    init(fallbackJSON: String?, endpoint: URL?, fetch: @escaping ManifestLoader,
         fetchImage: @escaping ImageLoader) {
        fallback = fallbackJSON.flatMap { PrivateAICampaign.decode(from: Data($0.utf8)) }
        campaign = fallback
        self.endpoint = endpoint
        self.fetch = fetch
        self.fetchImage = fetchImage
    }

    /// A newer refresh cancels its predecessor. Generation checks also protect
    /// against loaders that return a response after cancellation.
    func refresh() async {
        generation += 1
        let mine = generation
        requestTask?.cancel()
        campaign = fallback
        image = nil
        imageData = nil
        guard let endpoint else {
            requestTask = nil
            return
        }
        let request = Task<Void, Never> { [weak self] in
            guard let self else { return }
            await self.load(from: endpoint, generation: mine)
        }
        requestTask = request
        await withTaskCancellationHandler {
            await request.value
        } onCancel: {
            request.cancel()
        }
        if mine == generation { requestTask = nil }
    }

    private func load(from endpoint: URL, generation mine: Int) async {
        do {
            let remote = try await fetch(endpoint)
            guard mine == generation, !Task.isCancelled else { return }
            campaign = remote
            guard remote.imageURL != nil else { return }
            // A failed image never replaces the valid campaign or adds a notice.
            let data = try? await fetchImage(remote, endpoint)
            guard mine == generation, !Task.isCancelled,
                  let data, let decoded = Self.decodeImage(data) else { return }
            imageData = data
            image = decoded
        } catch {
            guard mine == generation, !Task.isCancelled else { return }
            campaign = fallback
            image = nil
            imageData = nil
        }
    }

    /// Inspect pixel dimensions before AppKit decodes the optional artwork.
    /// The client separately bounds network bytes and permits only PNG/JPEG.
    private static func decodeImage(_ data: Data) -> NSImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? NSNumber,
              let height = properties[kCGImagePropertyPixelHeight] as? NSNumber,
              (1...4096).contains(width.intValue), (1...4096).contains(height.intValue) else { return nil }
        return NSImage(data: data)
    }
}
