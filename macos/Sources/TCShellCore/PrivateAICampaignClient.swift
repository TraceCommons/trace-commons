import Foundation

/// Anonymous, bounded reads of campaign JSON and optional sibling artwork.
/// Each request owns an ephemeral session and cancels it on every exit path.
public struct PrivateAICampaignClient: Sendable {
    public enum Failure: Error, Equatable, Sendable {
        case invalidEndpoint, invalidResponse, tooLarge, invalidCampaign, invalidImage
    }

    static let manifestLimit = 16 * 1024
    static let imageLimit = 2 * 1024 * 1024

    struct Response: Sendable {
        let data: Data
        let response: HTTPURLResponse
    }

    private let load: @Sendable (URLRequest, Int) async throws -> Response

    public init() { load = Self.download }

    /// Injectable transport for policy tests; production always uses the
    /// ephemeral, redirect-refusing bounded streaming transport below.
    init(load: @escaping @Sendable (URLRequest, Int) async throws -> Response) { self.load = load }

    public func fetch(from endpoint: URL) async throws -> PrivateAICampaign {
        guard PrivateAICampaign.secureURL(endpoint) else { throw Failure.invalidEndpoint }
        let request = Self.request(endpoint, accept: "application/json")
        try Task.checkCancellation()
        let result = try await load(request, Self.manifestLimit)
        try Task.checkCancellation()
        try Self.validate(result.response, request: request, limit: Self.manifestLimit)
        guard result.data.count <= Self.manifestLimit else { throw Failure.tooLarge }
        guard let campaign = PrivateAICampaign.decode(from: result.data, endpoint: endpoint) else {
            throw Failure.invalidCampaign
        }
        return campaign
    }

    public func fetchImage(for campaign: PrivateAICampaign, from endpoint: URL) async throws -> Data? {
        try Task.checkCancellation()
        guard let image = campaign.imageURL else { return nil }
        guard PrivateAICampaign.imageAllowed(image, endpoint: endpoint) else { throw Failure.invalidImage }
        let request = Self.request(image, accept: "image/png, image/jpeg")
        let result = try await load(request, Self.imageLimit)
        try Task.checkCancellation()
        try Self.validate(result.response, request: request, limit: Self.imageLimit)
        guard result.data.count <= Self.imageLimit else { throw Failure.tooLarge }
        let mime = result.response.mimeType?.lowercased()
        let png = result.data.starts(with: [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
        let jpeg = result.data.starts(with: [0xff, 0xd8, 0xff])
        guard (mime == "image/png" && image.pathExtension.lowercased() == "png" && png)
            || (mime == "image/jpeg" && ["jpg", "jpeg"].contains(image.pathExtension.lowercased()) && jpeg)
        else { throw Failure.invalidImage }
        return result.data
    }

    private static func request(_ url: URL, accept: String) -> URLRequest {
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 5)
        request.httpShouldHandleCookies = false
        request.setValue(accept, forHTTPHeaderField: "Accept")
        return request
    }

    static func configuration() -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        configuration.urlCache = nil
        configuration.requestCachePolicy = .reloadIgnoringLocalCacheData
        configuration.timeoutIntervalForRequest = 5
        configuration.timeoutIntervalForResource = 5
        return configuration
    }

    private static func validate(_ response: HTTPURLResponse, request: URLRequest, limit: Int) throws {
        guard response.statusCode == 200, response.url == request.url else { throw Failure.invalidResponse }
        guard response.expectedContentLength <= Int64(limit) else { throw Failure.tooLarge }
        let mime = response.mimeType?.lowercased() ?? ""
        if request.value(forHTTPHeaderField: "Accept") == "application/json" {
            guard mime == "application/json" || (mime.hasPrefix("application/") && mime.hasSuffix("+json")) else {
                throw Failure.invalidResponse
            }
        } else {
            guard mime == "image/png" || mime == "image/jpeg" else { throw Failure.invalidImage }
        }
    }

    private static func download(_ request: URLRequest, limit: Int) async throws -> Response {
        let delegate = PrivateAICampaignSessionDelegate()
        let session = URLSession(configuration: configuration(), delegate: delegate, delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        return try await withTaskCancellationHandler {
            let (bytes, response) = try await session.bytes(for: request, delegate: delegate)
            guard let response = response as? HTTPURLResponse else { throw Failure.invalidResponse }
            try validate(response, request: request, limit: limit)
            return try await Response(data: read(bytes, limit: limit), response: response)
        } onCancel: {
            session.invalidateAndCancel()
        }
    }

    /// Stop at limit+1 rather than buffering the server's entire body, even
    /// when Content-Length is absent or false. Both response types use it.
    static func read<Bytes: AsyncSequence>(_ bytes: Bytes, limit: Int) async throws -> Data where Bytes.Element == UInt8 {
        try Task.checkCancellation()
        var data = Data()
        data.reserveCapacity(min(limit, manifestLimit))
        for try await byte in bytes {
            if data.count % 1024 == 0 { try Task.checkCancellation() }
            guard data.count < limit else { throw Failure.tooLarge }
            data.append(byte)
        }
        try Task.checkCancellation()
        return data
    }
}

/// No mutable state or credentials cross the session delegate boundary.
final class PrivateAICampaignSessionDelegate: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest
    ) async -> URLRequest? { nil }

    func urlSession(
        _ session: URLSession, task: URLSessionTask, didReceive challenge: URLAuthenticationChallenge
    ) async -> (URLSession.AuthChallengeDisposition, URLCredential?) {
        challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust
            ? (.performDefaultHandling, nil) : (.cancelAuthenticationChallenge, nil)
    }
}
